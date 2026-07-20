import { act } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ipc } from '../lib/ipc'
import { resultSets, runSummary, useEditor } from './editor'
import { useConfirm, usePrompt } from './prompt'
import type { QueryRun } from '../lib/types'

vi.mock('../lib/ipc', () => ({
  ipc: {
    runQuery: vi.fn(),
    cancelQuery: vi.fn(),
    saveQueryAt: vi.fn(),
    saveNamedQuery: vi.fn(),
  },
  isAppError: (e: unknown) =>
    typeof e === 'object' && e !== null && 'code' in e && 'message' in e,
  // Mirrors the real one: an AppError from the IPC boundary passes through
  // unchanged, so its code survives for the caller to branch on.
  toAppError: (e: unknown) =>
    typeof e === 'object' && e !== null && 'code' in e && 'message' in e
      ? e
      : {
          code: 'query',
          message: e instanceof Error ? e.message : String(e),
          detail: null,
          sqlstate: null,
        },
}))

beforeEach(() => {
  useEditor.getState().reset()
})

describe('tab lifecycle', () => {
  it('opens tabs with sequential default names and focuses the new one', () => {
    const { openTab } = useEditor.getState()
    const a = openTab()
    const b = openTab()

    const { tabs, activeTabId } = useEditor.getState()
    expect(tabs).toHaveLength(2)
    expect(activeTabId).toBe(b)
    expect(tabs[0].name).not.toBe(tabs[1].name)
    expect(a).not.toBe(b)
  })

  it('opens a saved query with its filename and path', () => {
    const id = useEditor
      .getState()
      .openTab({ name: 'dau_last_30d', content: 'SELECT 1;', savedPath: '/tmp/dau_last_30d.sql' })

    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.name).toBe('dau_last_30d')
    expect(tab.content).toBe('SELECT 1;')
    expect(tab.savedPath).toBe('/tmp/dau_last_30d.sql')
    expect(tab.dirty).toBe(false)
  })

  it('focuses the existing tab when the same saved query is reopened', () => {
    const { openTab } = useEditor.getState()
    const first = openTab({ name: 'q', content: 'SELECT 1;', savedPath: '/tmp/q.sql' })
    openTab() // something else in between
    const again = openTab({ name: 'q', content: 'SELECT 1;', savedPath: '/tmp/q.sql' })

    expect(again).toBe(first)
    expect(useEditor.getState().tabs).toHaveLength(2)
    expect(useEditor.getState().activeTabId).toBe(first)
  })

  it('marks a tab dirty once edited', () => {
    const id = useEditor.getState().openTab()
    expect(useEditor.getState().tabs[0].dirty).toBe(false)

    useEditor.getState().setContent(id, 'SELECT 1;')
    const tab = useEditor.getState().tabs[0]
    expect(tab.content).toBe('SELECT 1;')
    expect(tab.dirty).toBe(true)
  })
})

describe('closing tabs', () => {
  it('focuses the left neighbour', () => {
    const { openTab } = useEditor.getState()
    const a = openTab()
    const b = openTab()
    const c = openTab()

    useEditor.getState().setActive(c)
    useEditor.getState().closeTab(c)
    expect(useEditor.getState().activeTabId).toBe(b)

    useEditor.getState().closeTab(b)
    expect(useEditor.getState().activeTabId).toBe(a)
  })

  it('leaves the active tab alone when closing a different one', () => {
    const { openTab } = useEditor.getState()
    const a = openTab()
    const b = openTab()

    useEditor.getState().setActive(b)
    useEditor.getState().closeTab(a)
    expect(useEditor.getState().activeTabId).toBe(b)
  })

  it('clears the active id when the last tab closes', () => {
    const id = useEditor.getState().openTab()
    useEditor.getState().closeTab(id)

    expect(useEditor.getState().tabs).toHaveLength(0)
    expect(useEditor.getState().activeTabId).toBeNull()
  })
})

