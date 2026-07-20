import { useEffect, useLayoutEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'

import { suggestedQueryName } from '../lib/format'
import {
  clampTerminalHeight,
  maxTerminalHeight,
  TERM_BODY_DEFAULT,
  TERM_BODY_MIN,
  TERM_BODY_STEP,
} from '../lib/paneSize'
import type { SavedQuery } from '../lib/types'

import { useConnection } from '../state/connection'
import { promptForName } from '../state/prompt'
import { useTerminal } from '../state/terminal'
import { useUi } from '../state/ui'

/**
 * Collapsed 28px bar; the whole bar expands the pane.
 *
 * Takes no arguments; the `ConnectionInfo | null` it labels itself with comes
 * from the connection store.
 *
 * @returns `JSX.Element` — the bar, showing the connected database name or `—`
 *   when there is no connection info yet.
 */
function CollapsedBar() {
  const setTerminalCollapsed = useUi((s) => s.setTerminalCollapsed)
  const info = useConnection((s) => s.info)

  return (
    <button className="term__collapsed" onClick={() => setTerminalCollapsed(false)}>
      <span className="term__title">psql</span>
      <span className="term__subtitle">{info?.database ?? '—'}</span>
      <div className="spacer" />
      <span className="term__subtitle">▴ expand</span>
    </button>
  )
}

/**
 * Drag handle on the pane's top edge.
 *
 * The pointermove handler writes the new height straight to the `<pre>` node
 * instead of going through the store: a store update per pointermove would
 * re-render the whole scrollback (one span per segment) at pointer-event rate,
 * which is visibly janky on a long session. The store is committed once on
 * release, which is also the only point the value needs to be persisted.
 *
 * @param props - `{ bodyRef: React.RefObject<HTMLPreElement> }`
 *   - `bodyRef` — ref to the scrollback `<pre>`, whose inline `height` is
 *     written directly during a drag; `bodyRef.current` is `null` before mount,
 *     in which case the store's height stands in as the drag's starting value.
 * @returns `JSX.Element` — the horizontal separator on the pane's top edge,
 *   draggable by pointer and resizable by arrow/Home/End keys, with
 *   double-click resetting to the default height.
 */
function ResizeHandle({ bodyRef }: { bodyRef: React.RefObject<HTMLPreElement> }) {
  const terminalHeight = useUi((s) => s.terminalHeight)
  const setTerminalHeight = useUi((s) => s.setTerminalHeight)

  const handleRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null)

  const currentHeight = () => bodyRef.current?.offsetHeight ?? terminalHeight

  const resizeTo = (px: number) => {
    if (bodyRef.current) {
      bodyRef.current.style.height = `${clampTerminalHeight(px, window.innerHeight)}px`
    }
  }

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    // Also suppresses the text selection a drag across the pane would start.
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    dragRef.current = { startY: e.clientY, startHeight: currentHeight() }
    handleRef.current?.classList.add('term__resize--dragging')
  }

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (!drag) return
    // The handle is on the top edge, so dragging up (a smaller clientY) grows it.
    resizeTo(drag.startHeight + (drag.startY - e.clientY))
  }

  const endDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return
    dragRef.current = null
    e.currentTarget.releasePointerCapture(e.pointerId)
    handleRef.current?.classList.remove('term__resize--dragging')
    setTerminalHeight(currentHeight())
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const steps: Record<string, number> = {
      ArrowUp: terminalHeight + TERM_BODY_STEP,
      ArrowDown: terminalHeight - TERM_BODY_STEP,
      Home: TERM_BODY_MIN,
      End: maxTerminalHeight(window.innerHeight),
    }
    const next = steps[e.key]
    if (next === undefined) return
    e.preventDefault()
    setTerminalHeight(next)
  }

  return (
    <div
      ref={handleRef}
      className="term__resize"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize terminal pane"
      aria-valuenow={terminalHeight}
      aria-valuemin={TERM_BODY_MIN}
      aria-valuemax={maxTerminalHeight(window.innerHeight)}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onKeyDown}
      onDoubleClick={() => setTerminalHeight(TERM_BODY_DEFAULT)}
      title="Drag to resize · double-click to reset"
    />
  )
}

/**
 * The psql-style terminal pane.
 *
 * Typing goes to an offscreen input rather than a visible field: the prompt and
 * caret are drawn as part of the scrollback so they line up with the output
 * above them, while the real input keeps native IME, paste, and key repeat.
 * Clicking anywhere in the pane hands focus back to it.
 *
 * Takes no arguments; the scrollback (`Segment[]`), prompt string, and input
 * all come from the terminal store, and the pane's height in CSS pixels from
 * the UI store.
 *
 * @returns `JSX.Element` — the full pane: resize handle, header, and the
 *   scrollback `<pre>` with the live input line. Returns the 28px
 *   `<CollapsedBar />` instead whenever the pane is collapsed.
 */
