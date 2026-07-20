use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::connect::{Connection, ConnectionInfo, Profile};
use crate::db::grid::{PageRequest, PageResult};
use crate::db::{self, introspect};
use crate::error::{AppError, Result};
use crate::repl::session::{ReplOutput, ReplSession, ReplSessionInfo};
use crate::state::AppState;
use crate::store::{self, HistoryItem, SavedQuery};

/// Emitted on the `connection:status` channel for the titlebar pill.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub state: &'static str,
    pub latency_ms: Option<f64>,
    pub message: Option<String>,
}

// ------------------------------ profiles --------------------------------
//
// A profile is connection settings only. The password is never part of it and
// never crosses to the webview — it lives in the OS keychain keyed by profile
// id, so these commands keep two stores in step.

/// Profiles for the connection picker, passwords excluded.
#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>> {
    store::load_profiles(&state.paths)
}

/// Create or update a profile, and optionally its keychain entry.
///
/// `password` is three-valued: `None` leaves any stored password untouched (the
/// case where the user only edited the host or port), `Some("")` clears it, and
/// anything else replaces it. The profile is written before the keychain is
/// touched, so a keychain failure still leaves the edited profile saved.
#[tauri::command]
pub async fn save_profile(
    state: State<'_, AppState>,
    profile: Profile,
    password: Option<String>,
) -> Result<()> {
    store::upsert_profile(&state.paths, profile.clone())?;
    if let Some(pw) = password {
        if pw.is_empty() {
            crate::secrets::delete_password(&profile.id)?;
        } else {
            crate::secrets::set_password(&profile.id, &pw)?;
        }
    }
    Ok(())
}

/// Forget a profile and any password saved alongside it.
#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> Result<()> {
    store::remove_profile(&state.paths, &id)?;
    // Best-effort: a missing keychain entry is fine.
    let _ = crate::secrets::delete_password(&id);
    Ok(())
}

// ----------------------------- connection -------------------------------

/// Poll the server so the titlebar shows live latency, and so a dropped
/// connection is noticed before the user clicks something.
pub fn spawn_pinger(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            let state = app.state::<AppState>();
            let Some(conn) = state.connection_opt().await else {
                continue;
            };
            let status = match conn.ping().await {
                Ok(ms) => ConnectionStatus {
                    state: "connected",
                    latency_ms: Some(ms),
                    message: None,
                },
                Err(e) => ConnectionStatus {
                    state: "lost",
                    latency_ms: None,
                    message: Some(e.to_string()),
                },
            };
            let _ = app.emit("connection:status", status);
        }
    });
}

/// Dial a profile, install the result as the app's one live connection, and
/// narrate the attempt on `connection:status`.
///
/// The "connecting" event is emitted before the dial so the titlebar can show
/// progress across a slow handshake. On failure nothing is emitted and the
/// previous connection stays in place — the error goes back to the caller, which
/// owns telling the user.
async fn establish(
    app: &AppHandle,
    state: &AppState,
    profile: Profile,
    password: Option<String>,
) -> Result<ConnectionInfo> {
    let _ = app.emit(
        "connection:status",
        ConnectionStatus {
            state: "connecting",
            latency_ms: None,
            message: None,
        },
    );

    let conn = Connection::open(profile, password).await?;
    let info = conn.info.clone();
    let latency = conn.ping().await.ok();
    state.set_connection(Some(Arc::clone(&conn))).await;

    let _ = app.emit(
        "connection:status",
        ConnectionStatus {
            state: "connected",
            latency_ms: latency,
            message: None,
        },
    );
    Ok(info)
}

/// Connect using a saved profile, falling back to the keychain for the password.
///
/// An explicitly supplied password wins so that a wrong saved password is still
/// recoverable from the UI. A keychain read that fails is treated as "no
/// password" rather than an error: the server rejects the attempt if one was
/// actually needed, which is the clearer message.
#[tauri::command]
pub async fn connect_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    password: Option<String>,
) -> Result<ConnectionInfo> {
    let profiles = store::load_profiles(&state.paths)?;
    let profile = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| AppError::Invalid(format!("no such profile: {profile_id}")))?;

    // An explicitly supplied password wins; otherwise use the keychain.
    let password = match password {
        Some(pw) => Some(pw),
        None => crate::secrets::get_password(&profile_id).unwrap_or(None),
    };

    establish(&app, &state, profile, password).await
}

