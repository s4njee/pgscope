import { useCallback, useEffect, useMemo, useRef } from 'react'
import { autocompletion, closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { bracketMatching, indentOnInput } from '@codemirror/language'
import { PostgreSQL, sql } from '@codemirror/lang-sql'
import { highlightSelectionMatches, search, searchKeymap } from '@codemirror/search'
import { Compartment, EditorState } from '@codemirror/state'
import {
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from '@codemirror/view'

import { pgscopeEditorTheme, pgscopeSyntax } from '../lib/editorTheme'
import { formatMs, groupDigits } from '../lib/format'
import { ipc } from '../lib/ipc'
import type { QueryResultSet, StatementResult } from '../lib/types'
import { PlanTree } from './PlanTree'
import { resultSets, runSummary, useEditor, type QueryTab } from '../state/editor'
import { useExplorer } from '../state/explorer'
import { useUi } from '../state/ui'

/**
 * Schema for CodeMirror's SQL autocomplete, derived from the explorer tree.
 *
 * Reuses introspection we have already fetched, so completion costs nothing
 * extra and stays correct as the tree refreshes.
 *
 * Takes no arguments — the tree and selected-table metadata are read from the
 * explorer store.
 *
 * @returns `Record<string, string[]>` — CodeMirror's SQL `schema` option: each
 *   key is a relation name, both bare and `schema.name`-qualified, mapped to its
 *   column names. Only the selected table has columns; every other relation maps
 *   to `[]`, which still completes the name itself. Memoised, so the identity
 *   changes only when the tree or selection does.
 */
function useSchemaCompletion() {
  const tree = useExplorer((s) => s.tree)
  const meta = useExplorer((s) => s.meta)

  return useMemo(() => {
    const schema: Record<string, string[]> = {}
    for (const node of tree) {
      for (const rel of [...node.tables, ...node.views]) {
        // Both bare and schema-qualified, so `events` and `public.events` complete.
        schema[rel.name] = []
        schema[`${node.name}.${rel.name}`] = []
      }
    }
    // Columns for the selected table — the one being explored is the one most
    // likely to be typed about.
    if (meta) {
      const cols = meta.columns.map((c) => c.name)
      schema[meta.name] = cols
      schema[`${meta.schema}.${meta.name}`] = cols
    }
    return schema
  }, [tree, meta])
}

/**
 * One result set as a plain grid. Deliberately not the browsing `DataGrid`:
 * an arbitrary SELECT has no table behind it, so there is no column metadata to
 * colour or size by, and no row identity to expand or filter on.
 *
 * @param props - `{ set: QueryResultSet }`
 *   - `set` — one statement's rows and column names; a `null` cell is SQL NULL
 *     and renders as the literal `NULL` in the faint colour.
 * @returns `JSX.Element` — the grid, or a placeholder for no columns / 0 rows.
 */
function ResultGrid({ set }: { set: QueryResultSet }) {
  if (set.columns.length === 0) {
    return <div className="qe-empty">no columns</div>
  }
  if (set.rows.length === 0) {
    return <div className="qe-empty">0 rows</div>
  }

  const template = `44px ${set.columns.map(() => 'minmax(110px, 1fr)').join(' ')}`

  return (
    <div className="qe-grid">
      <div className="grid-header" style={{ gridTemplateColumns: template }}>
        <div className="grid-header__num">#</div>
        {set.columns.map((col, i) => (
          <div className="grid-header__cell" key={`${col}-${i}`} title={col}>
            <div className="grid-header__name">{col}</div>
          </div>
        ))}
      </div>
      {set.rows.map((row, r) => (
        <div className="grid-row" key={r} style={{ gridTemplateColumns: template }}>
          <div className="grid-cell grid-cell--num">{r + 1}</div>
          {row.map((value, c) => (
            <div
              className="grid-cell"
              key={c}
              style={{ color: value === null ? 'var(--text-faint)' : 'var(--text-secondary)' }}
              title={value ?? 'NULL'}
            >
              {value === null ? 'NULL' : value}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}

/**
 * Everything below the editor: the error, the plan, or the result sets.
 *
 * The branches are ordered by what the user most needs to see — an error wins
 * over stale rows from the previous run, which are still in `tab.run` and would
 * otherwise read as this run's output.
 *
 * @param props - `{ tab: QueryTab }`
 *   - `tab` — the tab whose `error`, `explain`, `run`, `view` and `activeResult`
 *     pick the branch; `run`/`explain` are `null` until the tab has been run.
 * @returns `JSX.Element` — the results container: an error, a `PlanTree`, the
 *   keyboard hint when nothing has run yet, or the active `ResultGrid`.
 */
function ResultPane({ tab }: { tab: QueryTab }) {
  const setActiveResult = useEditor((s) => s.setActiveResult)
  const setView = useEditor((s) => s.setView)
  const sets = resultSets(tab.run)
  const summary = runSummary(tab.run)

  // Once a tab has both rows and a plan, let the user flip between them
  // instead of losing one to the other.
  const switcher =
    tab.run && tab.explain ? (
      <div className="qe-view-switch">
        <button
          className={`qe-result-tab${tab.view === 'result' ? ' qe-result-tab--active' : ''}`}
          onClick={() => setView(tab.id, 'result')}
        >
          rows
        </button>
        <button
          className={`qe-result-tab${tab.view === 'plan' ? ' qe-result-tab--active' : ''}`}
          onClick={() => setView(tab.id, 'plan')}
        >
          plan
        </button>
      </div>
    ) : null

  if (tab.view === 'plan' && tab.explain && !tab.error) {
    return (
      <div className="qe-results">
        {switcher}
        <PlanTree result={tab.explain} />
      </div>
    )
  }

  if (tab.error) {
    return (
      <div className="qe-results">
        <div className="qe-error">
          <div className="qe-error__msg">{tab.error.message}</div>
          {tab.error.detail && <div className="qe-error__detail">{tab.error.detail}</div>}
        </div>
      </div>
    )
  }

  if (!tab.run) {
    return (
      <div className="qe-results">
        <div className="qe-empty">
          ⌘↵ runs the statement at the cursor · ⌘⇧↵ runs everything · Explain shows the plan
        </div>
      </div>
    )
  }

  if (sets.length === 0) {
    return (
      <div className="qe-results">
        <div className="qe-empty">{summary}</div>
      </div>
    )
  }

  const active = sets[Math.min(tab.activeResult, sets.length - 1)]
  const set = active.result as QueryResultSet

  return (
    <div className="qe-results">
      {switcher}
      {sets.length > 1 && (
        <div className="qe-result-tabs">
          {sets.map((s: StatementResult & { index: number }, i: number) => (
            <button
              key={s.index}
              className={`qe-result-tab${i === tab.activeResult ? ' qe-result-tab--active' : ''}`}
              onClick={() => setActiveResult(tab.id, i)}
              title={s.sql}
            >
              result {i + 1}
            </button>
          ))}
        </div>
      )}
      <div className="qe-grid-scroll">
        <ResultGrid set={set} />
      </div>
      <div className="qe-result-footer">
        <span>
          {groupDigits(set.rows.length)} row{set.rows.length === 1 ? '' : 's'}
          {set.truncated && (
            <span className="qe-truncated"> · truncated from {groupDigits(set.totalRows)}</span>
          )}
        </span>
        <div className="spacer" />
        <span className="qe-timing">{formatMs(active.timingMs)}</span>
      </div>
    </div>
  )
}

/**
 * One query tab: CodeMirror, its toolbar, and the results below.
 *
 * CodeMirror owns the document; React only mounts it once per tab id and reads
 * changes back out through the update listener. Driving the text from props
 * would fight the editor's own transaction history and lose undo.
 *
 * @param props - `{ tab: QueryTab }`
 *   - `tab` — the tab to render; its `id` is the editor's identity, so a change
 *     there tears down and rebuilds CodeMirror, while `content` changes do not.
 * @returns `JSX.Element` — the toolbar, the CodeMirror host, and the result pane.
 */
export function QueryEditor({ tab }: { tab: QueryTab }) {
  const { setContent, setCursor, run, cancel, explainQuery, save } = useEditor()
  const hostRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const schemaCompartment = useRef(new Compartment())
  const schema = useSchemaCompletion()

  // Keep handlers in a ref so the (deliberately non-reactive) editor keymap
  // always calls the current versions without rebuilding the editor.
  const handlers = useRef({ run, cancel, explainQuery, save, setContent, setCursor, tabId: tab.id })
  handlers.current = { run, cancel, explainQuery, save, setContent, setCursor, tabId: tab.id }

  const saveTab = useCallback(async (saveAs: boolean) => {
    const ok = await handlers.current.save(handlers.current.tabId, saveAs)
    // A new file needs to show up in the sidebar's SAVED QUERIES list.
    if (ok) useUi.getState().bumpSavedQueries()
  }, [])

  const runAtCursor = useCallback(async () => {
    const view = viewRef.current
    if (!view) return
    const { from, to } = view.state.selection.main
    const doc = view.state.doc.toString()

    // A selection runs exactly what's selected; otherwise the statement the
    // cursor sits in, resolved by the same lexer the REPL uses.
    if (from !== to) {
      await handlers.current.run(handlers.current.tabId, doc.slice(from, to))
      return
    }
    const stmt = await ipc.statementAtCursor(doc, from).catch(() => null)
    await handlers.current.run(handlers.current.tabId, stmt?.text ?? doc)
  }, [])

  const runAll = useCallback(async () => {
    const view = viewRef.current
    if (!view) return
    await handlers.current.run(handlers.current.tabId, view.state.doc.toString())
  }, [])

  // EXPLAIN takes a single statement, so it always targets the selection or
  // the statement under the cursor — never the whole buffer.
  const explain = useCallback(async (analyze: boolean) => {
    const view = viewRef.current
    if (!view) return
    const { from, to } = view.state.selection.main
    const doc = view.state.doc.toString()

    let target: string
    if (from !== to) {
      target = doc.slice(from, to)
    } else {
      const stmt = await ipc.statementAtCursor(doc, from).catch(() => null)
      target = stmt?.text ?? doc
    }
    await handlers.current.explainQuery(handlers.current.tabId, target, analyze)
  }, [])

  // Build the editor once per tab.
  useEffect(() => {
    const host = hostRef.current
    if (!host) return

    const state = EditorState.create({
      doc: tab.content,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        history(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        search({ top: true }),
        highlightSelectionMatches(),
        schemaCompartment.current.of(
          sql({ dialect: PostgreSQL, schema, upperCaseKeywords: true }),
        ),
        autocompletion({ activateOnTyping: true, icons: false }),
        pgscopeEditorTheme,
        pgscopeSyntax,
        keymap.of([
          { key: 'Mod-Enter', run: () => (void runAtCursor(), true) },
          { key: 'Mod-Shift-Enter', run: () => (void runAll(), true) },
          { key: 'Mod-s', run: () => (void saveTab(false), true) },
          { key: 'Mod-Shift-s', run: () => (void saveTab(true), true) },
          { key: 'Mod-e', run: () => (void explain(false), true) },
          { key: 'Mod-Shift-e', run: () => (void explain(true), true) },
          ...closeBracketsKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...defaultKeymap,
          indentWithTab,
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            handlers.current.setContent(handlers.current.tabId, update.state.doc.toString())
          }
          if (update.selectionSet) {
            handlers.current.setCursor(
              handlers.current.tabId,
              update.state.selection.main.head,
            )
          }
        }),
      ],
    })

    const view = new EditorView({ state, parent: host })
    viewRef.current = view
    view.focus()

    return () => {
      view.destroy()
      viewRef.current = null
    }
    // Rebuilt only when the tab identity changes — content is pushed through
    // CodeMirror's own transactions, not by recreating the editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id])

  // Swap the completion schema in place as introspection arrives.
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: schemaCompartment.current.reconfigure(
        sql({ dialect: PostgreSQL, schema, upperCaseKeywords: true }),
      ),
    })
  }, [schema])

  return (
    <div className="qe">
      <div className="qe-toolbar">
        <button
          className="btn-accent"
          onClick={() => void runAtCursor()}
          disabled={tab.running}
          title="Run the statement at the cursor (⌘↵)"
        >
          ▶ Run
        </button>
        <button
          className="btn-ghost"
          onClick={() => void runAll()}
          disabled={tab.running}
          title="Run every statement in this tab (⌘⇧↵)"
        >
          ▶▶ Run all
        </button>
        <button
          className="btn-ghost"
          onClick={() => void saveTab(false)}
          disabled={!tab.dirty && tab.savedPath !== null}
          title={tab.savedPath ? 'Save to the .sql file (⌘S)' : 'Save as a new .sql file (⌘S)'}
        >
          ⇩ Save
        </button>
        <button
          className="btn-ghost"
          onClick={() => void explain(false)}
          disabled={tab.running}
          title="Show the planner's query plan without running the statement (⌘E)"
        >
          ⋔ Explain
        </button>
        <button
          className="btn-ghost"
          onClick={() => void explain(true)}
          disabled={tab.running}
          title="Run the statement and show the real plan, inside a rolled-back transaction (⌘⇧E)"
        >
          ⋔ Analyze
        </button>
        {tab.running && (
          <button
            className="btn-ghost"
            onClick={() => void cancel(tab.id)}
            title="Cancel the running statement"
          >
            ■ Cancel
          </button>
        )}
        <div className="spacer" />
        {tab.running && <span className="qe-status">running…</span>}
        {/* Only describe the last run when it actually succeeded — otherwise
            the counts and timing belong to a previous query and contradict the
            error showing below. */}
        {!tab.running && tab.error && <span className="qe-status qe-failed">failed</span>}
        {!tab.running && !tab.error && tab.run && (
          <span className="qe-status">
            {tab.run.statements.length} statement
            {tab.run.statements.length === 1 ? '' : 's'} ·{' '}
            <span className="qe-timing">{formatMs(tab.run.totalTimingMs)}</span>
          </span>
        )}
      </div>

      <div className="qe-editor" ref={hostRef} />

      <ResultPane tab={tab} />
    </div>
  )
}
