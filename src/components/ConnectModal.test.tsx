import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useConnection } from '../state/connection'
import { ConnectModal } from './ConnectModal'
import type { ConnectionInfo } from '../lib/types'

vi.mock('../lib/ipc', () => ({
  ipc: {
    listProfiles: vi.fn(() => Promise.resolve([])),
    saveProfile: vi.fn(() => Promise.resolve()),
    deleteProfile: vi.fn(() => Promise.resolve()),
  },
  toAppError: (e: unknown) => ({
    code: 'invalid',
    message: String(e),
    detail: null,
    sqlstate: null,
  }),
}))

const connected: ConnectionInfo = {
  database: 'analytics_prod',
  host: 'localhost',
  port: 5432,
  user: 'pgscope',
  serverVersion: '18.0',
  isSuperuser: false,
}

/**
 * Open the modal in the given connection state, letting the profile fetch it
 * kicks off settle — otherwise every test races that promise's setState.
 *
 * @param info - `ConnectionInfo | null` — the connection to seed the store with; `null`
 *   puts it in the `'disconnected'` state the modal auto-opens in at launch.
 * @returns `Promise<RenderResult>` — testing-library's handle, resolving once the
 *   profile fetch has flushed.
 */
async function open(info: ConnectionInfo | null) {
  act(() => {
    useConnection.setState({
      modalOpen: true,
      info,
      state: info ? 'connected' : 'disconnected',
      error: null,
    })
  })
  const rendered = render(<ConnectModal />)
  await act(async () => {})
  return rendered
}

/**
 * Whether the modal is still open, read from the store rather than the DOM.
 *
 * Takes no arguments.
 *
 * @returns `boolean` — the store's current `modalOpen`.
 */
const isOpen = () => useConnection.getState().modalOpen

beforeEach(() => {
  act(() => {
    useConnection.setState({ modalOpen: false, info: null, state: 'disconnected', error: null })
  })
  document.body.innerHTML = ''
})

// The reported bug: the modal opens by itself at launch when there is no
// connection yet, and every exit was gated on already having one — so the very
// state it appears in was the state you could not leave.
describe('leaving the connect modal with no connection', () => {
  it('offers a cancel button', async () => {
    await open(null)
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy()
  })

  it('closes on the cancel button', async () => {
    await open(null)
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(isOpen()).toBe(false)
  })

  it('closes on Escape', async () => {
    await open(null)
    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' })
    })
    expect(isOpen()).toBe(false)
  })

  it('closes on a click outside the dialog', async () => {
    const { container } = await open(null)
    fireEvent.click(container.querySelector('.modal-scrim')!)
    expect(isOpen()).toBe(false)
  })
})

describe('the same exits work while connected', () => {
  it('closes on Escape', async () => {
    await open(connected)
    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' })
    })
    expect(isOpen()).toBe(false)
  })

  it('closes on the cancel button', async () => {
    await open(connected)
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(isOpen()).toBe(false)
  })
})

describe('what must not close it', () => {
  it('stays open when a click lands inside the dialog', async () => {
    // The scrim handler fires for clicks that bubble up from the panel too, so
    // it has to check the event target — otherwise using any field closes it.
    const { container } = await open(null)
    fireEvent.click(container.querySelector('.modal')!)
    expect(isOpen()).toBe(true)
  })

  it('stays open on a key that is not Escape', async () => {
    await open(null)
    act(() => {
      fireEvent.keyDown(window, { key: 'a' })
    })
    expect(isOpen()).toBe(true)
  })

  it('does not listen for Escape once closed', async () => {
    // A listener left attached would swallow Escape from the grid's cell
    // expansion and the context menu.
    const { unmount } = await open(null)
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    unmount()

    const seen = vi.fn()
    window.addEventListener('keydown', seen)
    fireEvent.keyDown(window, { key: 'Escape' })
    window.removeEventListener('keydown', seen)

    const event = seen.mock.calls[0]?.[0] as KeyboardEvent | undefined
    expect(event?.defaultPrevented).toBe(false)
  })
})