/// Dev convenience: connect straight to `PGSCOPE_DEV_URL` when it's set, so
/// `pnpm tauri dev` lands in a connected app with no clicks.
#[tauri::command]
pub async fn connect_dev_url(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ConnectionInfo>> {
    let Ok(url) = std::env::var("PGSCOPE_DEV_URL") else {
        return Ok(None);
    };
    if url.trim().is_empty() {
        return Ok(None);
    }

    let (profile, password) = Profile::from_url(&url, "dev", "dev")?;
    establish(&app, &state, profile, password).await.map(Some)
}

/// Tear down the current connection.
///
/// Clearing it also drops every terminal session and the editor's client (see
/// `AppState::set_connection`) — they are bound to the server that is going
/// away, so the frontend must treat its session ids as dead after this.
#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    state.set_connection(None).await;
    let _ = app.emit(
        "connection:status",
        ConnectionStatus {
            state: "disconnected",
            latency_ms: None,
            message: None,
        },
    );
    Ok(())
}

/// Round-trip time to the server in milliseconds, for an on-demand latency
/// reading between the pinger's 15-second ticks. Errors when disconnected.
#[tauri::command]
pub async fn ping(state: State<'_, AppState>) -> Result<f64> {
    state.connection().await?.ping().await
}

/// Server version, database and role of the live connection; `None` when there
/// isn't one, which is how the frontend restores its state on reload.
#[tauri::command]
pub async fn connection_info(state: State<'_, AppState>) -> Result<Option<ConnectionInfo>> {
    Ok(state.connection_opt().await.map(|c| c.info.clone()))
}

// ------------------------------ explorer --------------------------------
//
// All three introspect on the browse pool, which the server-side options make
// read-only with a 30s statement timeout: catalog queries on a large database
// can be slow, and they must never be able to write.

/// Schemas and the relations inside them, for the explorer tree.
#[tauri::command]
pub async fn schema_tree(state: State<'_, AppState>) -> Result<Vec<introspect::SchemaNode>> {
    let conn = state.connection().await?;
    introspect::schema_tree(&conn.pool).await
}

/// Columns, keys, indexes and size/row estimates for one relation — everything
/// the grid header, the details panel and the ER card render from.
#[tauri::command]
pub async fn table_meta(
    state: State<'_, AppState>,
    schema: String,
    table: String,
) -> Result<introspect::TableMeta> {
    let conn = state.connection().await?;
    introspect::table_meta(&conn.pool, &schema, &table).await
}

/// Foreign-key edges within one schema, for the ER diagram. Scoped to a single
/// schema because the diagram is, and a whole-database graph is unreadable.
#[tauri::command]
pub async fn fk_graph(state: State<'_, AppState>, schema: String) -> Result<introspect::FkGraph> {
    let conn = state.connection().await?;
    introspect::fk_graph(&conn.pool, &schema).await
}

// -------------------------------- grid ----------------------------------

/// One page of rows for the grid, on the read-only browse pool.
///
/// `req` carries the sort, filter and page number; `db::grid` turns that into
/// SQL and owns the identifier quoting. Cancellable while it runs — see below.
#[tauri::command]
pub async fn fetch_page(state: State<'_, AppState>, req: PageRequest) -> Result<PageResult> {
    let conn = state.connection().await?;
    let state_inner = state.inner();

    // Publish the statement's cancel handle *before* the query is awaited, so
    // `cancel_grid` can abort a slow page fetch while it is still running.
    let result = db::fetch_page(&conn.pool, &req, |token| {
        state_inner.set_grid_cancel(Some(token));
    })
    .await;

    // The query has finished either way; drop the now-stale handle.
    state.set_grid_cancel(None);
    result
}

/// Fetch one cell in full, for inline expansion in the grid.
///
/// Runs on the browse pool, so it inherits the read-only guarantee and the
/// statement timeout.
#[tauri::command]
pub async fn fetch_cell(
    state: State<'_, AppState>,
    req: db::cell::CellRequest,
) -> Result<db::cell::CellValue> {
    let conn = state.connection().await?;
    let client = conn.pool.get().await?;
    db::cell::fetch_cell(&client, &req).await
}

/// Render a row for the grid's copy actions (JSON / CSV / TSV / INSERT).
///
/// Runs in Rust so SQL literal quoting has one implementation, shared with the
/// query builder.
#[tauri::command]
pub async fn format_row(req: db::rowfmt::RowFormatRequest) -> Result<String> {
    Ok(db::rowfmt::format_row(&req))
}

/// A `WHERE` fragment matching a cell's value, for filter-to-value.
#[tauri::command]
pub async fn value_predicate(
    column: introspect::ColumnMeta,
    value: Option<String>,
    op: db::rowfmt::PredicateOp,
) -> Result<String> {
    Ok(db::rowfmt::value_predicate(&column, value.as_deref(), op))
}

