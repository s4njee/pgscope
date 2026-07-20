import { useEffect, useMemo, useState } from 'react'

import { ContextMenu, type MenuItem } from './ContextMenu'
import { ipc } from '../lib/ipc'
import { compactCount, historyKeyword, oneLine, relativeAge } from '../lib/format'
import {
  buildSavedMenu,
  duplicateName,
  type SavedMenuActions,
  type SavedMenuTarget,
} from '../lib/savedMenu'
import {
  buildSavedTree,
  folderOf,
  folderPaths,
  leafOf,
  nameInFolder,
  type TreeNode,
} from '../lib/savedTree'
import type { HistoryItem, Relation, SavedQuery, SchemaNode } from '../lib/types'
import { useConnection } from '../state/connection'
import { useEditor } from '../state/editor'
import { useExplorer } from '../state/explorer'
import { confirmAction, promptForName } from '../state/prompt'
import { useTerminal } from '../state/terminal'
import { useUi } from '../state/ui'

/**
 * The disclosure triangle shared by every collapsible row in the sidebar.
 *
 * @param props - `{ open: boolean }`
 *   - `open` — whether the owning row is expanded; picks `▾` over `▸`.
 * @returns `JSX.Element` — the caret span.
 */
function Caret({ open }: { open: boolean }) {
  return <span className="tree-caret">{open ? '▾' : '▸'}</span>
}

/**
 * One table or view in the database tree; selecting it loads it into the grid.
 *
 * @param props - `{ schema: string; rel: Relation }`
 *   - `schema` — the owning schema name, used both to select and for the
 *     `schema.table` tooltip.
 *   - `rel` — the table or view; its `estRows` is `reltuples`, where -1 means
 *     never analyzed and renders as an empty count.
 * @returns `JSX.Element` — the clickable row, marked selected when it matches
 *   the explorer's current selection.
 */
function TableRow({ schema, rel }: { schema: string; rel: Relation }) {
  const { selected, select } = useExplorer()
  const isSelected = selected?.schema === schema && selected?.table === rel.name

  return (
    <button
      className={`tree-row tree-row--table${isSelected ? ' tree-row--selected' : ''}`}
      onClick={() => void select(schema, rel.name)}
      title={`${schema}.${rel.name}`}
    >
      <span className="tree-square" />
      <span className="tree-label">{rel.name}</span>
      <span className="tree-count">{compactCount(rel.estRows)}</span>
    </button>
  )
}

/**
 * One schema and its relations. `public` starts expanded and views start
 * collapsed, since views are the rarer thing to be looking for.
 *
 * @param props - `{ node: SchemaNode }`
 *   - `node` — one schema and its relations; `node.name` doubles as the
 *     expand-state key, and `${node.name}/views` keys the nested views group.
 * @returns `JSX.Element` — the schema's header row, plus its table rows and the
 *   views group when expanded. The views group is omitted entirely when the
 *   schema has no views.
 */
function SchemaSection({ node }: { node: SchemaNode }) {
  const { isExpanded, toggleNode } = useExplorer()
  const open = isExpanded(node.name, node.name === 'public')
  const viewsKey = `${node.name}/views`
  const viewsOpen = isExpanded(viewsKey, false)

  return (
    <>
      <button className="tree-row tree-row--schema" onClick={() => toggleNode(node.name)}>
        <Caret open={open} />
        <span className="tree-label">{node.name}</span>
        <span className="tree-count">
          {open ? `${node.tables.length} tables` : 'schema'}
        </span>
      </button>

      {open && (
        <>
          {node.tables.map((t) => (
            <TableRow key={t.name} schema={node.name} rel={t} />
          ))}

          {node.views.length > 0 && (
            <>
              <button className="tree-row tree-row--group" onClick={() => toggleNode(viewsKey)}>
                <Caret open={viewsOpen} />
                <span className="tree-label">views</span>
                <span className="tree-count">{node.views.length}</span>
              </button>
              {viewsOpen &&
                node.views.map((v) => (
                  <TableRow key={v.name} schema={node.name} rel={v} />
                ))}
            </>
          )}
        </>
      )}
    </>
  )
}

/**
 * The sidebar's DATABASE section: connected database, then a section per schema.
 *
 * Takes no arguments; the `ConnectionInfo | null` and the `SchemaNode[]` tree
 * come from the connection and explorer stores.
 *
 * @returns `JSX.Element` — the DATABASE header and tree. The database row shows
 *   `—` when not connected, and a loading or error line stands in while the
 *   tree is still empty.
 */
