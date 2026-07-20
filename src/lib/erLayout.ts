import type { FkCard, FkEdge, FkGraph } from './types'

/**
 * Card geometry, seeded from the design's coordinates: 208px cards (224 when
 * selected) on a 3-column grid starting at (70, 60) with 322×270 steps.
 */
export const CARD_W = 208
export const CARD_W_SELECTED = 224
export const CARD_HEADER_H = 31
export const CARD_ROW_H = 21
export const CARD_PAD_Y = 12
export const COL_STEP = 322
export const ROW_STEP = 270
export const ORIGIN_X = 70
export const ORIGIN_Y = 60
export const COLUMNS = 3
/** Cap on drawn cards — beyond this the canvas stops being readable. */
export const MAX_CARDS = 9

export interface Point {
  x: number
  y: number
}

export interface PlacedCard extends FkCard {
  x: number
  y: number
  width: number
  height: number
  selected: boolean
}

/** A card grows with its column count; nothing scrolls inside one. */
export function cardHeight(card: FkCard): number {
  return CARD_HEADER_H + CARD_PAD_Y + card.columns.length * CARD_ROW_H
}

/**
 * Order cards so the selected table and its FK neighbourhood come first, then
 * the rest by FK degree. This keeps the interesting part of the graph in the
 * top-left where the eye lands, and makes layout deterministic across runs.
 */
export function orderCards(graph: FkGraph, selectedTable: string | null): FkCard[] {
  const degree = new Map<string, number>()
  for (const e of graph.edges) {
    degree.set(e.srcTable, (degree.get(e.srcTable) ?? 0) + 1)
    degree.set(e.tgtTable, (degree.get(e.tgtTable) ?? 0) + 1)
  }

  const neighbours = new Set<string>()
  if (selectedTable) {
    for (const e of graph.edges) {
      if (e.srcTable === selectedTable) neighbours.add(e.tgtTable)
      if (e.tgtTable === selectedTable) neighbours.add(e.srcTable)
    }
  }

  const rank = (c: FkCard): number => {
    if (c.table === selectedTable) return 0
    if (neighbours.has(c.table)) return 1
    return 2
  }

  return [...graph.cards].sort((a, b) => {
    const r = rank(a) - rank(b)
    if (r !== 0) return r
    const d = (degree.get(b.table) ?? 0) - (degree.get(a.table) ?? 0)
    if (d !== 0) return d
    return a.table.localeCompare(b.table)
  })
}

/** Place ordered cards on the grid, honouring any user-dragged positions. */
export function layoutCards(
  graph: FkGraph,
  selectedTable: string | null,
  overrides: Record<string, Point> = {},
): PlacedCard[] {
  const ordered = orderCards(graph, selectedTable).slice(0, MAX_CARDS)

  return ordered.map((card, i) => {
    const col = i % COLUMNS
    const row = Math.floor(i / COLUMNS)
    const override = overrides[card.table]
    const selected = card.table === selectedTable
    return {
      ...card,
      x: override?.x ?? ORIGIN_X + col * COL_STEP,
      y: override?.y ?? ORIGIN_Y + row * ROW_STEP,
      width: selected ? CARD_W_SELECTED : CARD_W,
      height: cardHeight(card),
      selected,
    }
  })
}

export interface PlacedEdge {
  key: string
  from: Point
  to: Point
}

/**
 * Connect the nearest pair of card-edge midpoints, so lines touch card borders
 * rather than running under the cards to their centres.
 */
export function layoutEdges(cards: PlacedCard[], edges: FkEdge[]): PlacedEdge[] {
  const byTable = new Map(cards.map((c) => [c.table, c]))
  const out: PlacedEdge[] = []

  for (const e of edges) {
    const a = byTable.get(e.srcTable)
    const b = byTable.get(e.tgtTable)
    if (!a || !b) continue // one end isn't drawn

    const anchorsA = anchorPoints(a)
    const anchorsB = anchorPoints(b)

    let best: { from: Point; to: Point; d: number } | null = null
    for (const pa of anchorsA) {
      for (const pb of anchorsB) {
        const d = (pa.x - pb.x) ** 2 + (pa.y - pb.y) ** 2
        if (!best || d < best.d) best = { from: pa, to: pb, d }
      }
    }
    if (best) {
      out.push({ key: e.name, from: best.from, to: best.to })
    }
  }

  return out
}

/** The four card-edge midpoints an FK line may attach to. */
function anchorPoints(c: PlacedCard): Point[] {
  const midX = c.x + c.width / 2
  const midY = c.y + c.height / 2
  return [
    { x: c.x, y: midY }, // left
    { x: c.x + c.width, y: midY }, // right
    { x: midX, y: c.y }, // top
    { x: midX, y: c.y + c.height }, // bottom
  ]
}

/** Canvas extent, so the surface scrolls when cards are dragged outward. */
export function canvasSize(cards: PlacedCard[]): { width: number; height: number } {
  let width = 0
  let height = 0
  for (const c of cards) {
    width = Math.max(width, c.x + c.width + ORIGIN_X)
    height = Math.max(height, c.y + c.height + ORIGIN_Y)
  }
  return { width, height }
}
