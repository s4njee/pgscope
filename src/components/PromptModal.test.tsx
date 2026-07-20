import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { confirmAction, useConfirm, usePrompt, type ConfirmOptions } from '../state/prompt'
import { PromptModal } from './PromptModal'

beforeEach(() => {
  act(() => {
    usePrompt.setState({ open: false, value: '', resolve: null })
    useConfirm.setState({ open: false, resolve: null })
  })
  document.body.innerHTML = ''
})

/**
 * Opens the dialog inside `act` so React has flushed by the time we assert.
 *
 * @param opts - `ConfirmOptions` — title, message, optional `confirmLabel` (defaulting to
 *   `Confirm`), and `danger` for a destructive action.
 * @returns `Promise<boolean>` — the dialog's answer: true on accept, false on cancel or
 *   Escape. Still pending on return, since the dialog has only just opened.
 */
function openConfirm(opts: ConfirmOptions): Promise<boolean> {
  let answer!: Promise<boolean>
  act(() => {
    answer = confirmAction(opts)
  })
  return answer
}

describe('ConfirmDialog', () => {
  it('resolves false on Escape', async () => {
    render(<PromptModal />)
    const answer = openConfirm({ title: 'Drop table', message: 'This cannot be undone.' })

    expect(screen.getByRole('dialog')).toBeInTheDocument()
    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' })
    })

    await expect(answer).resolves.toBe(false)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('resolves true on Enter for a non-destructive action', async () => {
    render(<PromptModal />)
    const answer = openConfirm({ title: 'Reconnect', message: 'Reopen the connection?' })

    act(() => {
      fireEvent.keyDown(window, { key: 'Enter' })
    })

    await expect(answer).resolves.toBe(true)
  })

  it('resolves true when the confirm button is clicked', async () => {
    render(<PromptModal />)
    const answer = openConfirm({
      title: 'Drop table',
      message: 'Gone forever.',
      confirmLabel: 'Drop',
    })

    fireEvent.click(screen.getByRole('button', { name: 'Drop' }))
    await expect(answer).resolves.toBe(true)
  })

  it('describes itself for screen readers', () => {
    render(<PromptModal />)
    void openConfirm({ title: 'Drop table', message: 'This cannot be undone.' })

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleName('Drop table')
    expect(dialog).toHaveAccessibleDescription('This cannot be undone.')
  })

  it('focuses Cancel — not the destructive button — when danger is set', async () => {
    render(<PromptModal />)
    void openConfirm({ title: 'Drop table', message: 'Gone forever.', danger: true })

    await waitFor(() => expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus())
  })

  it('focuses the confirm button when the action is not destructive', async () => {
    render(<PromptModal />)
    void openConfirm({ title: 'Reconnect', message: 'Reopen the connection?' })

    await waitFor(() => expect(screen.getByRole('button', { name: 'Confirm' })).toHaveFocus())
  })

  it('returns focus to whatever was focused before it opened', async () => {
    const opener = document.createElement('button')
    document.body.appendChild(opener)
    opener.focus()

    render(<PromptModal />)
    const answer = openConfirm({ title: 'Drop table', message: 'Gone forever.' })
    await waitFor(() => expect(screen.getByRole('button', { name: 'Confirm' })).toHaveFocus())

    act(() => useConfirm.getState().cancel())
    await answer
    await waitFor(() => expect(opener).toHaveFocus())
  })
})