function DatabaseTree() {
  const info = useConnection((s) => s.info)
  const { tree, loading, error } = useExplorer()

  return (
    <>
      <div className="section-header">DATABASE</div>
      <div className="tree">
        <div className="tree-row tree-row--db">
          <Caret open />
          <span className="tree-diamond">◆</span>
          <span className="tree-label">{info?.database ?? '—'}</span>
        </div>

        {loading && tree.length === 0 && <div className="sidebar__empty">loading…</div>}
        {error && <div className="sidebar__empty">{error.message}</div>}
        {tree.map((node) => (
          <SchemaSection key={node.name} node={node} />
        ))}
      </div>
    </>
  )
}

/**
 * The SAVED QUERIES folder tree, and every mutation reachable from its menu.
 *
 * The file list is local state refetched off a `savedQueriesVersion` counter
 * rather than a store, because the backend's directory is the source of truth —
 * every mutation goes there and is read back, so the tree cannot drift from
 * what is on disk. Renames and deletes also have to tell the editor, or an open
 * tab would keep saving to a path that has moved or gone.
 *
 * Takes no arguments; the `SavedQuery[]` list and `string[]` of folder paths
 * are fetched over IPC and refetched whenever the connection state or the
 * `savedQueriesVersion` counter changes.
 *
 * @returns `JSX.Element` — the SAVED QUERIES section: the folder tree, an
 *   "none" placeholder when empty, an inline error line when the last mutation
 *   failed, and the context menu while one is open.
 */
function SavedQueries() {
  const [items, setItems] = useState<SavedQuery[]>([])
  const [folderList, setFolderList] = useState<string[]>([])
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const openTab = useEditor((s) => s.openTab)
  const setTab = useUi((s) => s.setTab)
  const connState = useConnection((s) => s.state)
  const savedQueriesVersion = useUi((s) => s.savedQueriesVersion)

  useEffect(() => {
    ipc.savedQueries().then(setItems).catch(() => setItems([]))
    ipc.savedFolders().then(setFolderList).catch(() => setFolderList([]))
  }, [connState, savedQueriesVersion])

  const tree = useMemo(() => buildSavedTree(items, folderList), [items, folderList])
  const folders = useMemo(() => folderPaths(tree), [tree])

  const refresh = () => useUi.getState().bumpSavedQueries()

  /** Run a mutation, surfacing its error inline rather than throwing it away. */
  const attempt = async (fn: () => Promise<void>) => {
    setError(null)
    try {
      await fn()
      refresh()
    } catch (e) {
      const err = e as { message?: string }
      setError(err.message ?? String(e))
    }
  }

  // Saved queries open in an editor tab rather than the terminal prompt: they
  // are usually multi-line, which the single-line prompt handles badly.
  // Reopening the same file focuses its existing tab (see `openTab`).
  const openQuery = (q: SavedQuery) => {
    openTab({ name: leafOf(q.name), content: q.content.trimEnd(), savedPath: q.path })
    setTab('query')
  }

  const renameTo = (query: SavedQuery, newName: string) =>
    attempt(async () => {
      const saved = await ipc.renameSavedQuery(query.path, newName)
      // An open tab pointing at the old file has to follow it, or ⌘S would
      // write to a path that no longer exists.
      useEditor.getState().reconcileSavedPath(query.path, {
        path: saved.path,
        name: leafOf(saved.name),
      })
    })

  const actions: SavedMenuActions = {
    open: openQuery,

    rename: async (query) => {
      const next = await promptForName('Rename query', leafOf(query.name), 'Name')
      if (!next) return
      // Renaming keeps the query in its folder; moving is a separate action, so
      // typing a bare name in a folder doesn't silently move it to the root.
      void renameTo(query, nameInFolder(next, folderOf(query.name)))
    },

    duplicate: (query) =>
      void attempt(async () => {
        await ipc.saveNamedQuery(duplicateName(query, items), query.content)
      }),

    remove: async (query) => {
      const ok = await confirmAction({
        title: 'Delete saved query',
        message: `Delete “${leafOf(query.name)}”? This cannot be undone.`,
        confirmLabel: 'Delete',
        danger: true,
      })
      if (!ok) return
      void attempt(async () => {
        await ipc.deleteSavedQuery(query.path)
        // Detach any open tab from the deleted file, keeping its content.
        useEditor.getState().reconcileSavedPath(query.path, null)
      })
    },

    move: (query, folder) => void renameTo(query, nameInFolder(query.name, folder)),

    newFolder: async () => {
      const name = await promptForName('New folder', '', 'Folder name')
      if (!name) return
      void attempt(async () => {
        await ipc.createSavedFolder(name)
      })
    },

    renameFolder: async (path) => {
      const next = await promptForName('Rename folder', leafOf(path), 'Folder name')
      if (!next) return
      void attempt(async () => {
        // A real directory rename on the backend, which also works when the
        // folder is empty. It reports each query's old and new path so open
        // tabs can follow the files they were opened from.
        for (const moved of await ipc.renameSavedFolder(path, next)) {
          useEditor.getState().reconcileSavedPath(moved.from, {
            path: moved.query.path,
            name: leafOf(moved.query.name),
          })
        }
      })
    },

    copyName: (name) => {
      void navigator.clipboard.writeText(name).catch(() => {
        /* clipboard unavailable */
      })
    },
  }

  const openMenu = (e: React.MouseEvent, target: SavedMenuTarget) => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: buildSavedMenu(target, folders, actions).map((b) => ({
        label: b.label,
        hint: b.hint,
        disabled: b.disabled,
        separatorBefore: b.separatorBefore,
        onSelect: b.run,
      })),
    })
  }

  const toggleFolder = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })

  const renderNodes = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((node) => {
      // Indent from the existing 14px row padding, one step per level.
      const indent = { paddingLeft: 14 + depth * 12 }

      if (node.kind === 'folder') {
        const open = !collapsed.has(node.path)
        return (
          <div key={`folder:${node.path}`} style={{ display: 'contents' }}>
            <button
              className="tree-row tree-row--saved-folder"
              style={indent}
              onClick={() => toggleFolder(node.path)}
              onContextMenu={(e) => openMenu(e, { kind: 'folder', path: node.path })}
              title={node.path}
            >
              <Caret open={open} />
              <span className="tree-label">{node.label}</span>
              <span className="tree-count">{node.children.length}</span>
            </button>
            {open && renderNodes(node.children, depth + 1)}
          </div>
        )
      }

      return (
        <button
          key={node.query.path}
          className="tree-row"
          style={indent}
          onClick={() => openQuery(node.query)}
          onContextMenu={(e) => openMenu(e, { kind: 'query', query: node.query })}
          title={node.query.name}
        >
          <span className="saved-glyph">▪</span>
          <span className="tree-label">{node.label}</span>
          <span className="tree-count">.sql</span>
        </button>
      )
    })

  return (
    <>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />
      )}
      <div className="section-header section-header--tight">SAVED QUERIES</div>
      <div className="tree" onContextMenu={(e) => openMenu(e, { kind: 'background' })}>
        {tree.length === 0 && <div className="sidebar__empty">none</div>}
        {renderNodes(tree, 0)}
        {error && <div className="sidebar__error">{error}</div>}
      </div>
    </>
  )
}

