import { useEffect, useRef } from 'react'

import { useRestoreFocus } from '../lib/useRestoreFocus'
import { useConfirm, usePrompt } from '../state/prompt'

/**
 * The single mounted modal host. Only one of the two dialogs can be open at a
 * time — opening either dismisses the other (see `state/prompt.ts`).
 *
 * Takes no arguments — both dialogs read their own stores.
 *
 * @returns `JSX.Element` — a fragment holding both dialogs; each renders `null` while
 *   closed, so this is empty in the common case.
 */
export function PromptModal() {
  return (
    <>
      <TextPrompt />
      <ConfirmDialog />
    </>
  )
}

/**
 * The in-app replacement for `window.prompt`, which a Tauri webview does not
 * implement (see `state/prompt.ts`). Styled from the same tokens as the
 * connect modal.
 *
 * Takes no arguments — the title, label, and current value come from `usePrompt`.
 *
 * @returns `JSX.Element | null` — the dialog, or `null` while closed. Save is disabled
 *   for an all-whitespace value.
 */
function TextPrompt() {
  const { open, title, label, value, setValue, confirm, cancel } = usePrompt()
  const inputRef = useRef<HTMLInputElement>(null)

  useRestoreFocus(open)

  // Focus and select on open, so typing replaces the suggested name.
  useEffect(() => {
    if (!open) return
    const id = requestAnimationFrame(() => {
      inputRef.current?.focus()
      inputRef.current?.select()
    })
    return () => cancelAnimationFrame(id)
  }, [open])

  // Escape cancels even when focus has wandered.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        cancel()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, cancel])

  if (!open) return null

  return (
    <div className="modal-scrim" onClick={(e) => e.target === e.currentTarget && cancel()}>
      <div
        className="modal modal--prompt"
        role="dialog"
        aria-modal="true"
        aria-labelledby="prompt-title"
      >
        <h2 id="prompt-title" className="modal__title">
          {title}
        </h2>

        <div className="field">
          <label className="field__label" htmlFor="prompt-value">
            {label}
          </label>
          <input
            id="prompt-value"
            ref={inputRef}
            className="field__input"
            value={value}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                confirm()
              }
            }}
          />
        </div>

        <div className="modal__actions">
          <button className="btn-ghost" onClick={cancel}>
            Cancel
          </button>
          <button className="btn-accent" onClick={confirm} disabled={value.trim() === ''}>
            Save
          </button>
        </div>
      </div>
    </div>
  )
}

/**
 * The yes/no counterpart, replacing `window.confirm`.
 *
 * Takes no arguments — the message and the `danger` flag come from `useConfirm`.
 *
 * @returns `JSX.Element | null` — the dialog, or `null` while closed. Under `danger`,
 *   Cancel takes initial focus and Enter is left unbound.
 */
function ConfirmDialog() {
  const { open, title, message, confirmLabel, danger, accept, cancel } = useConfirm()
  const acceptRef = useRef<HTMLButtonElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)

  useRestoreFocus(open)

  // A destructive action starts with Cancel focused, so the reflex of hitting
  // Enter on a dialog you have not finished reading backs out instead of
  // deleting something.
  useEffect(() => {
    if (!open) return
    const id = requestAnimationFrame(() => {
      const target = danger ? cancelRef.current : acceptRef.current
      target?.focus()
    })
    return () => cancelAnimationFrame(id)
  }, [open, danger])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        cancel()
      } else if (e.key === 'Enter' && !danger) {
        // Deliberately not wired up for `danger`: there the focused Cancel
        // button already handles Enter natively, and hijacking the key would
        // undo the whole point of moving focus.
        e.preventDefault()
        accept()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, danger, accept, cancel])

  if (!open) return null

  return (
    <div className="modal-scrim" onClick={(e) => e.target === e.currentTarget && cancel()}>
      <div
        className="modal modal--confirm"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-message"
      >
        <h2 id="confirm-title" className="modal__title">
          {title}
        </h2>
        <p id="confirm-message" className="modal__subtitle">
          {message}
        </p>

        <div className="modal__actions">
          <button ref={cancelRef} className="btn-ghost" onClick={cancel}>
            Cancel
          </button>
          <button
            ref={acceptRef}
            className={danger ? 'btn-danger' : 'btn-accent'}
            onClick={accept}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}
