/**
 * Sizing rules for the resizable terminal pane.
 *
 * Kept pure and DOM-free so both the store setter and the drag handler run the
 * same clamp: a height persisted on a large monitor must not survive intact
 * into a session on a laptop screen.
 */

/** The design's default body height (`--h-term-body`). */
export const TERM_BODY_DEFAULT = 212

/** Roughly four lines at `--fs-terminal`/`--lh-terminal` plus the body padding. */
export const TERM_BODY_MIN = 60

/** The pane may never take more than this share of the window. */
export const TERM_BODY_MAX_FRACTION = 0.7

/** Keyboard resize increment, ~one text line. */
export const TERM_BODY_STEP = 18

/**
 * Largest body height allowed in a window of `viewportHeight`.
 *
 * When the window is so short that 70% of it is under the minimum, the minimum
 * wins: a pane too small to read anything in is a worse failure than one that
 * crowds the grid, and the grid still scrolls. So the returned maximum is never
 * below `TERM_BODY_MIN`, and `clampTerminalHeight` can collapse to a single
 * value rather than an empty range.
 */
export function maxTerminalHeight(viewportHeight: number): number {
  // Rounded, not floored: `700 * 0.7` is 489.999… in binary floating point, and
  // a maximum of 489px for a 70% rule reads as an off-by-one.
  const fraction = Math.round(viewportHeight * TERM_BODY_MAX_FRACTION)
  return Number.isFinite(fraction) ? Math.max(TERM_BODY_MIN, fraction) : TERM_BODY_MIN
}

/** Clamp a requested body height into the range allowed by the viewport. */
export function clampTerminalHeight(requested: number, viewportHeight: number): number {
  const max = maxTerminalHeight(viewportHeight)
  // A corrupt persisted value (`null` coerced, a hand-edited string) should land
  // on the design default rather than the minimum.
  const wanted = Number.isNaN(requested) ? TERM_BODY_DEFAULT : requested
  return Math.round(Math.min(Math.max(wanted, TERM_BODY_MIN), max))
}
