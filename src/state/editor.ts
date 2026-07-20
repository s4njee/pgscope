import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { ipc, isAppError, toAppError } from '../lib/ipc'
import { confirmAction, promptForName } from './prompt'
import type { AppError, ExplainResult, QueryRun } from '../lib/types'

export interface QueryTab {
  id: string
  /** Display name; derived from the filename when opened from a saved query. */
  name: string
  content: string
  /** Cursor offset, kept so run-at-cursor works and survives tab switches. */
  cursor: number
  /** Set when the tab's content differs from what was last saved/opened. */
  dirty: boolean
  /** Path of the saved query this tab came from, if any. */
  savedPath: string | null
  run: QueryRun | null
  explain: ExplainResult | null
  error: AppError | null
  running: boolean
  /** Which result set is showing, when a run produced several. */
  activeResult: number
  /** Whether the lower pane shows rows or the query plan. */
  view: 'result' | 'plan'
}

interface EditorStore {
  tabs: QueryTab[]
  activeTabId: string | null

  openTab: (init?: Partial<Pick<QueryTab, 'name' | 'content' | 'savedPath'>>) => string
  closeTab: (id: string) => void
  setActive: (id: string) => void
  setContent: (id: string, content: string) => void
  setCursor: (id: string, cursor: number) => void
  setActiveResult: (id: string, index: number) => void
  setView: (id: string, view: 'result' | 'plan') => void
  rename: (id: string, name: string) => void
  /**
   * Follow a saved query that was renamed or deleted in the sidebar.
   *
   * Pass null for a deletion. Any open tab pointing at that file is retargeted,
   * or — for a deletion — detached from the path and marked dirty, so ⌘S offers
   * to save it somewhere new instead of failing against a file that is gone.
   */
  reconcileSavedPath: (oldPath: string, next: { path: string; name: string } | null) => void

  /** Run the whole buffer, or `sql` when given (selection / statement at cursor). */
  run: (id: string, sql?: string) => Promise<void>
  /** EXPLAIN the given statement; `analyze` executes it (rolled back). */
  explainQuery: (id: string, sql: string, analyze: boolean) => Promise<void>
  /**
   * Write the tab to disk. Saves in place when the tab came from a file;
   * otherwise (or with `saveAs`) asks for a name. Resolves true on success.
   */
  save: (id: string, saveAs?: boolean) => Promise<boolean>
  cancel: (id: string) => Promise<void>

  activeTab: () => QueryTab | null
  reset: () => void
}

/**
 * Save under a new name, asking before destroying an existing query.
 *
 * The backend refuses a name already in use rather than overwriting silently,
 * so a plain "save as" onto an existing name would otherwise just fail. Returns
 * null when the user declines the overwrite, which the caller must treat as a
 * cancel rather than an error.
 *
 * @param name - `string` — the file name to save under, without a directory or
 *   extension; the backend resolves it inside the saved-queries directory.
 * @param content - `string` — the full SQL text to write.
 * @returns `Promise<SavedQuery | null>` — the saved file's path and name, or
 *   `null` when the user declined the overwrite.
 */
async function saveUnderName(name: string, content: string) {
  try {
    return await ipc.saveNamedQuery(name, content)
  } catch (e) {
    if (!isAppError(e) || e.code !== 'exists') throw e
    const ok = await confirmAction({
      title: 'Replace saved query',
      message: `“${name}” already exists. Replace its contents?`,
      confirmLabel: 'Replace',
      danger: true,
    })
    if (!ok) return null
    return await ipc.saveNamedQuery(name, content, true)
  }
}

let untitledCounter = 0

/**
 * `query 1`, `query 2`, … skipping names already taken.
 *
 * @param tabs - `QueryTab[]` — the currently open tabs, read only to collect
 *   the names already in use.
 * @returns `string` — the first free `query N` name. Advances a module-level
 *   counter, so repeated calls never return the same name.
 */
function nextName(tabs: QueryTab[]): string {
  const taken = new Set(tabs.map((t) => t.name))
  do {
    untitledCounter += 1
  } while (taken.has(`query ${untitledCounter}`))
  return `query ${untitledCounter}`
}

/**
 * A blank tab, or one seeded from `init` when opening a saved query.
 *
 * `tabs` is only read to pick a free untitled name. The cursor starts at the
 * end of any seeded content so opening a saved query leaves the caret where
 * someone would continue typing, not in front of their own SQL.
 *
 * @param tabs - `QueryTab[]` — the currently open tabs, used only to pick a
 *   free untitled name.
 * @param init - `Partial<QueryTab> | undefined` — seed values; `name`,
 *   `content` and `savedPath` are the fields callers actually pass. Omitted
 *   fields fall back to blank-tab defaults.
 * @returns `QueryTab` — a fresh tab with a new UUID, `dirty` false and the
 *   cursor at the end of any seeded content.
 */
function newTab(tabs: QueryTab[], init?: Partial<QueryTab>): QueryTab {
  return {
    id: crypto.randomUUID(),
    name: init?.name ?? nextName(tabs),
    content: init?.content ?? '',
    cursor: init?.content?.length ?? 0,
    dirty: false,
    savedPath: init?.savedPath ?? null,
    run: null,
    explain: null,
    error: null,
    running: false,
    activeResult: 0,
    view: 'result',
  }
}