/// Ask the server to abort an in-flight page fetch.
///
/// Takes the handle rather than cloning it, so cancelling twice, or with nothing
/// running, is a no-op instead of an error. Cancellation opens a second
/// connection and inherently races the query finishing on its own, so a failed
/// cancel is swallowed — there is nothing useful to tell the user.
#[tauri::command]
pub async fn cancel_grid(state: State<'_, AppState>) -> Result<()> {
    if let Some(token) = state.take_grid_cancel() {
        let _ = token.cancel_query(tokio_postgres::NoTls).await;
    }
    Ok(())
}

// ---------------------------- query editor ------------------------------

/// Split a buffer into statements, with offsets, for the editor.
///
/// Shares the REPL's lexer so "what counts as one statement" can't drift
/// between the two surfaces — the editor never reimplements SQL lexing.
#[tauri::command]
pub fn split_sql(sql: String) -> Vec<crate::repl::lexer::StatementRange> {
    crate::repl::lexer::statement_ranges(&sql)
}

/// The statement under a cursor offset, for run-statement-at-cursor.
#[tauri::command]
pub fn statement_at_cursor(
    sql: String,
    cursor: usize,
) -> Option<crate::repl::lexer::StatementRange> {
    crate::repl::lexer::statement_at(&sql, cursor)
}

/// Run SQL from an editor tab.
///
/// Every statement in `sql` runs in order on the editor's dedicated
/// (unrestricted) client, stopping at the first error.
#[tauri::command]
pub async fn run_query(state: State<'_, AppState>, sql: String) -> Result<db::query::QueryRun> {
    let statements: Vec<String> = crate::repl::lexer::statement_ranges(&sql)
        .into_iter()
        .map(|r| r.text)
        .collect();

    if statements.is_empty() {
        return Err(AppError::Invalid("nothing to run".into()));
    }

    let client = state.editor().await?;
    // Refresh the cancel handle: `editor()` may have reconnected.
    state.set_editor_cancel(Some(client.cancel_token()));

    db::query::exec_all(&client, &statements).await
}

/// EXPLAIN a statement from an editor tab.
///
/// `analyze` executes the statement for real; `explain()` wraps that in a
/// rolled-back transaction so inspecting an UPDATE can't modify data.
#[tauri::command]
pub async fn explain_query(
    state: State<'_, AppState>,
    sql: String,
    analyze: bool,
) -> Result<db::explain::ExplainResult> {
    // EXPLAIN takes exactly one statement, so pick the one to explain rather
    // than concatenating a whole buffer into invalid syntax.
    let statements = crate::repl::lexer::statement_ranges(&sql);
    let target = match statements.len() {
        0 => return Err(AppError::Invalid("nothing to explain".into())),
        1 => statements[0].text.clone(),
        _ => {
            return Err(AppError::Invalid(
                "EXPLAIN takes a single statement — put the cursor in one and try again".into(),
            ))
        }
    };

    let client = state.editor().await?;
    state.set_editor_cancel(Some(client.cancel_token()));
    db::explain::explain(&client, &target, analyze).await
}

/// Ask the server to abort whatever the editor client is running.
///
/// The token is cloned, not taken: the editor keeps one long-lived client, so
/// the handle stays valid between runs and only `editor()` reconnecting
/// replaces it. As with `cancel_grid`, a lost race is ignored.
#[tauri::command]
pub async fn cancel_query(state: State<'_, AppState>) -> Result<()> {
    if let Some(token) = state.editor_cancel_token() {
        let _ = token.cancel_query(tokio_postgres::NoTls).await;
    }
    Ok(())
}

// ------------------------------ terminal --------------------------------
//
// Each terminal tab owns a session, and each session owns an unrestricted
// client — not the browse pool. It has to: `search_path` changes, temp tables,
// open transactions and writes all have to survive from one submission to the
// next, and none of that works on a pooled read-only connection.

/// Start a terminal session and register it under a fresh id.
///
/// Inherits the current connection's profile, credentials and superuser flag so
/// the prompt and the meta-command output match what the user connected as.
#[tauri::command]
pub async fn repl_open(state: State<'_, AppState>) -> Result<ReplSessionInfo> {
    let conn = state.connection().await?;
    let session = ReplSession::open(
        conn.profile.clone(),
        conn.password.clone(),
        conn.info.database.clone(),
        conn.info.is_superuser,
        state.paths.clone(),
    )
    .await?;
    let info = session.info();
    state.add_repl(session).await;
    Ok(info)
}

