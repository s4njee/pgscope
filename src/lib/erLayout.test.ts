import { describe, expect, it } from 'vitest'

import {
  canvasSize,
  layoutCards,
  layoutEdges,
  orderCards,
  ORIGIN_X,
  ORIGIN_Y,
  COL_STEP,
  MAX_CARDS,
} from './erLayout'
import type { ColumnMeta, FkGraph } from './types'

/** A column defaulting to a plain `text` non-key; layout only reads the key flags. */
function c(name: string, extra: Partial<ColumnMeta> = {}): ColumnMeta {
  return { name, dataType: 'text', notNull: false, isPk: false, isFk: false, ...extra }
}

/** The design's relationship graph: 5 cards, 5 FK edges. */
function designGraph(): FkGraph {
  return {
    schema: 'public',
    totalTables: 7,
    cards: [
      { table: 'users', columns: [c('user_id', { isPk: true }), c('email'), c('plan')] },
      {
        table: 'sessions',
        columns: [c('session_id', { isPk: true, dataType: 'uuid' }), c('user_id', { isFk: true })],
      },
      {
        table: 'events',
        columns: [
          c('event_id', { isPk: true, dataType: 'bigint' }),
          c('session_id', { isFk: true, dataType: 'uuid' }),
          c('user_id', { isFk: true }),
        ],
      },
      {
        table: 'page_views',
        columns: [c('view_id', { isPk: true, dataType: 'bigint' }), c('session_id', { isFk: true })],
      },
      {
        table: 'experiment_exposures',
        columns: [c('exposure_id', { isPk: true }), c('user_id', { isFk: true })],
      },
    ],
    edges: [
      { name: 'fk1', srcTable: 'sessions', tgtTable: 'users', srcColumns: ['user_id'], tgtColumns: ['user_id'] },
      { name: 'fk2', srcTable: 'events', tgtTable: 'sessions', srcColumns: ['session_id'], tgtColumns: ['session_id'] },
      { name: 'fk3', srcTable: 'events', tgtTable: 'users', srcColumns: ['user_id'], tgtColumns: ['user_id'] },
      { name: 'fk4', srcTable: 'page_views', tgtTable: 'sessions', srcColumns: ['session_id'], tgtColumns: ['session_id'] },
      { name: 'fk5', srcTable: 'experiment_exposures', tgtTable: 'users', srcColumns: ['user_id'], tgtColumns: ['user_id'] },
    ],
  }
}

describe('orderCards', () => {
  it('puts the selected table first, then its FK neighbours', () => {
    const ordered = orderCards(designGraph(), 'events')
    expect(ordered[0].table).toBe('events')
    // events references sessions and users, so those come next.
    expect(ordered.slice(1, 3).map((c) => c.table).sort()).toEqual(['sessions', 'users'])
  })

  it('is deterministic across runs', () => {
    const a = orderCards(designGraph(), 'events').map((c) => c.table)
    const b = orderCards(designGraph(), 'events').map((c) => c.table)
    expect(a).toEqual(b)
  })

  it('falls back to FK degree when nothing is selected', () => {
    const ordered = orderCards(designGraph(), null)
    // users (3 edges) and sessions (3 edges) are the busiest nodes.
    expect(['users', 'sessions']).toContain(ordered[0].table)
  })
})

describe('layoutCards', () => {
  it('places the first card at the design origin', () => {
    const cards = layoutCards(designGraph(), 'events')
    expect(cards[0].x).toBe(ORIGIN_X)
    expect(cards[0].y).toBe(ORIGIN_Y)
  })

  it('steps across columns then wraps to the next row', () => {
    const cards = layoutCards(designGraph(), 'events')
    expect(cards[1].x).toBe(ORIGIN_X + COL_STEP)
    expect(cards[3].x).toBe(ORIGIN_X) // wrapped
    expect(cards[3].y).toBeGreaterThan(cards[0].y)
  })

  it('widens the selected card, as the design does for events', () => {
    const cards = layoutCards(designGraph(), 'events')
    const events = cards.find((c) => c.table === 'events')!
    const users = cards.find((c) => c.table === 'users')!
    expect(events.selected).toBe(true)
    expect(events.width).toBe(224)
    expect(users.width).toBe(208)
  })

  it('honours dragged positions', () => {
    const cards = layoutCards(designGraph(), 'events', { events: { x: 726, y: 140 } })
    const events = cards.find((c) => c.table === 'events')!
    expect([events.x, events.y]).toEqual([726, 140])
  })

  it('caps the number of drawn cards', () => {
    const graph = designGraph()
    for (let i = 0; i < 20; i++) {
      graph.cards.push({ table: `extra_${i}`, columns: [c('id', { isPk: true })] })
      graph.edges.push({
        name: `fk_extra_${i}`,
        srcTable: `extra_${i}`,
        tgtTable: 'users',
        srcColumns: ['id'],
        tgtColumns: ['user_id'],
      })
    }
    expect(layoutCards(graph, 'events').length).toBe(MAX_CARDS)
  })
})

describe('layoutEdges', () => {
  it('draws every design edge', () => {
    const graph = designGraph()
    const cards = layoutCards(graph, 'events')
    expect(layoutEdges(cards, graph.edges)).toHaveLength(5)
  })

  it('anchors endpoints on card borders, not centres', () => {
    const graph = designGraph()
    const cards = layoutCards(graph, 'events')
    const edges = layoutEdges(cards, graph.edges)

    for (const e of edges) {
      // Each endpoint must lie on some card's edge.
      const onBorder = (p: { x: number; y: number }) =>
        cards.some(
          (c) =>
            (Math.abs(p.x - c.x) < 0.01 || Math.abs(p.x - (c.x + c.width)) < 0.01 ||
              Math.abs(p.y - c.y) < 0.01 || Math.abs(p.y - (c.y + c.height)) < 0.01),
        )
      expect(onBorder(e.from)).toBe(true)
      expect(onBorder(e.to)).toBe(true)
    }
  })

  it('skips edges whose endpoint card was not drawn', () => {
    const graph = designGraph()
    const cards = layoutCards(graph, 'events').filter((c) => c.table !== 'users')
    const edges = layoutEdges(cards, graph.edges)
    // The three edges touching users are dropped.
    expect(edges).toHaveLength(2)
  })
})

describe('canvasSize', () => {
  it('covers every card plus a margin', () => {
    const cards = layoutCards(designGraph(), 'events')
    const size = canvasSize(cards)
    for (const c of cards) {
      expect(size.width).toBeGreaterThanOrEqual(c.x + c.width)
      expect(size.height).toBeGreaterThanOrEqual(c.y + c.height)
    }
  })
})
