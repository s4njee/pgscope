/**
 * Browser-only fixtures standing in for the Rust backend.
 *
 * `ipc.call` routes here whenever the app is not running inside the Tauri
 * shell, so the demo drives the real components and the real stores — nothing
 * is mocked above this line. That is the point: what a screenshot shows is the
 * actual UI, only the data is invented.
 *
 * The world modelled here mirrors `dev/seed.sql` object for object, so the demo
 * and the dev database show the same schema, columns and index names. Rows are
 * generated from a seeded PRNG rather than `Math.random`, because screenshots
 * have to be reproducible across reloads and machines.
 */

import type { TableLayout } from './columnLayout'
import type {
  AppError,
  CellRequest,
  CellValue,
  ColumnMeta,
  Completion,
  CompletionResult,
  ConnectionInfo,
  ExplainResult,
  FkGraph,
  HistoryItem,
  IndexMeta,
  MovedQuery,
  PageRequest,
  PageResult,
  PlanNode,
  Profile,
  QueryResultSet,
  QueryRun,
  RelKind,
  ReplOutput,
  ReplSession,
  RowFormatRequest,
  SavedQuery,
  SchemaNode,
  Segment,
  StatementRange,
  StatementResult,
  TableMeta,
  TableStats,
} from './types'

/** Matches the backend's `db::grid::PAGE_SIZE`, which the footer arithmetic assumes. */
const PAGE_SIZE = 50

/** The database the whole fixture pretends to be connected to. */
const DATABASE = 'analytics_prod'

/**
 * Fixed clock for every generated timestamp.
 *
 * Anchoring to a constant rather than `Date.now()` keeps two screenshots taken
 * a week apart byte-identical. History ages are the one deliberate exception,
 * since a relative age frozen in the past reads as a bug.
 */
const NOW_MS = Date.parse('2026-07-14T16:42:00Z')

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/**
 * Deterministic PRNG (mulberry32) — the same seed always yields the same run.
 *
 * @param seed - `number` — any 32-bit integer; distinct seeds give independent
 *   streams, which is how each table gets its own reproducible data.
 * @returns `() => number` — successive floats in `[0, 1)`.
 */
function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/**
 * Pick from a list by a PRNG draw, weighted towards the head when `bias > 1`.
 *
 * @param next - `() => number` — the PRNG.
 * @param items - `readonly T[]` — candidates; must be non-empty.
 * @param bias - `number` — exponent applied to the draw. 1 is uniform; larger
 *   values concentrate on early entries, which is what makes an event stream
 *   look like real traffic instead of a uniform sample.
 * @returns `T` — the chosen item.
 */
function pick<T>(next: () => number, items: readonly T[], bias = 1): T {
  const r = bias === 1 ? next() : Math.pow(next(), bias)
  return items[Math.min(items.length - 1, Math.floor(r * items.length))]
}

/**
 * An integer in `[lo, hi]` from one PRNG draw.
 *
 * @param next - `() => number` — the PRNG.
 * @param lo - `number` — inclusive lower bound.
 * @param hi - `number` — inclusive upper bound.
 * @returns `number` — the drawn integer.
 */
function int(next: () => number, lo: number, hi: number): number {
  return lo + Math.floor(next() * (hi - lo + 1))
}

/**
 * A lowercase hex string of fixed length, drawn from the PRNG.
 *
 * @param next - `() => number` — the PRNG.
 * @param len - `number` — number of hex digits.
 * @returns `string` — the digits, with no `0x` prefix.
 */
function hex(next: () => number, len: number): string {
  let out = ''
  for (let i = 0; i < len; i++) out += '0123456789abcdef'[int(next, 0, 15)]
  return out
}

/**
 * A v4-shaped UUID built from PRNG digits.
 *
 * @param next - `() => number` — the PRNG.
 * @returns `string` — the canonical 8-4-4-4-12 text form Postgres prints.
 */
function uuid(next: () => number): string {
  return [
    hex(next, 8),
    hex(next, 4),
    `4${hex(next, 3)}`,
    `${'89ab'[int(next, 0, 3)]}${hex(next, 3)}`,
    hex(next, 12),
  ].join('-')
}

/**
 * A timestamptz in Postgres' text output form.
 *
 * @param ms - `number` — Unix milliseconds.
 * @returns `string` — e.g. `2026-07-14 16:42:00.318+00`, the shape the text
 *   protocol delivers and therefore what the grid has to render.
 */
function ts(ms: number): string {
  const iso = new Date(ms).toISOString()
  return `${iso.slice(0, 10)} ${iso.slice(11, 23)}+00`
}

/**
 * A date in Postgres' text output form.
 *
 * @param ms - `number` — Unix milliseconds.
 * @returns `string` — e.g. `2026-07-14`.
 */
function dateOf(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10)
}

/**
 * Render an object the way `jsonb::text` does.
 *
 * jsonb does not preserve input key order — it stores keys sorted by length
 * then bytewise, and prints them that way. Reproducing that ordering is what
 * keeps the `properties` column looking like it came off the wire.
 *
 * @param obj - `Record<string, unknown>` — the value to render.
 * @returns `string` — compact JSON with jsonb's key order and spacing.
 */
function jsonbText(obj: Record<string, unknown>): string {
  const keys = Object.keys(obj).sort((a, b) => a.length - b.length || (a < b ? -1 : a > b ? 1 : 0))
  const body = keys.map((k) => `${JSON.stringify(k)}: ${JSON.stringify(obj[k])}`).join(', ')
  return `{${body}}`
}

/**
 * Reject with the backend's error shape.
 *
 * This path bypasses `toAppError`, so anything thrown from here reaches the
 * error banners unmodified — a bare string would surface as `[object Object]`
 * or worse.
 *
 * @param code - `AppError['code']` — the discriminant the UI switches on.
 * @param message - `string` — the one-line summary shown in the banner.
 * @param detail - `string | null` — the server's DETAIL line, if any.
 * @returns `never` — always throws.
 */
function fail(code: AppError['code'], message: string, detail: string | null = null): never {
  const err: AppError = { code, message, detail, sqlstate: null }
  throw err
}

/**
 * Sleep long enough that spinners and skeletons actually appear.
 *
 * Without this every fixture resolves in the same microtask and the loading
 * states — which are part of what the screenshots are meant to show — would
 * never render at all.
 *
 * @param ms - `number` — milliseconds to wait.
 * @returns `Promise<void>` — resolves after the delay.
 */
function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** Rotating latencies, so repeated calls do not all take exactly the same time. */
const LATENCIES = [92, 118, 84, 141, 103, 127]
let latencyTick = 0

/**
 * The next artificial round-trip time.
 *
 * @returns `number` — milliseconds in the 80–150ms band.
 */
function nextLatency(): number {
  return LATENCIES[latencyTick++ % LATENCIES.length]
}

/**
 * Quote an identifier the way `db::grid::quote_ident` does.
 *
 * @param ident - `string` — the raw name.
 * @returns `string` — double-quoted, with embedded quotes doubled.
 */
function quoteIdent(ident: string): string {
  return `"${ident.replace(/"/g, '""')}"`
}

/**
 * Quote a string literal the way `db::grid::quote_literal` does.
 *
 * @param s - `string` — the raw value.
 * @returns `string` — single-quoted, with embedded quotes doubled.
 */
function quoteLiteral(s: string): string {
  return `'${s.replace(/'/g, "''")}'`
}

/**
 * Base type name with modifiers and array suffix stripped.
 *
 * @param dataType - `string` — the introspected type, e.g. `character varying(20)[]`.
 * @returns `string` — the bare lowercased name, e.g. `character varying`.
 */
function baseType(dataType: string): string {
  return dataType.replace(/\[\]$/, '').split('(')[0].trim().toLowerCase()
}

// ---------------------------------------------------------------------------
// Catalog — mirrors dev/seed.sql
// ---------------------------------------------------------------------------

/**
 * Terse column spec, expanded into `ColumnMeta` by [`cols`].
 *
 * The flag string keeps the table definitions readable: `p` primary key,
 * `f` foreign key, `n` not null.
 */
type ColSpec = [name: string, dataType: string, flags?: string]

/**
 * Expand terse column specs into wire-shaped column metadata.
 *
 * @param specs - `ColSpec[]` — name, type and flag string per column.
 * @returns `ColumnMeta[]` — with `notNull` implied by a primary key, since
 *   Postgres never reports a nullable PK column.
 */
function cols(specs: ColSpec[]): ColumnMeta[] {
  return specs.map(([name, dataType, flags = '']) => ({
    name,
    dataType,
    notNull: flags.includes('n') || flags.includes('p'),
    isPk: flags.includes('p'),
    isFk: flags.includes('f'),
  }))
}

/** One relation in the fixture catalog, rows included. */
interface DemoRelation {
  schema: string
  name: string
  kind: RelKind
  columns: ColumnMeta[]
  indexes: IndexMeta[]
  stats: TableStats
  /** Foreign-key constraints originating here, for `\d` and the ER graph. */
  fks?: { name: string; columns: string[]; target: string; targetColumns: string[] }[]
  /** Deferred so a table nobody opens costs nothing to generate. */
  generate: () => (string | null)[][]
}