describe('running', () => {
  it('stores the run and clears the running flag', async () => {
    const { ipc } = await import('../lib/ipc')
    const run: QueryRun = {
      statements: [
        {
          sql: 'SELECT 1',
          result: { columns: ['?column?'], rows: [['1']], totalRows: 1, truncated: false },
          notices: [],
          timingMs: 1.2,
        },
      ],
      totalTimingMs: 1.5,
    }
    vi.mocked(ipc.runQuery).mockResolvedValue(run)

    const id = useEditor.getState().openTab({ content: 'SELECT 1;' })
    await useEditor.getState().run(id)

    const tab = useEditor.getState().tabs[0]
    expect(tab.run).toEqual(run)
    expect(tab.running).toBe(false)
    expect(tab.error).toBeNull()
  })

  it('keeps the previous result visible when a run fails', async () => {
    const { ipc } = await import('../lib/ipc')
    const good: QueryRun = {
      statements: [
        {
          sql: 'SELECT 1',
          result: { columns: ['a'], rows: [['1']], totalRows: 1, truncated: false },
          notices: [],
          timingMs: 1,
        },
      ],
      totalTimingMs: 1,
    }
    vi.mocked(ipc.runQuery).mockResolvedValueOnce(good)

    const id = useEditor.getState().openTab({ content: 'SELECT 1;' })
    await useEditor.getState().run(id)

    vi.mocked(ipc.runQuery).mockRejectedValueOnce(new Error('syntax error'))
    await useEditor.getState().run(id)

    const tab = useEditor.getState().tabs[0]
    expect(tab.error?.message).toBe('syntax error')
    expect(tab.run).toEqual(good) // last good result survives
    expect(tab.running).toBe(false)
  })

  it('runs an explicit selection instead of the whole buffer', async () => {
    const { ipc } = await import('../lib/ipc')
    vi.mocked(ipc.runQuery).mockResolvedValue({ statements: [], totalTimingMs: 0 })

    const id = useEditor.getState().openTab({ content: 'SELECT 1; SELECT 2;' })
    await useEditor.getState().run(id, 'SELECT 2')

    expect(ipc.runQuery).toHaveBeenLastCalledWith('SELECT 2')
  })

  it('does nothing for a blank buffer', async () => {
    const { ipc } = await import('../lib/ipc')
    vi.mocked(ipc.runQuery).mockClear()

    const id = useEditor.getState().openTab({ content: '   \n  ' })
    await useEditor.getState().run(id)

    expect(ipc.runQuery).not.toHaveBeenCalled()
  })
})

describe('result helpers', () => {
  const withRows = (sql: string) => ({
    sql,
    result: { columns: ['a'], rows: [['1']], totalRows: 1, truncated: false },
    notices: [],
    timingMs: 1,
  })
  const noRows = (sql: string) => ({ sql, result: null, notices: [], timingMs: 1 })

  it('lists only statements that returned rows', () => {
    const run: QueryRun = {
      statements: [noRows('SET x = 1'), withRows('SELECT 1'), noRows('COMMIT')],
      totalTimingMs: 3,
    }
    const sets = resultSets(run)
    expect(sets).toHaveLength(1)
    // The original statement index is preserved for the result tab tooltip.
    expect(sets[0].index).toBe(1)
  })

  it('summarises runs that produced no result sets', () => {
    const run: QueryRun = { statements: [noRows('SET x = 1')], totalTimingMs: 1 }
    expect(runSummary(run)).toBe('1 statement executed')

    const two: QueryRun = { statements: [noRows('SET a = 1'), noRows('SET b = 2')], totalTimingMs: 2 }
    expect(runSummary(two)).toBe('2 statements executed')
  })

  it('returns no summary when there are rows to show', () => {
    const run: QueryRun = { statements: [withRows('SELECT 1')], totalTimingMs: 1 }
    expect(runSummary(run)).toBeNull()
  })

  it('handles a null run', () => {
    expect(resultSets(null)).toEqual([])
    expect(runSummary(null)).toBeNull()
  })
})

// ------------------------------- saving ---------------------------------

const saveQueryAt = vi.mocked(ipc.saveQueryAt)
const saveNamedQuery = vi.mocked(ipc.saveNamedQuery)

/**
 * Answer the next prompt with `value`, or null to cancel.
 *
 * @param value - `string | null` — what the stubbed prompt resolves with;
 *   `null` stands in for the user cancelling the dialog.
 * @returns `() => void` — a restore function that puts the real `ask` back.
 *   The stub answers every prompt until it is called, not just the next one.
 */
function answerPrompt(value: string | null) {
  const original = usePrompt.getState().ask
  usePrompt.setState({ ask: async () => value } as never)
  return () => usePrompt.setState({ ask: original } as never)
}

