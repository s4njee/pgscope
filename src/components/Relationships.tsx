import { useCallback, useEffect, useRef, useState } from 'react'

import { shortType } from '../lib/cellColor'
import { canvasSize, layoutCards, layoutEdges, type PlacedCard, type Point } from '../lib/erLayout'
import { invoke } from '@tauri-apps/api/core'
import { ipc } from '../lib/ipc'
import type { ColumnMeta, FkGraph } from '../lib/types'
import { useExplorer } from '../state/explorer'

/**
 * PK/FK marker on an ER card row; nothing for an ordinary column.
 *
 * @param props - `{ col: ColumnMeta }`
 *   - `col` — the column; only `isPk` and `isFk` are read, and `isPk` wins when
 *     the column is both.
 * @returns `JSX.Element | null` — the `PK` or `FK` badge, or `null` when the
 *   column is neither, so the row renders no marker at all.
 */
function ColumnTag({ col }: { col: ColumnMeta }) {
  if (col.isPk) return <span className="er-col__tag er-col__tag--pk">PK</span>
  if (col.isFk) return <span className="er-col__tag er-col__tag--fk">FK</span>
  return null
}

interface CardProps {
  card: PlacedCard
  onSelect: (table: string) => void
  onDrag: (table: string, p: Point) => void
  onDragEnd: () => void
}

/**
 * One draggable table card on the ER canvas.
 *
 * Position is owned by the parent, so `onDrag` fires per pointer move and the
 * card is re-placed from the props that come back — the card never moves
 * itself, which is what keeps the FK lines attached to it while it moves.
 * Pointer capture keeps the drag alive when the cursor outruns the card.
 *
 * @param props - `CardProps` — `{ card: PlacedCard; onSelect: (table: string)
 *   => void; onDrag: (table: string, p: Point) => void; onDragEnd: () => void }`
 *   - `card` — the table, its columns, and the placed geometry (`x`/`y` in
 *     canvas pixels from the top-left, plus `width` and `selected`).
 *   - `onSelect` — called with the table name on a click that did not move the
 *     card (under a 3px threshold).
 *   - `onDrag` — called on every pointer move with the new top-left `Point`,
 *     clamped to non-negative canvas coordinates.
 *   - `onDragEnd` — called once on pointer up after a real drag, so the parent
 *     can persist the layout; not called for a plain click.
 * @returns `JSX.Element` — the positioned card: header, column rows with types,
 *   and each row's `ColumnTag`.
 */
function TableCard({ card, onSelect, onDrag, onDragEnd }: CardProps) {
  const [dragging, setDragging] = useState(false)
  const origin = useRef<{ px: number; py: number; x: number; y: number } | null>(null)
  const moved = useRef(false)

  const onPointerDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId)
    origin.current = { px: e.clientX, py: e.clientY, x: card.x, y: card.y }
    moved.current = false
    setDragging(true)
  }

  const onPointerMove = (e: React.PointerEvent) => {
    if (!origin.current) return
    const dx = e.clientX - origin.current.px
    const dy = e.clientY - origin.current.py
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) moved.current = true
    onDrag(card.table, {
      x: Math.max(0, origin.current.x + dx),
      y: Math.max(0, origin.current.y + dy),
    })
  }

  const onPointerUp = (e: React.PointerEvent) => {
    e.currentTarget.releasePointerCapture(e.pointerId)
    origin.current = null
    setDragging(false)
    // A click that didn't move the card selects the table.
    if (!moved.current) onSelect(card.table)
    else onDragEnd()
  }

  return (
    <div
      className={`er-card${card.selected ? ' er-card--selected' : ''}${dragging ? ' er-card--dragging' : ''}`}
      style={{ left: card.x, top: card.y, width: card.width }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <div className="er-card__header">{card.table}</div>
      <div className="er-card__cols">
        {card.columns.map((col) => (
          <div className="er-col" key={col.name}>
            <span className={`er-col__name${col.isPk ? ' er-col__name--key' : ''}`}>
              {col.name}
            </span>
            <span className="er-col__type" title={col.dataType}>{shortType(col.dataType)}</span>
            <ColumnTag col={col} />
          </div>
        ))}
      </div>
    </div>
  )
}

