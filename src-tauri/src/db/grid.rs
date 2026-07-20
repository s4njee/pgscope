use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

pub const PAGE_SIZE: i64 = 50;

/// Per-cell display cap. Wide `jsonb`/`text` values would otherwise ship
/// megabytes per page to the webview for cells that render ~40 visible chars.
pub const CELL_CAP_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    /// The ORDER BY keyword for this direction.
    ///
    /// # Arguments
    /// * `self` — `SortDir`: taken by value; the enum is `Copy`.
    ///
    /// # Returns
    /// `&'static str` — `"ASC"` or `"DESC"`, safe to interpolate as-is.
    fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    /// The opposite direction, used to fetch the last page by reading the tail
    /// of the inverted ordering instead of paying for a large OFFSET.
    ///
    /// # Arguments
    /// * `self` — `SortDir`: taken by value; the receiver is left unchanged.
    ///
    /// # Returns
    /// `SortDir` — the flipped direction.
    fn reversed(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

/// One term of an ORDER BY, in the order the user shift-clicked them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortKey {
    pub column: String,
    pub dir: SortDir,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub schema: String,
    pub table: String,
    /// Sort terms, most significant first. Empty means unsorted.
    #[serde(default)]
    pub sort: Vec<SortKey>,
    /// Raw SQL placed after `WHERE`. Deliberately not escaped: this is a
    /// database client and the browse session is read-only + time-limited.
    #[serde(default)]
    pub filter: Option<String>,
    /// Zero-based page index.
    #[serde(default)]
    pub page: i64,
}

/// Quote an identifier for interpolation: wrap in double quotes and double any
/// embedded quote. This is the only way identifiers ever reach a query string.
///
/// # Arguments
/// * `ident` — `&str`: a raw, unquoted identifier — a schema, table or column
///   name as introspected. Never pass already-quoted text; it would be
///   double-quoted.
///
/// # Returns
/// `String` — the quoted identifier, including the surrounding double quotes.
pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Quote a string literal (used for regclass lookups).
///
/// # Arguments
/// * `s` — `&str`: the raw, unquoted value; embedded single quotes are doubled.
///
/// # Returns
/// `String` — the literal, including the surrounding single quotes.
pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Reduce a user-typed filter to the bare predicate, or `None` if there isn't
/// one.
///
/// The text is passed through to SQL unescaped (see [`PageRequest::filter`]);
/// this only trims and strips a redundant leading `WHERE`, so that whitespace
/// alone doesn't produce a syntactically broken clause.
///
/// # Arguments
/// * `filter` — `Option<&str>`: raw user text, not escaped. `None` and
///   whitespace-only both mean "no filter".
///
/// # Returns
/// `Option<String>` — the trimmed predicate without its leading `WHERE`, or
/// `None` when nothing usable remains.
fn normalized_filter(filter: Option<&str>) -> Option<String> {
    let f = filter?.trim();
    if f.is_empty() {
        return None;
    }
    // Tolerate a leading WHERE — the placeholder text invites typing it.
    let stripped = f
        .strip_prefix("WHERE ")
        .or_else(|| f.strip_prefix("where "))
        .unwrap_or(f);
    let stripped = stripped.trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// ` WHERE …`, or empty. Shared so every query over a page agrees on the filter.
///
/// # Arguments
/// * `filter` — `Option<&str>`: raw user text; see [`normalized_filter`].
///
/// # Returns
/// `String` — a ` WHERE …` fragment with a leading space, or empty when there is
/// no filter, so it can be concatenated unconditionally.
pub fn where_clause(filter: Option<&str>) -> String {
    normalized_filter(filter)
        .map(|f| format!(" WHERE {f}"))
        .unwrap_or_default()
}

/// ` ORDER BY a ASC, b DESC`, or empty.
///
/// `reversed` flips **every** term — a partial flip would silently reorder the
/// tail rows within their groups, which is the subtle way a multi-column
/// last-page optimisation goes wrong.
///
/// # Arguments
/// * `sort` — `&[SortKey]`: ORDER BY terms in precedence order; empty means
///   unsorted. Column names are raw and get quoted here.
/// * `reversed` — `bool`: flips every key, for the last-page reverse scan.
///
/// # Returns
/// `String` — an ` ORDER BY …` fragment with a leading space, or empty when
/// `sort` is empty so it can be concatenated unconditionally.
pub fn order_clause(sort: &[SortKey], reversed: bool) -> String {
    if sort.is_empty() {
        return String::new();
    }
    let terms: Vec<String> = sort
        .iter()
        .map(|k| {
            let dir = if reversed { k.dir.reversed() } else { k.dir };
            format!("{} {}", quote_ident(&k.column), dir.sql())
        })
        .collect();
    format!(" ORDER BY {}", terms.join(", "))
}

/// The SQL for one page, plus whether the rows come back reversed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageQuery {
    pub sql: String,
    /// True when we flipped the sort to fetch the tail cheaply; the caller must
    /// reverse the rows before display.
    pub reverse_rows: bool,
}

