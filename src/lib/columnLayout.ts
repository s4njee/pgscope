/**
 * Applying a saved column layout to a table's real columns.
 *
 * A layout is stored by column *name*, but the table it describes can change
 * underneath it — a column added, dropped, or renamed between sessions. The
 * reconciliation below is the whole point of this module: applying a saved
 * order naively drops any column the layout has never heard of, which makes a
 * newly added column invisible in the grid with nothing on screen to explain it.
 */

import { columnWidth } from './cellColor'
import type { ColumnMeta } from './types'

/** Narrower than this and the header is unreadable and hard to grab again. */
export const MIN_COLUMN_WIDTH = 56
/** Wider than the window is never useful and strands the resize handle. */
export const MAX_COLUMN_WIDTH = 1200

export interface TableLayout {
  /** Column name -> pixel width. Absent means "use the computed default". */
  widths: Record<string, number>
  /** Display order by column name; may be stale relative to the real table. */
  order: string[]
}

/** A table the user has never touched: computed widths, server column order. */
export function emptyLayout(): TableLayout {
  return { widths: {}, order: [] }
}

/**
 * A column paired with its index in the *canonical* column list.
 *
 * Row values arrive positionally, matched to the unreordered columns, so every
 * lookup into a row must use this index rather than the display position.
 * Carrying it alongside the column makes that structural instead of a rule to
 * remember at each call site.
 */
export interface OrderedColumn {
  col: ColumnMeta
  index: number
}

/** Clamp a dragged width into a range that leaves the grid usable. */
export function clampColumnWidth(px: number): number {
  // Only NaN needs the special case: it compares false against everything, so
  // it would fall through the clamp untouched. ±Infinity is ordered and clamps
  // to the right end on its own.
  if (Number.isNaN(px)) return MIN_COLUMN_WIDTH
  return Math.round(Math.min(Math.max(px, MIN_COLUMN_WIDTH), MAX_COLUMN_WIDTH))
}

/**
 * Order the real columns by a saved layout.
 *
 * Names in `order` that no longer exist are skipped; columns the order has
 * never seen are appended in their natural order. Appending rather than
 * guessing a position: a column added to the table shows up predictably at the
 * end, where the user can move it, instead of somewhere that depends on how the
 * old order happened to interleave.
 */
export function orderColumns(columns: ColumnMeta[], order: string[]): OrderedColumn[] {
  const byName = new Map(columns.map((col, index) => [col.name, { col, index }]))
  const out: OrderedColumn[] = []
  const placed = new Set<string>()

  for (const name of order) {
    const hit = byName.get(name)
    // Skip both unknown names and any duplicate in a hand-edited layout file;
    // a column rendered twice would read as a data bug, not a layout one.
    if (hit && !placed.has(name)) {
      out.push(hit)
      placed.add(name)
    }
  }
  for (const [name, hit] of byName) {
    if (!placed.has(name)) out.push(hit)
  }
  return out
}

/** The grid-template track for one column: saved width, else the type default. */
export function trackFor(col: ColumnMeta, widths: Record<string, number>): string {
  const saved = widths[col.name]
  return saved === undefined ? columnWidth(col) : `${clampColumnWidth(saved)}px`
}

/** Row-number gutter plus one track per column, in display order. */
export function templateFor(ordered: OrderedColumn[], widths: Record<string, number>): string {
  return ['44px', ...ordered.map(({ col }) => trackFor(col, widths))].join(' ')
}

/**
 * Move the column at `from` to the insertion point `to`.
 *
 * `to` is an index into the list *as it looks now*, which is what
 * [`dropIndexAt`] computes from the on-screen headers. Removing the dragged
 * column first shifts everything after it left by one, so a rightward move has
 * to compensate — without this, dropping a column past its neighbour lands it
 * one slot short of where the indicator sat.
 */
export function moveColumn(order: string[], from: number, to: number): string[] {
  if (from < 0 || from >= order.length) return order
  const target = from < to ? to - 1 : to
  if (target === from) return order

  const next = order.slice()
  const [moved] = next.splice(from, 1)
  next.splice(Math.max(0, Math.min(target, next.length)), 0, moved)
  return next
}

/**
 * Which display slot a pointer at `x` is over, given each header's bounds.
 *
 * Returns an insertion index in 0..rects.length. The midpoint test is what
 * makes a drop feel like it lands where the indicator sits: past the middle of
 * a header means "after it", not "on it".
 */
export function dropIndexAt(x: number, rects: { left: number; right: number }[]): number {
  for (let i = 0; i < rects.length; i++) {
    const { left, right } = rects[i]
    if (x < left + (right - left) / 2) return i
  }
  return rects.length
}

/**
 * The order to persist after a drag, expressed as column names.
 *
 * Always writes a full, explicit order — including when the table has never
 * been reordered — so the saved layout does not depend on the column order the
 * server happened to report at the time it was written.
 */
export function reorderedNames(ordered: OrderedColumn[], from: number, to: number): string[] {
  return moveColumn(
    ordered.map(({ col }) => col.name),
    from,
    to,
  )
}
