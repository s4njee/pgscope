import { create } from 'zustand'

/**
 * A promise-based text prompt rendered in-app.
 *
 * `window.prompt` is unavailable in a Tauri webview — WKWebView only shows a
 * prompt if the host implements the text-input panel delegate, which wry does
 * not, so it returns null immediately and the caller silently does nothing.
 * This replaces it with a real modal styled from the design tokens.
 */
interface PromptState {
  open: boolean
  title: string
  label: string
  value: string
  /** Resolves with the entered text, or null when cancelled. */
  resolve: ((value: string | null) => void) | null

  ask: (opts: { title: string; label?: string; defaultValue?: string }) => Promise<string | null>
  setValue: (v: string) => void
  confirm: () => void
  cancel: () => void
}

export const usePrompt = create<PromptState>((set, get) => ({
  open: false,
  title: '',
  label: 'Name',
  value: '',
  resolve: null,

  ask: ({ title, label = 'Name', defaultValue = '' }) =>
    new Promise<string | null>((resolve) => {
      // A second prompt while one is open cancels the first rather than
      // stranding its promise unresolved. A confirm dialog is dismissed for
      // the same reason: only one modal is ever mounted, so the one being
      // replaced would otherwise never hear back.
      get().resolve?.(null)
      useConfirm.getState().cancel()
      set({ open: true, title, label, value: defaultValue, resolve })
    }),

  setValue: (value) => set({ value }),

  confirm: () => {
    const { resolve, value } = get()
    const trimmed = value.trim()
    set({ open: false, resolve: null, value: '' })
    resolve?.(trimmed === '' ? null : trimmed)
  },

  cancel: () => {
    const { resolve } = get()
    set({ open: false, resolve: null, value: '' })
    resolve?.(null)
  },
}))

/**
 * Convenience wrapper mirroring the shape of `window.prompt`.
 *
 * @param title - `string` — the modal's heading.
 * @param defaultValue - `string` — text the field starts with, pre-selected so
 *   typing replaces it. Defaults to `''` for an empty field.
 * @param label - `string` — the field's caption. Defaults to `Name`.
 * @returns `Promise<string | null>` — the trimmed entry, or `null` when the
 *   user cancelled, submitted only whitespace, or another modal displaced this
 *   one.
 */
export function promptForName(
  title: string,
  defaultValue = '',
  label = 'Name',
): Promise<string | null> {
  return usePrompt.getState().ask({ title, defaultValue, label })
}

export interface ConfirmOptions {
  title: string
  message: string
  /** Label for the affirmative button. Defaults to `Confirm`. */
  confirmLabel?: string
  /**
   * Marks the action as destructive: the confirm button turns red and focus
   * starts on Cancel instead.
   */
  danger?: boolean
}

/**
 * The yes/no counterpart to `usePrompt`. `window.confirm` is as unusable as
 * `window.prompt` under wry, for the same reason.
 */
interface ConfirmState {
  open: boolean
  title: string
  message: string
  confirmLabel: string
  danger: boolean
  /** Resolves true when confirmed, false when cancelled or displaced. */
  resolve: ((value: boolean) => void) | null

  ask: (opts: ConfirmOptions) => Promise<boolean>
  accept: () => void
  cancel: () => void
}

export const useConfirm = create<ConfirmState>((set, get) => ({
  open: false,
  title: '',
  message: '',
  confirmLabel: 'Confirm',
  danger: false,
  resolve: null,

  ask: ({ title, message, confirmLabel = 'Confirm', danger = false }) =>
    new Promise<boolean>((resolve) => {
      // Mirrors `usePrompt.ask`: whatever was on screen resolves negatively
      // rather than hanging. Declining is the safe answer for a question the
      // user never got to see.
      get().resolve?.(false)
      usePrompt.getState().cancel()
      set({ open: true, title, message, confirmLabel, danger, resolve })
    }),

  accept: () => {
    const { resolve } = get()
    set({ open: false, resolve: null })
    resolve?.(true)
  },

  cancel: () => {
    const { resolve } = get()
    set({ open: false, resolve: null })
    resolve?.(false)
  },
}))

/**
 * Convenience wrapper mirroring the shape of `window.confirm`.
 *
 * @param opts - `ConfirmOptions` — the dialog's `title` and `message`, plus the
 *   optional `confirmLabel` (default `Confirm`) and `danger` flag.
 * @returns `Promise<boolean>` — true only when the user confirmed. Cancelling,
 *   or being displaced by another modal, both resolve false.
 */
export function confirmAction(opts: ConfirmOptions): Promise<boolean> {
  return useConfirm.getState().ask(opts)
}
