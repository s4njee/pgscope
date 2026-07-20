import { useEffect, useRef } from 'react'

import { useConnection } from '../state/connection'
import { useEditor } from '../state/editor'
import { useExplorer } from '../state/explorer'
import { useGrid } from '../state/grid'
import { useUi } from '../state/ui'

/** Breadcrumb `db ▸ schema ▸ table`, with the table segment emphasised.
 *
 * Takes no arguments — the database name comes from the connection store and the
 * schema/table from the explorer selection.
 *
 * @returns `JSX.Element` — the breadcrumb row; an em dash placeholder when nothing is
 *   connected or selected, so no segments exist.
 */
function Breadcrumb() {
  const info = useConnection((s) => s.info)
  const selected = useExplorer((s) => s.selected)

  const segments = [info?.database, selected?.schema, selected?.table].filter(
    (s): s is string => Boolean(s),
  )

  if (segments.length === 0) {
    return <div className="breadcrumb">—</div>
  }

  return (
    <div className="breadcrumb">
      {segments.map((seg, i) => {
        const isLast = i === segments.length - 1
        return (
          <span key={`${seg}-${i}`} style={{ display: 'contents' }}>
            <span className={`breadcrumb__seg${isLast ? ' breadcrumb__seg--current' : ''}`}>
              {seg}
            </span>
            {!isLast && <span className="breadcrumb__sep">▸</span>}
          </span>
        )
      })}
    </div>
  )
}

/** The bar under the titlebar: view tabs, query tabs, breadcrumb, and grid filter.
 *
 * Takes no arguments — every piece of state comes from the UI, editor, explorer, and grid
 * stores.
 *
 * @returns `JSX.Element` — the toolbar row. The filter box and Refresh button are rendered
 *   only while the `data` view is active.
 */
export function Toolbar() {
  const { activeTab, setTab } = useUi()
  const tabs = useEditor((s) => s.tabs)
  const activeTabId = useEditor((s) => s.activeTabId)
  const openTab = useEditor((s) => s.openTab)
  const closeTab = useEditor((s) => s.closeTab)
  const setActive = useEditor((s) => s.setActive)
  const selected = useExplorer((s) => s.selected)
  const reloadMeta = useExplorer((s) => s.reloadMeta)
  const { filterDraft, setFilterDraft, applyFilter, load, loading } = useGrid()
  const filterRef = useRef<HTMLInputElement>(null)

  // ⌘F focuses the filter box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
        e.preventDefault()
        filterRef.current?.focus()
        filterRef.current?.select()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const refresh = () => {
    if (!selected) return
    void load(selected.schema, selected.table)
    void reloadMeta()
  }

  const onFilterKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (!selected) return
    if (e.key === 'Enter') {
      void applyFilter(selected.schema, selected.table, filterDraft)
    } else if (e.key === 'Escape') {
      setFilterDraft('')
      void applyFilter(selected.schema, selected.table, '')
      filterRef.current?.blur()
    }
  }

  // `+ New query` opens an editor tab. The terminal remains the place for
  // psql muscle memory; this is where longer work lives.
  const newQuery = () => {
    openTab()
    setTab('query')
  }

  return (
    <div className="toolbar">
      <Breadcrumb />

      <div className="tabgroup">
        <button
          className={`tab${activeTab === 'data' ? ' tab--active' : ''}`}
          onClick={() => setTab('data')}
        >
          Data
        </button>
        <button
          className={`tab${activeTab === 'relationships' ? ' tab--active' : ''}`}
          onClick={() => setTab('relationships')}
        >
          Relationships
        </button>

        {tabs.map((t) => {
          const isActive = activeTab === 'query' && activeTabId === t.id
          return (
            <div
              key={t.id}
              className={`tab tab--query${isActive ? ' tab--active' : ''}`}
              onClick={() => {
                setActive(t.id)
                setTab('query')
              }}
              title={t.name}
            >
              <span className="tab__label">{t.name}</span>
              {t.dirty && <span className="tab__dirty" title="Unsaved changes">•</span>}
              <span
                className="tab__close"
                title="Close tab"
                onClick={(e) => {
                  // Don't let the close click also select the tab.
                  e.stopPropagation()
                  closeTab(t.id)
                  // Closing the last query tab has to leave `query` mode, or
                  // the main area would render nothing.
                  if (useEditor.getState().tabs.length === 0) setTab('data')
                }}
              >
                ✕
              </span>
            </div>
          )
        })}
      </div>

      <div className="spacer" />

      {/* The filter box and Refresh act on the data grid, so they are hidden
          when the main area is showing something else. */}
      {activeTab === 'data' && (
        <>
          <div className="filter-box">
            <span className="filter-box__icon">⌕</span>
            <input
              ref={filterRef}
              className="filter-box__input"
              placeholder="WHERE event_name = …"
              value={filterDraft}
              disabled={!selected}
              onChange={(e) => setFilterDraft(e.target.value)}
              onKeyDown={onFilterKey}
              spellCheck={false}
            />
          </div>

          <button className="btn-ghost" onClick={refresh} disabled={!selected || loading}>
            ↻ Refresh
          </button>
        </>
      )}
      <button className="btn-accent" onClick={newQuery}>
        + New query
      </button>
    </div>
  )
}

export { Breadcrumb }
