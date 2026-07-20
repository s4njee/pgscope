import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type { TableLayout } from './columnLayout'
import type {
  AppError,
  CellRequest,
  CellValue,
  ColumnMeta,
  ConnectionInfo,
  CompletionResult,
  ConnectionStatus,
  ExplainResult,
  FkGraph,
  HistoryItem,
  MovedQuery,
  PageRequest,
  PageResult,
  PredicateOp,
  Profile,
  QueryRun,
  RowFormatRequest,
  ReplOutput,
  ReplSession,
  SavedQuery,
  SchemaNode,
  StatementRange,
  TableMeta,
} from './types'

/** True when running inside the Tauri shell (vs. a plain browser dev server). */
export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** Structural check — the backend's error shape does not survive as a class. */
export function isAppError(e: unknown): e is AppError {
  return typeof e === 'object' && e !== null && 'code' in e && 'message' in e
}

/** Normalise anything thrown across the IPC boundary into an AppError. */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e
  return {
    code: 'invalid',
    message: e instanceof Error ? e.message : String(e),
    detail: null,
    sqlstate: null,
  }
}

/**
 * The single funnel every backend command goes through.
 *
 * Its one job is the guarantee the rest of the app relies on: a rejection from
 * here is always an `AppError`, never a bare string, `Error`, or whatever else
 * Tauri happened to serialise. Callers can therefore read `.code`/`.message`
 * without re-checking, and the error banners have something to show.
 */
/**
 * Every command crosses the boundary here, which is also what makes a
 * backend-free demo possible: outside the Tauri shell there is no `invoke` to
 * call, so the same command names are served from in-memory fixtures instead.
 * That keeps the demo honest — it drives the real components and the real
 * stores, not a mock-up of them.
 *
 * @param cmd - `string` — the Rust command name, snake_case.
 * @param args - `Record<string, unknown> | undefined` — camelCase argument
 *   object, as Tauri expects.
 * @returns `Promise<T>` — the command's payload. Rejects with an `AppError`,
 *   never a raw driver string.
 */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    const { demoInvoke } = await import('./demo')
    return demoInvoke<T>(cmd, args)
  }
  try {
    return await invoke<T>(cmd, args)
  } catch (e) {
    throw toAppError(e)
  }
}

export const ipc = {
  // ---- connection ----
  listProfiles: () => call<Profile[]>('list_profiles'),
  saveProfile: (profile: Profile, password?: string) =>
    call<void>('save_profile', { profile, password }),
  deleteProfile: (id: string) => call<void>('delete_profile', { id }),
  connect: (profileId: string, password?: string) =>
    call<ConnectionInfo>('connect_profile', { profileId, password }),
  /** Connects using PGSCOPE_DEV_URL if set; returns null when it isn't. */
  connectDevUrl: () => call<ConnectionInfo | null>('connect_dev_url'),
  disconnect: () => call<void>('disconnect'),
  ping: () => call<number>('ping'),
  connectionInfo: () => call<ConnectionInfo | null>('connection_info'),

  // ---- explorer ----
  schemaTree: () => call<SchemaNode[]>('schema_tree'),
  tableMeta: (schema: string, table: string) => call<TableMeta>('table_meta', { schema, table }),
  fkGraph: (schema: string) => call<FkGraph>('fk_graph', { schema }),

  // ---- grid ----
  fetchPage: (req: PageRequest) => call<PageResult>('fetch_page', { req }),
  fetchCell: (req: CellRequest) => call<CellValue>('fetch_cell', { req }),
  formatRow: (req: RowFormatRequest) => call<string>('format_row', { req }),
  valuePredicate: (column: ColumnMeta, value: string | null, op: PredicateOp) =>
    call<string>('value_predicate', { column, value, op }),
  cancelGrid: () => call<void>('cancel_grid'),

  // ---- query editor ----
  splitSql: (sql: string) => call<StatementRange[]>('split_sql', { sql }),
  statementAtCursor: (sql: string, cursor: number) =>
    call<StatementRange | null>('statement_at_cursor', { sql, cursor }),
  runQuery: (sql: string) => call<QueryRun>('run_query', { sql }),
  explainQuery: (sql: string, analyze: boolean) =>
    call<ExplainResult>('explain_query', { sql, analyze }),
  cancelQuery: () => call<void>('cancel_query'),

  // ---- terminal ----
  replOpen: () => call<ReplSession>('repl_open'),
  replExec: (sessionId: string, input: string) =>
    call<ReplOutput>('repl_exec', { sessionId, input }),
  replComplete: (sessionId: string, line: string, cursor: number) =>
    call<CompletionResult>('repl_complete', { sessionId, line, cursor }),
  replCancel: (sessionId: string) => call<void>('repl_cancel', { sessionId }),
  replReset: (sessionId: string) => call<ReplSession>('repl_reset', { sessionId }),

  // ---- sidebar ----
  historyList: () => call<HistoryItem[]>('history_list'),
  historyAppend: (input: string) => call<void>('history_append', { input }),
  savedQueries: () => call<SavedQuery[]>('saved_queries'),
  /** Refuses an existing name unless `overwrite`, so a save cannot silently
   *  destroy another query. Rejects with an `exists`-coded AppError. */
  saveNamedQuery: (name: string, content: string, overwrite = false) =>
    call<SavedQuery>('save_named_query', { name, content, overwrite }),
  saveQueryAt: (path: string, content: string) =>
    call<SavedQuery>('save_query_at', { path, content }),
  renameSavedQuery: (path: string, newName: string) =>
    call<SavedQuery>('rename_saved_query', { path, newName }),
  deleteSavedQuery: (path: string) => call<void>('delete_saved_query', { path }),
  createSavedFolder: (name: string) => call<string>('create_saved_folder', { name }),
  savedFolders: () => call<string[]>('saved_folders'),
  renameSavedFolder: (path: string, newName: string) =>
    call<MovedQuery[]>('rename_saved_folder', { path, newName }),

  // ---- grid layout ----
  gridLayoutLoad: () => call<Record<string, TableLayout>>('grid_layout_load'),
  gridLayoutSave: (key: string, layout: TableLayout) =>
    call<void>('grid_layout_save', { key, layout }),

  // ---- window ----
  windowMinimize: () => call<void>('window_minimize'),
  windowToggleMaximize: () => call<void>('window_toggle_maximize'),
  windowClose: () => call<void>('window_close'),
  /** 'macos' | 'windows' | 'linux' — drives titlebar treatment. */
  platform: () => call<string>('platform_name'),
}

/**
 * Subscribe to connection state pushed by the backend — drops and reconnects
 * originate there, so they arrive as events rather than as a command's result.
 * Resolves to the unsubscribe function; the subscription is not live until it
 * does, so an effect must await it before its cleanup can run.
 */
export function onConnectionStatus(cb: (s: ConnectionStatus) => void): Promise<UnlistenFn> {
  return listen<ConnectionStatus>('connection:status', (e) => cb(e.payload))
}
