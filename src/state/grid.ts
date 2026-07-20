import { create } from 'zustand'

import { ipc, toAppError } from '../lib/ipc'
import { clampColumnWidth, emptyLayout, type TableLayout } from '../lib/columnLayout'
import type { AppError, PageResult, SortDir, SortKey } from '../lib/types'

/**
 * Storage key for a table's column layout.
 *
 * @param schema - `string` — the schema name, unquoted.
 * @param table - `string` — the table name, unquoted.
 * @returns `string` — the two joined as `schema.table`, the key layouts are
 *   stored under.
 */
export function layoutKey(schema: string, table: string): string {
  return `${schema}.${table}`
}

interface GridStore {
  result: PageResult | null
  loading: boolean
  error: AppError | null

  page: number
  /** Sort terms, most significant first. */
  sort: SortKey[]
  /** Applied filter (what's actually in the query). */
  filter: string
  /** What's typed in the box, which may differ until Enter. */
  filterDraft: string
  selectedRow: number | null
  /** The cell opened inline, if any. */
  expanded: { row: number; column: string } | null

  /** Column widths and order for the table currently shown. */
  layout: TableLayout
  loadLayout: (schema: string, table: string) => Promise<void>
  setColumnWidth: (schema: string, table: string, column: string, px: number) => void
  /** Drop one column's saved width so it falls back to the type default. */
  clearColumnWidth: (schema: string, table: string, column: string) => void
  setColumnOrder: (schema: string, table: string, order: string[]) => void
  resetLayout: (schema: string, table: string) => void

  load: (schema: string, table: string) => Promise<void>
  setPage: (schema: string, table: string, page: number) => Promise<void>
  /**
   * Toggle a column's sort. `append` (shift-click) adds it as a lower-priority
   * term instead of replacing the whole sort.
   */
  toggleSort: (schema: string, table: string, col: string, append?: boolean) => Promise<void>
  applyFilter: (schema: string, table: string, filter: string) => Promise<void>
  setFilterDraft: (v: string) => void
  /**
   * Add a predicate to the current filter, ANDing with whatever is there so
   * filter-to-value narrows rather than replaces.
   */
  addFilter: (schema: string, table: string, predicate: string) => Promise<void>
  selectRow: (i: number | null) => void
  /** Open a cell inline; passing the already-open cell closes it. */
  toggleExpanded: (row: number, column: string) => void
  collapse: () => void
  /** Clear per-table state when the selection changes. */
  resetFor: () => void
}

