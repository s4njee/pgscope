import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useGrid } from './grid'

vi.mock('../lib/ipc', () => ({
  ipc: { fetchPage: vi.fn().mockResolvedValue(null), cancelGrid: vi.fn() },
  toAppError: (e: unknown) => ({
    code: 'query',
    message: String(e),
    detail: null,
    sqlstate: null,
  }),
}))

/**
 * Compact view of the sort, for readable assertions.
 *
 * Takes no arguments — it reads the live grid store.
 *
 * @returns `string[]` — one `column:dir` entry per sort term, most significant
 *   first; empty when unsorted.
 */
const sortOf = () => useGrid.getState().sort.map((k) => `${k.column}:${k.dir}`)

/**
 * One header click on `public.events`; `shift` is what adds a key rather than replacing.
 *
 * @param col - `string` — the column name whose header was clicked.
 * @param shift - `boolean` — true for a shift-click, which appends a
 *   lower-priority sort term instead of replacing the whole sort. Defaults to
 *   false.
 * @returns `Promise<void>` — resolves once the store has applied the new sort
 *   and the follow-up page load has settled.
 */
async function click(col: string, shift = false) {
  await useGrid.getState().toggleSort('public', 'events', col, shift)
}

describe('multi-column sort', () => {
  beforeEach(() => {
    useGrid.getState().resetFor()
  })

  it('starts unsorted', () => {
    expect(sortOf()).toEqual([])
  })

  it('sorts ascending on first click, descending on second', async () => {
    await click('event_name')
    expect(sortOf()).toEqual(['event_name:asc'])
    await click('event_name')
    expect(sortOf()).toEqual(['event_name:desc'])
    // And back again — a plain click never removes the sort entirely.
    await click('event_name')
    expect(sortOf()).toEqual(['event_name:asc'])
  })

  it('replaces the sort on a plain click of another column', async () => {
    await click('event_name')
    await click('event_id')
    expect(sortOf()).toEqual(['event_id:asc'])
  })

  it('appends on shift-click, preserving precedence', async () => {
    await click('event_name')
    await click('event_id', true)
    expect(sortOf()).toEqual(['event_name:asc', 'event_id:asc'])

    await click('user_id', true)
    expect(sortOf()).toEqual(['event_name:asc', 'event_id:asc', 'user_id:asc'])
  })

  it('shift-click cycles a key asc -> desc -> removed', async () => {
    await click('event_name')
    await click('event_id', true)
    expect(sortOf()).toEqual(['event_name:asc', 'event_id:asc'])

    await click('event_id', true)
    expect(sortOf()).toEqual(['event_name:asc', 'event_id:desc'])

    // Third shift-click drops it, so the interaction can undo itself.
    await click('event_id', true)
    expect(sortOf()).toEqual(['event_name:asc'])
  })

  it('shift-click keeps a key in place rather than moving it to the end', async () => {
    await click('a')
    await click('b', true)
    await click('c', true)
    // Flipping the middle key must not reorder the sort.
    await click('b', true)
    expect(sortOf()).toEqual(['a:asc', 'b:desc', 'c:asc'])
  })

  it('never produces a duplicate column', async () => {
    await click('event_name')
    await click('event_name', true)
    await click('event_name', true)
    const columns = useGrid.getState().sort.map((k) => k.column)
    expect(new Set(columns).size).toBe(columns.length)
  })

  it('a plain click collapses a multi-column sort to one key', async () => {
    await click('a')
    await click('b', true)
    await click('c', true)

    // Clicking a column that is already sorted always toggles its direction —
    // the same rule whether it was the only key or one of several. Preserving
    // the direction here instead would make the outcome depend on how the sort
    // was built, which is harder to predict.
    await click('b')
    expect(sortOf()).toEqual(['b:desc'])

    // Clicking an unsorted column collapses to it ascending.
    await click('a')
    expect(sortOf()).toEqual(['a:asc'])
  })

  it('resets to page 0 when the sort changes', async () => {
    useGrid.setState({ page: 7 })
    await click('event_name')
    expect(useGrid.getState().page).toBe(0)
  })

  it('clears row selection and any open expansion', async () => {
    useGrid.setState({ selectedRow: 3, expanded: { row: 3, column: 'properties' } })
    await click('event_name')
    expect(useGrid.getState().selectedRow).toBeNull()
    expect(useGrid.getState().expanded).toBeNull()
  })
})

describe('addFilter', () => {
  beforeEach(() => {
    useGrid.getState().resetFor()
  })

  it('sets the filter when there is none', async () => {
    await useGrid.getState().addFilter('public', 'events', `"a" = 1`)
    expect(useGrid.getState().filter).toBe(`"a" = 1`)
  })

  it('ANDs onto an existing filter', async () => {
    await useGrid.getState().addFilter('public', 'events', `"a" = 1`)
    await useGrid.getState().addFilter('public', 'events', `"b" = 2`)
    expect(useGrid.getState().filter).toBe(`("a" = 1) AND "b" = 2`)
  })

  it('parenthesises the existing filter so an OR cannot change meaning', async () => {
    // Without the parens this becomes `a = 1 OR b = 2 AND c = 3`, which binds
    // as `a = 1 OR (b = 2 AND c = 3)` — a strictly wider result set.
    await useGrid.getState().applyFilter('public', 'events', `a = 1 OR b = 2`)
    await useGrid.getState().addFilter('public', 'events', `"c" = 3`)
    expect(useGrid.getState().filter).toBe(`(a = 1 OR b = 2) AND "c" = 3`)
  })

  it('keeps the draft in sync so the box shows what is applied', async () => {
    await useGrid.getState().addFilter('public', 'events', `"a" = 1`)
    expect(useGrid.getState().filterDraft).toBe(useGrid.getState().filter)
  })

  it('resets to the first page', async () => {
    useGrid.setState({ page: 5 })
    await useGrid.getState().addFilter('public', 'events', `"a" = 1`)
    expect(useGrid.getState().page).toBe(0)
  })
})