export function Terminal() {
  const { terminalCollapsed, setTerminalCollapsed } = useUi()
  const terminalHeight = useUi((s) => s.terminalHeight)
  const connState = useConnection((s) => s.state)
  const info = useConnection((s) => s.info)
  const {
    sessionId,
    segments,
    prompt,
    input,
    running,
    timing,
    open,
    submit,
    cancel,
    clear,
    setInput,
    complete,
    appendSegments,
    recallPrev,
    recallNext,
  } = useTerminal()

  const bodyRef = useRef<HTMLPreElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  /** The connection the current session was opened for, so a switch reopens. */
  const openedFor = useRef<string | null>(null)

  // Save the last submitted statement as a named .sql in the saved-queries dir,
  // so it shows up in the sidebar panel.
  const saveLast = async () => {
    const { history, input } = useTerminal.getState()
    const statement = (input.trim() || history[history.length - 1] || '').trim()
    if (!statement) {
      appendSegments([{ text: 'nothing to save\n', kind: 'dim' }])
      return
    }
    const name = await promptForName('Save query as', suggestedQueryName(statement), 'File name')
    if (!name) return

    try {
      const q = await invoke<SavedQuery>('save_named_query', {
        name,
        content: statement + '\n',
      })
      appendSegments([{ text: `-- saved ${q.name}.sql\n`, kind: 'dim' }])
      useUi.getState().bumpSavedQueries()
    } catch (e) {
      const err = e as { message?: string }
      appendSegments([{ text: `${err.message ?? String(e)}\n`, kind: 'error' }])
    }
  }

  // Which server and database the session must belong to. Reconnecting to the
  // same place leaves this unchanged, so a transient drop keeps its scrollback.
  const connKey = info ? `${info.user}@${info.host}:${info.port}/${info.database}` : null

  // Open a session once connected, and open a *fresh* one whenever the
  // connection changes underneath. The backend drops every session on a switch,
  // so holding the old id would leave the pane talking to a session that no
  // longer exists — and before that, to the previous database.
  useEffect(() => {
    if (connState !== 'connected' || !connKey) return
    if (openedFor.current === connKey && sessionId) return
    openedFor.current = connKey
    if (sessionId) useTerminal.getState().reset()
    void open()
  }, [connState, connKey, sessionId, open])

  // Re-clamp when the window shrinks. Rehydration already clamps a persisted
  // height, but a window resized *during* a session would otherwise leave a
  // pane taller than the 70% rule allows — and dragging it back is the one
  // thing a squeezed-out grid makes hard.
  useEffect(() => {
    const onResize = () => {
      const { terminalHeight: h, setTerminalHeight: set } = useUi.getState()
      if (h !== clampTerminalHeight(h, window.innerHeight)) set(h)
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  // Keep the newest output in view.
  useLayoutEffect(() => {
    const el = bodyRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [segments, input])

  // ⌘K clears, ⌘J toggles the pane, ⌘T focuses the prompt.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return
      if (e.key === 'k') {
        e.preventDefault()
        clear()
      } else if (e.key === 'j') {
        e.preventDefault()
        useUi.getState().toggleTerminal()
      } else if (e.key === 't') {
        e.preventDefault()
        setTerminalCollapsed(false)
        requestAnimationFrame(() => inputRef.current?.focus())
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [clear, setTerminalCollapsed])

  if (terminalCollapsed) return <CollapsedBar />

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Tab') {
      // Tab is completion here, not focus movement.
      e.preventDefault()
      const cursor = e.currentTarget.selectionStart ?? input.length
      void complete(cursor).then((next) => {
        if (next === null) return
        // Restore the caret after React re-renders with the completed text.
        requestAnimationFrame(() => {
          inputRef.current?.setSelectionRange(next, next)
        })
      })
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      void submit(input)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      recallPrev()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      recallNext()
    } else if (e.key === 'c' && e.ctrlKey) {
      e.preventDefault()
      void cancel()
    }
  }

  return (
    <div className="term">
      <ResizeHandle bodyRef={bodyRef} />

      <div className="term__header">
        <span className="term__title">psql</span>
        <span className="term__subtitle">
          {info?.database ?? '—'} · session 1
        </span>
        <div className="spacer" />
        <span
          className={`term__timing${timing ? '' : ' term__timing--off'}`}
          onClick={() => void submit('\\timing')}
          title="Toggle \\timing"
        >
          Timing {timing ? 'on' : 'off'}
        </span>
        <span className="term__action" onClick={() => void saveLast()} title="Save the last statement as a .sql file">
          save
        </span>
        <span className="term__action" onClick={clear}>
          clear
        </span>
        <span className="term__action" onClick={() => setTerminalCollapsed(true)}>
          ▾ collapse
        </span>
      </div>

      <pre
        className="term__body"
        ref={bodyRef}
        style={{ height: terminalHeight }}
        onClick={() => inputRef.current?.focus()}
      >
        {segments.map((seg, i) => (
          <span key={i} className={`seg--${seg.kind}`}>
            {seg.text}
          </span>
        ))}

        {/* Live input line, with the blinking block cursor after it. */}
        <span className="seg--prompt">{prompt} </span>
        <span className="seg--body">{input}</span>
        <span className={`term__cursor${running ? ' term__cursor--running' : ''}`} />

        <input
          ref={inputRef}
          className="term__hidden-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
          spellCheck={false}
          autoComplete="off"
          aria-label="psql input"
        />
      </pre>
    </div>
  )
}