export const useGrid = create<GridStore>((set, get) => ({
  result: null,
  loading: false,
  error: null,
  page: 0,
  sort: [],
  filter: '',
  filterDraft: '',
  selectedRow: null,
  expanded: null,
  layout: emptyLayout(),

  load: async (schema, table) => {
    const { page, sort, filter } = get()
    set({ loading: true })
    try {
      const result = await ipc.fetchPage({
        schema,
        table,
        page,
        sort,
        filter: filter || null,
      })
      set({ result, loading: false, error: null })
    } catch (e) {
      // Keep the last good rows on screen; the footer shows the error.
      set({ loading: false, error: toAppError(e) })
    }
  },

  setPage: async (schema, table, page) => {
    set({ page: Math.max(0, page), selectedRow: null, expanded: null })
    await get().load(schema, table)
  },

  toggleSort: async (schema, table, col, append = false) => {
    const { sort } = get()
    const existing = sort.find((k) => k.column === col)

    let next: SortKey[]
    if (!append) {
      // Plain click: sort by this column alone, flipping if it already led.
      const dir: SortDir = existing?.dir === 'asc' ? 'desc' : 'asc'
      next = [{ column: col, dir }]
    } else if (!existing) {
      next = [...sort, { column: col, dir: 'asc' }]
    } else if (existing.dir === 'asc') {
      next = sort.map((k) => (k.column === col ? { ...k, dir: 'desc' as SortDir } : k))
    } else {
      // asc -> desc -> removed, so shift-clicking can undo itself.
      next = sort.filter((k) => k.column !== col)
    }

    set({ sort: next, page: 0, selectedRow: null, expanded: null })
    await get().load(schema, table)
  },

  applyFilter: async (schema, table, filter) => {
    set({ filter, filterDraft: filter, page: 0, selectedRow: null, expanded: null })
    await get().load(schema, table)
  },

  setFilterDraft: (filterDraft) => set({ filterDraft }),

  addFilter: async (schema, table, predicate) => {
    const current = get().filter.trim()
    // Parenthesise the existing filter so an OR inside it can't change meaning
    // when a new term is ANDed on.
    const next = current ? `(${current}) AND ${predicate}` : predicate
    await get().applyFilter(schema, table, next)
  },
  selectRow: (selectedRow) => set({ selectedRow }),

  toggleExpanded: (row, column) =>
    set((s) => ({
      expanded:
        s.expanded && s.expanded.row === row && s.expanded.column === column
          ? null
          : { row, column },
    })),

  collapse: () => set({ expanded: null }),

  loadLayout: async (schema, table) => {
    try {
      const all = await ipc.gridLayoutLoad()
      set({ layout: all[layoutKey(schema, table)] ?? emptyLayout() })
    } catch {
      // A layout we cannot read is not a reason to fail to show data.
      set({ layout: emptyLayout() })
    }
  },

  setColumnWidth: (schema, table, column, px) => {
    const layout = {
      ...get().layout,
      widths: { ...get().layout.widths, [column]: clampColumnWidth(px) },
    }
    set({ layout })
    persist(schema, table, layout)
  },

  clearColumnWidth: (schema, table, column) => {
    const widths = { ...get().layout.widths }
    delete widths[column]
    const layout = { ...get().layout, widths }
    set({ layout })
    persist(schema, table, layout)
  },

  setColumnOrder: (schema, table, order) => {
    const layout = { ...get().layout, order }
    set({ layout })
    persist(schema, table, layout)
  },

  resetLayout: (schema, table) => {
    const layout = emptyLayout()
    set({ layout })
    persist(schema, table, layout)
  },

  resetFor: () =>
    set({
      page: 0,
      sort: [],
      filter: '',
      filterDraft: '',
      selectedRow: null,
      expanded: null,
      result: null,
      error: null,
      // Not cleared to `emptyLayout()` — `loadLayout` replaces it, and blanking
      // it here would flash the default widths on every table change.
    }),
}))

/**
 * Write a table's layout, sending the whole thing every time.
 *
 * The backend replaces a table's entry wholesale rather than merging per
 * column, so a partial save would silently drop the other columns' widths.
 * Failures are swallowed: losing a column width is not worth an error banner
 * over the data the user actually came for.
 *
 * @param schema - `string` — the schema name, unquoted.
 * @param table - `string` — the table name, unquoted.
 * @param layout - `TableLayout` — the complete layout (all column widths plus
 *   the order), not just the part that changed.
 * @returns `void` — returns immediately; the write is fire-and-forget and its
 *   completion is never observed, so returning does not mean it landed.
 */
function persist(schema: string, table: string, layout: TableLayout) {
  ipc.gridLayoutSave(layoutKey(schema, table), layout).catch(() => {})
}

/**
 * Last page index, or null when the total is unknown (count timed out).
 *
 * @param result - `PageResult | null` — the page currently loaded, carrying
 *   `total` and `pageSize`; `total` is `null` when the COUNT did not finish.
 * @returns `number | null` — the zero-based index of the final page, `0` for
 *   an empty table, or `null` when the row count is unknown.
 */
export function lastPageIndex(result: PageResult | null): number | null {
  if (!result || result.total === null) return null
  if (result.total === 0) return 0
  return Math.floor((result.total - 1) / result.pageSize)
}

/**
 * Whether the Next-page control is live. `page` is zero-based.
 *
 * Deliberately optimistic when the row count is unknown: a full page is treated
 * as "there may be more", so a slow COUNT never strands the user on what is
 * actually the middle of a table.
 *
 * @param result - `PageResult | null` — the page currently loaded; `null`
 *   before any load, which disables the control.
 * @param page - `number` — the zero-based index of the page on screen.
 * @returns `boolean` — true when a next page may exist. With an unknown total
 *   this is a guess from a full page, so it can be true with nothing beyond.
 */
export function canGoForward(result: PageResult | null, page: number): boolean {
  if (!result) return false
  // A full page implies there may be more, even without a reliable total.
  const last = lastPageIndex(result)
  if (last === null) return result.rows.length === result.pageSize
  return page < last
}
