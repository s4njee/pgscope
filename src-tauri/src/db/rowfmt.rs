//! Rendering rows and values for the grid's context menu.
//!
//! Copy-as-INSERT and filter-to-value both need SQL literals quoted correctly
//! for the column's type, so this lives next to [`quote_literal`] rather than
//! being reimplemented in the frontend — two copies of escaping rules drift,
//! and the one that drifts produces SQL that looks fine and isn't.

use serde::{Deserialize, Serialize};

use super::grid::{quote_ident, quote_literal};
use super::introspect::ColumnMeta;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowFormat {
    Json,
    Csv,
    Tsv,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PredicateOp {
    Eq,
    NotEq,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowFormatRequest {
    pub schema: String,
    pub table: String,
    pub columns: Vec<ColumnMeta>,
    pub values: Vec<Option<String>>,
    pub format: RowFormat,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormattedRow {
    pub text: String,
}

/// Base type with modifiers and array suffix stripped: `varchar(20)[]` → `varchar`.
///
/// # Arguments
/// * `data_type` — `&str`: the type name as introspected, possibly with a length
///   modifier and an array suffix.
///
/// # Returns
/// `String` — the bare type name, lowercased.
fn base_type(data_type: &str) -> String {
    data_type
        .trim_end_matches("[]")
        .split('(')
        .next()
        .unwrap_or(data_type)
        .trim()
        .to_lowercase()
}

/// Whether a value of this type is written without quotes in SQL.
///
/// Only exact numeric and boolean literals qualify. Everything else — including
/// dates, uuids and json — is quoted and, where needed, cast.
///
/// # Arguments
/// * `data_type` — `&str`: the full type name; the `[]` suffix is checked before
///   the base name, so array types are rejected outright.
///
/// # Returns
/// `bool` — `true` only for exact numeric and boolean scalar types.
fn is_bare_literal(data_type: &str) -> bool {
    // Arrays always need quoting even when their element type is numeric.
    if data_type.trim_end_matches(' ').ends_with("[]") {
        return false;
    }
    matches!(
        base_type(data_type).as_str(),
        "smallint"
            | "integer"
            | "int"
            | "int2"
            | "int4"
            | "int8"
            | "bigint"
            | "decimal"
            | "numeric"
            | "real"
            | "double precision"
            | "float4"
            | "float8"
            | "boolean"
            | "bool"
    )
}

/// A value written as a SQL literal for its column's type.
///
/// `NULL` is the keyword, not the string `'NULL'` — the distinction that makes
/// a generated INSERT correct rather than subtly wrong.
///
/// # Arguments
/// * `column` — `&ColumnMeta`: only `data_type` is read; it decides quoting and
///   any trailing cast.
/// * `value` — `Option<&str>`: the cell as it arrived over the text protocol,
///   unquoted. `None` is a real SQL NULL, distinct from an empty string.
///
/// # Returns
/// `String` — a literal ready to interpolate: `NULL`, a bare number or boolean,
/// or a single-quoted string with `::jsonb` appended for json columns.
pub fn sql_literal(column: &ColumnMeta, value: Option<&str>) -> String {
    let Some(v) = value else {
        return "NULL".to_string();
    };

    if is_bare_literal(&column.data_type) {
        // Guard against a non-numeric slipping through (an enum named like a
        // number, a corrupted text cell): quote it rather than emit bare junk.
        let numeric = v
            .parse::<f64>()
            .is_ok()
            .then_some(true)
            .or_else(|| matches!(v, "true" | "false" | "t" | "f").then_some(true))
            .unwrap_or(false);
        if numeric {
            return v.to_string();
        }
    }

    let quoted = quote_literal(v);
    match base_type(&column.data_type).as_str() {
        // jsonb comparison needs the cast or Postgres compares as text.
        "json" | "jsonb" => format!("{quoted}::jsonb"),
        _ => quoted,
    }
}

/// A `WHERE`-clause fragment matching (or excluding) this value.
///
/// NULL becomes `IS NULL` / `IS NOT NULL`: `= NULL` is never true, so the
/// obvious construction would silently return nothing.
///
/// # Arguments
/// * `column` — `&ColumnMeta`: `name` is quoted as an identifier, `data_type`
///   steers the literal.
/// * `value` — `Option<&str>`: the raw cell; `None` selects the `IS [NOT] NULL`
///   form instead of a comparison.
/// * `op` — `PredicateOp`: `Eq` or `NotEq`.
///
/// # Returns
/// `String` — a bare predicate with no leading `WHERE` and no surrounding
/// space, ready to drop into the grid's filter box.
pub fn value_predicate(column: &ColumnMeta, value: Option<&str>, op: PredicateOp) -> String {
    let ident = quote_ident(&column.name);
    match (value, op) {
        (None, PredicateOp::Eq) => format!("{ident} IS NULL"),
        (None, PredicateOp::NotEq) => format!("{ident} IS NOT NULL"),
        (Some(_), op) => {
            let operator = match op {
                PredicateOp::Eq => "=",
                PredicateOp::NotEq => "<>",
            };
            format!("{ident} {operator} {}", sql_literal(column, value))
        }
    }
}

/// Quote a CSV field per RFC 4180: wrap in quotes when it contains a comma,
/// quote, CR or LF, and double any embedded quote.
///
/// # Arguments
/// * `value` — `Option<&str>`: the raw cell. `None` renders as an unquoted empty
///   field, which CSV cannot distinguish from an empty string.
///
/// # Returns
/// `String` — the field, quoted only when it needs to be.
fn csv_field(value: Option<&str>) -> String {
    let Some(v) = value else {
        // An unquoted empty field, so it round-trips as NULL rather than "".
        return String::new();
    };
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// TSV escaping: tabs and newlines would break the row, so they become escapes.
///
/// # Arguments
/// * `value` — `Option<&str>`: the raw cell; `None` renders as an empty field.
///
/// # Returns
/// `String` — the field with backslashes, tabs, CR and LF backslash-escaped, so
/// it contains no real separator or line break.
fn tsv_field(value: Option<&str>) -> String {
    value
        .unwrap_or("")
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Render a row in the requested format.
///
/// # Arguments
/// * `req` — `&RowFormatRequest`: columns, values and the target format.
///   `values` is indexed positionally against `columns`; a short `values` is
///   tolerated and the missing cells are treated as NULL rather than panicking.
///
/// # Returns
/// `String` — the rendered row. CSV carries a header line and a trailing
/// newline; TSV and INSERT do not. JSON is pretty-printed, falling back to `{}`
/// if serialisation somehow fails.
pub fn format_row(req: &RowFormatRequest) -> String {
    let columns = &req.columns;
    let values = &req.values;
    let get = |i: usize| values.get(i).and_then(|v| v.as_deref());

    match req.format {
        RowFormat::Json => {
            // Built through serde so escaping is the JSON spec's, not ours.
            let mut map = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let value = match get(i) {
                    None => serde_json::Value::Null,
                    Some(v) => json_value(col, v),
                };
                map.insert(col.name.clone(), value);
            }
            serde_json::to_string_pretty(&serde_json::Value::Object(map))
                .unwrap_or_else(|_| "{}".to_string())
        }

        RowFormat::Csv => {
            let header = columns
                .iter()
                .map(|c| csv_field(Some(&c.name)))
                .collect::<Vec<_>>()
                .join(",");
            let row = (0..columns.len())
                .map(|i| csv_field(get(i)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{header}\n{row}\n")
        }

        RowFormat::Tsv => (0..columns.len())
            .map(|i| tsv_field(get(i)))
            .collect::<Vec<_>>()
            .join("\t"),

        RowFormat::Insert => {
            let cols = columns
                .iter()
                .map(|c| quote_ident(&c.name))
                .collect::<Vec<_>>()
                .join(", ");
            let vals = columns
                .iter()
                .enumerate()
                .map(|(i, col)| sql_literal(col, get(i)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "INSERT INTO {}.{} ({cols})\nVALUES ({vals});",
                quote_ident(&req.schema),
                quote_ident(&req.table)
            )
        }
    }
}

/// Typed JSON value for a column, so numbers and booleans aren't stringified.
///
/// # Arguments
/// * `column` — `&ColumnMeta`: `data_type` picks the interpretation; array types
///   are always left as strings.
/// * `raw` — `&str`: the cell as text. Never `None` — the caller maps NULL to
///   `Value::Null` before getting here.
///
/// # Returns
/// `serde_json::Value` — a typed value where the text parses as one, otherwise
/// `Value::String(raw)` rather than an error.
fn json_value(column: &ColumnMeta, raw: &str) -> serde_json::Value {
    let base = base_type(&column.data_type);
    let is_array = column.data_type.ends_with("[]");

    if !is_array {
        match base.as_str() {
            "json" | "jsonb" => {
                // Embed the document itself rather than a string of it.
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
                    return v;
                }
            }
            "boolean" | "bool" => {
                if let Some(b) = parse_bool(raw) {
                    return serde_json::Value::Bool(b);
                }
            }
            _ if is_bare_literal(&column.data_type) => {
                if let Ok(n) = raw.parse::<i64>() {
                    return serde_json::Value::from(n);
                }
                if let Ok(f) = raw.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        return serde_json::Value::Number(n);
                    }
                }
            }
            _ => {}
        }
    }
    serde_json::Value::String(raw.to_string())
}

/// Interpret a boolean as it arrives over the text protocol.
///
/// Postgres sends `t`/`f`; the spelled-out forms are accepted because the same
/// values can reach here from a hand-edited cell. Anything else yields `None`,
/// and the caller keeps the raw string rather than guessing.
///
/// # Arguments
/// * `raw` — `&str`: matched exactly, so `"True"` and `" t"` do not parse.
///
/// # Returns
/// `Option<bool>` — the value, or `None` when the text is not a recognised
/// spelling.
fn parse_bool(raw: &str) -> Option<bool> {
    match raw {
        "t" | "true" | "TRUE" => Some(true),
        "f" | "false" | "FALSE" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal column; only the name and type steer formatting, so the key
    /// and nullability flags stay off.
    ///
    /// # Arguments
    /// * `name` — `&str`: raw, unquoted column name.
    /// * `data_type` — `&str`: the type name, spelled as introspection returns it.
    ///
    /// # Returns
    /// `ColumnMeta` — with `not_null`, `is_pk` and `is_fk` all `false`.
    fn col(name: &str, data_type: &str) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            data_type: data_type.into(),
            not_null: false,
            is_pk: false,
            is_fk: false,
        }
    }

    /// One column per type the formatters treat differently: integer, text,
    /// jsonb and timestamptz.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `Vec<ColumnMeta>` — five columns, positionally matching
    /// [`events_values`].
    fn events_columns() -> Vec<ColumnMeta> {
        vec![
            col("event_id", "bigint"),
            col("user_id", "text"),
            col("event_name", "text"),
            col("properties", "jsonb"),
            col("created_at", "timestamp with time zone"),
        ]
    }

    /// Values matching `events_columns`, with `user_id` left NULL so every
    /// format has to prove it renders a missing value correctly.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `Vec<Option<String>>` — five cells, the second `None`.
    fn events_values() -> Vec<Option<String>> {
        vec![
            Some("48213904".into()),
            None,
            Some("signup".into()),
            Some(r#"{"plan": "pro"}"#.into()),
            Some("2026-07-18 09:41:22.114+00".into()),
        ]
    }

    /// The one `events` row above, with only the output format varying — that
    /// is the whole axis these tests compare along.
    ///
    /// # Arguments
    /// * `format` — `RowFormat`: the only field that varies between tests.
    ///
    /// # Returns
    /// `RowFormatRequest` — `public.events` with the fixture row.
    fn req(format: RowFormat) -> RowFormatRequest {
        RowFormatRequest {
            schema: "public".into(),
            table: "events".into(),
            columns: events_columns(),
            values: events_values(),
            format,
        }
    }

    // ------------------------------ literals ------------------------------

    #[test]
    fn numeric_and_boolean_columns_are_bare() {
        assert_eq!(sql_literal(&col("n", "bigint"), Some("42")), "42");
        assert_eq!(sql_literal(&col("n", "numeric"), Some("1.5")), "1.5");
        assert_eq!(sql_literal(&col("b", "boolean"), Some("true")), "true");
        assert_eq!(sql_literal(&col("b", "boolean"), Some("f")), "f");
    }

    #[test]
    fn text_like_columns_are_quoted() {
        assert_eq!(sql_literal(&col("s", "text"), Some("signup")), "'signup'");
        assert_eq!(
            sql_literal(&col("u", "uuid"), Some("9b2e41aa")),
            "'9b2e41aa'"
        );
        // Dates are quoted, not bare, despite looking numeric-ish.
        assert_eq!(
            sql_literal(&col("d", "timestamp with time zone"), Some("2026-07-18")),
            "'2026-07-18'"
        );
    }

    #[test]
    fn embedded_quotes_are_doubled() {
        // The classic: an apostrophe would otherwise terminate the literal.
        assert_eq!(sql_literal(&col("s", "text"), Some("it's")), "'it''s'");
        assert_eq!(
            sql_literal(&col("s", "text"), Some("'; DROP TABLE users; --")),
            "'''; DROP TABLE users; --'"
        );
    }

    #[test]
    fn null_is_the_keyword_not_a_string() {
        assert_eq!(sql_literal(&col("s", "text"), None), "NULL");
        assert_eq!(sql_literal(&col("n", "bigint"), None), "NULL");
    }

    #[test]
    fn json_literals_are_cast() {
        assert_eq!(
            sql_literal(&col("p", "jsonb"), Some(r#"{"a": 1}"#)),
            r#"'{"a": 1}'::jsonb"#
        );
    }

    #[test]
    fn a_non_numeric_value_in_a_numeric_column_is_still_quoted() {
        // Defensive: emitting this bare would produce invalid SQL.
        assert_eq!(sql_literal(&col("n", "integer"), Some("abc")), "'abc'");
    }

    #[test]
    fn arrays_are_quoted_even_with_numeric_elements() {
        assert_eq!(
            sql_literal(&col("a", "integer[]"), Some("{1,2,3}")),
            "'{1,2,3}'"
        );
    }

    // ----------------------------- predicates -----------------------------

    #[test]
    fn equality_predicate_matches_the_backlog_example() {
        assert_eq!(
            value_predicate(&col("event_name", "text"), Some("signup"), PredicateOp::Eq),
            r#""event_name" = 'signup'"#
        );
    }

    #[test]
    fn null_uses_is_null_not_equals() {
        // `= NULL` is never true — the bug this exists to avoid.
        assert_eq!(
            value_predicate(&col("user_id", "text"), None, PredicateOp::Eq),
            r#""user_id" IS NULL"#
        );
        assert_eq!(
            value_predicate(&col("user_id", "text"), None, PredicateOp::NotEq),
            r#""user_id" IS NOT NULL"#
        );
    }

    #[test]
    fn not_equal_uses_the_sql_operator() {
        assert_eq!(
            value_predicate(&col("n", "bigint"), Some("42"), PredicateOp::NotEq),
            r#""n" <> 42"#
        );
    }

    #[test]
    fn predicate_quotes_hostile_column_names() {
        assert_eq!(
            value_predicate(&col(r#"we"ird"#, "text"), Some("x"), PredicateOp::Eq),
            r#""we""ird" = 'x'"#
        );
    }

    // ------------------------------- INSERT -------------------------------

    #[test]
    fn insert_renders_columns_and_typed_values() {
        let text = format_row(&req(RowFormat::Insert));
        assert!(
            text.starts_with(r#"INSERT INTO "public"."events""#),
            "got {text}"
        );
        assert!(text.contains(r#""event_id", "user_id", "event_name""#));
        // bare number, NULL keyword, quoted text, cast json
        assert!(text.contains("48213904"));
        assert!(text.contains("NULL"));
        assert!(text.contains("'signup'"));
        assert!(text.contains("::jsonb"));
        assert!(text.ends_with(");"));
    }

    // -------------------------------- CSV ---------------------------------

    #[test]
    fn csv_has_a_header_and_one_row() {
        let text = format_row(&req(RowFormat::Csv));
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("event_id,user_id,event_name"));
    }

    #[test]
    fn csv_quotes_fields_containing_separators() {
        let mut r = req(RowFormat::Csv);
        r.columns = vec![col("a", "text"), col("b", "text"), col("c", "text")];
        r.values = vec![
            Some("has,comma".into()),
            Some(r#"has"quote"#.into()),
            Some("has\nnewline".into()),
        ];
        let text = format_row(&r);

        // Note: a quoted field may itself contain a newline, so the record is
        // NOT simply "the second physical line" — assert on the whole output.
        assert!(text.contains(r#""has,comma""#), "got {text}");
        assert!(text.contains(r#""has""quote""#), "got {text}");
        assert!(text.contains("\"has\nnewline\""), "got {text}");
        // The header is unquoted; only the value line needed quoting.
        assert!(text.starts_with("a,b,c\n"));
    }

    #[test]
    fn csv_writes_null_as_an_empty_unquoted_field() {
        let mut r = req(RowFormat::Csv);
        r.columns = vec![col("a", "text"), col("b", "text")];
        r.values = vec![None, Some(String::new())];
        let row = format_row(&r)
            .trim_end()
            .split('\n')
            .nth(1)
            .unwrap()
            .to_string();
        // NULL and empty-string are both empty here; CSV cannot distinguish them,
        // which is why JSON exists as an option.
        assert_eq!(row, ",");
    }

    // -------------------------------- TSV ---------------------------------

    #[test]
    fn tsv_escapes_tabs_and_newlines() {
        let mut r = req(RowFormat::Tsv);
        r.columns = vec![col("a", "text"), col("b", "text")];
        r.values = vec![Some("has\ttab".into()), Some("has\nnewline".into())];
        let text = format_row(&r);
        assert_eq!(text, "has\\ttab\thas\\nnewline");
        // Exactly one real tab: the separator.
        assert_eq!(text.matches('\t').count(), 1);
    }

    // -------------------------------- JSON --------------------------------

    #[test]
    fn json_uses_typed_values_not_strings() {
        let text = format_row(&req(RowFormat::Json));
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();

        // A bigint is a number, not "48213904".
        assert_eq!(parsed["event_id"], serde_json::json!(48213904i64));
        // NULL is null, distinct from "".
        assert_eq!(parsed["user_id"], serde_json::Value::Null);
        assert_eq!(parsed["event_name"], serde_json::json!("signup"));
        // jsonb is embedded as a document, not a string of one.
        assert_eq!(parsed["properties"], serde_json::json!({"plan": "pro"}));
        // A timestamp stays a string.
        assert!(parsed["created_at"].is_string());
    }

    #[test]
    fn json_escapes_correctly() {
        let mut r = req(RowFormat::Json);
        r.columns = vec![col("a", "text")];
        r.values = vec![Some("quote\" newline\n tab\t".into())];
        let text = format_row(&r);
        // Round-trips, which is the real assertion.
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["a"], serde_json::json!("quote\" newline\n tab\t"));
    }

    #[test]
    fn json_booleans_are_typed() {
        let mut r = req(RowFormat::Json);
        r.columns = vec![col("b", "boolean"), col("c", "boolean")];
        r.values = vec![Some("t".into()), Some("false".into())];
        let parsed: serde_json::Value = serde_json::from_str(&format_row(&r)).unwrap();
        assert_eq!(parsed["b"], serde_json::json!(true));
        assert_eq!(parsed["c"], serde_json::json!(false));
    }

    #[test]
    fn malformed_json_falls_back_to_a_string() {
        let mut r = req(RowFormat::Json);
        r.columns = vec![col("p", "jsonb")];
        r.values = vec![Some("{not json".into())];
        let parsed: serde_json::Value = serde_json::from_str(&format_row(&r)).unwrap();
        assert!(parsed["p"].is_string());
    }

    #[test]
    fn a_row_with_missing_values_does_not_panic() {
        let mut r = req(RowFormat::Insert);
        r.values = vec![Some("1".into())]; // fewer values than columns
        let text = format_row(&r);
        assert!(text.contains("NULL"), "missing values become NULL: {text}");
    }
}