const PLANS = ['free', 'pro', 'team', 'enterprise'] as const
const DEVICES = ['desktop-chrome', 'mobile-safari', 'desktop-firefox', 'tablet-safari', 'desktop-edge']
const EVENT_NAMES = ['page_view', 'session_start', 'click', 'feature_used', 'signup', 'purchase']
const PATHS = ['/', '/pricing', '/docs/quickstart', '/blog/postgres-18', '/app/dashboard', '/app/queries', '/changelog']
const REFERRERS = [
  'https://news.ycombinator.com/',
  'https://www.google.com/',
  'https://github.com/',
  'https://lobste.rs/',
  null,
]
const EXPERIMENTS = ['onboarding_v3', 'pricing_page_copy', 'query_editor_autorun', 'sidebar_density']
const FEATURES = ['saved_queries', 'er_diagram', 'explain_viewer', 'terminal', 'column_reorder']
const FIRST_NAMES = ['ada', 'grace', 'linus', 'nadia', 'omar', 'priya', 'ravi', 'sofia', 'theo', 'yuki']
const LAST_NAMES = ['okafor', 'lindqvist', 'moreau', 'nakamura', 'silva', 'kowalski', 'haddad', 'weber']
const DOMAINS = ['acme.io', 'northwind.dev', 'globex.co', 'initech.com', 'umbrella.sh']

const USER_COUNT = 480
const SESSION_COUNT = 960
const EVENT_COUNT = 2400
const PAGE_VIEW_COUNT = 1200
const EXPOSURE_COUNT = 600
const PROPERTY_COUNT = 320
const FUNNEL_COUNT = 214

/**
 * The generated user ids, shared so foreign keys in other tables resolve.
 *
 * @returns `string[]` — one id per row of `public.users`, in table order.
 */
const userIds = memo((): string[] => {
  const next = rng(0x5eed_0001)
  return Array.from({ length: USER_COUNT }, () => `usr_${hex(next, 8)}`)
})

/**
 * The generated session ids, shared with `events` and `page_views`.
 *
 * @returns `string[]` — one uuid per row of `public.sessions`, in table order.
 */
const sessionIds = memo((): string[] => {
  const next = rng(0x5eed_0002)
  return Array.from({ length: SESSION_COUNT }, () => uuid(next))
})

/**
 * Wrap a generator so it runs at most once.
 *
 * @param make - `() => T` — the expensive computation.
 * @returns `() => T` — the memoised accessor.
 */
function memo<T>(make: () => T): () => T {
  let cached: T | undefined
  return () => (cached ??= make())
}

/**
 * Properties payload appropriate to an event name.
 *
 * @param next - `() => number` — the PRNG.
 * @param name - `string` — the event name being generated.
 * @returns `Record<string, unknown>` — the object to render as jsonb.
 */
function propertiesFor(next: () => number, name: string): Record<string, unknown> {
  switch (name) {
    case 'page_view':
      return { path: pick(next, PATHS), ms: int(next, 41, 2180), referrer: pick(next, REFERRERS) }
    case 'click':
      return { path: pick(next, PATHS), target: `cta_${pick(next, ['signup', 'demo', 'docs', 'pricing'])}`, x: int(next, 12, 1380), y: int(next, 40, 900) }
    case 'signup':
      return { plan: pick(next, PLANS), source: pick(next, ['organic', 'referral', 'paid', 'docs']) }
    case 'purchase':
      return { plan: pick(next, PLANS.slice(1)), seats: int(next, 1, 40), amount_usd: int(next, 20, 4800) }
    case 'session_start':
      return { device: pick(next, DEVICES), tz: pick(next, ['America/New_York', 'Europe/Berlin', 'Asia/Tokyo', 'UTC']) }
    default:
      return { feature: pick(next, FEATURES), count: int(next, 1, 24) }
  }
}