export const useEditor = create<EditorStore>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeTabId: null,

      openTab: (init) => {
        // Reopening a saved query focuses the existing tab rather than
        // stacking duplicates of the same file.
        if (init?.savedPath) {
          const existing = get().tabs.find((t) => t.savedPath === init.savedPath)
          if (existing) {
            set({ activeTabId: existing.id })
            return existing.id
          }
        }

        const tab = newTab(get().tabs, init)
        set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }))
        return tab.id
      },

      closeTab: (id) =>
        set((s) => {
          const index = s.tabs.findIndex((t) => t.id === id)
          const tabs = s.tabs.filter((t) => t.id !== id)
          if (s.activeTabId !== id) return { tabs, activeTabId: s.activeTabId }
          // Focus the neighbour on the left, or the new first tab.
          const next = tabs[Math.max(0, index - 1)] ?? null
          return { tabs, activeTabId: next?.id ?? null }
        }),

      setActive: (activeTabId) => set({ activeTabId }),

      setContent: (id, content) =>
        set((s) => ({
          tabs: s.tabs.map((t) => (t.id === id ? { ...t, content, dirty: true } : t)),
        })),

      setCursor: (id, cursor) =>
        set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, cursor } : t)) })),

      setActiveResult: (id, activeResult) =>
        set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, activeResult } : t)) })),

      setView: (id, view) =>
        set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, view } : t)) })),

      rename: (id, name) =>
        set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, name } : t)) })),

      reconcileSavedPath: (oldPath, next) =>
        set((s) => ({
          tabs: s.tabs.map((t) => {
            if (t.savedPath !== oldPath) return t
            return next
              ? { ...t, savedPath: next.path, name: next.name }
              : // The tab keeps its content — losing unsaved work because the
                // file was deleted elsewhere would be the worse outcome.
                { ...t, savedPath: null, dirty: true }
          }),
        })),

      run: async (id, sql) => {
        const tab = get().tabs.find((t) => t.id === id)
        if (!tab) return
        const source = (sql ?? tab.content).trim()
        if (!source) return

        set((s) => ({
          tabs: s.tabs.map((t) => (t.id === id ? { ...t, running: true, error: null } : t)),
        }))

        try {
          const run = await ipc.runQuery(source)
          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.id === id
              ? { ...t, run, running: false, error: null, activeResult: 0, view: 'result' }
              : t,
            ),
          }))
        } catch (e) {
          // Keep the previous result visible; the error shows in the footer.
          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.id === id ? { ...t, running: false, error: toAppError(e) } : t,
            ),
          }))
        }
      },

      explainQuery: async (id, sql, analyze) => {
        const source = sql.trim()
        if (!source) return

        set((s) => ({
          tabs: s.tabs.map((t) => (t.id === id ? { ...t, running: true, error: null } : t)),
        }))

        try {
          const explain = await ipc.explainQuery(source, analyze)
          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.id === id ? { ...t, explain, running: false, error: null, view: 'plan' } : t,
            ),
          }))
        } catch (e) {
          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.id === id ? { ...t, running: false, error: toAppError(e) } : t,
            ),
          }))
        }
      },

      save: async (id, saveAs = false) => {
        const tab = get().tabs.find((t) => t.id === id)
        if (!tab) return false

        try {
          let saved
          if (tab.savedPath && !saveAs) {
            saved = await ipc.saveQueryAt(tab.savedPath, tab.content)
          } else {
            const suggestion = tab.savedPath ? `${tab.name}_copy` : tab.name
            const name = await promptForName('Save query as', suggestion, 'File name')
            if (!name) return false
            saved = await saveUnderName(name, tab.content)
            // Cancelled at the overwrite confirmation.
            if (!saved) return false
          }

          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.id === id
                ? { ...t, dirty: false, savedPath: saved.path, name: saved.name, error: null }
                : t,
            ),
          }))
          return true
        } catch (e) {
          set((s) => ({
            tabs: s.tabs.map((t) => (t.id === id ? { ...t, error: toAppError(e) } : t)),
          }))
          return false
        }
      },

      cancel: async (id) => {
        try {
          await ipc.cancelQuery()
        } catch {
          /* the statement may have finished on its own */
        }
        set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, running: false } : t)) }))
      },

      activeTab: () => {
        const { tabs, activeTabId } = get()
        return tabs.find((t) => t.id === activeTabId) ?? null
      },

      reset: () => set({ tabs: [], activeTabId: null }),
    }),
    {
      name: 'pgscope.editor',
      // Persist the text, not the results — reopening the app should restore
      // what you were writing, but re-running is the user's call.
      partialize: (s) => ({
        tabs: s.tabs.map((t) => ({
          ...t,
          run: null,
          explain: null,
          error: null,
          running: false,
        })),
        activeTabId: s.activeTabId,
      }),
    },
  ),
)

/**
 * Result sets from a run, paired with the statement that produced each.
 *
 * @param run - `QueryRun | null` — a completed run; `null` before anything has
 *   been executed in the tab.
 * @returns `(StatementResult & { index: number })[]` — only the statements that
 *   returned rows. `index` is the statement's position in the original run,
 *   not in the filtered array, so it still identifies the statement after the
 *   row-less ones are dropped. Empty when `run` is `null`.
 */
export function resultSets(run: QueryRun | null) {
  if (!run) return []
  return run.statements
    .map((s, i) => ({ ...s, index: i }))
    .filter((s) => s.result !== null)
}

/**
 * Footer summary for a run with no result sets — `INSERT`/`UPDATE`/`SET` and
 * friends return no rows, so the grid would otherwise show nothing at all.
 *
 * @param run - `QueryRun | null` — a completed run; `null` before anything has
 *   been executed in the tab.
 * @returns `string | null` — a line like `3 statements executed`, or `null`
 *   when there is nothing to summarise: either no run yet, or at least one
 *   statement returned rows and the grid already shows them.
 */
export function runSummary(run: QueryRun | null): string | null {
  if (!run) return null
  const withRows = run.statements.filter((s) => s.result !== null).length
  if (withRows > 0) return null
  const n = run.statements.length
  return `${n} statement${n === 1 ? '' : 's'} executed`
}
