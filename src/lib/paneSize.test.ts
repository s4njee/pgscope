import { describe, expect, it } from 'vitest'

import {
  clampTerminalHeight,
  maxTerminalHeight,
  TERM_BODY_DEFAULT,
  TERM_BODY_MIN,
} from './paneSize'

describe('clampTerminalHeight', () => {
  it('leaves a reasonable height alone', () => {
    expect(clampTerminalHeight(300, 900)).toBe(300)
    expect(clampTerminalHeight(TERM_BODY_DEFAULT, 880)).toBe(TERM_BODY_DEFAULT)
  })

  it('refuses to shrink the pane to nothing', () => {
    expect(clampTerminalHeight(0, 900)).toBe(TERM_BODY_MIN)
    expect(clampTerminalHeight(-500, 900)).toBe(TERM_BODY_MIN)
  })

  it('caps the pane at 70% of the viewport so the grid keeps some height', () => {
    expect(clampTerminalHeight(5000, 1000)).toBe(700)
  })

  it('re-clamps a height persisted from a taller window', () => {
    // 820px was fine on a 1200px-tall external display; on a 700px laptop
    // window it would leave the grid with ~0 rows.
    expect(clampTerminalHeight(820, 1200)).toBe(820)
    expect(clampTerminalHeight(820, 700)).toBe(490)
  })

  it('keeps the minimum when the viewport is too short for the fraction', () => {
    // 70% of 80px is 56px, under the minimum — the minimum wins.
    expect(clampTerminalHeight(400, 80)).toBe(TERM_BODY_MIN)
    expect(clampTerminalHeight(10, 80)).toBe(TERM_BODY_MIN)
    expect(clampTerminalHeight(200, 0)).toBe(TERM_BODY_MIN)
  })

  it('falls back to the default for a corrupt value', () => {
    expect(clampTerminalHeight(Number.NaN, 900)).toBe(TERM_BODY_DEFAULT)
  })

  it('always returns a whole number of pixels', () => {
    expect(clampTerminalHeight(212.4, 933)).toBe(212)
    expect(Number.isInteger(clampTerminalHeight(1e9, 901))).toBe(true)
  })
})

describe('maxTerminalHeight', () => {
  it('is the viewport fraction, floored at the minimum', () => {
    expect(maxTerminalHeight(1000)).toBe(700)
    expect(maxTerminalHeight(50)).toBe(TERM_BODY_MIN)
  })

  it('never reports a maximum below the minimum', () => {
    for (const vh of [0, 1, 50, 85, 100, 400, 2000]) {
      expect(maxTerminalHeight(vh)).toBeGreaterThanOrEqual(TERM_BODY_MIN)
    }
  })
})