/**
 * Recent terminal commands; clicking one recalls it into the prompt.
 *
 * Takes no arguments; the entries come from `ipc.historyList()` as
 * `HistoryItem[]`, whose `at` is Unix seconds.
 *
 * @returns `JSX.Element` — the HISTORY section: a header plus up to 20 rows,
 *   newest first, or an "none" placeholder when there is no history.
 */
function History() {
  const [items, setItems] = useState<HistoryItem[]>([])
  const [, setTick] = useState(0)
  const setInput = useTerminal((s) => s.setInput)
  const terminalHistory = useTerminal((s) => s.history)
  const setTerminalCollapsed = useUi((s) => s.setTerminalCollapsed)

  // Refetch when a command is submitted (terminalHistory grows).
  useEffect(() => {
    ipc.historyList().then(setItems).catch(() => setItems([]))
  }, [terminalHistory.length])

  // Re-render periodically so "· 2m" ages without interaction.
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 30_000)
    return () => clearInterval(id)
  }, [])

  const now = Date.now() / 1000

  const recall = (input: string) => {
    setInput(input)
    setTerminalCollapsed(false)
    requestAnimationFrame(() => {
      document.querySelector<HTMLInputElement>('.term__hidden-input')?.focus()
    })
  }

  // Newest first, as in the design.
  const shown = [...items].reverse().slice(0, 20)

  return (
    <>
      <div className="section-header section-header--tight">HISTORY</div>
      <div className="tree">
        {shown.length === 0 && <div className="sidebar__empty">none</div>}
        {shown.map((h, i) => {
          const { keyword, rest } = historyKeyword(h.input)
          return (
            <button
              key={`${h.at}-${i}`}
              className="history-row"
              onClick={() => recall(h.input)}
              title={h.input}
            >
              <span className="history-row__kw">{keyword}</span>
              {rest && ` ${oneLine(rest, 28)}`}{' '}
              <span className="history-row__age">· {relativeAge(now - h.at, 'short')}</span>
            </button>
          )
        })}
      </div>
    </>
  )
}

/**
 * The left rail: database tree, saved queries, and command history.
 *
 * Takes no arguments.
 *
 * @returns `JSX.Element` — the sidebar container holding the three sections,
 *   separated by dividers.
 */
export function Sidebar() {
  return (
    <div className="sidebar">
      <DatabaseTree />
      <div className="divider" />
      <SavedQueries />
      <div className="divider" />
      <History />
    </div>
  )
}