const RELATIONS: DemoRelation[] = [
  // ---- public, alphabetical: this is the order the backend's introspection
  // query returns, and the sidebar renders the array as given. ----
  {
    schema: 'public',
    name: 'event_properties',
    kind: 'table',
    columns: cols([
      ['property_id', 'bigint', 'p'],
      ['event_name', 'text', 'n'],
      ['key', 'text', 'n'],
      ['value_type', 'text', 'n'],
      ['sample_value', 'text'],
      ['occurrences', 'bigint', 'n'],
      ['first_seen', 'timestamp with time zone', 'n'],
      ['last_seen', 'timestamp with time zone', 'n'],
    ]),
    indexes: [
      { name: 'event_properties_pkey', definition: 'btree (property_id) · unique' },
      { name: 'idx_event_properties_name_key', definition: 'btree (event_name, key)' },
    ],
    stats: { estRows: 321, totalBytes: 106496, indexBytes: 49152, lastAutovacuumSecs: 8_412 },
    generate: () => {
      const next = rng(0x5eed_0007)
      // This is the catalog of what `events.properties` actually contains, so a
      // key's declared type and its sample have to agree — a `path` typed
      // `boolean` reads as a bug in the app rather than as plausible data.
      const shapes: [key: string, valueType: string, samples: readonly (string | null)[]][] = [
        ['path', 'string', PATHS],
        ['referrer', 'string', REFERRERS],
        ['target', 'string', ['cta_signup', 'cta_demo', 'cta_docs', 'cta_pricing']],
        ['plan', 'string', PLANS],
        ['source', 'string', ['organic', 'referral', 'paid', 'docs']],
        ['device', 'string', DEVICES],
        ['tz', 'string', ['America/New_York', 'Europe/Berlin', 'Asia/Tokyo', 'UTC']],
        ['feature', 'string', FEATURES],
        ['ms', 'number', ['41', '214', '883', '2180']],
        ['count', 'number', ['1', '3', '7', '24']],
        ['seats', 'number', ['1', '5', '18', '40']],
        ['amount_usd', 'number', ['20', '240', '1200', '4800']],
        ['x', 'number', ['12', '412', '918', '1380']],
        ['y', 'number', ['40', '288', '604', '900']],
        ['is_trial', 'boolean', ['true', 'false']],
        ['returning', 'boolean', ['true', 'false']],
      ]
      return Array.from({ length: PROPERTY_COUNT }, (_, i) => {
        const name = EVENT_NAMES[i % EVENT_NAMES.length]
        const [key, valueType, samples] = shapes[(i * 7) % shapes.length]
        return [
          String(700_000 + i),
          name,
          key,
          valueType,
          pick(next, samples),
          String(int(next, 120, 1_840_000)),
          ts(NOW_MS - int(next, 60, 320) * 86_400_000),
          ts(NOW_MS - int(next, 0, 3) * 3_600_000),
        ]
      })
    },
  },
  {
    schema: 'public',
    name: 'events',
    kind: 'table',
    columns: cols([
      ['event_id', 'bigint', 'p'],
      ['user_id', 'text', 'f'],
      ['session_id', 'uuid', 'f'],
      ['event_name', 'text', 'n'],
      ['properties', 'jsonb', 'n'],
      ['created_at', 'timestamp with time zone', 'n'],
    ]),
    // Exactly the four the details panel lists, in its order.
    indexes: [
      { name: 'events_pkey', definition: 'btree (event_id) · unique' },
      { name: 'idx_events_user_created', definition: 'btree (user_id, created_at DESC)' },
      { name: 'idx_events_name', definition: 'btree (event_name)' },
      { name: 'brin_events_created_at', definition: 'brin (created_at)' },
    ],
    stats: { estRows: 2412, totalBytes: 1_179_648, indexBytes: 401_408, lastAutovacuumSecs: 2_418 },
    fks: [
      { name: 'events_user_id_fkey', columns: ['user_id'], target: 'users', targetColumns: ['user_id'] },
      { name: 'events_session_id_fkey', columns: ['session_id'], target: 'sessions', targetColumns: ['session_id'] },
    ],
    generate: () => {
      const next = rng(0x5eed_0003)
      const users = userIds()
      const sessions = sessionIds()
      return Array.from({ length: EVENT_COUNT }, (_, i) => {
        const name = pick(next, EVENT_NAMES, 1.9)
        // Descending time, so the natural (unsorted) order reads newest-first.
        const at = NOW_MS - i * 37_000 - int(next, 0, 36_000)
        return [
          String(4_812_003_100 + (EVENT_COUNT - i)),
          next() < 0.05 ? null : pick(next, users),
          next() < 0.03 ? null : pick(next, sessions),
          name,
          jsonbText(propertiesFor(next, name)),
          ts(at),
        ]
      })
    },
  },
  {
    schema: 'public',
    name: 'experiment_exposures',
    kind: 'table',
    columns: cols([
      ['exposure_id', 'bigint', 'p'],
      ['user_id', 'text', 'f'],
      ['experiment', 'text', 'n'],
      ['variant', 'text', 'n'],
      ['exposed_at', 'timestamp with time zone', 'n'],
    ]),
    indexes: [
      { name: 'experiment_exposures_pkey', definition: 'btree (exposure_id) · unique' },
      { name: 'idx_exposures_experiment', definition: 'btree (experiment, variant)' },
    ],
    stats: { estRows: 600, totalBytes: 155_648, indexBytes: 65_536, lastAutovacuumSecs: 19_804 },
    fks: [{ name: 'experiment_exposures_user_id_fkey', columns: ['user_id'], target: 'users', targetColumns: ['user_id'] }],
    generate: () => {
      const next = rng(0x5eed_0006)
      const users = userIds()
      return Array.from({ length: EXPOSURE_COUNT }, (_, i) => [
        String(310_000 + i),
        next() < 0.02 ? null : pick(next, users),
        pick(next, EXPERIMENTS),
        pick(next, ['control', 'treatment', 'treatment_b'], 1.4),
        ts(NOW_MS - i * 611_000 - int(next, 0, 600_000)),
      ])
    },
  },
  {
    schema: 'public',
    name: 'funnels',
    kind: 'table',
    columns: cols([
      ['funnel_id', 'integer', 'p'],
      ['name', 'text', 'n'],
      ['steps', 'text[]', 'n'],
      ['owner', 'text', 'n'],
      ['window_days', 'integer', 'n'],
      ['is_active', 'boolean', 'n'],
      ['created_at', 'timestamp with time zone', 'n'],
    ]),
    indexes: [
      { name: 'funnels_pkey', definition: 'btree (funnel_id) · unique' },
      { name: 'funnels_name_key', definition: 'btree (name) · unique' },
    ],
    stats: { estRows: 214, totalBytes: 81_920, indexBytes: 49_152, lastAutovacuumSecs: 121_320 },
    generate: () => {
      const next = rng(0x5eed_0008)
      const shapes = [
        ['signup', 'activate', 'purchase'],
        ['page_view', 'click', 'signup'],
        ['session_start', 'feature_used'],
        ['page_view', 'signup', 'feature_used', 'purchase'],
      ]
      return Array.from({ length: FUNNEL_COUNT }, (_, i) => {
        const steps = pick(next, shapes)
        return [
          String(1 + i),
          `${pick(next, ['signup', 'activation', 'expansion', 'retention', 'checkout'])}_${pick(next, ['weekly', 'mobile', 'enterprise', 'self_serve'])}_${i + 1}`,
          `{${steps.join(',')}}`,
          `${pick(next, FIRST_NAMES)}@${pick(next, DOMAINS)}`,
          String(pick(next, [1, 3, 7, 14, 30])),
          next() < 0.72 ? 't' : 'f',
          ts(NOW_MS - int(next, 1, 540) * 86_400_000),
        ]
      })
    },
  },
  {
    schema: 'public',
    name: 'page_views',
    kind: 'table',
    columns: cols([
      ['view_id', 'bigint', 'p'],
      ['session_id', 'uuid', 'f'],
      ['path', 'text', 'n'],
      ['referrer', 'text'],
    ]),
    indexes: [
      { name: 'page_views_pkey', definition: 'btree (view_id) · unique' },
      { name: 'idx_page_views_session', definition: 'btree (session_id)' },
    ],
    stats: { estRows: 1200, totalBytes: 311_296, indexBytes: 114_688, lastAutovacuumSecs: 6_140 },
    fks: [{ name: 'page_views_session_id_fkey', columns: ['session_id'], target: 'sessions', targetColumns: ['session_id'] }],
    generate: () => {
      const next = rng(0x5eed_0005)
      const sessions = sessionIds()
      return Array.from({ length: PAGE_VIEW_COUNT }, (_, i) => [
        String(880_000 + (PAGE_VIEW_COUNT - i)),
        next() < 0.02 ? null : pick(next, sessions),
        pick(next, PATHS, 1.5),
        pick(next, REFERRERS),
      ])
    },
  },
  {
    schema: 'public',
    name: 'sessions',
    kind: 'table',
    columns: cols([
      ['session_id', 'uuid', 'p'],
      ['user_id', 'text', 'fn'],
      ['device', 'text', 'n'],
      ['started_at', 'timestamp with time zone', 'n'],
    ]),
    indexes: [
      { name: 'sessions_pkey', definition: 'btree (session_id) · unique' },
      { name: 'idx_sessions_user_started', definition: 'btree (user_id, started_at DESC)' },
    ],
    stats: { estRows: 958, totalBytes: 262_144, indexBytes: 98_304, lastAutovacuumSecs: 4_902 },
    fks: [{ name: 'sessions_user_id_fkey', columns: ['user_id'], target: 'users', targetColumns: ['user_id'] }],
    generate: () => {
      const next = rng(0x5eed_0004)
      const users = userIds()
      const ids = sessionIds()
      return ids.map((id, i) => [
        id,
        pick(next, users),
        pick(next, DEVICES, 1.6),
        ts(NOW_MS - i * 92_000 - int(next, 0, 90_000)),
      ])
    },
  },
  {
    schema: 'public',
    name: 'users',
    kind: 'table',
    columns: cols([
      ['user_id', 'text', 'p'],
      ['email', 'text', 'n'],
      ['plan', 'text', 'n'],
      ['created_at', 'timestamp with time zone', 'n'],
    ]),
    indexes: [{ name: 'users_pkey', definition: 'btree (user_id) · unique' }],
    stats: { estRows: 479, totalBytes: 90_112, indexBytes: 32_768, lastAutovacuumSecs: 31_268 },
    generate: () => {
      const next = rng(0x5eed_0009)
      return userIds().map((id) => [
        id,
        `${pick(next, FIRST_NAMES)}.${pick(next, LAST_NAMES)}${int(next, 2, 99)}@${pick(next, DOMAINS)}`,
        pick(next, PLANS, 1.7),
        ts(NOW_MS - int(next, 1, 720) * 86_400_000),
      ])
    },
  },

  // ---- public views ----
  {
    schema: 'public',
    name: 'dau_last_30d',
    kind: 'view',
    columns: cols([
      ['day', 'date'],
      ['active_users', 'bigint'],
      ['events', 'bigint'],
    ]),
    indexes: [],
    stats: { estRows: -1, totalBytes: null, indexBytes: null, lastAutovacuumSecs: null },
    generate: () => {
      const next = rng(0x5eed_0010)
      return Array.from({ length: 30 }, (_, i) => {
        const active = 3100 + int(next, -240, 380) + i * 12
        return [dateOf(NOW_MS - i * 86_400_000), String(active), String(active * int(next, 6, 11))]
      })
    },
  },
  {
    schema: 'public',
    name: 'funnel_signup_activate',
    kind: 'view',
    columns: cols([
      ['plan', 'text'],
      ['signups', 'bigint'],
      ['activations', 'bigint'],
      ['purchases', 'bigint'],
    ]),
    indexes: [],
    stats: { estRows: -1, totalBytes: null, indexBytes: null, lastAutovacuumSecs: null },
    generate: () => [
      ['free', '18422', '9104', '412'],
      ['pro', '6218', '4877', '2841'],
      ['team', '2104', '1893', '1502'],
      ['enterprise', '318', '301', '288'],
    ],
  },
  {
    schema: 'public',
    name: 'top_events_hourly',
    kind: 'view',
    columns: cols([
      ['hour', 'timestamp with time zone'],
      ['event_name', 'text'],
      ['events', 'bigint'],
    ]),
    indexes: [],
    stats: { estRows: -1, totalBytes: null, indexBytes: null, lastAutovacuumSecs: null },
    generate: () => {
      const next = rng(0x5eed_0011)
      const out: (string | null)[][] = []
      for (let h = 0; h < 24; h++) {
        const hour = ts(Math.floor((NOW_MS - h * 3_600_000) / 3_600_000) * 3_600_000)
        for (const name of EVENT_NAMES) {
          out.push([hour, name, String(int(next, 40, 9400))])
        }
      }
      return out
    },
  },

  // ---- analytics ----
  {
    schema: 'analytics',
    name: 'daily_active_users',
    kind: 'table',
    columns: cols([
      ['day', 'date', 'p'],
      ['active_users', 'integer', 'n'],
      ['new_users', 'integer', 'n'],
      ['events', 'bigint', 'n'],
    ]),
    indexes: [{ name: 'daily_active_users_pkey', definition: 'btree (day) · unique' }],
    stats: { estRows: 365, totalBytes: 73_728, indexBytes: 32_768, lastAutovacuumSecs: 74_120 },
    generate: () => {
      const next = rng(0x5eed_0012)
      return Array.from({ length: 365 }, (_, i) => {
        const active = 2400 + Math.round(700 * Math.sin(i / 24)) + int(next, -160, 160)
        return [dateOf(NOW_MS - i * 86_400_000), String(active), String(int(next, 12, 190)), String(active * int(next, 5, 12))]
      })
    },
  },
  {
    schema: 'analytics',
    name: 'retention_cohorts',
    kind: 'table',
    columns: cols([
      ['cohort_week', 'date', 'p'],
      ['week_offset', 'integer', 'p'],
      ['cohort_size', 'integer', 'n'],
      ['retained', 'integer', 'n'],
    ]),
    indexes: [{ name: 'retention_cohorts_pkey', definition: 'btree (cohort_week, week_offset) · unique' }],
    stats: { estRows: 312, totalBytes: 65_536, indexBytes: 32_768, lastAutovacuumSecs: 74_118 },
    generate: () => {
      const next = rng(0x5eed_0013)
      const out: (string | null)[][] = []
      for (let w = 0; w < 26; w++) {
        const week = dateOf(NOW_MS - w * 7 * 86_400_000)
        const size = 900 + int(next, -180, 220)
        for (let off = 0; off < 12; off++) {
          out.push([week, String(off), String(size), String(Math.round(size * Math.pow(0.86, off)))])
        }
      }
      return out
    },
  },
  {
    schema: 'analytics',
    name: 'event_totals',
    kind: 'view',
    columns: cols([
      ['event_name', 'text'],
      ['total_events', 'bigint'],
      ['unique_users', 'bigint'],
      ['first_seen', 'timestamp with time zone'],
      ['last_seen', 'timestamp with time zone'],
    ]),
    indexes: [],
    stats: { estRows: -1, totalBytes: null, indexBytes: null, lastAutovacuumSecs: null },
    generate: () => {
      const next = rng(0x5eed_0014)
      return EVENT_NAMES.map((name, i) => [
        name,
        String(1_842_311 >> i),
        String(41_882 >> i),
        ts(NOW_MS - 720 * 86_400_000),
        ts(NOW_MS - int(next, 60, 4000) * 1000),
      ])
    },
  },
]

