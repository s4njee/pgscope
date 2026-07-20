import { describe, expect, it } from 'vitest'

import {
  clampColumnWidth,
  dropIndexAt,
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  moveColumn,
  orderColumns,
  reorderedNames,
  templateFor,
  trackFor,
} from './columnLayout'
import type { ColumnMeta } from './types'

/** A minimal column; only `name` matters to ordering, `dataType` to width. */
function col(name: string, dataType = 'text'): ColumnMeta {
  return { name, dataType, notNull: false, isPk: false, isFk: false }
}

const names = (ordered: { col: ColumnMeta }[]) => ordered.map((o) => o.col.name)

describe('orderColumns', () => {
  it('returns the natural order when nothing is saved', () => {
    const columns = [col('a'), col('b'), col('c')]
    expect(names(orderColumns(columns, []))).toEqual(['a', 'b', 'c'])
  })

  it('applies a saved order', () => {
    const columns = [col('a'), col('b'), col('c')]
    expect(names(orderColumns(columns, ['c', 'a', 'b']))).toEqual(['c', 'a', 'b'])
  })

  it('appends a column the saved order has never seen', () => {
    // The bug this module exists to prevent: a column added to the table since
    // the layout was saved must not vanish from the grid.
    const columns = [col('a'), col('b'), col('added')]
    expect(names(orderColumns(columns, ['b', 'a']))).toEqual(['b', 'a', 'added'])
  })

  it('appends several new columns in their natural order', () => {
    const columns = [col('a'), col('new1'), col('b'), col('new2')]
    expect(names(orderColumns(columns, ['b', 'a']))).toEqual(['b', 'a', 'new1', 'new2'])
  })

  it('skips a saved name whose column is gone', () => {
    const columns = [col('a'), col('b')]
    expect(names(orderColumns(columns, ['a', 'dropped', 'b']))).toEqual(['a', 'b'])
  })

  it('ignores a duplicate in a hand-edited layout file', () => {
    // Rendering the same column twice would read as a data bug rather than a
    // layout one, so it is worth defending against.
    const columns = [col('a'), col('b')]
    expect(names(orderColumns(columns, ['a', 'a', 'b']))).toEqual(['a', 'b'])
  })

  it('survives a layout that shares no names with the table at all', () => {
    const columns = [col('a'), col('b')]
    expect(names(orderColumns(columns, ['x', 'y']))).toEqual(['a', 'b'])
  })

  it('carries the canonical index, not the display position', () => {
    // Row values arrive positionally against the unreordered columns, so a
    // reordered grid reading row[displayIndex] would show the wrong data under
    // the right header — silently, and only once a column has been moved.
    const columns = [col('a'), col('b'), col('c')]
    const ordered = orderColumns(columns, ['c', 'a', 'b'])
    expect(ordered.map((o) => o.index)).toEqual([2, 0, 1])
  })

  it('handles an empty column list', () => {
    expect(orderColumns([], ['a'])).toEqual([])
  })
})

describe('clampColumnWidth', () => {
  it('keeps a sensible width', () => {
    expect(clampColumnWidth(180)).toBe(180)
  })

  it('refuses to shrink a column to nothing', () => {
    // A zero-width column takes its resize handle with it, so the user cannot
    // drag it back — the failure is unrecoverable from the UI.
    expect(clampColumnWidth(0)).toBe(MIN_COLUMN_WIDTH)
    expect(clampColumnWidth(-40)).toBe(MIN_COLUMN_WIDTH)
  })

  it('caps an absurd width', () => {
    expect(clampColumnWidth(99999)).toBe(MAX_COLUMN_WIDTH)
  })

  it('falls back for a non-finite width', () => {
    expect(clampColumnWidth(NaN)).toBe(MIN_COLUMN_WIDTH)
    expect(clampColumnWidth(Infinity)).toBe(MAX_COLUMN_WIDTH)
  })

  it('rounds to whole pixels', () => {
    expect(clampColumnWidth(180.6)).toBe(181)
  })
})

describe('tracks and template', () => {
  it('uses the type default when no width is saved', () => {
    // `boolean` has a narrow default; the point is that it is not a px value
    // coming from the layout.
    expect(trackFor(col('flag', 'boolean'), {})).toBe('80px')
  })

  it('uses a saved width when there is one', () => {
    expect(trackFor(col('flag', 'boolean'), { flag: 200 })).toBe('200px')
  })

  it('clamps a saved width that is out of range', () => {
    // A corrupt or hand-edited file must not be able to render a column
    // unusable.
    expect(trackFor(col('a'), { a: 0 })).toBe(`${MIN_COLUMN_WIDTH}px`)
  })

  it('builds a template with the row-number gutter first', () => {
    const ordered = orderColumns([col('a'), col('b')], [])
    expect(templateFor(ordered, { a: 100 })).toBe('44px 100px 132px')
  })

  it('builds the template in display order, not natural order', () => {
    const ordered = orderColumns([col('a'), col('b')], ['b', 'a'])
    expect(templateFor(ordered, { a: 100, b: 300 })).toBe('44px 300px 100px')
  })
})

describe('moveColumn', () => {
  it('moves a column to the right', () => {
    expect(moveColumn(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'a', 'c'])
  })

  it('moves a column to the left', () => {
    expect(moveColumn(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b'])
  })

  it('moves a column to the very end', () => {
    expect(moveColumn(['a', 'b', 'c'], 0, 3)).toEqual(['b', 'c', 'a'])
  })

  it('is a no-op when the target is the source', () => {
    const order = ['a', 'b', 'c']
    expect(moveColumn(order, 1, 1)).toBe(order)
  })

  it('clamps an out-of-range target instead of losing the column', () => {
    expect(moveColumn(['a', 'b'], 0, 99)).toEqual(['b', 'a'])
    expect(moveColumn(['a', 'b'], 1, -5)).toEqual(['b', 'a'])
  })

  it('ignores an out-of-range source', () => {
    const order = ['a', 'b']
    expect(moveColumn(order, 5, 0)).toBe(order)
  })
})

describe('dropIndexAt', () => {
  const rects = [
    { left: 0, right: 100 },
    { left: 100, right: 200 },
    { left: 200, right: 300 },
  ]

  it('drops before a header when left of its midpoint', () => {
    expect(dropIndexAt(10, rects)).toBe(0)
    expect(dropIndexAt(120, rects)).toBe(1)
  })

  it('drops after a header when right of its midpoint', () => {
    // Past the middle means "after it" — otherwise the drop lands one slot
    // short of where the indicator was sitting.
    expect(dropIndexAt(60, rects)).toBe(1)
    expect(dropIndexAt(160, rects)).toBe(2)
  })

  it('drops at the end when past every header', () => {
    expect(dropIndexAt(999, rects)).toBe(3)
  })

  it('drops at the start when left of everything', () => {
    expect(dropIndexAt(-50, rects)).toBe(0)
  })

  it('handles an empty header list', () => {
    expect(dropIndexAt(10, [])).toBe(0)
  })
})

describe('reorderedNames', () => {
  it('writes a full explicit order even for a never-reordered table', () => {
    // Persisting only the moved column would leave the rest at the mercy of
    // whatever order the server reported next time.
    const ordered = orderColumns([col('a'), col('b'), col('c')], [])
    expect(reorderedNames(ordered, 2, 0)).toEqual(['c', 'a', 'b'])
  })

  it('composes with an order that was already customised', () => {
    const ordered = orderColumns([col('a'), col('b'), col('c')], ['c', 'b', 'a'])
    expect(reorderedNames(ordered, 0, 3)).toEqual(['b', 'a', 'c'])
  })
})
