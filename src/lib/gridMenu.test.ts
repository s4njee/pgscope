import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ipc } from './ipc'
import { buildGridMenu, valueHint, type GridMenuActions } from './gridMenu'
import type { ColumnMeta } from './types'

vi.mock('./ipc', () => ({
  ipc: { formatRow: vi.fn(), valuePredicate: vi.fn() },
}))

const formatRow = vi.mocked(ipc.formatRow)
const valuePredicate = vi.mocked(ipc.valuePredicate)

/** A column with the flags defaulted off; the menu branches on type and `isPk`. */
function col(name: string, dataType: string, extra: Partial<ColumnMeta> = {}): ColumnMeta {
  return { name, dataType, notNull: false, isPk: false, isFk: false, ...extra }
}

const COLUMNS = [
  col('event_id', 'bigint', { isPk: true }),
  col('event_name', 'text'),
  col('properties', 'jsonb'),
]

/** Fresh spies per test, with `canExpand` permissive so items are not filtered out. */
function makeActions() {
  return {
    copy: vi.fn().mockResolvedValue(undefined),
    applyFilter: vi.fn(),
    expandCell: vi.fn(),
    canExpand: vi.fn().mockReturnValue(true),
  } satisfies GridMenuActions & Record<string, unknown>
}

/**
 * The menu for one cell of the fixed `events` row, returned alongside its
 * actions so a test can both inspect items and assert on what invoking one did.
 */
function menu(columnIndex: number, values: (string | null)[], actions = makeActions()) {
  return {
    items: buildGridMenu(
      { schema: 'public', table: 'events', columns: COLUMNS, values, columnIndex, rowIndex: 4 },
      actions,
    ),
    actions,
  }
}

const VALUES = ['48213904', 'signup', '{"plan":"pro"}']

describe('valueHint', () => {
  it('labels NULL and empty distinctly', () => {
    expect(valueHint(null)).toBe('NULL')
    expect(valueHint('')).toBe("''")
  })

  it('truncates long values', () => {
    expect(valueHint('x'.repeat(80)).length).toBeLessThanOrEqual(24)
  })
})

describe('buildGridMenu', () => {
  beforeEach(() => {
    formatRow.mockReset()
    valuePredicate.mockReset()
  })

  it('offers copy, expand, three row formats and two filters', () => {
    const labels = menu(1, VALUES).items.map((i) => i.label)
    expect(labels).toEqual([
      'Copy cell',
      'Expand cell',
      'Copy row as JSON',
      'Copy row as CSV',
      'Copy row as INSERT',
      'Filter: event_name = value',
      'Filter: event_name <> value',
    ])
  })

  it('copies the raw cell value', async () => {
    const { items, actions } = menu(1, VALUES)
    await items[0].run()
    expect(actions.copy).toHaveBeenCalledWith('signup')
  })

  it('disables copy for NULL rather than copying the word', async () => {
    const { items } = menu(1, ['1', null, '{}'])
    const copy = items.find((i) => i.label === 'Copy cell')!
    expect(copy.disabled).toBe(true)
    expect(copy.hint).toBe('NULL')
  })

  it('switches the filter labels to IS NULL for a null cell', () => {
    const labels = menu(1, ['1', null, '{}']).items.map((i) => i.label)
    expect(labels).toContain('Filter: event_name IS NULL')
    expect(labels).toContain('Filter: event_name IS NOT NULL')
    expect(labels).not.toContain('Filter: event_name = value')
  })

  it('disables expand when the cell cannot be opened', () => {
    const actions = makeActions()
    actions.canExpand.mockReturnValue(false)
    const { items } = menu(0, VALUES, actions)
    expect(items.find((i) => i.label === 'Expand cell')!.disabled).toBe(true)
  })

  it('expand targets the right row and column', async () => {
    const { items, actions } = menu(2, VALUES)
    await items.find((i) => i.label === 'Expand cell')!.run()
    expect(actions.expandCell).toHaveBeenCalledWith(4, 'properties')
  })

  it('asks the backend to format the whole row, not just the cell', async () => {
    formatRow.mockResolvedValue('{"event_id": 48213904}')
    const { items, actions } = menu(1, VALUES)
    await items.find((i) => i.label === 'Copy row as JSON')!.run()

    expect(formatRow).toHaveBeenCalledWith({
      schema: 'public',
      table: 'events',
      columns: COLUMNS,
      values: VALUES,
      format: 'json',
    })
    expect(actions.copy).toHaveBeenCalledWith('{"event_id": 48213904}')
  })

  it('requests each row format with the right tag', async () => {
    formatRow.mockResolvedValue('x')
    const { items } = menu(1, VALUES)
    for (const [label, format] of [
      ['Copy row as JSON', 'json'],
      ['Copy row as CSV', 'csv'],
      ['Copy row as INSERT', 'insert'],
    ] as const) {
      await items.find((i) => i.label === label)!.run()
      expect(formatRow).toHaveBeenLastCalledWith(expect.objectContaining({ format }))
    }
  })

  it('builds the predicate in the backend, never in the UI', async () => {
    valuePredicate.mockResolvedValue(`"event_name" = 'signup'`)
    const { items, actions } = menu(1, VALUES)
    await items.find((i) => i.label === 'Filter: event_name = value')!.run()

    // Quoting must come from Rust so it matches the query builder's rules.
    expect(valuePredicate).toHaveBeenCalledWith(COLUMNS[1], 'signup', 'eq')
    expect(actions.applyFilter).toHaveBeenCalledWith(`"event_name" = 'signup'`)
  })

  it('passes null through to the predicate builder', async () => {
    valuePredicate.mockResolvedValue(`"event_name" IS NULL`)
    const { items } = menu(1, ['1', null, '{}'])
    await items.find((i) => i.label === 'Filter: event_name IS NULL')!.run()
    expect(valuePredicate).toHaveBeenCalledWith(COLUMNS[1], null, 'eq')
  })

  it('uses <> for the exclusion filter', async () => {
    valuePredicate.mockResolvedValue('x')
    const { items } = menu(1, VALUES)
    await items.find((i) => i.label === 'Filter: event_name <> value')!.run()
    expect(valuePredicate).toHaveBeenCalledWith(COLUMNS[1], 'signup', 'noteq')
  })

  it('groups actions with separators', () => {
    const items = menu(1, VALUES).items
    expect(items.find((i) => i.label === 'Copy row as JSON')!.separatorBefore).toBe(true)
    expect(items.find((i) => i.label.startsWith('Filter:'))!.separatorBefore).toBe(true)
  })
})