/// Submit one line of terminal input — SQL, or a backslash meta-command.
///
/// Holds the session mutex for the whole statement, which is exactly why
/// `repl_cancel` reaches for a token kept outside that mutex. Errors only if the
/// session id is unknown: a failing statement comes back as rendered output, the
/// way psql prints an error rather than exiting.
#[tauri::command]
pub async fn repl_exec(
    state: State<'_, AppState>,
    session_id: String,
    input: String,
) -> Result<ReplOutput> {
    let session = state.repl(&session_id).await?;
    let mut guard = session.lock().await;
    Ok(guard.submit(&input).await)
}

/// Tab completion for the terminal's input line.
///
/// Runs on the session's own client so it sees the same search_path and any
/// temp objects the user created in this session.
#[tauri::command]
pub async fn repl_complete(
    state: State<'_, AppState>,
    session_id: String,
    line: String,
    cursor: usize,
) -> Result<crate::repl::complete::CompletionResult> {
    let session = state.repl(&session_id).await?;
    let guard = session.lock().await;
    guard.complete(&line, cursor).await
}

/// Interrupt the statement a session is currently running — the terminal's Ctrl-C.
#[tauri::command]
pub async fn repl_cancel(state: State<'_, AppState>, session_id: String) -> Result<()> {
    // The token lives outside the session mutex precisely so this works while
    // a statement is running and holding that lock.
    let token = state
        .repl_cancel_token(&session_id)
        .await
        .ok_or_else(|| AppError::NoSession(session_id.clone()))?;
    let _ = token.cancel_query(tokio_postgres::NoTls).await;
    Ok(())
}

/// Discard a wedged session (aborted transaction, dead socket) and hand back a
/// replacement.
///
/// The replacement is a genuinely new session with a new id, so the frontend has
/// to adopt the returned info — continuing to use the old id fails.
#[tauri::command]
pub async fn repl_reset(state: State<'_, AppState>, session_id: String) -> Result<ReplSessionInfo> {
    state.remove_repl(&session_id).await;
    repl_open(state).await
}

// ------------------------------- sidebar --------------------------------

/// Terminal history oldest-first, capped at the most recent 1000 entries.
/// Unparseable lines are skipped rather than failing the whole read.
#[tauri::command]
pub async fn history_list(state: State<'_, AppState>) -> Result<Vec<HistoryItem>> {
    store::load_history(&state.paths)
}

/// Record a terminal submission, stamped with the current time. Blank input is
/// dropped, so the frontend can call this unconditionally.
#[tauri::command]
pub async fn history_append(state: State<'_, AppState>, input: String) -> Result<()> {
    store::append_history(&state.paths, &input)
}

/// Every saved query with its contents, sorted by name.
///
/// Because a query's name *is* its path under the saved-queries directory, that
/// sort groups each folder's contents together and the sidebar can build its
/// tree in one pass.
#[tauri::command]
pub async fn saved_queries(state: State<'_, AppState>) -> Result<Vec<SavedQuery>> {
    store::load_saved_queries(&state.paths)
}

