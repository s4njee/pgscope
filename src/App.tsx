import { useEffect, useState } from 'react'

import { ConnectModal } from './components/ConnectModal'
import { PromptModal } from './components/PromptModal'
import { DataGrid } from './components/DataGrid'
import { DetailsPanel } from './components/DetailsPanel'
import { QueryEditor } from './components/QueryEditor'
import { Relationships } from './components/Relationships'
import { Sidebar } from './components/Sidebar'
import { Terminal } from './components/Terminal'
import { Titlebar } from './components/Titlebar'
import { Toolbar } from './components/Toolbar'
import { ipc, onConnectionStatus } from './lib/ipc'
import { useConnection } from './state/connection'
import { useEditor } from './state/editor'
import { useExplorer } from './state/explorer'
import { useUi } from './state/ui'

export default function App() {
  const [platform, setPlatform] = useState('macos')
  const { activeTab, theme, showDetails, toggleDetails, setTab } = useUi()
  const connState = useConnection((s) => s.state)
  const setStatus = useConnection((s) => s.setStatus)
  const openModal = useConnection((s) => s.openModal)
  const loadTree = useExplorer((s) => s.loadTree)
  const queryTab = useEditor((s) => s.tabs.find((t) => t.id === s.activeTabId) ?? null)
  const openTab = useEditor((s) => s.openTab)

  useEffect(() => {
    ipc.platform().then(setPlatform).catch(() => setPlatform('macos'))
  }, [])

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document.documentElement.style.colorScheme = theme === 'light' ? 'light' : 'dark'
  }, [theme])

  // Live connection status from the backend pinger.
  useEffect(() => {
    const unlisten = onConnectionStatus((s) => setStatus(s.state, s.latencyMs))
    return () => {
      void unlisten.then((fn) => fn())
    }
  }, [setStatus])

  // On launch: reuse an existing connection, else try PGSCOPE_DEV_URL, else
  // show the connect modal.
  useEffect(() => {
    let cancelled = false
    const boot = async () => {
      try {
        const existing = await ipc.connectionInfo()
        if (cancelled) return
        if (existing) {
          setStatus('connected')
          return
        }
      } catch {
        /* fall through to the dev URL / modal */
      }

      const connected = await useConnection.getState().connectDev()
      if (!cancelled && !connected) openModal()
    }
    void boot()
    return () => {
      cancelled = true
    }
  }, [setStatus, openModal])

  // Load the tree whenever a connection comes up.
  useEffect(() => {
    if (connState === 'connected') void loadTree()
  }, [connState, loadTree])

  // ⌘I details, ⌘1/⌘2 tabs. (⌘R/⌘F live in Toolbar, ⌘K/⌘J/⌘T in Terminal.)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return
      if (e.key === 'n') {
        e.preventDefault()
        openTab()
        setTab('query')
      } else if (e.key === 'i') {
        e.preventDefault()
        toggleDetails()
      } else if (e.key === '1') {
        e.preventDefault()
        setTab('data')
      } else if (e.key === '2') {
        e.preventDefault()
        setTab('relationships')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggleDetails, setTab, openTab])

  return (
    <div className="window" data-theme={theme}>
      <Titlebar platform={platform} />
      <Toolbar />

      <div className="body-row">
        <Sidebar />
        <div className="main">
          {activeTab === 'data' && (
            <>
              <DataGrid />
              {showDetails && <DetailsPanel />}
            </>
          )}
          {activeTab === 'relationships' && <Relationships />}
          {activeTab === 'query' &&
            (queryTab ? (
              // Keyed so switching tabs rebuilds the editor with that tab's
              // document rather than reusing the previous one's state.
              <QueryEditor key={queryTab.id} tab={queryTab} />
            ) : (
              <div className="grid-pane">
                <div className="grid-state">no query tab open</div>
              </div>
            ))}
        </div>
      </div>

      <Terminal />
      <ConnectModal />
      <PromptModal />
    </div>
  )
}
