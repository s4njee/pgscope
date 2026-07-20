import { useEffect, useMemo, useState } from 'react'

import { CellExpansion } from './CellExpansion'
import { ContextMenu, type MenuItem } from './ContextMenu'
import { GridHeader } from './GridHeader'
import { buildGridMenu } from '../lib/gridMenu'
import { orderColumns, templateFor, type OrderedColumn } from '../lib/columnLayout'
import { cellColor, shouldTruncate } from '../lib/cellColor'
import { formatMs, groupDigits, rowRange } from '../lib/format'
import type { ColumnMeta } from '../lib/types'
import { useExplorer } from '../state/explorer'
import { canGoForward, lastPageIndex, useGrid } from '../state/grid'

/**
 * The grid's rows, plus any inline cell expansion opened under one.
 *
 * Takes both column lists: `ordered` is what to draw and in what order, while
 * `columns` stays in the canonical order the row values are matched to — PK
 * lookups and the expansion's column count must use that one.
 *
 * @param props - `{ columns: ColumnMeta[]; ordered: OrderedColumn[]; template: string }`
 *   - `columns` — canonical column order, matching the positions of the row values.
 *   - `ordered` — the same columns in display order, each carrying its canonical index.
 *   - `template` — the CSS `grid-template-columns` value shared with the header.
 * @returns `JSX.Element | null` — the row fragment, or `null` when there is no page
 *   loaded or no table selected.
 */
function GridRows({
  columns,
  ordered,
  template,
}: {
  columns: ColumnMeta[]
  ordered: OrderedColumn[]
  template: string
}) {
  const { result, selectedRow, selectRow, expanded, toggleExpanded, collapse, addFilter } =
    useGrid()
  const selected = useExplorer((s) => s.selected)
  const { page, sort, filter } = useGrid()
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  if (!result || !selected) return null

  const offset = result.page * result.pageSize
  const pkColumns = columns.filter((c) => c.isPk)

  /** PK values for a row, so the expansion can find it again exactly. */
  const pkFor = (row: (string | null)[]) =>
    pkColumns
      .map((c) => ({ column: c.name, value: row[columns.indexOf(c)] }))
      .filter((p): p is { column: string; value: string } => p.value !== null)

  const pageRequest = {
    schema: selected.schema,
    table: selected.table,
    page,
    sort,
    filter: filter || null,
  }

  const openMenu = (e: React.MouseEvent, rowIndex: number, columnIndex: number) => {
    e.preventDefault()
    const built = buildGridMenu(
      {
        schema: selected.schema,
        table: selected.table,
        columns,
        values: result.rows[rowIndex],
        columnIndex,
        rowIndex,
      },
      {
        copy: async (text) => {
          try {
            await navigator.clipboard.writeText(text)
          } catch {
            /* clipboard unavailable */
          }
        },
        applyFilter: (predicate) =>
          void addFilter(selected.schema, selected.table, predicate),
        expandCell: (r, col) => toggleExpanded(r, col),
        canExpand: (col, value) => value !== null && shouldTruncate(col),
      },
    )
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: built.map((b) => ({
        label: b.label,
        hint: b.hint,
        disabled: b.disabled,
        separatorBefore: b.separatorBefore,
        onSelect: b.run,
      })),
    })
  }

  return (
    <>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />
      )}
      {result.rows.map((row, i) => {
        const openHere = expanded?.row === i
        return (
          <div key={offset + i} style={{ display: 'contents' }}>
            <div
              className={`grid-row${selectedRow === i ? ' grid-row--selected' : ''}${
                openHere ? ' grid-row--expanded' : ''
              }`}
              style={{ gridTemplateColumns: template }}
              onClick={() => selectRow(selectedRow === i ? null : i)}
            >
              <div className="grid-cell grid-cell--num">{offset + i + 1}</div>
              {/* Display order, but every row lookup uses the canonical index
                  the column carries — reading row[displayIndex] would show the
                  wrong data under the right header once a column is moved. */}
              {ordered.map(({ col, index: ci }) => {
                const value = row[ci] ?? null
                // Only cells that are cut off (or json, which arrives minified)
                // are worth opening, so the affordance stays out of the way
                // everywhere else.
                const expandable = value !== null && shouldTruncate(col)
                const isOpen = openHere && expanded?.column === col.name
                return (
                  <div
                    key={col.name}
                    className={`grid-cell${expandable ? ' grid-cell--expandable' : ''}${
                      isOpen ? ' grid-cell--open' : ''
                    }`}
                    style={{ color: cellColor(col, value) }}
                    title={value ?? 'NULL'}
                    onDoubleClick={(e) => {
                      if (!expandable) return
                      e.stopPropagation()
                      toggleExpanded(i, col.name)
                    }}
                    onContextMenu={(e) => openMenu(e, i, ci)}
                  >
                    {value === null ? 'NULL' : value}
                    {expandable && (
                      <span
                        className="grid-cell__expand"
                        title="Expand this value"
                        onClick={(e) => {
                          e.stopPropagation()
                          toggleExpanded(i, col.name)
                        }}
                      >
                        {isOpen ? '▴' : '⤢'}
                      </span>
                    )}
                  </div>
                )
              })}
            </div>

            {openHere && expanded && (
              <CellExpansion
                column={columns.find((c) => c.name === expanded.column)!}
                pk={pkFor(row)}
                page={pageRequest}
                rowIndex={i}
                columnCount={columns.length}
                onClose={collapse}
              />
            )}
          </div>
        )
      })}
    </>
  )
}