/**
 * The ER view: FK-linked cards for the selected schema, laid out and draggable.
 *
 * Takes no arguments — the selected schema and table come from the explorer
 * store, and the graph plus saved card positions are fetched for that schema.
 *
 * @returns `JSX.Element` — the canvas: the load error, a loading placeholder
 *   while the `FkGraph` is in flight, an empty state when the schema has no
 *   foreign keys, or the SVG edge layer with a `TableCard` per placed card.
 */
export function Relationships() {
  const { selected, select } = useExplorer()
  const [graph, setGraph] = useState<FkGraph | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [overrides, setOverrides] = useState<Record<string, Point>>({})
  const schema = selected?.schema ?? null

  // Load the graph and any saved card positions for this schema.
  useEffect(() => {
    if (!schema) return
    let cancelled = false
    setError(null)
    ipc
      .fkGraph(schema)
      .then((g) => !cancelled && setGraph(g))
      .catch((e) => !cancelled && setError(e.message ?? String(e)))

    invoke<Record<string, Record<string, [number, number]>>>('er_layout_load')
      .then((layout) => {
        if (cancelled) return
        const saved = layout[schema] ?? {}
        const points: Record<string, Point> = {}
        for (const [table, [x, y]] of Object.entries(saved)) points[table] = { x, y }
        setOverrides(points)
      })
      .catch(() => setOverrides({}))

    return () => {
      cancelled = true
    }
  }, [schema])

  const persist = useCallback(() => {
    if (!schema) return
    const positions: Record<string, [number, number]> = {}
    for (const [table, p] of Object.entries(overrides)) positions[table] = [p.x, p.y]
    invoke('er_layout_save', { schema, positions }).catch(() => {})
  }, [schema, overrides])

  if (error) {
    return (
      <div className="er-canvas">
        <div className="er-empty">{error}</div>
      </div>
    )
  }

  if (!graph) {
    return (
      <div className="er-canvas">
        <div className="er-empty">loading…</div>
      </div>
    )
  }

  const cards = layoutCards(graph, selected?.table ?? null, overrides)
  const edges = layoutEdges(cards, graph.edges)
  const size = canvasSize(cards)

  if (cards.length === 0) {
    return (
      <div className="er-canvas">
        <div className="er-empty">no foreign keys in {graph.schema}</div>
        <div className="er-caption">
          {graph.schema} schema · 0 of {graph.totalTables} tables · FK graph
        </div>
      </div>
    )
  }

  return (
    <div className="er-canvas">
      <div className="er-surface" style={{ width: size.width, height: size.height }}>
        <svg className="er-svg" width={size.width} height={size.height}>
          {edges.map((e) => (
            <g key={e.key}>
              <line
                x1={e.from.x}
                y1={e.from.y}
                x2={e.to.x}
                y2={e.to.y}
                stroke="var(--er-line)"
                strokeWidth={1.5}
              />
              <circle cx={e.from.x} cy={e.from.y} r={3} fill="var(--accent)" />
              <circle cx={e.to.x} cy={e.to.y} r={3} fill="var(--accent)" />
            </g>
          ))}
        </svg>

        {cards.map((card) => (
          <TableCard
            key={card.table}
            card={card}
            onSelect={(t) => schema && void select(schema, t)}
            onDrag={(t, p) => setOverrides((o) => ({ ...o, [t]: p }))}
            onDragEnd={persist}
          />
        ))}
      </div>

      <div className="er-caption">
        {graph.schema} schema · {cards.length} of {graph.totalTables} tables · FK graph
      </div>
    </div>
  )
}