describe('saving a tab', () => {
  beforeEach(() => {
    saveQueryAt.mockReset()
    saveNamedQuery.mockReset()
  })

  it('saves in place when the tab came from a file', async () => {
    const path = '/data/saved_queries/dau.sql'
    const id = useEditor.getState().openTab({ name: 'dau', savedPath: path, content: 'SELECT 1;' })
    useEditor.getState().setContent(id, 'SELECT 2;')
    saveQueryAt.mockResolvedValue({ name: 'dau', path, content: 'SELECT 2;' })

    const ok = await useEditor.getState().save(id)

    expect(ok).toBe(true)
    // Same path, edited content — no prompt and no new file.
    expect(saveQueryAt).toHaveBeenCalledWith(path, 'SELECT 2;')
    expect(saveNamedQuery).not.toHaveBeenCalled()

    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.dirty).toBe(false)
    expect(tab.savedPath).toBe(path)
  })

  it('prompts for a name when the tab has never been saved', async () => {
    const id = useEditor.getState().openTab({ content: 'SELECT 1;' })
    const restore = answerPrompt('my_query')
    saveNamedQuery.mockResolvedValue({
      name: 'my_query',
      path: '/data/saved_queries/my_query.sql',
      content: 'SELECT 1;',
    })

    const ok = await useEditor.getState().save(id)
    restore()

    expect(ok).toBe(true)
    expect(saveNamedQuery).toHaveBeenCalledWith('my_query', 'SELECT 1;')
    expect(saveQueryAt).not.toHaveBeenCalled()

    // The tab adopts the file, so the next save goes in place.
    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.savedPath).toBe('/data/saved_queries/my_query.sql')
    expect(tab.name).toBe('my_query')
    expect(tab.dirty).toBe(false)
  })

  it('prompts on save-as even when the tab already has a file', async () => {
    const id = useEditor
      .getState()
      .openTab({ name: 'dau', savedPath: '/data/saved_queries/dau.sql', content: 'SELECT 1;' })
    const restore = answerPrompt('dau_copy')
    saveNamedQuery.mockResolvedValue({
      name: 'dau_copy',
      path: '/data/saved_queries/dau_copy.sql',
      content: 'SELECT 1;',
    })

    const ok = await useEditor.getState().save(id, true)
    restore()

    expect(ok).toBe(true)
    expect(saveNamedQuery).toHaveBeenCalledWith('dau_copy', 'SELECT 1;')
    expect(saveQueryAt).not.toHaveBeenCalled()
    // The tab follows the copy, leaving the original untouched.
    expect(useEditor.getState().tabs.find((t) => t.id === id)!.savedPath).toBe(
      '/data/saved_queries/dau_copy.sql',
    )
  })

  it('writes nothing when the prompt is cancelled', async () => {
    const id = useEditor.getState().openTab({ content: 'SELECT 1;' })
    useEditor.getState().setContent(id, 'SELECT 2;')
    const restore = answerPrompt(null)

    const ok = await useEditor.getState().save(id)
    restore()

    expect(ok).toBe(false)
    expect(saveNamedQuery).not.toHaveBeenCalled()
    expect(saveQueryAt).not.toHaveBeenCalled()
    // Cancelling must not pretend the tab is saved.
    expect(useEditor.getState().tabs.find((t) => t.id === id)!.dirty).toBe(true)
  })

  it('surfaces a failure and leaves the tab dirty', async () => {
    const path = '/data/saved_queries/dau.sql'
    const id = useEditor.getState().openTab({ name: 'dau', savedPath: path })
    useEditor.getState().setContent(id, 'SELECT 2;')
    saveQueryAt.mockRejectedValue(
      new Error('refusing to write outside the saved-queries directory'),
    )

    const ok = await useEditor.getState().save(id)

    expect(ok).toBe(false)
    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.error?.message).toContain('refusing to write')
    // Still unsaved — the user must not think their edit landed.
    expect(tab.dirty).toBe(true)
  })

  it('does nothing for an unknown tab id', async () => {
    const ok = await useEditor.getState().save('no-such-tab')
    expect(ok).toBe(false)
    expect(saveQueryAt).not.toHaveBeenCalled()
    expect(saveNamedQuery).not.toHaveBeenCalled()
  })
})