/**
 * Pager, row range, and the SQL that produced the page — or the error if it failed.
 *
 * Takes no arguments — the page, result, and error all come from the grid store.
 *
 * @returns `JSX.Element` — the footer strip. Row totals render as `~`-prefixed when the
 *   count is a planner estimate, and are omitted entirely when unknown.
 */
function PagingFooter() {
  const selected = useExplorer((s) => s.selected)
  const { result, page, setPage, error, loading } = useGrid()

  const last = lastPageIndex(result)
  const forward = canGoForward(result, page)
  const back = page > 0

  const go = (p: number) => {
    if (!selected) return
    void setPage(selected.schema, selected.table, p)
  }

  const totalLabel = () => {
    if (!result) return ''
    if (result.total === null) return `rows ${rowRange(result.page, result.pageSize, result.rows.length)}`
    const approx = result.totalIsEstimate ? '~' : ''
    return (
      <>
        rows {rowRange(result.page, result.pageSize, result.rows.length)} of{' '}
        <span className="grid-footer__total">
          {approx}
          {groupDigits(result.total)}
        </span>
      </>
    )
  }

  return (
    <div className="grid-footer">
      <div className="pager">
        <button className="pager__btn" disabled={!back} onClick={() => go(0)} title="First page">
          ⇤
        </button>
        <button
          className="pager__btn"
          disabled={!back}
          onClick={() => go(page - 1)}
          title="Previous page"
        >
          ←
        </button>
        <button
          className="pager__btn"
          disabled={!forward}
          onClick={() => go(page + 1)}
          title="Next page"
        >
          →
        </button>
        <button
          className="pager__btn"
          disabled={last === null || page >= last}
          onClick={() => last !== null && go(last)}
          title={last === null ? 'Unknown total' : 'Last page'}
        >
          ⇥
        </button>
      </div>

      <span>{totalLabel()}</span>

      <div className="spacer" />

      {error ? (
        <span className="grid-footer__error" title={error.detail ?? error.message}>
          {error.message}
        </span>
      ) : (
        result && (
          <span className="grid-footer__sql" title={result.sql}>
            {result.sql} — <span className="grid-footer__timing">{formatMs(result.timingMs)}</span>
            {loading && ' …'}
          </span>
        )
      )}
    </div>
  )
}

/**
 * The table-browsing pane: header, rows, and pager for the selected table.
 *
 * Owns the reload-on-selection effect and the two window-level key handlers,
 * which are bound to `window` rather than the pane because the grid holds no
 * DOM focus of its own — clicking a row selects it without focusing anything.
 *
 * Takes no arguments — the selected table and its page come from the explorer and grid
 * stores.
 *
 * @returns `JSX.Element` — the pane: a placeholder when no table is selected, otherwise
 *   header, rows, and pager.
 */
export function DataGrid() {
  const { selected, meta } = useExplorer()
  const { result, loading, load, resetFor, layout, loadLayout } = useGrid()

  // Reload whenever the selected table changes.
  useEffect(() => {
    if (!selected) return
    resetFor()
    void loadLayout(selected.schema, selected.table)
    void load(selected.schema, selected.table)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected?.schema, selected?.table])

  // Esc closes an open cell expansion.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && useGrid.getState().expanded) {
        e.preventDefault()
        useGrid.getState().collapse()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // ⌘C copies the selected row as TSV.
  useEffect(() => {
    const onKey = async (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key !== 'c') return
      const { result: r, selectedRow } = useGrid.getState()
      if (!r || selectedRow === null) return
      if (window.getSelection()?.toString()) return // let normal copy win
      const tsv = r.rows[selectedRow].map((v) => v ?? '').join('\t')
      try {
        await navigator.clipboard.writeText(tsv)
      } catch {
        /* clipboard unavailable */
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const columns = meta?.columns ?? []
  const ordered = useMemo(() => orderColumns(columns, layout.order), [columns, layout.order])
  const template = useMemo(() => templateFor(ordered, layout.widths), [ordered, layout.widths])

  if (!selected) {
    return (
      <div className="grid-pane">
        <div className="grid-state">select a table</div>
      </div>
    )
  }

  return (
    <div className="grid-pane">
      <div className="grid-scroll">
        {columns.length > 0 && <GridHeader ordered={ordered} template={template} />}
        {result && result.rows.length > 0 && (
          <GridRows columns={columns} ordered={ordered} template={template} />
        )}
        {result && result.rows.length === 0 && !loading && (
          <div className="grid-state">0 rows</div>
        )}
        {!result && loading && <div className="grid-state">loading…</div>}
      </div>
      <PagingFooter />
    </div>
  )
}
