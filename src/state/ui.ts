import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { clampTerminalHeight, TERM_BODY_DEFAULT } from '../lib/paneSize'

/**
 * Which surface fills the main area.
 *
 * `query` defers to the editor store for *which* tab is showing; the toolbar's
 * segmented control renders Data | Relationships followed by one entry per open
 * query tab, so "what's in the main area" stays a single control as in the
 * design.
 */
export type Tab = 'data' | 'relationships' | 'query'
export type Theme = 'dark' | 'black' | 'light'

/**
 * Window height, or the design's window height when there is no DOM.
 *
 * Takes no arguments.
 *
 * @returns `number` — `window.innerHeight` in px, falling back to the design's
 *   880px under SSR or tests where `window` is undefined.
 */
const viewportHeight = () =>
  typeof window === 'undefined' ? 880 : window.innerHeight

interface UiState {
  activeTab: Tab
  theme: Theme
  showDetails: boolean
  terminalCollapsed: boolean
  /** Height of `.term__body` in px; the header sits above it at a fixed height. */
  terminalHeight: number
  setTab: (t: Tab) => void
  setTheme: (theme: Theme) => void
  toggleDetails: () => void
  setDetails: (v: boolean) => void
  toggleTerminal: () => void
  setTerminalCollapsed: (v: boolean) => void
  setTerminalHeight: (px: number) => void
  /**
   * Bumped whenever the saved-queries directory changes, so the sidebar
   * refetches without polling.
   */
  savedQueriesVersion: number
  bumpSavedQueries: () => void
}

/**
 * View state, mirroring the design's mock state
 * (`activeTab` / `showDetails` / `terminalCollapsed`) with the same defaults.
 * Persisted so the app reopens the way it was left.
 */
export const useUi = create<UiState>()(
  persist(
    (set) => ({
      activeTab: 'data',
      theme: 'dark',
      showDetails: true,
      terminalCollapsed: false,
      terminalHeight: TERM_BODY_DEFAULT,
      setTab: (activeTab) => set({ activeTab }),
      setTheme: (theme) => set({ theme }),
      toggleDetails: () => set((s) => ({ showDetails: !s.showDetails })),
      setDetails: (showDetails) => set({ showDetails }),
      toggleTerminal: () => set((s) => ({ terminalCollapsed: !s.terminalCollapsed })),
      setTerminalCollapsed: (terminalCollapsed) => set({ terminalCollapsed }),
      setTerminalHeight: (px) =>
        set({ terminalHeight: clampTerminalHeight(px, viewportHeight()) }),
      savedQueriesVersion: 0,
      bumpSavedQueries: () => set((s) => ({ savedQueriesVersion: s.savedQueriesVersion + 1 })),
    }),
    {
      name: 'pgscope.ui',
      partialize: (s) => ({
        activeTab: s.activeTab,
        theme: s.theme,
        showDetails: s.showDetails,
        terminalCollapsed: s.terminalCollapsed,
        terminalHeight: s.terminalHeight,
      }),
      // The stored height came from whatever window the app was last closed in.
      // Re-clamp against the current one so a pane sized on an external display
      // cannot come back taller than a laptop screen allows.
      onRehydrateStorage: () => (state) => {
        if (state) state.terminalHeight = clampTerminalHeight(state.terminalHeight, viewportHeight())
      },
    },
  ),
)