/// Build the page query.
///
/// Every value is projected as `::text` so any column type — enums, domains,
/// extension types, arrays — renders without a per-type decoder. This is also
/// how psql itself displays data.
///
/// `total_rows` enables the last-page optimisation: rather than
/// `OFFSET 48213854` (which makes Postgres walk 48M rows), we reverse the sort
/// and take the first 50, then flip them back. Without a sort column there is
/// no stable "last page", so we fall back to OFFSET.
///
/// # Arguments
/// * `req` — `&PageRequest`: schema, table, sort, filter and zero-based page.
///   Its `filter` is raw SQL; its sort columns are validated against `columns`.
/// * `columns` — `&[String]`: the introspected column names, in display order.
///   They form the projection and are the only names a sort key may reference.
/// * `total_rows` — `Option<i64>`: known row count, exact or estimated. `None`
///   disables the last-page optimisation entirely.
///
/// # Returns
/// `Result<PageQuery>` — the SQL plus a flag for whether the rows arrive
/// reversed. `Err` (`AppError::Invalid`) when `columns` is empty, a sort column
/// is unknown or duplicated, or `page` is negative.
pub fn build_page_query(
    req: &PageRequest,
    columns: &[String],
    total_rows: Option<i64>,
) -> Result<PageQuery> {
    if columns.is_empty() {
        return Err(AppError::Invalid("table has no columns".into()));
    }

    // Every sort column must be one we introspected — never free text.
    for key in &req.sort {
        if !columns.iter().any(|c| c == &key.column) {
            return Err(AppError::Invalid(format!(
                "unknown sort column: {}",
                key.column
            )));
        }
    }
    // Repeating a column makes the later term dead and the UI ambiguous.
    for (i, key) in req.sort.iter().enumerate() {
        if req.sort[..i].iter().any(|k| k.column == key.column) {
            return Err(AppError::Invalid(format!(
                "duplicate sort column: {}",
                key.column
            )));
        }
    }
    if req.page < 0 {
        return Err(AppError::Invalid("page must be >= 0".into()));
    }

    let projection = columns
        .iter()
        .map(|c| format!("{}::text", quote_ident(c)))
        .collect::<Vec<_>>()
        .join(", ");

    let relation = format!("{}.{}", quote_ident(&req.schema), quote_ident(&req.table));
    let wheres = where_clause(req.filter.as_deref());
    let offset = req.page * PAGE_SIZE;

    // Reverse-scan the tail when the requested page is the last one and we know
    // where that is. Requires a sort for a deterministic ordering.
    let use_reverse = match total_rows {
        Some(total) if total > 0 && !req.sort.is_empty() => {
            let last_page = (total - 1) / PAGE_SIZE;
            req.page == last_page && offset > PAGE_SIZE * 4
        }
        _ => false,
    };

    if use_reverse {
        let total = total_rows.unwrap();
        // The tail may be a partial page.
        let tail_len = total - offset;
        let sql = format!(
            "SELECT {projection} FROM {relation}{wheres}{} LIMIT {}",
            order_clause(&req.sort, true),
            tail_len.clamp(1, PAGE_SIZE),
        );
        return Ok(PageQuery {
            sql,
            reverse_rows: true,
        });
    }

    let sql = format!(
        "SELECT {projection} FROM {relation}{wheres}{} LIMIT {PAGE_SIZE} OFFSET {offset}",
        order_clause(&req.sort, false)
    );
    Ok(PageQuery {
        sql,
        reverse_rows: false,
    })
}