describe('following a saved query that moved or vanished', () => {
  const openFrom = (path: string, name: string) =>
    useEditor.getState().openTab({ name, content: 'SELECT 1;', savedPath: path })

  it('retargets a tab when its file is renamed', () => {
    const id = openFrom('/q/old.sql', 'old')
    useEditor.getState().reconcileSavedPath('/q/old.sql', { path: '/q/new.sql', name: 'new' })

    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.savedPath).toBe('/q/new.sql')
    expect(tab.name).toBe('new')
  })

  it('leaves other tabs alone', () => {
    const other = openFrom('/q/other.sql', 'other')
    openFrom('/q/old.sql', 'old')
    useEditor.getState().reconcileSavedPath('/q/old.sql', { path: '/q/new.sql', name: 'new' })

    const tab = useEditor.getState().tabs.find((t) => t.id === other)!
    expect(tab.savedPath).toBe('/q/other.sql')
  })

  it('detaches a tab whose file was deleted, keeping its content', () => {
    // Losing unsaved edits because the file was deleted in the sidebar would be
    // a far worse outcome than an orphaned tab.
    const id = openFrom('/q/doomed.sql', 'doomed')
    useEditor.getState().setContent(id, 'SELECT 2; -- edited')
    useEditor.getState().reconcileSavedPath('/q/doomed.sql', null)

    const tab = useEditor.getState().tabs.find((t) => t.id === id)!
    expect(tab.savedPath).toBeNull()
    expect(tab.content).toBe('SELECT 2; -- edited')
    // Dirty, so ⌘S offers a new name rather than writing to a file that is gone.
    expect(tab.dirty).toBe(true)
  })

  it('does nothing when no tab points at that file', () => {
    const id = openFrom('/q/a.sql', 'a')
    useEditor.getState().reconcileSavedPath('/q/unrelated.sql', null)
    expect(useEditor.getState().tabs.find((t) => t.id === id)!.savedPath).toBe('/q/a.sql')
  })
})

describe('saving over an existing query', () => {
  const exists = { code: 'exists', message: 'a saved query named "dau" already exists' }

  beforeEach(() => {
    saveQueryAt.mockReset()
    saveNamedQuery.mockReset()
    act(() => {
      usePrompt.setState({ open: false, value: '', resolve: null })
      useConfirm.setState({ open: false, resolve: null })
    })
  })

  /** Answer the name prompt, then the overwrite confirmation, with `replace`. */
  async function saveAsAnswering(id: string, name: string, replace: boolean) {
    const done = useEditor.getState().save(id, true)
    await vi.waitFor(() => expect(usePrompt.getState().open).toBe(true))
    act(() => {
      usePrompt.getState().setValue(name)
      usePrompt.getState().confirm()
    })
    await vi.waitFor(() => expect(useConfirm.getState().open).toBe(true))
    act(() => {
      if (replace) useConfirm.getState().accept()
      else useConfirm.getState().cancel()
    })
    return done
  }

  it('asks before replacing, then retries with overwrite', async () => {
    const id = useEditor.getState().openTab({ name: 'scratch', content: 'SELECT 2;' })
    saveNamedQuery
      .mockRejectedValueOnce(exists)
      .mockResolvedValueOnce({ name: 'dau', path: '/q/dau.sql', content: 'SELECT 2;' })

    expect(await saveAsAnswering(id, 'dau', true)).toBe(true)
    // Second call carries the overwrite flag the first deliberately lacked.
    expect(saveNamedQuery).toHaveBeenNthCalledWith(1, 'dau', 'SELECT 2;')
    expect(saveNamedQuery).toHaveBeenNthCalledWith(2, 'dau', 'SELECT 2;', true)
  })

  it('writes nothing when the replacement is declined', async () => {
    // Declining must not fall through to an overwrite, and must not surface as
    // an error either — the user cancelled, nothing went wrong.
    const id = useEditor.getState().openTab({ name: 'scratch', content: 'SELECT 2;' })
    saveNamedQuery.mockRejectedValueOnce(exists)

    expect(await saveAsAnswering(id, 'dau', false)).toBe(false)
    expect(saveNamedQuery).toHaveBeenCalledTimes(1)
    expect(useEditor.getState().tabs.find((t) => t.id === id)!.error).toBeNull()
  })

  it('still reports an unrelated failure as an error', async () => {
    const id = useEditor.getState().openTab({ name: 'scratch', content: 'SELECT 2;' })
    saveNamedQuery.mockRejectedValueOnce({ code: 'storage', message: 'disk full' })

    const done = useEditor.getState().save(id, true)
    await vi.waitFor(() => expect(usePrompt.getState().open).toBe(true))
    act(() => {
      usePrompt.getState().setValue('dau')
      usePrompt.getState().confirm()
    })
    expect(await done).toBe(false)
    expect(useEditor.getState().tabs.find((t) => t.id === id)!.error?.message).toBe('disk full')
  })
})
