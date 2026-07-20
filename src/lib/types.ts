/**
 * Wire types — hand-mirrored from the serde structs in `src-tauri/src/`.
 * Keep field names in sync with the `#[serde(rename_all = "camelCase")]` structs.
 */

export interface AppError {
  code:
    | 'not_connected'
    | 'connect'
    | 'auth'
    | 'tls'
    | 'query'
    // A name is already taken; the caller may retry with an overwrite flag.
    | 'exists'
    | 'cancelled'
    | 'timeout'
    | 'no_session'
    | 'invalid'
    | 'storage'
    | 'keychain'
  message: string
  detail: string | null
  sqlstate: string | null
}

export type SslMode = 'disable' | 'prefer' | 'require'

export interface Profile {
  id: string
  name: string
  host: string
  port: number
  database: string
  user: string
  sslmode: SslMode
}

export interface ConnectionInfo {
  database: string
  host: string
  port: number
  user: string
  serverVersion: string
  isSuperuser: boolean
}

export type ConnectionState = 'disconnected' | 'connecting' | 'connected' | 'lost'

export interface ConnectionStatus {
  state: ConnectionState
  latencyMs?: number | null
  message?: string | null
}

export type RelKind = 'table' | 'view'

export interface Relation {
  name: string
  kind: RelKind
  /** `reltuples`; -1 means never analyzed. */
  estRows: number
}

export interface SchemaNode {
  name: string
  tables: Relation[]
  views: Relation[]
}

export interface ColumnMeta {
  name: string
  dataType: string
  notNull: boolean
  isPk: boolean
  isFk: boolean
}

export interface IndexMeta {
  name: string
  definition: string
}

export interface TableStats {
  estRows: number | null
  totalBytes: number | null
  indexBytes: number | null
  lastAutovacuumSecs: number | null
}

export interface TableMeta {
  schema: string
  name: string
  kind: RelKind
  columns: ColumnMeta[]
  indexes: IndexMeta[]
  stats: TableStats
}

export type SortDir = 'asc' | 'desc'

/** One ORDER BY term, in the order the user added it. */
export interface SortKey {
  column: string
  dir: SortDir
}

export interface PageRequest {
  schema: string
  table: string
  /** Sort terms, most significant first. Empty means unsorted. */
  sort: SortKey[]
  filter?: string | null
  page: number
}

export interface PageResult {
  rows: (string | null)[][]
  timingMs: number
  total: number | null
  totalIsEstimate: boolean
  page: number
  pageSize: number
  sql: string
}

export type CellFormat = 'json' | 'text'

export interface CellValue {
  column: string
  dataType: string
  /** null for SQL NULL. */
  value: string | null
  format: CellFormat
  /** Server-side byte length, before any capping. */
  totalBytes: number
  truncated: boolean
  /** False when the row was located by page position rather than primary key. */
  locatedByPk: boolean
}

export interface CellRequest {
  column: string
  pk: { column: string; value: string }[]
  page: PageRequest
  rowIndex: number
}

export type RowFormat = 'json' | 'csv' | 'tsv' | 'insert'
export type PredicateOp = 'eq' | 'noteq'

export interface RowFormatRequest {
  schema: string
  table: string
  columns: ColumnMeta[]
  values: (string | null)[]
  format: RowFormat
}

export interface FkEdge {
  name: string
  srcTable: string
  tgtTable: string
  srcColumns: string[]
  tgtColumns: string[]
}

export interface FkCard {
  table: string
  columns: ColumnMeta[]
}

export interface FkGraph {
  schema: string
  cards: FkCard[]
  edges: FkEdge[]
  totalTables: number
}

/** Colour classes for terminal output, mapping to the design's palette. */
export type SegmentKind = 'prompt' | 'body' | 'dim' | 'error'

export interface Segment {
  text: string
  kind: SegmentKind
}

export interface ReplSession {
  sessionId: string
  /** e.g. `analytics_prod=#` */
  prompt: string
  timing: boolean
  expanded: boolean
}

export interface ReplOutput {
  segments: Segment[]
  /** Prompt to show next — reflects continuation state. */
  prompt: string
  /** True when the buffer is mid-statement (continuation prompt showing). */
  incomplete: boolean
  timing: boolean
  expanded: boolean
}

/** A statement's position in an editor buffer, in character offsets. */
export interface StatementRange {
  start: number
  end: number
  /** Trimmed text, without the terminating `;`. */
  text: string
}

export interface QueryResultSet {
  columns: string[]
  rows: (string | null)[][]
  /** Rows the server produced, before truncation. */
  totalRows: number
  truncated: boolean
}

export interface StatementResult {
  sql: string
  /** Present only for statements that return rows. */
  result: QueryResultSet | null
  notices: string[]
  timingMs: number
}

export interface QueryRun {
  statements: StatementResult[]
  totalTimingMs: number
}

export interface PlanNode {
  /** Stable path id (`0.1.0`), used for keys and expand state. */
  id: string
  nodeType: string
  startupCost: number | null
  totalCost: number | null
  planRows: number | null
  planWidth: number | null

  // EXPLAIN ANALYZE only.
  actualStartupTime: number | null
  actualTotalTime: number | null
  actualRows: number | null
  actualLoops: number | null

  /** Time this node alone accounted for, across all loops. */
  selfTimeMs: number | null
  /** Cost this node alone accounted for. */
  selfCost: number | null
  /** actual ÷ planned rows; >1 means the planner under-estimated. */
  rowRatio: number | null
  /**
   * True when this node ran under a Gather, so `actualLoops` counts parallel
   * workers. Its `selfTimeMs` is then CPU summed across workers, which can
   * exceed the query's wall-clock time.
   */
  parallel: boolean

  details: [string, string][]
  children: PlanNode[]
}

export interface ExplainResult {
  plan: PlanNode
  planningTimeMs: number | null
  executionTimeMs: number | null
  /** True when the statement really executed. */
  analyzed: boolean
  /** True when that execution was rolled back. */
  rolledBack: boolean
  sql: string
  maxSelfTimeMs: number | null
  maxSelfCost: number | null
}

export type CompletionKind =
  | 'keyword'
  | 'table'
  | 'view'
  | 'column'
  | 'schema'
  | 'function'
  | 'meta'

export interface Completion {
  value: string
  kind: CompletionKind
  /** Shown beside the candidate when listing, e.g. a column's type. */
  detail: string | null
}

export interface CompletionResult {
  /** Character offset where the replaced token starts. */
  start: number
  /** Character offset where it ends (the cursor). */
  end: number
  items: Completion[]
  /** Longest prefix shared by every candidate. */
  commonPrefix: string
}

export interface HistoryItem {
  input: string
  /** Unix seconds. */
  at: number
}

export interface SavedQuery {
  /** Path under the saved-queries directory without the extension, e.g.
   *  `reports/dau`. Always `/`-separated, whatever the host's separator is. */
  name: string
  path: string
  content: string
}

/** One query's before/after paths from a folder rename. */
export interface MovedQuery {
  from: string
  query: SavedQuery
}