/** Every relation by `schema.name`, so lookups do not scan the array. */
const BY_KEY = new Map(RELATIONS.map((r) => [`${r.schema}.${r.name}`, r]))

/** Generated rows, kept once produced — paging must not regenerate the pool. */
const ROW_CACHE = new Map<string, (string | null)[][]>()

/**
 * Look up a relation, rejecting the way the backend does for a missing one.
 *
 * @param schema - `string` — the schema name, unquoted.
 * @param table - `string` — the relation name, unquoted. A stale name may
 *   arrive from the persisted explorer selection, so this has to reject
 *   cleanly rather than assume the default table.
 * @returns `DemoRelation` — the catalog entry.
 */
function relationOf(schema: string, table: string): DemoRelation {
  const rel = BY_KEY.get(`${schema}.${table}`)
  if (!rel) fail('query', `relation "${schema}.${table}" does not exist`, 'demo fixture has no such relation')
  return rel
}

/**
 * All rows of a relation, generated on first use.
 *
 * @param rel - `DemoRelation` — the catalog entry.
 * @returns `(string | null)[][]` — the full row pool, positionally matched to
 *   `rel.columns`.
 */
function rowsOf(rel: DemoRelation): (string | null)[][] {
  const key = `${rel.schema}.${rel.name}`
  let rows = ROW_CACHE.get(key)
  if (!rows) {
    rows = rel.generate()
    ROW_CACHE.set(key, rows)
  }
  return rows
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

/** Values Postgres prints unquoted, and which therefore sort numerically here. */
const NUMERIC_RE = /^-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/

/**
 * Compare two cells the way an `ORDER BY` on that column would.
 *
 * Numeric text sorts numerically rather than lexically, and NULLs sort last on
 * ascending — both are Postgres defaults, and getting either wrong makes a
 * sorted screenshot look subtly broken.
 *
 * @param a - `string | null` — left cell.
 * @param b - `string | null` — right cell.
 * @returns `number` — negative, zero or positive, for ascending order.
 */
function compareCells(a: string | null, b: string | null): number {
  if (a === b) return 0
  if (a === null) return 1
  if (b === null) return -1
  if (NUMERIC_RE.test(a) && NUMERIC_RE.test(b)) return Number(a) - Number(b)
  return a < b ? -1 : 1
}

/**
 * A predicate for one simple comparison, or `null` when it does not parse.
 *
 * Only the forms the grid itself generates (`value_predicate`) and the obvious
 * hand-typed ones are understood. Anything else falls back to a substring
 * match, which keeps the filter box useful without pretending to be a parser.
 *
 * @param term - `string` — one predicate, already split on `AND`.
 * @param index - `Map<string, number>` — column name to positional index.
 * @returns `((row: (string | null)[]) => boolean) | null` — the test, or null.
 */
function compileTerm(
  term: string,
  index: Map<string, number>,
): ((row: (string | null)[]) => boolean) | null {
  const unquote = (s: string) => (s.startsWith('"') ? s.slice(1, -1).replace(/""/g, '"') : s)
  const ident = String.raw`("[^"]*"|[A-Za-z_][A-Za-z0-9_$]*)`

  const nullMatch = term.match(new RegExp(`^${ident}\\s+IS\\s+(NOT\\s+)?NULL$`, 'i'))
  if (nullMatch) {
    const i = index.get(unquote(nullMatch[1]))
    if (i === undefined) return null
    const negated = Boolean(nullMatch[2])
    return (row) => (row[i] === null) !== negated
  }

  const likeMatch = term.match(new RegExp(`^${ident}\\s+(I?LIKE)\\s+'((?:[^']|'')*)'$`, 'i'))
  if (likeMatch) {
    const i = index.get(unquote(likeMatch[1]))
    if (i === undefined) return null
    const pattern = likeMatch[3].replace(/''/g, "'")
    const rx = new RegExp(
      `^${pattern.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/%/g, '.*').replace(/_/g, '.')}$`,
      likeMatch[2].toLowerCase() === 'ilike' ? 'i' : '',
    )
    return (row) => row[i] !== null && rx.test(row[i] as string)
  }

  const cmpMatch = term.match(new RegExp(`^${ident}\\s*(<>|!=|>=|<=|=|>|<)\\s*(.+)$`, 'i'))
  if (cmpMatch) {
    const i = index.get(unquote(cmpMatch[1]))
    if (i === undefined) return null
    const op = cmpMatch[2]
    let literal = cmpMatch[3].trim().replace(/::[A-Za-z_ ]+$/, '')
    if (literal.startsWith("'") && literal.endsWith("'")) literal = literal.slice(1, -1).replace(/''/g, "'")
    return (row) => {
      const v = row[i]
      if (v === null) return false
      const c = compareCells(v, literal)
      switch (op) {
        case '=':
          return c === 0
        case '<>':
        case '!=':
          return c !== 0
        case '>':
          return c > 0
        case '>=':
          return c >= 0
        case '<':
          return c < 0
        default:
          return c <= 0
      }
    }
  }
  return null
}

/**
 * Turn a filter expression into a row test.
 *
 * @param filter - `string` — the raw filter box contents; a leading `WHERE` is
 *   tolerated, as the backend does.
 * @param columns - `ColumnMeta[]` — used to resolve column names to positions.
 * @returns `(row: (string | null)[]) => boolean` — the test to apply.
 */
function compileFilter(filter: string, columns: ColumnMeta[]): (row: (string | null)[]) => boolean {
  const text = filter.trim().replace(/^where\s+/i, '').trim()
  if (!text) return () => true

  const index = new Map(columns.map((c, i) => [c.name, i]))
  const terms = text.split(/\s+AND\s+/i).map((t) => compileTerm(t.trim(), index))
  if (terms.every((t) => t !== null)) {
    const tests = terms as ((row: (string | null)[]) => boolean)[]
    return (row) => tests.every((t) => t(row))
  }

  // Unparseable: match anything containing the text, so typing a value still
  // narrows the grid instead of emptying it.
  const needle = text.toLowerCase()
  return (row) => row.some((v) => v !== null && v.toLowerCase().includes(needle))
}

/**
 * The SQL the backend would have run for this page, for the footer strip.
 *
 * @param req - `PageRequest` — the request being served.
 * @param columns - `ColumnMeta[]` — the relation's columns, cast to text.
 * @returns `string` — a single-line statement, matching `db::grid`'s shape.
 */
function pageSql(req: PageRequest, columns: ColumnMeta[]): string {
  const projection = columns.map((c) => `${quoteIdent(c.name)}::text`).join(', ')
  const relation = `${quoteIdent(req.schema)}.${quoteIdent(req.table)}`
  const where = req.filter?.trim() ? ` WHERE ${req.filter.trim().replace(/^where\s+/i, '')}` : ''
  const order = req.sort.length
    ? ` ORDER BY ${req.sort.map((k) => `${quoteIdent(k.column)} ${k.dir.toUpperCase()}`).join(', ')}`
    : ''
  return `SELECT ${projection} FROM ${relation}${where}${order} LIMIT ${PAGE_SIZE} OFFSET ${req.page * PAGE_SIZE}`
}

/**
 * Serve one page of a relation, honouring paging, sort and filter.
 *
 * @param req - `PageRequest` — schema, table, sort terms, filter and 0-based page.
 * @returns `PageResult` — rows for that page plus the exact total, since a
 *   fixture can always count.
 */
function fetchPage(req: PageRequest): PageResult {
  const rel = relationOf(req.schema, req.table)
  let rows = rowsOf(rel)

  if (req.filter?.trim()) rows = rows.filter(compileFilter(req.filter, rel.columns))

  if (req.sort.length > 0) {
    const keys = req.sort
      .map((k) => ({ i: rel.columns.findIndex((c) => c.name === k.column), dir: k.dir }))
      .filter((k) => k.i >= 0)
    rows = rows.slice().sort((a, b) => {
      for (const { i, dir } of keys) {
        const c = compareCells(a[i], b[i])
        if (c !== 0) return dir === 'desc' ? -c : c
      }
      return 0
    })
  }

  const offset = req.page * PAGE_SIZE
  return {
    rows: rows.slice(offset, offset + PAGE_SIZE),
    timingMs: 3.1 + (req.sort.length ? 9.4 : 0) + (req.filter ? 6.2 : 0),
    total: rows.length,
    totalIsEstimate: false,
    page: req.page,
    pageSize: PAGE_SIZE,
    sql: pageSql(req, rel.columns),
  }
}

/**
 * The full value behind one cell, as the expansion panel shows it.
 *
 * @param req - `CellRequest` — column, primary key, originating page and the
 *   row's index within that page.
 * @returns `CellValue` — json values arrive pretty-printed, matching what
 *   Postgres' `jsonb_pretty` would deliver.
 */
function fetchCell(req: CellRequest): CellValue {
  const page = fetchPage(req.page)
  const rel = relationOf(req.page.schema, req.page.table)
  const i = rel.columns.findIndex((c) => c.name === req.column)
  if (i < 0) fail('query', `column "${req.column}" does not exist`)

  const raw = page.rows[req.rowIndex]?.[i] ?? null
  const column = rel.columns[i]
  const isJson = baseType(column.dataType) === 'json' || baseType(column.dataType) === 'jsonb'

  let value = raw
  if (isJson && raw !== null) {
    try {
      value = JSON.stringify(JSON.parse(raw), null, 4)
    } catch {
      value = raw
    }
  }

  return {
    column: req.column,
    dataType: column.dataType,
    value,
    format: isJson ? 'json' : 'text',
    totalBytes: raw === null ? 0 : new TextEncoder().encode(raw).length,
    truncated: false,
    // The panel warns when a row was found by position; only a real PK avoids it.
    locatedByPk: req.pk.length > 0,
  }
}

/**
 * A value written as a SQL literal for its column's type.
 *
 * @param column - `ColumnMeta` — only `dataType` is read.
 * @param value - `string | null` — the raw cell; `null` is a real SQL NULL.
 * @returns `string` — `NULL`, a bare number or boolean, or a quoted string with
 *   `::jsonb` appended for json columns.
 */
function sqlLiteral(column: ColumnMeta, value: string | null): string {
  if (value === null) return 'NULL'
  const base = baseType(column.dataType)
  const bare =
    !column.dataType.trimEnd().endsWith('[]') &&
    ['smallint', 'integer', 'int', 'int2', 'int4', 'int8', 'bigint', 'decimal', 'numeric', 'real', 'double precision', 'float4', 'float8', 'boolean', 'bool'].includes(base)
  if (bare && (NUMERIC_RE.test(value) || ['true', 'false', 't', 'f'].includes(value))) return value

  const quoted = quoteLiteral(value)
  return base === 'json' || base === 'jsonb' ? `${quoted}::jsonb` : quoted
}

/**
 * Render a row in one of the copy formats.
 *
 * @param req - `RowFormatRequest` — columns, values and target format.
 * @returns `string` — CSV carries a header line and a trailing newline; TSV and
 *   INSERT do not; JSON is pretty-printed.
 */
function formatRow(req: RowFormatRequest): string {
  const at = (i: number) => req.values[i] ?? null

  switch (req.format) {
    case 'json': {
      const obj: Record<string, unknown> = {}
      req.columns.forEach((c, i) => {
        const raw = at(i)
        if (raw === null) {
          obj[c.name] = null
          return
        }
        const base = baseType(c.dataType)
        if (!c.dataType.endsWith('[]') && NUMERIC_RE.test(raw) && base !== 'text' && base !== 'uuid') {
          obj[c.name] = Number(raw)
        } else if (raw === 't' || raw === 'f') {
          obj[c.name] = raw === 't'
        } else if (base === 'json' || base === 'jsonb') {
          try {
            obj[c.name] = JSON.parse(raw)
          } catch {
            obj[c.name] = raw
          }
        } else {
          obj[c.name] = raw
        }
      })
      return JSON.stringify(obj, null, 2)
    }
    case 'csv': {
      const field = (v: string | null) =>
        v === null ? '' : /[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v
      const header = req.columns.map((c) => field(c.name)).join(',')
      return `${header}\n${req.columns.map((_, i) => field(at(i))).join(',')}\n`
    }
    case 'tsv':
      return req.columns
        .map((_, i) => (at(i) ?? '').replace(/\\/g, '\\\\').replace(/\t/g, '\\t').replace(/\n/g, '\\n').replace(/\r/g, '\\r'))
        .join('\t')
    default: {
      const names = req.columns.map((c) => quoteIdent(c.name)).join(', ')
      const values = req.columns.map((c, i) => sqlLiteral(c, at(i))).join(', ')
      return `INSERT INTO ${quoteIdent(req.schema)}.${quoteIdent(req.table)} (${names})\nVALUES (${values});`
    }
  }
}

// ---------------------------------------------------------------------------
// Terminal
// ---------------------------------------------------------------------------

/** Per-session REPL state; the buffer is what drives the continuation prompt. */
interface DemoReplState {
  buffer: string
  timing: boolean
  expanded: boolean
}

const REPL_SESSIONS = new Map<string, DemoReplState>()

/**
 * The prompt for a session, reflecting whether a statement is mid-flight.
 *
 * @param state - `DemoReplState` — the session's buffer and flags.
 * @returns `string` — `analytics_prod=#` fresh, `analytics_prod-#` continuing.
 *   The trailing space is the terminal's, not ours.
 */
function promptFor(state: DemoReplState): string {
  return `${DATABASE}${state.buffer.trim() ? '-' : '='}#`
}

/**
 * The `(N rows)` footer psql prints under a result set.
 *
 * @param n - `number` — the row count.
 * @returns `string` — `(1 row)` or `(N rows)`, newline-terminated.
 */
function rowFooter(n: number): string {
  return `(${n} row${n === 1 ? '' : 's'})\n`
}

/**
 * Render a result set in psql's aligned (`border 1`) style.
 *
 * Reproduced from `src-tauri/src/repl/table.rs` down to the padding rules: the
 * header centres with the extra space on the right, numeric columns right-align,
 * and the final column carries no trailing pad. Those details are most of what
 * makes terminal output read as genuine rather than as a table someone drew.
 *
 * @param headers - `string[]` — column names.
 * @param rows - `(string | null)[][]` — values, positional against `headers`.
 * @returns `string` — the header, rule, rows and `(N rows)` footer.
 */
function formatAligned(headers: string[], rows: (string | null)[][]): string {
  if (headers.length === 0) return `--\n${rowFooter(rows.length)}`

  const text = (v: string | null) => v ?? ''
  const numeric = headers.map(
    (_, i) =>
      rows.some((r) => r[i] !== null) && rows.every((r) => r[i] === null || NUMERIC_RE.test(r[i] as string)),
  )
  const widths = headers.map((h, i) => rows.reduce((w, r) => Math.max(w, text(r[i]).length), h.length))

  const header = headers
    .map((h, i) => {
      const pad = widths[i] - h.length
      const left = Math.floor(pad / 2)
      return ` ${' '.repeat(left)}${h}${' '.repeat(pad - left)} `
    })
    .join('|')
  const rule = widths.map((w) => '-'.repeat(w + 2)).join('+')

  const body = rows
    .map((r) =>
      headers
        .map((_, i) => {
          const v = text(r[i])
          const last = i === headers.length - 1
          const padded = numeric[i] ? v.padStart(widths[i]) : last ? v : v.padEnd(widths[i])
          return ` ${padded}${last ? '' : ' '}`
        })
        .join('|'),
    )
    .map((line) => `${line}\n`)
    .join('')

  return `${header}\n${rule}\n${body}${rowFooter(rows.length)}`
}

/** The canonical scrollback from the design: top event names by volume. */
const TOP_EVENTS: [string, string][] = [
  ['page_view', '1842311'],
  ['session_start', '409112'],
  ['click', '322480'],
  ['signup', '12055'],
  ['purchase', '4310'],
]

/**
 * Answer a submitted REPL line.
 *
 * @param state - `DemoReplState` — mutated in place for `\timing`, `\x` and the
 *   continuation buffer.
 * @param input - `string` — the line as typed, without a trailing newline.
 * @returns `Segment[]` — the body only. The terminal echoes the prompt and the
 *   input itself, so repeating them here would double every line.
 */
function replRun(state: DemoReplState, input: string): Segment[] {
  const line = input.trim()
  if (!line && !state.buffer) return []

  // Meta commands are only recognised at the start of a fresh statement.
  if (!state.buffer && line.startsWith('\\')) return metaCommand(state, line)

  state.buffer = `${state.buffer} ${line}`.trim()
  if (!state.buffer.endsWith(';')) return []

  const statement = state.buffer.slice(0, -1).trim()
  state.buffer = ''

  const started = performance.now()
  const segments = runStatement(state, statement)
  if (state.timing) {
    // A fixture has no real elapsed time worth showing, so this is the design's
    // figure nudged by the actual (tiny) render cost rather than invented anew.
    segments.push({ text: `Time: ${(428.116 + (performance.now() - started)).toFixed(3)} ms\n`, kind: 'dim' })
  }
  return segments
}

/**
 * Run one complete SQL statement.
 *
 * @param state - `DemoReplState` — read for the expanded flag.
 * @param statement - `string` — the statement, without its terminating `;`.
 * @returns `Segment[]` — one body segment, or an error segment.
 */
function runStatement(state: DemoReplState, statement: string): Segment[] {
  const lower = statement.toLowerCase()
  if (!/^(select|table|with|show|explain)\b/.test(lower)) {
    // The driver does not surface real command tags, so every non-returning
    // statement renders the same way the backend renders it.
    return [{ text: 'OK\n', kind: 'body' }]
  }

  const set = resultSetFor(statement)
  if (!set) {
    return [{ text: `ERROR:  relation "${lower.match(/from\s+([\w.]+)/)?.[1] ?? '?'}" does not exist\n`, kind: 'error' }]
  }
  if (state.expanded) return [{ text: formatExpanded(set), kind: 'body' }]
  return [{ text: formatAligned(set.columns, set.rows), kind: 'body' }]
}

/**
 * Render a result set in psql's expanded (`\x`) form.
 *
 * @param set - `QueryResultSet` — columns and rows.
 * @returns `string` — one `-[ RECORD n ]` block per row.
 */
function formatExpanded(set: QueryResultSet): string {
  if (set.rows.length === 0) return '(0 rows)\n'
  const nameWidth = set.columns.reduce((w, c) => Math.max(w, c.length), 0)
  const valueWidth = set.rows.reduce(
    (w, r) => r.reduce((m, v) => Math.max(m, (v ?? '').length), w),
    0,
  )
  return set.rows
    .map((row, i) => {
      const label = `-[ RECORD ${i + 1} ]`
      const width = Math.max(nameWidth + 3 + valueWidth, label.length)
      const rule = label + '-'.repeat(Math.max(0, width - label.length))
      const lines = set.columns.map((c, j) => `${c.padEnd(nameWidth)} | ${row[j] ?? ''}`)
      return `${rule}\n${lines.join('\n')}\n`
    })
    .join('')
}

/**
 * The result set a `SELECT` would produce.
 *
 * A query naming a real relation returns that relation's first rows, so
 * `select * from users;` shows the same data the grid does. Anything else falls
 * back to the design's top-events summary.
 *
 * @param statement - `string` — the statement text.
 * @returns `QueryResultSet | null` — null when it names a relation that does
 *   not exist, so the caller can report the error psql would.
 */
function resultSetFor(statement: string): QueryResultSet | null {
  const from = statement.match(/\bfrom\s+([A-Za-z_][\w.]*)/i)
  if (!from) {
    return { columns: ['event_name', 'count'], rows: TOP_EVENTS.map(([a, b]) => [a, b]), totalRows: TOP_EVENTS.length, truncated: false }
  }

  const ref = from[1].toLowerCase()
  const rel = BY_KEY.get(ref.includes('.') ? ref : `public.${ref}`)
  if (!rel) {
    // A grouped count over a missing relation is still the design's shape, but
    // an unknown name should error rather than quietly return something.
    if (/group\s+by/i.test(statement)) {
      return { columns: ['event_name', 'count'], rows: TOP_EVENTS.map(([a, b]) => [a, b]), totalRows: TOP_EVENTS.length, truncated: false }
    }
    return null
  }

  // A grouped count over events is the canonical demo query; keep its answer.
  if (rel.name === 'events' && /group\s+by/i.test(statement) && /count\s*\(/i.test(statement)) {
    return { columns: ['event_name', 'count'], rows: TOP_EVENTS.map(([a, b]) => [a, b]), totalRows: TOP_EVENTS.length, truncated: false }
  }

  const limit = Number(statement.match(/\blimit\s+(\d+)/i)?.[1] ?? 20)
  const all = rowsOf(rel)
  return {
    columns: rel.columns.map((c) => c.name),
    rows: all.slice(0, Math.min(limit, 200)),
    totalRows: all.length,
    truncated: all.length > Math.min(limit, 200),
  }
}

/** What `\?` prints, trimmed to the commands this fixture actually answers. */
const HELP_TEXT = `General
  \\q                     quit (pgscope keeps the session open)
  \\timing [on|off]       toggle timing of commands
  \\x [on|off]            toggle expanded output

Informational
  \\d[+]  NAME            describe table, view or index
  \\dt[+]                 list tables
  \\dn                    list schemas
  \\conninfo              display information about the current connection
`

/**
 * Answer a backslash command.
 *
 * @param state - `DemoReplState` — toggled in place by `\timing` and `\x`.
 * @param line - `string` — the command as typed, including the backslash.
 * @returns `Segment[]` — the rendered response.
 */
function metaCommand(state: DemoReplState, line: string): Segment[] {
  const [cmd, ...rest] = line.split(/\s+/)
  const arg = rest.join(' ')

  switch (cmd) {
    case '\\timing': {
      state.timing = arg === '' ? !state.timing : arg.toLowerCase() === 'on'
      return [{ text: `Timing is ${state.timing ? 'on' : 'off'}.\n`, kind: 'dim' }]
    }
    case '\\x': {
      state.expanded = arg === '' ? !state.expanded : arg.toLowerCase() === 'on'
      return [{ text: `Expanded display is ${state.expanded ? 'on' : 'off'}.\n`, kind: 'dim' }]
    }
    case '\\dt':
    case '\\dt+': {
      const rows = RELATIONS.filter((r) => r.kind === 'table' && r.schema === 'public').map((r) => [
        r.schema,
        r.name,
        'table',
        'postgres',
      ])
      return [{ text: formatAligned(['Schema', 'Name', 'Type', 'Owner'], rows), kind: 'body' }]
    }
    case '\\dn':
      return [
        {
          text: formatAligned(
            ['Name', 'Owner'],
            [
              ['analytics', 'postgres'],
              ['public', 'pg_database_owner'],
            ],
          ),
          kind: 'body',
        },
      ]
    case '\\conninfo':
      return [
        {
          text: formatAligned(
            ['Database', 'User', 'Host', 'Port', 'Server version'],
            [[DATABASE, 'postgres', '127.0.0.1', '5432', '17.4']],
          ),
          kind: 'body',
        },
      ]
    case '\\d':
    case '\\d+':
      return describe(arg.trim())
    case '\\encoding':
      return [{ text: 'UTF8\n', kind: 'body' }]
    case '\\q':
      return [
        { text: 'Use the window controls to close pgscope; the session stays open.\n', kind: 'dim' },
      ]
    case '\\?':
      return [{ text: `${HELP_TEXT}\n`, kind: 'dim' }]
    default:
      return [{ text: `invalid command ${cmd}\nTry \\? for help.\n`, kind: 'error' }]
  }
}

/**
 * `\d <relation>` — columns, then indexes, then foreign keys.
 *
 * @param name - `string` — the relation, optionally schema-qualified. Empty
 *   lists the relations instead, as psql's bare `\d` does.
 * @returns `Segment[]` — dim headings with aligned tables under them.
 */
function describe(name: string): Segment[] {
  if (!name) return metaCommand({ buffer: '', timing: false, expanded: false }, '\\dt')

  const key = name.includes('.') ? name : `public.${name}`
  const rel = BY_KEY.get(key)
  if (!rel) return [{ text: `Did not find any relation named "${name}".\n`, kind: 'dim' }]

  const segments: Segment[] = [
    { text: `${rel.kind === 'view' ? 'View' : 'Table'} "${rel.schema}.${rel.name}"\n`, kind: 'dim' },
    {
      text: formatAligned(
        ['Column', 'Type', 'Nullable', 'Default'],
        rel.columns.map((c) => [c.name, c.dataType, c.notNull ? 'not null' : '', '']),
      ),
      kind: 'body',
    },
  ]

  if (rel.indexes.length > 0) {
    segments.push({ text: '\nIndexes:\n', kind: 'dim' })
    segments.push({
      text: formatAligned(['Index', 'Definition'], rel.indexes.map((i) => [i.name, i.definition])),
      kind: 'body',
    })
  }
  if (rel.fks && rel.fks.length > 0) {
    segments.push({ text: '\nForeign-key constraints:\n', kind: 'dim' })
    segments.push({
      text: formatAligned(
        ['Constraint', 'Definition'],
        rel.fks.map((fk) => [
          fk.name,
          `FOREIGN KEY (${fk.columns.join(', ')}) REFERENCES ${fk.target}(${fk.targetColumns.join(', ')})`,
        ]),
      ),
      kind: 'body',
    })
  }
  return segments
}

const SQL_KEYWORDS = [
  'select', 'from', 'where', 'group by', 'order by', 'limit', 'offset', 'join',
  'left join', 'inner join', 'insert into', 'update', 'delete from', 'values',
  'having', 'distinct', 'count', 'sum', 'avg', 'date_trunc', 'now',
]

/**
 * Tab-completion candidates for a REPL line.
 *
 * @param line - `string` — the whole input line.
 * @param cursor - `number` — character offset of the cursor.
 * @returns `CompletionResult` — the token's bounds, the candidates, and the
 *   longest shared prefix the terminal inserts when several match.
 */
function replComplete(line: string, cursor: number): CompletionResult {
  let start = cursor
  while (start > 0 && /[\w.\\]/.test(line[start - 1])) start--
  const token = line.slice(start, cursor).toLowerCase()

  const items: Completion[] = []
  if (token.startsWith('\\')) {
    for (const meta of ['\\d', '\\dt', '\\dn', '\\df', '\\timing', '\\x', '\\conninfo', '\\encoding', '\\?', '\\q']) {
      if (meta.startsWith(token)) items.push({ value: meta, kind: 'meta', detail: null })
    }
  } else {
    for (const rel of RELATIONS) {
      if (rel.name.startsWith(token)) {
        items.push({ value: rel.name, kind: rel.kind, detail: rel.schema })
      }
    }
    for (const rel of RELATIONS) {
      for (const c of rel.columns) {
        if (c.name.startsWith(token) && !items.some((i) => i.value === c.name)) {
          items.push({ value: c.name, kind: 'column', detail: c.dataType })
        }
      }
    }
    for (const kw of SQL_KEYWORDS) {
      if (kw.startsWith(token)) items.push({ value: kw, kind: 'keyword', detail: null })
    }
    for (const s of ['public', 'analytics']) {
      if (s.startsWith(token)) items.push({ value: s, kind: 'schema', detail: null })
    }
  }

  let commonPrefix = items.length > 0 ? items[0].value : token
  for (const item of items) {
    let i = 0
    while (i < commonPrefix.length && i < item.value.length && commonPrefix[i] === item.value[i]) i++
    commonPrefix = commonPrefix.slice(0, i)
  }

  return { start, end: cursor, items, commonPrefix }
}

// ---------------------------------------------------------------------------
// Query editor
// ---------------------------------------------------------------------------

/**
 * Split a buffer into statements with accurate character offsets.
 *
 * Quoted strings, quoted identifiers and both comment forms are skipped so a
 * `;` inside them does not split the buffer — the one thing a naive splitter
 * gets wrong in a way the user notices.
 *
 * @param sql - `string` — the whole editor buffer.
 * @returns `StatementRange[]` — one per non-empty statement, `text` trimmed and
 *   without its terminating `;`.
 */
function splitSql(sql: string): StatementRange[] {
  const out: StatementRange[] = []
  let start = 0
  let i = 0

  const push = (end: number, dropSemicolon: boolean) => {
    const raw = sql.slice(start, end)
    const body = dropSemicolon ? raw.replace(/;\s*$/, '') : raw
    if (body.trim()) out.push({ start, end, text: body.trim() })
  }

  while (i < sql.length) {
    const c = sql[i]
    if (c === "'" || c === '"') {
      const quote = c
      i++
      while (i < sql.length && !(sql[i] === quote && sql[i + 1] !== quote)) i += sql[i] === quote ? 2 : 1
      i++
    } else if (c === '-' && sql[i + 1] === '-') {
      while (i < sql.length && sql[i] !== '\n') i++
    } else if (c === '/' && sql[i + 1] === '*') {
      i += 2
      while (i < sql.length && !(sql[i] === '*' && sql[i + 1] === '/')) i++
      i += 2
    } else if (c === ';') {
      push(i + 1, true)
      start = i + 1
      i++
    } else {
      i++
    }
  }
  push(sql.length, false)
  return out
}

/**
 * Run a buffer, one result set per statement that returns rows.
 *
 * @param sql - `string` — the buffer to execute.
 * @returns `QueryRun` — per-statement results and the summed timing.
 */
function runQuery(sql: string): QueryRun {
  const ranges = splitSql(sql)
  if (ranges.length === 0) fail('query', 'no statement to run')

  const statements: StatementResult[] = ranges.map((range, i) => {
    const returns = /^\s*(select|table|with|show|values)\b/i.test(range.text)
    const set = returns ? resultSetFor(range.text) : null
    if (returns && !set) {
      fail('query', `relation "${range.text.match(/from\s+([\w.]+)/i)?.[1] ?? '?'}" does not exist`, 'demo fixture has no such relation')
    }
    return {
      sql: range.text,
      result: set,
      notices: returns ? [] : ['statement executed against the demo fixture; nothing was written'],
      timingMs: 4.2 + i * 2.7 + (set ? set.rows.length * 0.02 : 0),
    }
  })

  return { statements, totalTimingMs: statements.reduce((t, s) => t + s.timingMs, 0) }
}

/**
 * The demo plan: a Gather over a parallel seq scan, hash-joined to users and
 * aggregated.
 *
 * The shape is chosen for what it exercises in the viewer — the parallel seq
 * scan is the heaviest node so it earns both the slowest bar and the seq-scan
 * flag, and the hash join's row estimate is off by 40× so the misestimation
 * badge appears next to it.
 *
 * @param analyze - `boolean` — false strips every measured field, which is what
 *   a plain `EXPLAIN` returns and what puts the viewer in cost mode.
 * @param sql - `string` — the statement being explained.
 * @returns `ExplainResult` — the tree plus its summary strip.
 */
function explainQuery(sql: string, analyze: boolean): ExplainResult {
  const node = (
    id: string,
    nodeType: string,
    costs: [number, number],
    plan: [number, number],
    actual: [number, number, number, number] | null,
    self: [number, number],
    details: [string, string][],
    children: PlanNode[] = [],
    parallel = false,
  ): PlanNode => ({
    id,
    nodeType,
    startupCost: costs[0],
    totalCost: costs[1],
    planRows: plan[0],
    planWidth: plan[1],
    actualStartupTime: analyze && actual ? actual[0] : null,
    actualTotalTime: analyze && actual ? actual[1] : null,
    actualRows: analyze && actual ? actual[2] : null,
    actualLoops: analyze && actual ? actual[3] : null,
    selfTimeMs: analyze ? self[0] : null,
    selfCost: self[1],
    rowRatio: analyze && actual ? actual[2] / plan[0] : null,
    parallel,
    details,
    children,
  })

  const seqScanEvents = node(
    '0.0.0.0',
    'Parallel Seq Scan',
    [0, 41_882.3],
    [202_596, 82],
    [0.031, 604.118, 202_180, 3],
    [812.44, 41_882.3],
    [
      ['Relation', 'public.events'],
      ['Filter', "(created_at > (now() - '30 days'::interval))"],
      ['Rows Removed by Filter', '408,612'],
      ['Workers Planned', '2'],
      ['Workers Launched', '2'],
    ],
    [],
    true,
  )

  const gather = node(
    '0.0.0',
    'Gather',
    [1000, 62_411.9],
    [486_231, 82],
    [0.884, 741.203, 486_231, 1],
    [96.31, 20_529.6],
    [
      ['Workers Planned', '2'],
      ['Workers Launched', '2'],
    ],
    [seqScanEvents],
  )

  const seqScanUsers = node(
    '0.0.1.0',
    'Seq Scan',
    [0, 9.79],
    [268, 41],
    [0.014, 0.412, 267, 1],
    [41.06, 9.79],
    [
      ['Relation', 'public.users'],
      ['Filter', "(plan <> 'free'::text)"],
      ['Rows Removed by Filter', '212'],
    ],
  )

  const hash = node(
    '0.0.1',
    'Hash',
    [9.79, 9.79],
    [268, 41],
    [0.492, 0.492, 267, 1],
    [58.22, 0],
    [
      ['Buckets', '1024'],
      ['Batches', '1'],
      ['Memory Usage', '24kB'],
    ],
    [seqScanUsers],
  )

  // The join's estimate is the deliberately bad one: 12,000 planned against
  // 486,231 actual is a 40.5x under-estimate, which is what the warn badge shows.
  const hashJoin = node(
    '0.0',
    'Hash Join',
    [11.5, 68_204.4],
    [12_000, 118],
    [1.204, 984.551, 486_231, 1],
    [214.87, 5792.5],
    [
      ['Hash Cond', '(e.user_id = u.user_id)'],
      ['Join Type', 'Inner'],
    ],
    [gather, hash],
  )

  const aggregate = node(
    '0',
    'HashAggregate',
    [68_234.4, 68_236.9],
    [3, 40],
    [1187.402, 1187.418, 3, 1],
    [12.73, 32.5],
    [
      ['Group Key', 'u.plan'],
      ['Output', 'u.plan, count(*), count(DISTINCT e.user_id)'],
    ],
    [hashJoin],
  )

  return {
    plan: aggregate,
    planningTimeMs: 0.842,
    executionTimeMs: analyze ? 1187.63 : null,
    analyzed: analyze,
    // pgscope wraps EXPLAIN ANALYZE in a transaction it rolls back, so an
    // analyzed plan never leaves writes behind.
    rolledBack: analyze,
    sql,
    maxSelfTimeMs: analyze ? 812.44 : null,
    maxSelfCost: 41_882.3,
  }
}

// ---------------------------------------------------------------------------
// Sidebar fixtures
// ---------------------------------------------------------------------------

const SAVED_FOLDERS = ['reports', 'reports/weekly', 'scratch']

const SAVED_QUERIES: SavedQuery[] = [
  {
    name: 'reports/dau_last_30d',
    path: '/Users/demo/Library/Application Support/pgscope/saved/reports/dau_last_30d.sql',
    content:
      "SELECT date_trunc('day', created_at)::date AS day,\n       count(DISTINCT user_id)            AS active_users,\n       count(*)                           AS events\nFROM public.events\nWHERE created_at > now() - interval '30 days'\nGROUP BY 1\nORDER BY 1 DESC;\n",
  },
  {
    name: 'reports/top_events_hourly',
    path: '/Users/demo/Library/Application Support/pgscope/saved/reports/top_events_hourly.sql',
    content:
      "SELECT date_trunc('hour', created_at) AS hour,\n       event_name,\n       count(*) AS events\nFROM public.events\nWHERE created_at > now() - interval '24 hours'\nGROUP BY 1, 2\nORDER BY 1 DESC, 3 DESC;\n",
  },
  {
    name: 'reports/weekly/funnel_signup_activate',
    path: '/Users/demo/Library/Application Support/pgscope/saved/reports/weekly/funnel_signup_activate.sql',
    content:
      "SELECT u.plan,\n       count(*) FILTER (WHERE e.event_name = 'signup')       AS signups,\n       count(*) FILTER (WHERE e.event_name = 'feature_used') AS activations,\n       count(*) FILTER (WHERE e.event_name = 'purchase')     AS purchases\nFROM public.users u\nLEFT JOIN public.events e ON e.user_id = u.user_id\nGROUP BY 1\nORDER BY 2 DESC;\n",
  },
  {
    name: 'retention_by_cohort',
    path: '/Users/demo/Library/Application Support/pgscope/saved/retention_by_cohort.sql',
    content:
      'SELECT cohort_week,\n       week_offset,\n       retained::numeric / cohort_size AS retention\nFROM analytics.retention_cohorts\nWHERE week_offset <= 8\nORDER BY cohort_week DESC, week_offset;\n',
  },
  {
    name: 'scratch/slow_events',
    path: '/Users/demo/Library/Application Support/pgscope/saved/scratch/slow_events.sql',
    content:
      "SELECT event_name, properties->>'path' AS path, count(*)\nFROM public.events\nWHERE (properties->>'ms')::int > 1000\nGROUP BY 1, 2\nORDER BY 3 DESC\nLIMIT 50;\n",
  },
]

/** Terminal history, oldest first — the sidebar reverses it to show recents. */
const HISTORY_INPUTS: [string, number][] = [
  ['\\dt', 5_400],
  ['select count(*) from events;', 4_920],
  ['\\d events', 3_180],
  ["select event_name, count(*) from events group by 1 order by 2 desc;", 1_640],
  ['\\timing', 1_020],
  ["select * from users where plan = 'enterprise' limit 20;", 610],
  ['explain analyze select u.plan, count(*) from events e join users u using (user_id) group by 1;', 240],
  ['\\dn', 45],
]

/**
 * A profile list for the connect modal, so the fallback path has content too.
 *
 * @returns `Profile[]` — two saved connections.
 */
function profiles(): Profile[] {
  return [
    { id: 'demo-local', name: 'analytics (local)', host: '127.0.0.1', port: 5432, database: DATABASE, user: 'postgres', sslmode: 'disable' },
    { id: 'demo-staging', name: 'analytics (staging)', host: 'db.staging.internal', port: 5432, database: 'analytics_staging', user: 'app_ro', sslmode: 'require' },
  ]
}

/** The connection every demo command pretends to be served by. */
const CONNECTION: ConnectionInfo = {
  database: DATABASE,
  host: '127.0.0.1',
  port: 5432,
  user: 'postgres',
  serverVersion: '17.4',
  isSuperuser: true,
}

/**
 * The sidebar tree: `public` with the seed's seven tables and three views,
 * plus the `analytics` rollup schema.
 *
 * Estimates are deliberately a little off from the generated counts, the way
 * `reltuples` is between analyzes.
 *
 * @returns `SchemaNode[]` — schemas in the order the sidebar renders them.
 */
function schemaTree(): SchemaNode[] {
  const nodes: SchemaNode[] = ['public', 'analytics'].map((name) => ({
    name,
    tables: RELATIONS.filter((r) => r.schema === name && r.kind === 'table').map((r) => ({
      name: r.name,
      kind: 'table' as const,
      estRows: r.stats.estRows ?? -1,
    })),
    views: RELATIONS.filter((r) => r.schema === name && r.kind === 'view').map((r) => ({
      name: r.name,
      kind: 'view' as const,
      estRows: -1,
    })),
  }))
  return nodes
}

/**
 * Relationship cards and edges for a schema — the five FK edges from the seed.
 *
 * @param schema - `string` — the schema to graph.
 * @returns `FkGraph` — only the tables that participate in a foreign key get a
 *   card, with `totalTables` reporting how many exist overall.
 */
function fkGraph(schema: string): FkGraph {
  const tables = RELATIONS.filter((r) => r.schema === schema && r.kind === 'table')
  const edges = tables.flatMap((r) =>
    (r.fks ?? []).map((fk) => ({
      name: fk.name,
      srcTable: r.name,
      tgtTable: fk.target,
      srcColumns: fk.columns,
      tgtColumns: fk.targetColumns,
    })),
  )
  const involved = new Set(edges.flatMap((e) => [e.srcTable, e.tgtTable]))
  return {
    schema,
    cards: tables.filter((t) => involved.has(t.name)).map((t) => ({ table: t.name, columns: t.columns })),
    edges,
    totalTables: tables.length,
  }
}

/**
 * Metadata for one relation.
 *
 * @param schema - `string` — the schema name.
 * @param table - `string` — the relation name.
 * @returns `TableMeta` — columns, indexes and stats for the requested relation,
 *   not a fixed one; the details panel is keyed off exactly this.
 */
function tableMeta(schema: string, table: string): TableMeta {
  const rel = relationOf(schema, table)
  const stats: TableStats = { ...rel.stats }
  return { schema: rel.schema, name: rel.name, kind: rel.kind, columns: rel.columns, indexes: rel.indexes, stats }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/** Layouts the user has saved this session; reset on reload, like a fresh app. */
const GRID_LAYOUTS: Record<string, TableLayout> = {}
const ER_LAYOUTS: Record<string, Record<string, [number, number]>> = {}

let replCounter = 0

/**
 * Serve a backend command from fixtures.
 *
 * @param cmd - `string` — the Rust command name, snake_case, exactly as `ipc`
 *   would have passed it to Tauri.
 * @param args - `Record<string, unknown> | undefined` — the camelCase argument
 *   object. Arguments are honoured wherever they change what is shown, so the
 *   demo responds to paging, sorting, filtering and table selection rather than
 *   replaying one canned screen.
 * @returns `Promise<T>` — the command's payload, after a short artificial
 *   latency. Rejects with an `AppError`-shaped object for unknown commands —
 *   this path bypasses `toAppError`, so a bare string would surface unusably.
 */
export async function demoInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await delay(nextLatency())
  const a = (args ?? {}) as Record<string, never>
  const done = <R>(value: R): T => value as unknown as T

  switch (cmd) {
    // ---- connection ----
    case 'list_profiles':
      return done(profiles())
    case 'connection_info':
      // Null on boot so the app takes the `connect_dev_url` path, which is the
      // one that populates `info` as well as the connection state.
      return done(null)
    case 'connect_dev_url':
    case 'connect_profile':
      return done(CONNECTION)
    case 'ping':
      return done(2.8)
    case 'platform_name':
      return done('macos')

    // ---- explorer ----
    case 'schema_tree':
      return done(schemaTree())
    case 'table_meta':
      return done(tableMeta(a.schema as string, a.table as string))
    case 'fk_graph':
      return done(fkGraph(a.schema as string))

    // ---- grid ----
    case 'fetch_page':
      return done(fetchPage(a.req as unknown as PageRequest))
    case 'fetch_cell':
      return done(fetchCell(a.req as unknown as CellRequest))
    case 'format_row':
      return done(formatRow(a.req as unknown as RowFormatRequest))
    case 'value_predicate': {
      const column = a.column as unknown as ColumnMeta
      const value = (a.value as string | null) ?? null
      const ident = quoteIdent(column.name)
      if (value === null) return done(`${ident} IS ${a.op === 'noteq' ? 'NOT ' : ''}NULL`)
      return done(`${ident} ${a.op === 'noteq' ? '<>' : '='} ${sqlLiteral(column, value)}`)
    }

    // ---- query editor ----
    case 'split_sql':
      return done(splitSql(a.sql as string))
    case 'statement_at_cursor': {
      const sql = a.sql as string
      const cursor = a.cursor as unknown as number
      const ranges = splitSql(sql)
      const hit = ranges.find((r) => cursor >= r.start && cursor <= r.end) ?? ranges[ranges.length - 1]
      return done(hit ?? null)
    }
    case 'run_query':
      return done(runQuery(a.sql as string))
    case 'explain_query':
      return done(explainQuery(a.sql as string, Boolean(a.analyze)))

    // ---- terminal ----
    case 'repl_open':
    case 'repl_reset': {
      const sessionId = (a.sessionId as string) ?? `demo-repl-${++replCounter}`
      const state: DemoReplState = { buffer: '', timing: false, expanded: false }
      REPL_SESSIONS.set(sessionId, state)
      return done<ReplSession>({ sessionId, prompt: promptFor(state), timing: false, expanded: false })
    }
    case 'repl_exec': {
      const state = REPL_SESSIONS.get(a.sessionId as string)
      if (!state) fail('no_session', 'terminal session has gone away', 'reopen the terminal pane')
      const segments = replRun(state, a.input as string)
      return done<ReplOutput>({
        segments,
        prompt: promptFor(state),
        incomplete: state.buffer.trim().length > 0,
        timing: state.timing,
        expanded: state.expanded,
      })
    }
    case 'repl_complete':
      return done(replComplete(a.line as string, a.cursor as unknown as number))

    // ---- sidebar ----
    case 'history_list':
      // Ages are relative to the real clock: a frozen "3 days ago" would read
      // as staleness rather than as fixture data.
      return done<HistoryItem[]>(
        HISTORY_INPUTS.map(([input, ago]) => ({ input, at: Math.floor(Date.now() / 1000) - ago })),
      )
    case 'saved_queries':
      return done(SAVED_QUERIES)
    case 'saved_folders':
      return done(SAVED_FOLDERS)
    case 'save_named_query':
    case 'save_query_at': {
      const name = ((a.name as string) ?? (a.path as string) ?? 'untitled').replace(/\.sql$/, '')
      return done<SavedQuery>({
        name,
        path: `/Users/demo/Library/Application Support/pgscope/saved/${name}.sql`,
        content: (a.content as string) ?? '',
      })
    }
    case 'rename_saved_query': {
      const existing = SAVED_QUERIES.find((q) => q.path === a.path)
      const name = (a.newName as string) ?? 'untitled'
      return done<SavedQuery>({
        name,
        path: `/Users/demo/Library/Application Support/pgscope/saved/${name}.sql`,
        content: existing?.content ?? '',
      })
    }
    case 'create_saved_folder':
      return done((a.name as string) ?? 'new folder')
    case 'rename_saved_folder':
      // Always an array: the sidebar iterates the result unguarded.
      return done<MovedQuery[]>([])

    // ---- layout ----
    case 'grid_layout_load':
      return done({ ...GRID_LAYOUTS })
    case 'grid_layout_save':
      GRID_LAYOUTS[a.key as string] = a.layout as unknown as TableLayout
      return done(undefined)
    case 'er_layout_load':
      return done({ ...ER_LAYOUTS })
    case 'er_layout_save':
      ER_LAYOUTS[a.schema as string] = a.positions as unknown as Record<string, [number, number]>
      return done(undefined)

    // ---- accepted no-ops ----
    // Nothing here has state worth mutating, but every one is reachable from a
    // button, and a rejection would put an error banner on a working demo.
    case 'save_profile':
    case 'delete_profile':
    case 'disconnect':
    case 'cancel_grid':
    case 'cancel_query':
    case 'history_append':
    case 'delete_saved_query':
    case 'repl_cancel':
    case 'window_minimize':
    case 'window_toggle_maximize':
    case 'window_close':
      return done(undefined)

    default:
      fail('invalid', `unknown command "${cmd}"`, 'the demo fixture has no handler for this command')
  }
}