/// `count(*)` for a filtered browse. Only used when a filter is active — the
/// unfiltered total comes from `reltuples`, which is free.
///
/// # Arguments
/// * `req` — `&PageRequest`: only `schema`, `table` and `filter` are read; sort
///   and page do not affect a count.
///
/// # Returns
/// `String` — the complete `SELECT count(*) …` statement.
pub fn build_count_query(req: &PageRequest) -> String {
    let relation = format!("{}.{}", quote_ident(&req.schema), quote_ident(&req.table));
    format!(
        "SELECT count(*) FROM {relation}{}",
        where_clause(req.filter.as_deref())
    )
}

/// The display string shown in the grid footer: the real SQL, ellipsized.
///
/// # Arguments
/// * `sql` — `&str`: the statement as sent; its `::text` casts are stripped for
///   readability, so the result is for display only, not for re-execution.
/// * `max` — `usize`: cap in characters, not bytes, including the ellipsis.
///
/// # Returns
/// `String` — the cleaned SQL, ending in `…` when it had to be cut.
pub fn display_sql(sql: &str, max: usize) -> String {
    let cleaned = sql.replace("::text", "");
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    let truncated: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Clamp a cell value for transport, marking truncation with an ellipsis.
///
/// # Arguments
/// * `value` — `String`: one cell, taken by value and returned unchanged when it
///   already fits. The cap is [`CELL_CAP_BYTES`] bytes, not characters.
///
/// # Returns
/// `String` — the value, or a prefix cut at a char boundary at or below the cap
/// with `…` appended.
pub fn cap_cell(value: String) -> String {
    if value.len() <= CELL_CAP_BYTES {
        return value;
    }
    // Cut on a char boundary at or below the cap.
    let mut end = CELL_CAP_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult {
    pub rows: Vec<Vec<Option<String>>>,
    pub timing_ms: f64,
    /// Estimated (unfiltered) or exact (filtered) total; None when a filtered
    /// count timed out.
    pub total: Option<i64>,
    pub total_is_estimate: bool,
    pub page: i64,
    pub page_size: i64,
    pub sql: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three `events` column names, enough to tell selected from rejected ones.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `Vec<String>` — raw, unquoted column names in display order.
    fn cols() -> Vec<String> {
        ["event_id", "user_id", "event_name"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// The baseline request — first page, no sort, no filter. Tests mutate the
    /// one field they exercise rather than restating the whole struct.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `PageRequest` — `public.events`, page 0, empty sort, no filter.
    fn req() -> PageRequest {
        PageRequest {
            schema: "public".into(),
            table: "events".into(),
            sort: Vec::new(),
            filter: None,
            page: 0,
        }
    }

    #[test]
    fn quotes_identifiers_and_doubles_embedded_quotes() {
        assert_eq!(quote_ident("events"), r#""events""#);
        assert_eq!(quote_ident("Mixed Case"), r#""Mixed Case""#);
        // The injection case: a quote in the identifier must be doubled, not
        // allowed to terminate the quoted string.
        assert_eq!(quote_ident(r#"weird"name"#), r#""weird""name""#);
        assert_eq!(
            quote_ident(r#"a"; DROP TABLE users; --"#),
            r#""a""; DROP TABLE users; --""#
        );
    }

    #[test]
    fn quotes_literals() {
        assert_eq!(quote_literal("public"), "'public'");
        assert_eq!(quote_literal("it's"), "'it''s'");
    }

    #[test]
    fn builds_a_basic_first_page() {
        let q = build_page_query(&req(), &cols(), None).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT "event_id"::text, "user_id"::text, "event_name"::text FROM "public"."events" LIMIT 50 OFFSET 0"#
        );
        assert!(!q.reverse_rows);
    }

    #[test]
    fn applies_offset_per_page() {
        let mut r = req();
        r.page = 3;
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(q.sql.ends_with("LIMIT 50 OFFSET 150"));
    }

    #[test]
    fn applies_sort_and_direction() {
        let mut r = req();
        r.sort = vec![SortKey {
            column: "event_name".into(),
            dir: SortDir::Desc,
        }];
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(q.sql.contains(r#"ORDER BY "event_name" DESC"#));
    }

    #[test]
    fn rejects_a_sort_column_that_is_not_a_real_column() {
        let mut r = req();
        r.sort = vec![SortKey {
            column: "1; DROP TABLE users".into(),
            dir: SortDir::Asc,
        }];
        let err = build_page_query(&r, &cols(), None).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[test]
    fn rejects_negative_pages() {
        let mut r = req();
        r.page = -1;
        assert!(build_page_query(&r, &cols(), None).is_err());
    }

    #[test]
    fn embeds_the_filter_verbatim() {
        let mut r = req();
        r.filter = Some("event_name = 'signup'".into());
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(q.sql.contains("WHERE event_name = 'signup'"));
    }

    #[test]
    fn strips_a_leading_where_from_the_filter() {
        let mut r = req();
        r.filter = Some("WHERE event_name = 'signup'".into());
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(q.sql.contains("WHERE event_name = 'signup'"));
        assert!(!q.sql.contains("WHERE WHERE"));
    }

    #[test]
    fn ignores_a_blank_filter() {
        let mut r = req();
        r.filter = Some("   ".into());
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(!q.sql.contains("WHERE"));
    }

    #[test]
    fn last_page_reverses_the_sort_instead_of_deep_offsetting() {
        // 48,213,904 rows: the last page starts at offset 48,213,850.
        let total = 48_213_904;
        let last_page = (total - 1) / PAGE_SIZE;
        let mut r = req();
        r.sort = vec![SortKey {
            column: "event_id".into(),
            dir: SortDir::Desc,
        }];
        r.page = last_page;

        let q = build_page_query(&r, &cols(), Some(total)).unwrap();
        assert!(q.reverse_rows, "should reverse-scan the tail");
        // Ascending, because the display order is descending.
        assert!(q.sql.contains(r#"ORDER BY "event_id" ASC"#));
        assert!(!q.sql.contains("OFFSET"), "must not deep-offset: {}", q.sql);
    }

    #[test]
    fn last_page_limit_matches_a_partial_tail() {
        // 104 rows => pages 0,1,2 with the last holding 4 rows.
        let total = 104;
        let mut r = req();
        r.sort = vec![SortKey {
            column: "event_id".into(),
            dir: SortDir::Asc,
        }];
        r.page = 2;
        // Offset 100 is not deep enough to bother reversing.
        let q = build_page_query(&r, &cols(), Some(total)).unwrap();
        assert!(!q.reverse_rows);

        // With a deep offset the tail length is respected.
        let total = 10_004;
        let last_page = (total - 1) / PAGE_SIZE;
        r.page = last_page;
        let q = build_page_query(&r, &cols(), Some(total)).unwrap();
        assert!(q.reverse_rows);
        assert!(q.sql.contains("LIMIT 4"), "tail of 4 rows: {}", q.sql);
    }

    #[test]
    fn no_reverse_without_a_sort() {
        let total = 48_213_904;
        let mut r = req();
        r.page = (total - 1) / PAGE_SIZE;
        let q = build_page_query(&r, &cols(), Some(total)).unwrap();
        assert!(!q.reverse_rows, "no stable tail without an ORDER BY");
        assert!(q.sql.contains("OFFSET"));
    }

    // ------------------------ multi-column sort ------------------------

    /// Keeps the multi-key sort lists at the call site readable.
    ///
    /// # Arguments
    /// * `column` — `&str`: raw, unquoted column name.
    /// * `dir` — `SortDir`: the direction for this term.
    ///
    /// # Returns
    /// `SortKey` — the two fields packed into one term.
    fn key(column: &str, dir: SortDir) -> SortKey {
        SortKey {
            column: column.into(),
            dir,
        }
    }

    #[test]
    fn orders_by_every_key_in_order() {
        let mut r = req();
        r.sort = vec![
            key("event_name", SortDir::Asc),
            key("event_id", SortDir::Desc),
        ];
        let q = build_page_query(&r, &cols(), None).unwrap();
        assert!(
            q.sql
                .contains(r#"ORDER BY "event_name" ASC, "event_id" DESC"#),
            "got {}",
            q.sql
        );
    }

    #[test]
    fn an_empty_sort_produces_no_order_by() {
        let q = build_page_query(&req(), &cols(), None).unwrap();
        assert!(!q.sql.contains("ORDER BY"));
    }

    #[test]
    fn rejects_an_unknown_column_anywhere_in_the_sort() {
        let mut r = req();
        // Valid first, hostile second — a check that only looked at the head
        // would let this through.
        r.sort = vec![
            key("event_id", SortDir::Asc),
            key("1; DROP TABLE users", SortDir::Asc),
        ];
        assert!(build_page_query(&r, &cols(), None).is_err());
    }

    #[test]
    fn rejects_a_duplicated_sort_column() {
        let mut r = req();
        r.sort = vec![
            key("event_id", SortDir::Asc),
            key("event_id", SortDir::Desc),
        ];
        let err = build_page_query(&r, &cols(), None).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// The subtle one: a partial flip reorders rows within their groups.
    #[test]
    fn the_last_page_reverses_every_sort_key() {
        let total = 48_213_904;
        let mut r = req();
        r.sort = vec![
            key("event_name", SortDir::Asc),
            key("event_id", SortDir::Desc),
        ];
        r.page = (total - 1) / PAGE_SIZE;

        let q = build_page_query(&r, &cols(), Some(total)).unwrap();
        assert!(q.reverse_rows);
        assert!(
            q.sql
                .contains(r#"ORDER BY "event_name" DESC, "event_id" ASC"#),
            "both keys must flip, got {}",
            q.sql
        );
        assert!(!q.sql.contains("OFFSET"));
    }

    #[test]
    fn order_clause_renders_and_reverses() {
        let sort = vec![key("a", SortDir::Asc), key("b", SortDir::Desc)];
        assert_eq!(order_clause(&sort, false), r#" ORDER BY "a" ASC, "b" DESC"#);
        assert_eq!(order_clause(&sort, true), r#" ORDER BY "a" DESC, "b" ASC"#);
        assert_eq!(order_clause(&[], false), "");
    }

    #[test]
    fn order_clause_quotes_hostile_identifiers() {
        let sort = vec![key(r#"we"ird"#, SortDir::Asc)];
        assert_eq!(order_clause(&sort, false), r#" ORDER BY "we""ird" ASC"#);
    }

    #[test]
    fn where_clause_is_shared_and_strips_a_leading_where() {
        assert_eq!(where_clause(Some("a = 1")), " WHERE a = 1");
        assert_eq!(where_clause(Some("WHERE a = 1")), " WHERE a = 1");
        assert_eq!(where_clause(Some("   ")), "");
        assert_eq!(where_clause(None), "");
    }

    #[test]
    fn count_query_includes_the_filter() {
        let mut r = req();
        r.filter = Some("event_name = 'signup'".into());
        assert_eq!(
            build_count_query(&r),
            r#"SELECT count(*) FROM "public"."events" WHERE event_name = 'signup'"#
        );
    }

    #[test]
    fn caps_oversized_cells_on_a_char_boundary() {
        let value = "é".repeat(CELL_CAP_BYTES); // 2 bytes each
        let capped = cap_cell(value);
        assert!(capped.ends_with('…'));
        assert!(capped.is_char_boundary(capped.len()));
    }

    #[test]
    fn leaves_small_cells_alone() {
        assert_eq!(cap_cell("hello".into()), "hello");
    }

    #[test]
    fn display_sql_drops_casts_and_ellipsizes() {
        let sql = r#"SELECT "a"::text FROM "public"."events" LIMIT 50 OFFSET 0"#;
        assert_eq!(
            display_sql(sql, 200),
            r#"SELECT "a" FROM "public"."events" LIMIT 50 OFFSET 0"#
        );
        assert!(display_sql(sql, 20).ends_with('…'));
        assert_eq!(display_sql(sql, 20).chars().count(), 20);
    }
}