/// Overwrite an existing saved query in place (⌘S on a tab opened from a file).
///
/// `store::save_query_at` validates that the path really is inside the
/// saved-queries directory — the webview supplies it, so it is not trusted.
#[tauri::command]
pub async fn save_query_at(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<SavedQuery> {
    store::save_query_at(&state.paths, &path, &content)
}

/// Save a query under a name rather than a path (Save As, and first save of a
/// scratch tab).
///
/// `name` may contain `/` to nest the query in folders; it is sanitised into a
/// relative path that cannot climb out of the saved-queries directory.
///
/// Refuses an existing name with `AppError::Exists` unless `overwrite` is set,
/// so the caller can confirm before destroying someone's query. Omitting the
/// argument means "do not overwrite".
#[tauri::command]
pub async fn save_named_query(
    state: State<'_, AppState>,
    name: String,
    content: String,
    overwrite: Option<bool>,
) -> Result<SavedQuery> {
    store::save_query(&state.paths, &name, &content, overwrite.unwrap_or(false))
}

/// Rename or move a saved query. `newName` may contain `/` to place it in a
/// folder. Both the source path and the destination are validated as being
/// inside the saved-queries directory.
#[tauri::command]
pub async fn rename_saved_query(
    state: State<'_, AppState>,
    path: String,
    new_name: String,
) -> Result<SavedQuery> {
    store::rename_saved_query(&state.paths, &path, &new_name)
}

/// Delete one saved query. The webview supplies `path`, so `store` re-validates
/// that it resolves to a `.sql` file inside the saved-queries directory.
#[tauri::command]
pub async fn delete_saved_query(state: State<'_, AppState>, path: String) -> Result<()> {
    store::delete_saved_query(&state.paths, &path)
}

/// Rename a folder in the sidebar tree, moving its queries with it.
///
/// Only the last segment of `path` is replaced — the folder stays where it is.
/// Returns one entry per query that moved, each carrying its old path, so the
/// frontend can re-point editor tabs that were opened from those files.
#[tauri::command]
pub async fn rename_saved_folder(
    state: State<'_, AppState>,
    path: String,
    new_name: String,
) -> Result<Vec<store::MovedQuery>> {
    store::rename_saved_folder(&state.paths, &path, &new_name)
}

/// Folder paths relative to the saved-queries directory, slash-separated.
///
/// Listed separately from the queries so the sidebar can show a folder the user
/// created but has not put anything in yet.
#[tauri::command]
pub async fn saved_folders(state: State<'_, AppState>) -> Result<Vec<String>> {
    store::list_saved_folders(&state.paths)
}

/// Create an empty folder. Returns the sanitised name actually used on disk,
/// which may differ from what was asked for — the caller should display that.
#[tauri::command]
pub async fn create_saved_folder(state: State<'_, AppState>, name: String) -> Result<String> {
    store::create_saved_folder(&state.paths, &name)
}

// ------------------------------ ER layout -------------------------------

/// Dragged ER card positions for every schema.
///
/// A missing or corrupt file reads as empty rather than erroring: a lost diagram
/// layout re-lays-out on its own and is not worth blocking the view over.
#[tauri::command]
pub async fn er_layout_load(state: State<'_, AppState>) -> Result<store::ErLayout> {
    store::load_er_layout(&state.paths)
}

/// Persist the card positions for one schema.
///
/// `positions` replaces that schema's entry wholesale — it is the full set for
/// the schema, not a delta — while other schemas are left as they were.
#[tauri::command]
pub async fn er_layout_save(
    state: State<'_, AppState>,
    schema: String,
    positions: std::collections::HashMap<String, [f64; 2]>,
) -> Result<()> {
    let mut layout = store::load_er_layout(&state.paths)?;
    layout.insert(schema, positions);
    store::save_er_layout(&state.paths, &layout)
}

// ------------------------------ grid layout -----------------------------

/// Remembered column widths and orders for every table visited, keyed
/// `"schema.table"`. A table with no entry gets the grid's computed defaults.
#[tauri::command]
pub async fn grid_layout_load(state: State<'_, AppState>) -> Result<store::GridLayout> {
    store::load_grid_layout(&state.paths)
}

/// Persist one table's column widths and order, keyed `"schema.table"`.
///
/// `store` clamps the widths and caps how many tables are remembered, so neither
/// a degenerate width nor a long browsing session can leave a column impossible
/// to grab again or grow the file without bound.
#[tauri::command]
pub async fn grid_layout_save(
    state: State<'_, AppState>,
    key: String,
    layout: store::TableLayout,
) -> Result<()> {
    store::save_table_layout(&state.paths, &key, &layout)
}

// -------------------------------- window --------------------------------
//
// The titlebar is drawn in the webview — macOS overlays the native traffic
// lights on it, everywhere else `decorations` is off (see `lib.rs`). Either way
// there is no OS-drawn button to press, so the window controls have to round
// trip through these commands.

/// Which titlebar layout the frontend should draw: macOS puts the traffic
/// lights on the left and supplies them itself; Windows and Linux get ours on
/// the right.
#[tauri::command]
pub fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// Minimise, from the custom titlebar's button.
#[tauri::command]
pub async fn window_minimize(window: tauri::Window) -> Result<()> {
    window
        .minimize()
        .map_err(|e| AppError::Invalid(e.to_string()))
}

/// Maximise or restore. Toggled here rather than in the frontend so the decision
/// reads the window's real state instead of a mirrored copy that drifts when the
/// user resizes by dragging.
#[tauri::command]
pub async fn window_toggle_maximize(window: tauri::Window) -> Result<()> {
    let maximized = window
        .is_maximized()
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let r = if maximized {
        window.unmaximize()
    } else {
        window.maximize()
    };
    r.map_err(|e| AppError::Invalid(e.to_string()))
}

/// Close the window, from the custom titlebar's button.
#[tauri::command]
pub async fn window_close(window: tauri::Window) -> Result<()> {
    window.close().map_err(|e| AppError::Invalid(e.to_string()))
}
