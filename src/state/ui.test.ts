import { beforeEach, describe, expect, it } from 'vitest'

import { TERM_BODY_DEFAULT, TERM_BODY_MIN } from '../lib/paneSize'
import { useUi } from './ui'

/**
 * jsdom defaults to 768px tall; set it explicitly so the maths is readable.
 *
 * @param height - `number` — the window height in px to report as
 *   `window.innerHeight`.
 * @returns `void` — on return `window.innerHeight` reads back as `height`. The
 *   property is redefined as configurable, so later calls can override it.
 */
function setViewport(height: number) {
  Object.defineProperty(window, 'innerHeight', { value: height, configurable: true })
}

describe('setTerminalHeight', () => {
  beforeEach(() => {
    setViewport(1000)
    useUi.setState({ terminalHeight: TERM_BODY_DEFAULT })
  })

  it('stores a reasonable height as-is', () => {
    useUi.getState().setTerminalHeight(320)
    expect(useUi.getState().terminalHeight).toBe(320)
  })

  it('clamps against the live window rather than trusting the caller', () => {
    useUi.getState().setTerminalHeight(5000)
    expect(useUi.getState().terminalHeight).toBe(700)

    useUi.getState().setTerminalHeight(1)
    expect(useUi.getState().terminalHeight).toBe(TERM_BODY_MIN)
  })
})

describe('rehydration', () => {
  it('re-clamps a height persisted from a larger monitor', async () => {
    // 820px was legal in a 1200px window; the app now reopens at 700px.
    localStorage.setItem(
      'pgscope.ui',
      JSON.stringify({ state: { terminalHeight: 820, terminalCollapsed: false }, version: 0 }),
    )
    setViewport(700)

    await useUi.persist.rehydrate()

    expect(useUi.getState().terminalHeight).toBe(490)
  })

  it('restores the selected color theme', async () => {
    localStorage.setItem(
      'pgscope.ui',
      JSON.stringify({ state: { theme: 'black' }, version: 0 }),
    )

    await useUi.persist.rehydrate()

    expect(useUi.getState().theme).toBe('black')
  })
})
