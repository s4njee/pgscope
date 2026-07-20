import { create } from 'zustand'

import { ipc, toAppError } from '../lib/ipc'
import { candidateColumns } from '../lib/format'
import type { Segment } from '../lib/types'

/** Scrollback cap (plan.md §5.7) — a long session must not grow without bound. */
const SCROLLBACK_CHAR_CAP = 200_000

interface TerminalStore {
  sessionId: string | null
  prompt: string
  /** Rendered scrollback. */
  segments: Segment[]
  /** Current input line. */
  input: string
  /** True while a statement is running. */
  running: boolean
  /** True when the buffer is mid-statement (continuation prompt). */
  incomplete: boolean
  timing: boolean
  expanded: boolean
  /** Submitted lines, for Up/Down recall. */
  history: string[]
  historyIndex: number | null
  /**
   * The line as it stood after the last Tab. A second Tab on an unchanged line
   * lists the candidates, matching psql's two-stage completion.
   */
  lastCompletedLine: string | null

  open: () => Promise<void>
  submit: (input: string) => Promise<void>
  cancel: () => Promise<void>
  clear: () => void
  setInput: (v: string) => void
  recallPrev: () => void
  recallNext: () => void
  /** Tab completion; returns the new cursor offset, or null if nothing changed. */
  complete: (cursor: number) => Promise<number | null>
  appendSegments: (segs: Segment[]) => void
  reset: () => void
}

/**
 * Cap terminal scrollback, dropping whole segments from the oldest end.
 *
 * Trimming by segment rather than by character keeps each segment's colouring
 * intact — half a rendered table is worse than not having it. The last segment
 * is never dropped, so a single oversized result still shows rather than
 * leaving the terminal blank after the command that produced it.
 *
 * @param segments - `Segment[]` — the whole scrollback, oldest first.
 * @returns `Segment[]` — the same array when already under the 200,000-char
 *   cap, otherwise a new one with leading segments dropped. At least one
 *   segment always survives, so the result can still exceed the cap.
 */
function trimScrollback(segments: Segment[]): Segment[] {
  let total = 0
  for (const s of segments) total += s.text.length
  if (total <= SCROLLBACK_CHAR_CAP) return segments

  // Drop from the front until under the cap.
  const out = [...segments]
  while (total > SCROLLBACK_CHAR_CAP && out.length > 1) {
    total -= out[0].text.length
    out.shift()
  }
  return out
}

export const useTerminal = create<TerminalStore>((set, get) => ({
  sessionId: null,
  prompt: 'psql',
  segments: [],
  input: '',
  running: false,
  incomplete: false,
  timing: true,
  expanded: false,
  history: [],
  historyIndex: null,
  lastCompletedLine: null,

  open: async () => {
    try {
      const session = await ipc.replOpen()
      set({
        sessionId: session.sessionId,
        prompt: session.prompt,
        timing: session.timing,
        expanded: session.expanded,
      })
      const items = await ipc.historyList()
      set({ history: items.map((h) => h.input) })
    } catch (e) {
      const err = toAppError(e)
      get().appendSegments([{ text: `${err.message}\n`, kind: 'error' }])
    }
  },

  submit: async (input) => {
    const { sessionId, prompt } = get()
    if (!sessionId) return

    // Echo the submitted line the way a terminal does, before running it.
    get().appendSegments([
      { text: prompt + ' ', kind: 'prompt' },
      { text: input + '\n', kind: 'body' },
    ])

    set((s) => ({
      running: true,
      input: '',
      historyIndex: null,
      history: input.trim() ? [...s.history, input] : s.history,
    }))

    try {
      const out = await ipc.replExec(sessionId, input)
      get().appendSegments(out.segments)
      set({
        prompt: out.prompt,
        incomplete: out.incomplete,
        timing: out.timing,
        expanded: out.expanded,
        running: false,
      })
      if (input.trim() && !out.incomplete) {
        void ipc.historyAppend(input).catch(() => {})
      }
    } catch (e) {
      const err = toAppError(e)
      get().appendSegments([{ text: `${err.message}\n`, kind: 'error' }])
      set({ running: false })
    }
  },

  cancel: async () => {
    const { sessionId, running } = get()
    if (!sessionId) return
    if (!running) {
      // psql: Ctrl+C on an idle prompt clears the pending buffer.
      get().appendSegments([{ text: '^C\n', kind: 'dim' }])
      set({ input: '' })
      return
    }
    get().appendSegments([{ text: '^C\n', kind: 'dim' }])
    try {
      await ipc.replCancel(sessionId)
    } catch {
      /* the statement may have finished on its own */
    }
  },

  clear: () => set({ segments: [] }),

  setInput: (input) => set({ input, lastCompletedLine: null }),

  recallPrev: () => {
    const { history, historyIndex } = get()
    if (history.length === 0) return
    const next = historyIndex === null ? history.length - 1 : Math.max(0, historyIndex - 1)
    set({ historyIndex: next, input: history[next] })
  },

  recallNext: () => {
    const { history, historyIndex } = get()
    if (historyIndex === null) return
    const next = historyIndex + 1
    if (next >= history.length) {
      set({ historyIndex: null, input: '' })
    } else {
      set({ historyIndex: next, input: history[next] })
    }
  },

  complete: async (cursor) => {
    const { sessionId, input, lastCompletedLine } = get()
    if (!sessionId) return null

    let result
    try {
      result = await ipc.replComplete(sessionId, input, cursor)
    } catch {
      return null
    }
    if (result.items.length === 0) return null

    const token = input.slice(result.start, result.end)
    const single = result.items.length === 1

    // One match completes fully (with a trailing space, as psql does);
    // several insert only the unambiguous part.
    const insert = single ? result.items[0].value + ' ' : result.commonPrefix
    const grew = insert.length > token.length

    if (grew) {
      const next = input.slice(0, result.start) + insert + input.slice(result.end)
      set({ input: next, lastCompletedLine: next })
      return result.start + insert.length
    }

    // No progress possible. A second Tab on the same line lists the options.
    if (lastCompletedLine === input) {
      const values = result.items.map((i) => i.value)
      get().appendSegments([
        { text: `${get().prompt} `, kind: 'prompt' },
        { text: input + '\n', kind: 'body' },
        { text: candidateColumns(values), kind: 'dim' },
      ])
      set({ lastCompletedLine: null })
    } else {
      set({ lastCompletedLine: input })
    }
    return null
  },

  appendSegments: (segs) =>
    set((s) => ({ segments: trimScrollback([...s.segments, ...segs]) })),

  reset: () =>
    set({ sessionId: null, segments: [], input: '', running: false, incomplete: false }),
}))
