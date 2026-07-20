//! Fetching one cell's full value for inline expansion.
//!
//! The grid caps every cell at [`CELL_CAP_BYTES`] so a page of wide `jsonb` or
//! `text` doesn't ship megabytes to the webview — but those capped values are
//! precisely the ones worth opening. This re-fetches a single cell without that
//! cap, and lets Postgres pretty-print JSON via `jsonb_pretty`.

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;

use super::grid::{order_clause, quote_ident, where_clause, PageRequest};
use crate::error::{AppError, Result};

/// Ceiling for an expanded value. Far above the grid's 8KB cap, but still
/// bounded — a 1GB `text` column must not take the app down.
pub const EXPANDED_CAP_BYTES: usize = 4 * 1024 * 1024;

/// One primary-key component identifying the row.
#[derive(Debug, Clone, Deserialize)]
pub struct PkValue {
    pub column: String,
    /// Text form, as the grid received it.
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellRequest {
    pub column: String,
    /// Primary-key values, when the table has one. Preferred, because it stays
    /// correct regardless of paging or concurrent inserts.
    #[serde(default)]
    pub pk: Vec<PkValue>,
    /// The page the row came from, used both for the PK-less fallback and to
    /// carry schema/table/sort/filter.
    pub page: PageRequest,
    /// Row's index within its page, for the fallback.
    pub row_index: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CellFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellValue {
    pub column: String,
    pub data_type: String,
    /// None for SQL NULL.
    pub value: Option<String>,
    pub format: CellFormat,
    /// Byte length of the value on the server, before any capping.
    pub total_bytes: i64,
    pub truncated: bool,
    /// True when the row was located by primary key rather than by offset.
    pub located_by_pk: bool,
}

/// Whether the column's `format_type` name is one Postgres can pretty-print.
///
/// Matched exactly, not by substring: `jsonpath` is a text type, not a document.
///
/// # Arguments
/// * `data_type` — `&str`: the `format_type` name as Postgres reports it, e.g.
///   `jsonb`, `text`, `character varying(20)`.
///
/// # Returns
/// `bool` — true only for exactly `json` or `jsonb`.
fn is_json_type(data_type: &str) -> bool {
    matches!(data_type, "json" | "jsonb")
}

/// Build the projection for one column: pretty-printed when it is JSON, plain
/// text otherwise. Also returns the raw octet length, so the UI can say when a
/// value was still too big to show in full.
///
/// # Arguments
/// * `column` — `&str`: a raw, unquoted column name; quoted here via
///   `quote_ident`.
/// * `data_type` — `&str`: the column's `format_type` name, used only to decide
///   whether to pretty-print.
///
/// # Returns
/// `String` — a select-list fragment producing the `value` and `total_bytes`
/// aliases the caller reads back.
fn projection(column: &str, data_type: &str) -> String {
    let ident = quote_ident(column);
    if is_json_type(data_type) {
        // jsonb_pretty gives Postgres's own canonical indentation; casting from
        // `json` first normalises whitespace the same way.
        format!("jsonb_pretty({ident}::jsonb) AS value, octet_length({ident}::text)::bigint AS total_bytes")
    } else {
        format!("{ident}::text AS value, octet_length({ident}::text)::bigint AS total_bytes")
    }
}

/// Look up the column's declared type, so the projection can branch on it.
///
/// # Arguments
/// * `client` — `&Client`: any connection; the lookup is a plain catalogue read.
/// * `schema` — `&str`: raw schema name, bound as a parameter, not interpolated.
/// * `table` — `&str`: raw table name, likewise bound.
/// * `column` — `&str`: raw column name, likewise bound.
///
/// # Returns
/// `Result<String>` — the `format_type` name. `Err` is `AppError::Invalid` when
/// no such live column exists on that relation.
async fn column_type(client: &Client, schema: &str, table: &str, column: &str) -> Result<String> {
    let row = client
        .query_opt(
            "SELECT format_type(a.atttypid, a.atttypmod) AS data_type
             FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attname = $3
               AND a.attnum > 0 AND NOT a.attisdropped",
            &[&schema, &table, &column],
        )
        .await?
        .ok_or_else(|| AppError::Invalid(format!("no such column: {column}")))?;
    Ok(row.get("data_type"))
}

/// Fetch a single cell in full.
///
/// Prefers locating the row by primary key. Without one, it re-runs the page's
/// own query and takes the nth row — correct as long as the ordering is stable,
/// which is the same caveat the grid itself already lives with.
///
/// # Arguments
/// * `client` — `&Client`: a single connection, so the fallback's `OFFSET` scan
///   isn't subject to the browse pool's statement timeout.
/// * `req` — `&CellRequest`: the column plus the row's identity — primary-key
///   values when present, otherwise page and row index.
///
/// # Returns
/// `Result<CellValue>` — the value capped at `EXPANDED_CAP_BYTES` with
/// `truncated` set when it was clipped, and `value` `None` for SQL NULL. `Err`
/// is `AppError::Invalid` for a negative row index or a row that no longer
/// matches, otherwise a query failure.
pub async fn fetch_cell(client: &Client, req: &CellRequest) -> Result<CellValue> {
    let schema = &req.page.schema;
    let table = &req.page.table;
    let data_type = column_type(client, schema, table, &req.column).await?;
    let relation = format!("{}.{}", quote_ident(schema), quote_ident(table));
    let proj = projection(&req.column, &data_type);

    let (sql, params): (String, Vec<String>) = if !req.pk.is_empty() {
        // `col::text = $n` compares in the same text domain the grid displayed,
        // so it works for every type without per-type binding.
        let mut clauses = Vec::new();
        let mut values = Vec::new();
        for (i, pk) in req.pk.iter().enumerate() {
            clauses.push(format!("{}::text = ${}", quote_ident(&pk.column), i + 1));
            values.push(pk.value.clone());
        }
        (
            format!(
                "SELECT {proj} FROM {relation} WHERE {} LIMIT 1",
                clauses.join(" AND ")
            ),
            values,
        )
    } else {
        if req.row_index < 0 {
            return Err(AppError::Invalid("row index must be >= 0".into()));
        }
        // Reuse the grid's own clause builders: locating a row by position is
        // only correct if this query orders and filters exactly as the page did.
        let wheres = where_clause(req.page.filter.as_deref());
        let order = order_clause(&req.page.sort, false);
        let offset = req.page.page * super::grid::PAGE_SIZE + req.row_index;
        (
            format!("SELECT {proj} FROM {relation}{wheres}{order} LIMIT 1 OFFSET {offset}"),
            Vec::new(),
        )
    };

    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let row = client
        .query_opt(sql.as_str(), &param_refs)
        .await?
        .ok_or_else(|| AppError::Invalid("row not found — it may have changed".into()))?;

    let raw: Option<String> = row.get("value");
    let total_bytes: i64 = row.get::<_, Option<i64>>("total_bytes").unwrap_or(0);

    let (value, truncated) = match raw {
        None => (None, false),
        Some(v) if v.len() > EXPANDED_CAP_BYTES => {
            let mut end = EXPANDED_CAP_BYTES;
            while end > 0 && !v.is_char_boundary(end) {
                end -= 1;
            }
            (Some(v[..end].to_string()), true)
        }
        Some(v) => (Some(v), false),
    };

    Ok(CellValue {
        column: req.column.clone(),
        format: if is_json_type(&data_type) {
            CellFormat::Json
        } else {
            CellFormat::Text
        },
        data_type,
        value,
        total_bytes,
        truncated,
        located_by_pk: !req.pk.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_types_are_pretty_printed() {
        let p = projection("properties", "jsonb");
        assert!(p.contains("jsonb_pretty"), "got {p}");
        assert!(p.contains(r#""properties""#));
    }

    #[test]
    fn non_json_types_are_plain_text() {
        let p = projection("event_name", "text");
        assert!(!p.contains("jsonb_pretty"));
        assert!(p.contains(r#""event_name"::text"#));
    }

    #[test]
    fn json_detection_covers_both_json_types() {
        assert!(is_json_type("json"));
        assert!(is_json_type("jsonb"));
        assert!(!is_json_type("text"));
        // A type merely *containing* "json" is not a JSON column.
        assert!(!is_json_type("jsonpath"));
    }

    #[test]
    fn projection_quotes_hostile_column_names() {
        let p = projection(r#"we"ird"#, "text");
        assert!(p.contains(r#""we""ird""#), "got {p}");
    }

    #[test]
    fn every_projection_reports_the_true_size() {
        // The UI needs the pre-cap length to say "truncated from N".
        for ty in ["jsonb", "text", "bytea"] {
            assert!(projection("c", ty).contains("octet_length"), "type {ty}");
        }
    }
}
