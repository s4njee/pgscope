import { describe, expect, it } from 'vitest'

import {
  CELL_COLOR_VAR,
  cellColor,
  cellColorClass,
  columnBadge,
  columnWidth,
  headerTypeLine,
  shortType,
} from './cellColor'
import type { ColumnMeta } from './types'

/** A column with the flags defaulted off, so each case states only what it varies. */
function col(partial: Partial<ColumnMeta> & { name: string; dataType: string }): ColumnMeta {
  return { notNull: false, isPk: false, isFk: false, ...partial }
}

/** The `events` columns exactly as the design's grid shows them. */
const EVENTS = {
  event_id: col({ name: 'event_id', dataType: 'bigint', isPk: true, notNull: true }),
  user_id: col({ name: 'user_id', dataType: 'text', isFk: true }),
  session_id: col({ name: 'session_id', dataType: 'uuid', isFk: true }),
  event_name: col({ name: 'event_name', dataType: 'text', notNull: true }),
  properties: col({ name: 'properties', dataType: 'jsonb', notNull: true }),
  created_at: col({ name: 'created_at', dataType: 'timestamptz', notNull: true }),
}

describe('cellColorClass — reproduces the design exactly', () => {
  it('colours each events column the way the mock does', () => {
    // event_id → #8b99b0 (secondary)
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.event_id)]).toBe('var(--text-secondary)')
    // user_id, session_id → #6b7a92 (dim)
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.user_id)]).toBe('var(--text-dim)')
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.session_id)]).toBe('var(--text-dim)')
    // event_name → #7cb9e8 (accent light)
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.event_name)]).toBe('var(--accent-light)')
    // properties → #d9b26a (amber)
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.properties)]).toBe('var(--amber)')
    // created_at → #6b7a92 (dim)
    expect(CELL_COLOR_VAR[cellColorClass(EVENTS.created_at)]).toBe('var(--text-dim)')
  })
})

describe('cellColorClass — generalises to other types', () => {
  it('treats json as json even when it is also a key', () => {
    expect(cellColorClass(col({ name: 'j', dataType: 'json', isPk: true }))).toBe('json')
  })

  it('treats uuid as a reference-ish value', () => {
    expect(cellColorClass(col({ name: 'id', dataType: 'uuid' }))).toBe('fk')
  })

  it('handles type modifiers and arrays', () => {
    expect(cellColorClass(col({ name: 'v', dataType: 'varchar(255)' }))).toBe('text')
    expect(cellColorClass(col({ name: 'v', dataType: 'text[]' }))).toBe('text')
    expect(cellColorClass(col({ name: 'n', dataType: 'numeric(10,2)' }))).toBe('pk')
  })

  it('falls back to text for enums and unknown types', () => {
    expect(cellColorClass(col({ name: 'e', dataType: 'my_enum' }))).toBe('text')
  })

  it('is case-insensitive', () => {
    expect(cellColorClass(col({ name: 'j', dataType: 'JSONB' }))).toBe('json')
  })
})

describe('cellColor', () => {
  it('renders NULL in the faint colour regardless of type', () => {
    expect(cellColor(EVENTS.properties, null)).toBe('var(--text-faint)')
    expect(cellColor(EVENTS.event_name, null)).toBe('var(--text-faint)')
  })
})

describe('headerTypeLine — matches the design grid header', () => {
  it('renders the second header line for each events column', () => {
    expect(headerTypeLine(EVENTS.event_id)).toBe('bigint · PK')
    expect(headerTypeLine(EVENTS.user_id)).toBe('text · FK')
    expect(headerTypeLine(EVENTS.session_id)).toBe('uuid · FK')
    expect(headerTypeLine(EVENTS.event_name)).toBe('text')
    expect(headerTypeLine(EVENTS.properties)).toBe('jsonb')
    expect(headerTypeLine(EVENTS.created_at)).toBe('timestamptz')
  })
})

describe('columnBadge — PK > FK > NN precedence', () => {
  it('badges the events columns as the details panel does', () => {
    expect(columnBadge(EVENTS.event_id)).toBe('PK')
    expect(columnBadge(EVENTS.user_id)).toBe('FK')
    expect(columnBadge(EVENTS.session_id)).toBe('FK')
    expect(columnBadge(EVENTS.event_name)).toBe('NN')
    expect(columnBadge(EVENTS.properties)).toBe('NN')
    expect(columnBadge(EVENTS.created_at)).toBe('NN')
  })

  it('prefers PK when a column is both PK and NN', () => {
    expect(columnBadge(col({ name: 'id', dataType: 'bigint', isPk: true, notNull: true }))).toBe('PK')
  })

  it('returns null for a plain nullable column', () => {
    expect(columnBadge(col({ name: 'x', dataType: 'text' }))).toBeNull()
  })
})

describe('columnWidth — defaults to the design track sizes', () => {
  it('sizes the events columns as the design template does', () => {
    // grid-template-columns: 44px 110px 104px 150px 132px 1fr 215px
    expect(columnWidth(EVENTS.event_id)).toBe('110px')
    expect(columnWidth(EVENTS.user_id)).toBe('104px')
    expect(columnWidth(EVENTS.session_id)).toBe('150px')
    expect(columnWidth(EVENTS.event_name)).toBe('132px')
    expect(columnWidth(EVENTS.properties)).toBe('minmax(200px, 1fr)')
    expect(columnWidth(EVENTS.created_at)).toBe('215px')
  })
})

describe('shortType — the design labels types with short aliases', () => {
  it('abbreviates the canonical names Postgres reports', () => {
    // format_type() returns the long form; the design shows `timestamptz`.
    expect(shortType('timestamp with time zone')).toBe('timestamptz')
    expect(shortType('timestamp without time zone')).toBe('timestamp')
    expect(shortType('character varying')).toBe('varchar')
    expect(shortType('double precision')).toBe('float8')
    expect(shortType('boolean')).toBe('bool')
  })

  it('preserves modifiers and array suffixes', () => {
    expect(shortType('character varying(255)')).toBe('varchar(255)')
    expect(shortType('timestamp with time zone[]')).toBe('timestamptz[]')
  })

  it('leaves already-short and unknown types alone', () => {
    expect(shortType('jsonb')).toBe('jsonb')
    expect(shortType('bigint')).toBe('bigint')
    expect(shortType('uuid')).toBe('uuid')
    expect(shortType('my_custom_enum')).toBe('my_custom_enum')
  })

  it('feeds the grid header type line', () => {
    expect(
      headerTypeLine({
        name: 'created_at',
        dataType: 'timestamp with time zone',
        notNull: true,
        isPk: false,
        isFk: false,
      }),
    ).toBe('timestamptz')
  })
})
