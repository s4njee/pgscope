import { beforeEach, describe, expect, it } from 'vitest'

import { confirmAction, promptForName, useConfirm, usePrompt } from './prompt'

/** A promise that has not settled resolves to this instead of hanging the test. */
const PENDING = { pending: true }

/**
 * Races `promise` against one microtask, so "still pending" is assertable.
 *
 * @param promise - `Promise<T>` — the promise under test, typically a modal's
 *   pending answer.
 * @returns `Promise<T | typeof PENDING>` — `promise`'s value if it settles
 *   within one microtask, otherwise the `PENDING` sentinel. A promise settling
 *   later than that still reads as pending.
 */
function settledWithin<T>(promise: Promise<T>): Promise<T | typeof PENDING> {
  return Promise.race([promise, Promise.resolve().then(() => PENDING)])
}

beforeEach(() => {
  usePrompt.setState({ open: false, value: '', resolve: null })
  useConfirm.setState({ open: false, resolve: null })
})

describe('confirmAction', () => {
  it('resolves true when confirmed', async () => {
    const answer = confirmAction({ title: 'Drop table', message: 'This cannot be undone.' })
    expect(useConfirm.getState().open).toBe(true)

    useConfirm.getState().accept()
    await expect(answer).resolves.toBe(true)
    expect(useConfirm.getState().open).toBe(false)
  })

  it('resolves false when cancelled', async () => {
    const answer = confirmAction({ title: 'Drop table', message: 'This cannot be undone.' })
    useConfirm.getState().cancel()
    await expect(answer).resolves.toBe(false)
  })

  it('carries the label and danger flag through to the store', () => {
    void confirmAction({
      title: 'Drop table',
      message: 'This cannot be undone.',
      confirmLabel: 'Drop',
      danger: true,
    })
    expect(useConfirm.getState()).toMatchObject({ confirmLabel: 'Drop', danger: true })
  })

  it('defaults to a non-destructive Confirm button', () => {
    void confirmAction({ title: 'Reconnect', message: 'Reopen the connection?' })
    expect(useConfirm.getState()).toMatchObject({ confirmLabel: 'Confirm', danger: false })
  })

  it('resolves a displaced dialog false rather than leaving it pending', async () => {
    const first = confirmAction({ title: 'First', message: 'one' })
    const second = confirmAction({ title: 'Second', message: 'two' })

    await expect(settledWithin(first)).resolves.toBe(false)
    expect(useConfirm.getState().title).toBe('Second')

    useConfirm.getState().accept()
    await expect(second).resolves.toBe(true)
  })
})

describe('prompt and confirm displacing each other', () => {
  it('cancels an open text prompt when a confirm opens', async () => {
    const name = promptForName('Save query', 'untitled')
    const answer = confirmAction({ title: 'Discard', message: 'Throw away the draft?' })

    await expect(settledWithin(name)).resolves.toBeNull()
    expect(usePrompt.getState().open).toBe(false)
    expect(useConfirm.getState().open).toBe(true)

    useConfirm.getState().cancel()
    await expect(answer).resolves.toBe(false)
  })

  it('cancels an open confirm when a text prompt opens', async () => {
    const answer = confirmAction({ title: 'Discard', message: 'Throw away the draft?' })
    const name = promptForName('Save query', 'untitled')

    await expect(settledWithin(answer)).resolves.toBe(false)
    expect(useConfirm.getState().open).toBe(false)
    expect(usePrompt.getState().open).toBe(true)

    usePrompt.getState().confirm()
    await expect(name).resolves.toBe('untitled')
  })
})
