//! Ad-hoc query execution for the editor tabs.
//!
//! Distinct from [`crate::db::grid`], which pages a known table on the
//! read-only browse pool. The editor runs whatever the user typed, so it needs
//! an unrestricted connection (like the terminal) and returns *structured*
//! rows for the result grid rather than psql-formatted text.

use serde::Serialize;
use tokio_postgres::{Client, SimpleQueryMessage};

use crate::error::Result;

/// Per-statement output cap, mirroring the terminal's. A `SELECT *` on a large
/// table must not push the whole thing into the webview.
pub const MAX_ROWS: usize = 10_000;

/// Per-cell display cap, matching the grid's.
pub const CELL_CAP_BYTES: usize = 8 * 1024;

/// One result set produced by a statement.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows the server actually produced, before `MAX_ROWS` truncation.
    pub total_rows: usize,
    pub truncated: bool,
}

/// The outcome of running one statement.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementResult {
    /// The SQL that produced this, for the result header.
    pub sql: String,
    /// Present for statements that return rows.
    pub result: Option<ResultSet>,
    /// Server-side notices (RAISE NOTICE, etc.).
    pub notices: Vec<String>,
    pub timing_ms: f64,
}

/// The outcome of a run — one entry per statement executed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRun {
    pub statements: Vec<StatementResult>,
    pub total_timing_ms: f64,
}

/// Truncate an oversized cell to [`CELL_CAP_BYTES`], appending an ellipsis so
/// the UI can tell a clipped value from a short one.
///
/// The cut backs off to a char boundary, so the result may be a few bytes under
/// the cap. Values within it are returned untouched and unmarked.
///
/// # Arguments
/// * `value` — `String`: one cell's text form, taken by value so the common
///   under-cap case can hand the same allocation straight back.
///
/// # Returns
/// `String` — the original when within `CELL_CAP_BYTES`, otherwise a prefix with
/// `…` appended; the ellipsis is the only marker that clipping happened.
fn cap_cell(value: String) -> String {
    if value.len() <= CELL_CAP_BYTES {
        return value;
    }
    let mut end = CELL_CAP_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// Run one statement and collect its rows.
///
/// Uses the simple query protocol so values arrive as text — the same choice
/// the grid and terminal make, and the reason any column type renders without
/// a per-type decoder.
///
/// # Arguments
/// * `client` — `&Client`: an unrestricted editor connection, not the read-only
///   browse pool.
/// * `sql` — `&str`: one statement exactly as the user typed it, sent verbatim.
///
/// # Returns
/// `Result<StatementResult>` — `result` is `None` for statements that return no
/// rows, and rows beyond `MAX_ROWS` are dropped with `truncated` set while
/// `total_rows` still counts them all. `Err` is the server's error for the
/// statement.
pub async fn exec_one(client: &Client, sql: &str) -> Result<StatementResult> {
    let t0 = std::time::Instant::now();
    let messages = client.simple_query(sql).await?;
    let timing_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut set: Option<ResultSet> = None;

    for msg in messages {
        match msg {
            SimpleQueryMessage::RowDescription(desc) => {
                set = Some(ResultSet {
                    columns: desc.iter().map(|c| c.name().to_string()).collect(),
                    ..Default::default()
                });
            }
            SimpleQueryMessage::Row(row) => {
                if let Some(rs) = set.as_mut() {
                    rs.total_rows += 1;
                    if rs.rows.len() < MAX_ROWS {
                        rs.rows.push(
                            (0..row.len())
                                .map(|i| row.get(i).map(|v| cap_cell(v.to_string())))
                                .collect(),
                        );
                    } else {
                        rs.truncated = true;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(StatementResult {
        sql: sql.to_string(),
        result: set,
        notices: Vec::new(),
        timing_ms,
    })
}

/// Run every statement in order, stopping at the first error.
///
/// Stopping matters: if statement 2 of 5 fails, running 3–5 against a
/// half-applied state is rarely what anyone wants, and psql behaves the same
/// way outside an explicit transaction block.
///
/// # Arguments
/// * `client` — `&Client`: an unrestricted editor connection; every statement
///   runs on this same session, so `SET` and transaction control carry over.
/// * `statements` — `&[String]`: already-split statements in execution order; an
///   empty slice is a no-op run.
///
/// # Returns
/// `Result<QueryRun>` — one `StatementResult` per statement, plus wall-clock
/// time for the whole run. `Err` on the first failing statement, discarding the
/// results of those that already succeeded.
pub async fn exec_all(client: &Client, statements: &[String]) -> Result<QueryRun> {
    let t0 = std::time::Instant::now();
    let mut out = Vec::with_capacity(statements.len());

    for sql in statements {
        out.push(exec_one(client, sql).await?);
    }

    Ok(QueryRun {
        statements: out,
        total_timing_ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_oversized_cells_on_a_char_boundary() {
        let capped = cap_cell("é".repeat(CELL_CAP_BYTES));
        assert!(capped.ends_with('…'));
        assert!(capped.is_char_boundary(capped.len()));
    }

    #[test]
    fn leaves_small_cells_alone() {
        assert_eq!(cap_cell("hello".into()), "hello");
    }
}
