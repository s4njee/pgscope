import { create } from 'zustand'

import { ipc, toAppError } from '../lib/ipc'
import type { AppError, ConnectionInfo, ConnectionState } from '../lib/types'

interface ConnectionStore {
  state: ConnectionState
  info: ConnectionInfo | null
  latencyMs: number | null
  error: AppError | null
  /** Whether the connect modal is showing. */
  modalOpen: boolean

  setStatus: (state: ConnectionState, latencyMs?: number | null) => void
  openModal: () => void
  closeModal: () => void
  connect: (profileId: string, password?: string) => Promise<boolean>
  /** Try PGSCOPE_DEV_URL; returns true if it connected. */
  connectDev: () => Promise<boolean>
  disconnect: () => Promise<void>
}

export const useConnection = create<ConnectionStore>((set) => ({
  state: 'disconnected',
  info: null,
  latencyMs: null,
  error: null,
  modalOpen: false,

  setStatus: (state, latencyMs) =>
    set((s) => ({ state, latencyMs: latencyMs ?? (state === 'connected' ? s.latencyMs : null) })),

  openModal: () => set({ modalOpen: true }),
  closeModal: () => set({ modalOpen: false }),

  connect: async (profileId, password) => {
    set({ state: 'connecting', error: null })
    try {
      const info = await ipc.connect(profileId, password)
      set({ state: 'connected', info, error: null, modalOpen: false })
      return true
    } catch (e) {
      set({ state: 'disconnected', error: toAppError(e), info: null })
      return false
    }
  },

  connectDev: async () => {
    set({ state: 'connecting', error: null })
    try {
      const info = await ipc.connectDevUrl()
      if (!info) {
        set({ state: 'disconnected' })
        return false
      }
      set({ state: 'connected', info, error: null, modalOpen: false })
      return true
    } catch (e) {
      set({ state: 'disconnected', error: toAppError(e) })
      return false
    }
  },

  disconnect: async () => {
    try {
      await ipc.disconnect()
    } finally {
      set({ state: 'disconnected', info: null, latencyMs: null })
    }
  },
}))

/**
 * Titlebar text: `pgscope — analytics_prod@localhost:5432`.
 *
 * @param info - `ConnectionInfo | null` — the live connection's database, host
 *   and port; `null` means no session is open.
 * @returns `string` — the window title, falling back to `pgscope — not
 *   connected` when `info` is `null`.
 */
export function windowTitle(info: ConnectionInfo | null): string {
  if (!info) return 'pgscope — not connected'
  return `pgscope — ${info.database}@${info.host}:${info.port}`
}
