import type { ColumnMeta } from './types'

/**
 * Per-column cell colouring (plan.md §4.4).
 *
 * The design hand-picks colours for the `events` table; these rules generalise
 * that choice to any table while reproducing `events` exactly:
 * event_id (PK) secondary, user_id/session_id (FK) dim, event_name (text)
 * accent-light, properties (jsonb) amber, created_at (timestamp) dim.
 */
export type CellColorClass =
  | 'pk'
  | 'fk'
  | 'text'
  | 'json'
  | 'temporal'
  | 'numeric'
  | 'null'

const JSON_TYPES = /^(json|jsonb)$/
const TEMPORAL_TYPES = /^(timestamp|timestamptz|date|time|timetz|interval|timestamp with time zone|timestamp without time zone|time with time zone|time without time zone)$/
const NUMERIC_TYPES = /^(smallint|integer|int|int2|int4|int8|bigint|decimal|numeric|real|double precision|float4|float8|money|serial|bigserial)$/
const TEXTUAL_TYPES = /^(text|varchar|character varying|character|char|bpchar|name|citext)$/

/**
 * Postgres reports canonical SQL type names (`timestamp with time zone`), but
 * the design labels columns with the short aliases (`timestamptz`) throughout —
 * the grid header, details panel, and ER cards. Long names also overflow the
 * narrow ER card and details rows. Abbreviate for display.
 */
const TYPE_ALIASES: Record<string, string> = {
  'timestamp with time zone': 'timestamptz',
  'timestamp without time zone': 'timestamp',
  'time with time zone': 'timetz',
  'time without time zone': 'time',
  'character varying': 'varchar',
  'character': 'char',
  'double precision': 'float8',
  'bit varying': 'varbit',
  'boolean': 'bool',
  'integer': 'int4',
}

/** Display name for a type, keeping any modifier or array suffix intact. */
export function shortType(dataType: string): string {
  const lower = dataType.toLowerCase()

  // Preserve any modifier/array suffix: `character varying(20)[]` → `varchar(20)[]`.
  for (const [long, short] of Object.entries(TYPE_ALIASES)) {
    if (lower === long) return short
    if (lower.startsWith(long + '(')) return short + dataType.slice(long.length)
    if (lower === long + '[]') return short + '[]'
    if (lower.startsWith(long + '(') || lower.startsWith(long + '[')) {
      return short + dataType.slice(long.length)
    }
  }
  return dataType
}

/** Base type, with any modifier or array suffix stripped: `varchar(20)[]` → `varchar`. */
function baseType(dataType: string): string {
  return dataType
    .replace(/\[\]$/, '')
    .replace(/\(.*\)$/, '')
    .trim()
    .toLowerCase()
}

/**
 * Which colour role a column's values take.
 *
 * Decided from the column, not the value, so a column reads as one colour down
 * the whole grid — scanning for the odd row out is the point. The order of the
 * tests below is the precedence the design implies, not an arbitrary chain.
 */
export function cellColorClass(col: ColumnMeta): CellColorClass {
  const t = baseType(col.dataType)

  // json is visually distinct regardless of key role — the design's amber.
  if (JSON_TYPES.test(t)) return 'json'
  // FK columns read as references, not values.
  if (col.isFk) return 'fk'
  if (col.isPk) return 'pk'
  if (TEMPORAL_TYPES.test(t)) return 'temporal'
  if (t === 'uuid') return 'fk'
  if (NUMERIC_TYPES.test(t)) return 'pk'
  if (TEXTUAL_TYPES.test(t)) return 'text'
  // Enums, domains, extension types: treat as text.
  return 'text'
}

/** CSS custom property holding the colour for a class. */
export const CELL_COLOR_VAR: Record<CellColorClass, string> = {
  pk: 'var(--text-secondary)',
  fk: 'var(--text-dim)',
  temporal: 'var(--text-dim)',
  numeric: 'var(--text-secondary)',
  text: 'var(--accent-light)',
  json: 'var(--amber)',
  null: 'var(--text-faint)',
}

/**
 * The CSS colour for one cell. `null` is the one case the value decides: a SQL
 * NULL is faint everywhere so it never reads as the literal text "NULL".
 */
export function cellColor(col: ColumnMeta, value: string | null): string {
  if (value === null) return CELL_COLOR_VAR.null
  return CELL_COLOR_VAR[cellColorClass(col)]
}

/** Columns the design lets overflow with an ellipsis rather than wrap. */
export function shouldTruncate(col: ColumnMeta): boolean {
  const t = baseType(col.dataType)
  return JSON_TYPES.test(t) || t === 'uuid' || TEXTUAL_TYPES.test(t)
}

/**
 * Column widths by type class, defaulting to the design's values for the
 * equivalent `events` columns (plan.md §5.4).
 */
export function columnWidth(col: ColumnMeta): string {
  const t = baseType(col.dataType)
  if (JSON_TYPES.test(t)) return 'minmax(200px, 1fr)'
  if (t === 'uuid') return '150px'
  if (TEMPORAL_TYPES.test(t)) return '215px'
  if (NUMERIC_TYPES.test(t)) return '110px'
  if (t === 'boolean' || t === 'bool') return '80px'
  if (TEXTUAL_TYPES.test(t)) return col.isFk ? '104px' : '132px'
  return '140px'
}

/** The type line under a header name: `bigint · PK`, `text · FK`, `jsonb`. */
export function headerTypeLine(col: ColumnMeta): string {
  const t = shortType(col.dataType)
  if (col.isPk) return `${t} · PK`
  if (col.isFk) return `${t} · FK`
  return t
}

/** Details-panel badge, with PK > FK > NN precedence. */
export function columnBadge(col: ColumnMeta): 'PK' | 'FK' | 'NN' | null {
  if (col.isPk) return 'PK'
  if (col.isFk) return 'FK'
  if (col.notNull) return 'NN'
  return null
}
