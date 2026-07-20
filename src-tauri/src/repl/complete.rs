//! Tab completion for the psql pane.
//!
//! The work splits in two: [`analyze`] decides *what kind* of thing the cursor
//! is sitting on — a table, a column of some specific table, a keyword, a
//! backslash command — and [`complete`] turns that into candidates by querying
//! the catalog. Only the second half needs a database, so the interesting logic
//! is a pure function with tests.
//!
//! Context matters more than it looks. `SELECT ev` and `FROM ev` want completely
//! different candidates, and `e.ev` wants the columns of whatever `e` was
//! aliased to earlier in the statement — which means parsing the FROM clause.

use serde::Serialize;
use tokio_postgres::Client;

use crate::db::grid::quote_literal;
use crate::error::Result;

/// A table mentioned in the statement, with the alias it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
    pub alias: Option<String>,
}

impl TableRef {
    /// Whether `qualifier` refers to this table — its alias, or its name when
    /// it has no alias.
    ///
    /// # Arguments
    /// * `qualifier` — `&str`: the raw text before the dot, unquoted and in
    ///   whatever case the user typed; compared case-insensitively.
    ///
    /// # Returns
    /// `bool` — whether this reference is what `qualifier` names.
    fn matches_qualifier(&self, qualifier: &str) -> bool {
        let q = qualifier.to_lowercase();
        match &self.alias {
            Some(a) => a.to_lowercase() == q,
            None => self.name.to_lowercase() == q,
        }
    }
}

/// What the cursor is positioned to complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// `\ti` — a backslash command name.
    MetaCommand { prefix: String },
    /// `\d ev` — a relation name as a meta-command argument.
    MetaArg { prefix: String },
    /// `FROM ev` — a relation.
    Relation { prefix: String },
    /// `WHERE ev` — a column of any table in scope, plus keywords.
    Column {
        prefix: String,
        tables: Vec<TableRef>,
    },
    /// `e.ev` — a column of the table `e` resolves to.
    Qualified {
        qualifier: String,
        prefix: String,
        tables: Vec<TableRef>,
    },
    /// Anything else: keywords, and relations as a fallback.
    Keyword { prefix: String },
}

impl Context {
    /// The partial token being completed, whatever kind of context this is.
    ///
    /// Empty when the cursor sits at a word boundary, which means "offer
    /// everything valid here" rather than "no candidates".
    ///
    /// # Arguments
    /// * `&self` — `&Context`: any variant; all of them carry a prefix.
    ///
    /// # Returns
    /// `&str` — the partial token, borrowed from the context; for
    /// [`Context::MetaCommand`] it includes the leading backslash.
    pub fn prefix(&self) -> &str {
        match self {
            Self::MetaCommand { prefix }
            | Self::MetaArg { prefix }
            | Self::Relation { prefix }
            | Self::Column { prefix, .. }
            | Self::Qualified { prefix, .. }
            | Self::Keyword { prefix } => prefix,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    Keyword,
    Table,
    View,
    Column,
    Schema,
    Function,
    Meta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub value: String,
    pub kind: CompletionKind,
    /// Shown beside the candidate when listing, e.g. a column's type.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompletionResult {
    /// Character offset where the replaced token starts.
    pub start: usize,
    /// Character offset where it ends (the cursor).
    pub end: usize,
    pub items: Vec<Completion>,
    /// Longest prefix common to every candidate, for psql's "insert as much as
    /// is unambiguous" behaviour.
    pub common_prefix: String,
}

/// Keywords offered in a general position. Deliberately short: a huge list
/// makes completion noisier, not more useful.
const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS NULL",
    "IS NOT NULL",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "WITH",
    "UNION",
    "UNION ALL",
    "INTERSECT",
    "EXCEPT",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "RETURNING",
    "CREATE TABLE",
    "CREATE INDEX",
    "ALTER TABLE",
    "DROP TABLE",
    "TRUNCATE",
    "EXPLAIN",
    "EXPLAIN ANALYZE",
    "ANALYZE",
    "VACUUM",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "ASC",
    "DESC",
    "NULLS",
    "INTERVAL",
    "NOW",
    "DATE_TRUNC",
    "EXTRACT",
    "CAST",
];

/// The backslash commands the REPL implements (see `super::meta`).
///
/// Base spellings only — the `S` and `+` modifiers are suffixes the user adds,
/// and offering every combination would bury the commands themselves.
const META_COMMANDS: &[&str] = &[
    "\\d",
    "\\dt",
    "\\dv",
    "\\dm",
    "\\di",
    "\\ds",
    "\\df",
    "\\dn",
    "\\du",
    "\\dx",
    "\\l",
    "\\list",
    "\\i",
    "\\conninfo",
    "\\encoding",
    "\\timing",
    "\\x",
    "\\?",
    "\\h",
    "\\help",
    "\\q",
    "\\quit",
];

/// Keywords after which a relation name is expected.
const RELATION_LEADS: &[&str] = &[
    "from", "join", "update", "into", "table", "truncate", "analyze", "vacuum",
    "on", // `JOIN x ON` is column-ish, but handled below by the column branch
];

/// Keywords after which a column name is expected.
const COLUMN_LEADS: &[&str] = &[
    "select",
    "where",
    "and",
    "or",
    "by",
    "having",
    "on",
    "set",
    "returning",
    "distinct",
];

/// Whether `c` can appear inside an unquoted identifier, and so belongs to the
/// token under the cursor.
///
/// `is_alphanumeric` rather than an ASCII test, because Postgres identifiers may
/// be non-ASCII. `$` and quoted identifiers are not handled: a token boundary
/// there just means completion offers less, not that it misbehaves.
///
/// # Arguments
/// * `c` — `char`: any character, including non-ASCII.
///
/// # Returns
/// `bool` — whether `c` continues the token under the cursor.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Split the line at the cursor into everything before the token being
/// completed, and the token itself.
///
/// # Arguments
/// * `line` — `&str`: the raw input line, unquoted and unparsed.
/// * `cursor` — `usize`: a character offset, clamped to the line's length.
///
/// # Returns
/// `(String, String, usize)` — the text before the token, the token itself
/// (empty at a word boundary), and the token's start offset in characters.
fn token_at(line: &str, cursor: usize) -> (String, String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let cursor = cursor.min(chars.len());

    let mut start = cursor;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..cursor].iter().collect();
    let before: String = chars[..start].iter().collect();
    (before, prefix, start)
}

/// The last significant word before the token, lowercased.
///
/// Two-word leads (`GROUP BY`, `ORDER BY`) collapse to their second word, which
/// is what the lead tables key on.
///
/// # Arguments
/// * `before` — `&str`: everything left of the token being completed.
///
/// # Returns
/// `String` — the last identifier-ish word, lowercased; empty when there is
/// none.
fn last_word(before: &str) -> String {
    before
        .split(|c: char| !is_ident_char(c))
        .rfind(|w| !w.is_empty())
        .unwrap_or("")
        .to_lowercase()
}

/// Extract the tables a statement puts in scope, with their aliases.
///
/// Scans for FROM/JOIN/UPDATE/INTO and reads the identifier that follows, plus
/// an optional alias (with or without AS). Good enough for completion; it is
/// not a SQL parser and does not try to be.
///
/// # Arguments
/// * `statement` — `&str`: raw SQL text, possibly incomplete or mid-typing.
///
/// # Returns
/// `Vec<TableRef>` — every relation found, in the order it appears; empty when
/// there is no FROM-like clause.
pub fn table_refs(statement: &str) -> Vec<TableRef> {
    // Tokenise into words, keeping dots attached so `public.events` stays whole.
    let tokens: Vec<String> = statement
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_end_matches(';').to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let mut out: Vec<TableRef> = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let word = tokens[i].to_lowercase();
        let introduces = matches!(word.as_str(), "from" | "join" | "update" | "into");
        if !introduces {
            i += 1;
            continue;
        }

        i += 1;
        if i >= tokens.len() {
            break;
        }

        // The relation itself, possibly schema-qualified.
        let raw = tokens[i].trim_matches('"');
        if raw.is_empty()
            || !raw
                .chars()
                .next()
                .is_some_and(|c| is_ident_char(c) || c == '"')
        {
            continue;
        }
        let (schema, name) = match raw.split_once('.') {
            Some((s, n)) => (
                Some(s.trim_matches('"').to_string()),
                n.trim_matches('"').to_string(),
            ),
            None => (None, raw.to_string()),
        };
        i += 1;

        // An optional alias: `events e` or `events AS e`, but not a keyword.
        let mut alias = None;
        if i < tokens.len() {
            let next = tokens[i].to_lowercase();
            if next == "as" {
                i += 1;
                if i < tokens.len() {
                    alias = Some(tokens[i].trim_matches('"').to_string());
                    i += 1;
                }
            } else if !is_reserved_after_relation(&next) {
                alias = Some(tokens[i].trim_matches('"').to_string());
                i += 1;
            }
        }

        if !name.is_empty() {
            out.push(TableRef {
                schema,
                name,
                alias,
            });
        }
    }

    out
}

/// Words that follow a relation but are not an alias.
///
/// # Arguments
/// * `word` — `&str`: a single token, expected already lowercased.
///
/// # Returns
/// `bool` — true when the word is a keyword and so must not be taken as an
/// alias.
fn is_reserved_after_relation(word: &str) -> bool {
    matches!(
        word,
        "where"
            | "join"
            | "inner"
            | "left"
            | "right"
            | "full"
            | "cross"
            | "on"
            | "group"
            | "order"
            | "having"
            | "limit"
            | "offset"
            | "union"
            | "intersect"
            | "except"
            | "set"
            | "values"
            | "returning"
            | "using"
            | "and"
            | "or"
            | "as"
    )
}

/// Decide what the cursor is completing. Pure — no database access.
///
/// # Arguments
/// * `line` — `&str`: the whole input line, SQL or backslash command.
/// * `cursor` — `usize`: a character offset; need not be at the line's end.
///
/// # Returns
/// `Context` — the classification, carrying the prefix and any tables in
/// scope; [`Context::Keyword`] is the fallback, never a failure.
pub fn analyze(line: &str, cursor: usize) -> Context {
    let (before, prefix, start) = token_at(line, cursor);
    let chars: Vec<char> = line.chars().collect();

    // A backslash command: `\ti`, or `\d ` expecting a relation.
    let trimmed_start = before.trim_start();
    if trimmed_start.starts_with('\\') {
        // Is the backslash immediately before the token (still typing the
        // command), or earlier (typing its argument)?
        let after_backslash = trimmed_start.trim_end();
        if after_backslash
            .chars()
            .filter(|c| c.is_whitespace())
            .count()
            == 0
            && before
                .trim_end()
                .ends_with(|c: char| c == '\\' || is_ident_char(c))
            && !before.ends_with(char::is_whitespace)
        {
            return Context::MetaCommand {
                prefix: format!("\\{prefix}"),
            };
        }
        return Context::MetaArg { prefix };
    }
    if prefix.is_empty() && before.trim_end().ends_with('\\') {
        return Context::MetaCommand {
            prefix: "\\".to_string(),
        };
    }

    let tables = table_refs(line);

    // A qualified reference: the character before the token is a dot, and the
    // thing before that is an identifier.
    if start > 0 && chars[start - 1] == '.' {
        let mut q_end = start - 1;
        let mut q_start = q_end;
        while q_start > 0 && is_ident_char(chars[q_start - 1]) {
            q_start -= 1;
        }
        let qualifier: String = chars[q_start..q_end].iter().collect();
        q_end = q_start; // silence unused warning path
        let _ = q_end;
        if !qualifier.is_empty() {
            return Context::Qualified {
                qualifier,
                prefix,
                tables,
            };
        }
    }

    let lead = last_word(&before);

    if RELATION_LEADS.contains(&lead.as_str()) && lead != "on" {
        return Context::Relation { prefix };
    }
    if COLUMN_LEADS.contains(&lead.as_str()) {
        return Context::Column { prefix, tables };
    }

    Context::Keyword { prefix }
}

/// Longest prefix shared by every candidate.
///
/// # Arguments
/// * `items` — `&[Completion]`: the candidates; only `value` is read, and an
///   empty slice yields no prefix.
///
/// # Returns
/// `String` — the shared start, cased as the first candidate has it; empty
/// when nothing is shared.
fn common_prefix(items: &[Completion]) -> String {
    let mut iter = items.iter().map(|i| i.value.as_str());
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut prefix = first.to_string();
    for value in iter {
        let shared: String = prefix
            .chars()
            .zip(value.chars())
            // Case-insensitive comparison, but keep the first candidate's case.
            .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
            .map(|(a, _)| a)
            .collect();
        prefix = shared;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

/// ASCII-case-insensitive prefix test, so `sel` and `SEL` both reach `SELECT`.
///
/// Compares by byte slice, which is sound only because `value` is always a
/// keyword from [`KEYWORDS`] and those are ASCII; a multi-byte `value` would
/// panic on a non-boundary split.
///
/// # Arguments
/// * `value` — `&str`: the candidate; must be ASCII, see above.
/// * `prefix` — `&str`: what the user typed, in any case.
///
/// # Returns
/// `bool` — whether `value` starts with `prefix` ignoring ASCII case.
fn starts_with_ci(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Match the case the user is typing: an uppercase prefix gets uppercase
/// keywords, anything else lowercase. psql does the same.
///
/// # Arguments
/// * `keyword` — `&str`: the canonical uppercase spelling from [`KEYWORDS`].
/// * `prefix` — `&str`: what the user typed; empty counts as uppercase.
///
/// # Returns
/// `String` — the keyword re-cased to match.
fn cased_keyword(keyword: &str, prefix: &str) -> String {
    let typed_upper = prefix.chars().any(|c| c.is_uppercase());
    if typed_upper || prefix.is_empty() {
        keyword.to_string()
    } else {
        keyword.to_lowercase()
    }
}

/// SQL keywords matching `prefix`, cased to follow what the user typed.
///
/// # Arguments
/// * `prefix` — `&str`: the partial token; empty matches every keyword.
///
/// # Returns
/// `Vec<Completion>` — matches in [`KEYWORDS`] order, each of kind
/// [`CompletionKind::Keyword`] with no detail.
fn keyword_candidates(prefix: &str) -> Vec<Completion> {
    KEYWORDS
        .iter()
        .filter(|k| starts_with_ci(k, prefix))
        .map(|k| Completion {
            value: cased_keyword(k, prefix),
            kind: CompletionKind::Keyword,
            detail: None,
        })
        .collect()
}

/// Backslash commands matching `prefix`, which includes the leading `\`.
///
/// Matched case-sensitively, unlike keywords: `\dt` and `\dT` are different
/// commands in psql.
///
/// # Arguments
/// * `prefix` — `&str`: the partial command **including** its backslash; a
///   bare `"\\"` matches every command.
///
/// # Returns
/// `Vec<Completion>` — matches of kind [`CompletionKind::Meta`], base
/// spellings only.
fn meta_candidates(prefix: &str) -> Vec<Completion> {
    META_COMMANDS
        .iter()
        .filter(|c| c.starts_with(prefix))
        .map(|c| Completion {
            value: c.to_string(),
            kind: CompletionKind::Meta,
            detail: None,
        })
        .collect()
}

/// Relations whose name starts with `prefix`, across user schemas.
///
/// # Arguments
/// * `client` — `&Client`: the REPL's own connection; the query runs on it
///   directly.
/// * `prefix` — `&str`: passed as a bind parameter, so it needs no quoting.
///
/// # Returns
/// `Result<Vec<Completion>>` — tables and views, capped at 200 rows and with
/// `public` first; `Err` when the catalog query itself fails.
async fn relation_candidates(client: &Client, prefix: &str) -> Result<Vec<Completion>> {
    let rows = client
        .query(
            "SELECT c.relname, c.relkind, n.nspname
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE c.relkind IN ('r','p','v','m')
               AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'
               AND c.relname ILIKE $1 || '%'
             ORDER BY (n.nspname = 'public') DESC, c.relname
             LIMIT 200",
            &[&prefix],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let kind = match r.get::<_, i8>("relkind") as u8 as char {
                'v' | 'm' => CompletionKind::View,
                _ => CompletionKind::Table,
            };
            let schema: String = r.get("nspname");
            Completion {
                value: r.get("relname"),
                kind,
                detail: (schema != "public").then_some(schema),
            }
        })
        .collect())
}

/// Columns of the given tables whose name starts with `prefix`.
///
/// # Arguments
/// * `client` — `&Client`: the REPL's own connection.
/// * `tables` — `&[TableRef]`: relations to look in; an empty slice yields no
///   candidates, and names that resolve to nothing are skipped silently.
/// * `prefix` — `&str`: passed as a bind parameter, so it needs no quoting.
///
/// # Returns
/// `Result<Vec<Completion>>` — columns in `attnum` order, deduplicated across
/// tables, each detailed with its type; `Err` when a catalog query fails.
async fn column_candidates(
    client: &Client,
    tables: &[TableRef],
    prefix: &str,
) -> Result<Vec<Completion>> {
    let mut out = Vec::new();
    for table in tables {
        let qualified = match &table.schema {
            Some(s) => format!("{s}.{}", table.name),
            None => table.name.clone(),
        };

        // to_regclass returns NULL rather than erroring on an unknown name,
        // which matters because the user may be mid-typing a table name.
        let sql = format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod) AS data_type
             FROM pg_attribute a
             WHERE a.attrelid = to_regclass({})
               AND a.attnum > 0 AND NOT a.attisdropped
               AND a.attname ILIKE $1 || '%'
             ORDER BY a.attnum",
            quote_literal(&qualified)
        );

        let rows = client.query(sql.as_str(), &[&prefix]).await?;
        for r in rows {
            let value: String = r.get("attname");
            // Don't offer the same column twice when several tables share it.
            if out.iter().any(|c: &Completion| c.value == value) {
                continue;
            }
            out.push(Completion {
                value,
                kind: CompletionKind::Column,
                detail: Some(r.get::<_, String>("data_type")),
            });
        }
    }
    Ok(out)
}

/// Produce completions for the cursor position.
///
/// # Arguments
/// * `client` — `&Client`: the REPL's own connection, used for the catalog
///   lookups the pure [`analyze`] half cannot do.
/// * `line` — `&str`: the whole input line.
/// * `cursor` — `usize`: a character offset into `line`.
///
/// # Returns
/// `Result<CompletionResult>` — the candidates plus the character range they
/// replace, which for a meta-command reaches back over the backslash; `Err`
/// when a catalog query fails.
pub async fn complete(client: &Client, line: &str, cursor: usize) -> Result<CompletionResult> {
    let context = analyze(line, cursor);
    let (_, _token, start) = token_at(line, cursor);

    let items = match &context {
        Context::MetaCommand { prefix } => meta_candidates(prefix),

        Context::MetaArg { prefix } => relation_candidates(client, prefix).await?,

        Context::Relation { prefix } => relation_candidates(client, prefix).await?,

        Context::Qualified {
            qualifier,
            prefix,
            tables,
        } => {
            // Resolve the qualifier to a table, by alias or by name; if it
            // matches nothing in scope, treat it as a schema or bare table name.
            let matched: Vec<TableRef> = tables
                .iter()
                .filter(|t| t.matches_qualifier(qualifier))
                .cloned()
                .collect();
            let targets = if matched.is_empty() {
                vec![TableRef {
                    schema: None,
                    name: qualifier.clone(),
                    alias: None,
                }]
            } else {
                matched
            };
            column_candidates(client, &targets, prefix).await?
        }

        Context::Column { prefix, tables } => {
            let mut items = column_candidates(client, tables, prefix).await?;
            items.extend(keyword_candidates(prefix));
            items
        }

        Context::Keyword { prefix } => {
            let mut items = keyword_candidates(prefix);
            // Relations too, so `SELECT * FROM` isn't the only way to reach one.
            if !prefix.is_empty() {
                items.extend(relation_candidates(client, prefix).await?);
            }
            items
        }
    };

    // Meta-command completion replaces the backslash as well as the word.
    let start = if matches!(context, Context::MetaCommand { .. }) {
        start.saturating_sub(1)
    } else {
        start
    };
    Ok(CompletionResult {
        common_prefix: common_prefix(&items),
        start,
        end: cursor,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Analyzes with the cursor at the end of `line`, the only position that
    /// matters for most cases — mid-line cursors get their own tests.
    ///
    /// # Arguments
    /// * `line` — `&str`: the input to analyze, cursor implied at its end.
    ///
    /// # Returns
    /// `Context` — whatever [`analyze`] reports for that position.
    fn at_end(line: &str) -> Context {
        analyze(line, line.chars().count())
    }

    // ------------------------------ contexts ------------------------------

    #[test]
    fn a_relation_is_expected_after_from() {
        assert_eq!(
            at_end("SELECT * FROM ev"),
            Context::Relation {
                prefix: "ev".into()
            }
        );
    }

    #[test]
    fn a_relation_is_expected_after_join_and_update() {
        assert!(matches!(
            at_end("SELECT * FROM a JOIN ev"),
            Context::Relation { .. }
        ));
        assert!(matches!(at_end("UPDATE ev"), Context::Relation { .. }));
        assert!(matches!(at_end("INSERT INTO ev"), Context::Relation { .. }));
    }

    #[test]
    fn a_column_is_expected_after_select_and_where() {
        assert!(matches!(at_end("SELECT ev"), Context::Column { .. }));
        assert!(matches!(
            at_end("SELECT * FROM events WHERE ev"),
            Context::Column { .. }
        ));
        assert!(matches!(
            at_end("SELECT * FROM events WHERE a = 1 AND ev"),
            Context::Column { .. }
        ));
    }

    #[test]
    fn group_and_order_by_expect_columns() {
        // `BY` is the significant word, and it leads columns.
        assert!(matches!(
            at_end("SELECT * FROM events GROUP BY ev"),
            Context::Column { .. }
        ));
        assert!(matches!(
            at_end("SELECT * FROM events ORDER BY ev"),
            Context::Column { .. }
        ));
    }

    #[test]
    fn a_qualified_reference_is_detected() {
        let ctx = at_end("SELECT * FROM events e WHERE e.ev");
        match ctx {
            Context::Qualified {
                qualifier, prefix, ..
            } => {
                assert_eq!(qualifier, "e");
                assert_eq!(prefix, "ev");
            }
            other => panic!("expected Qualified, got {other:?}"),
        }
    }

    #[test]
    fn a_qualified_reference_works_with_an_empty_prefix() {
        let ctx = at_end("SELECT * FROM events e WHERE e.");
        match ctx {
            Context::Qualified {
                qualifier, prefix, ..
            } => {
                assert_eq!(qualifier, "e");
                assert_eq!(prefix, "");
            }
            other => panic!("expected Qualified, got {other:?}"),
        }
    }

    #[test]
    fn backslash_commands_are_detected() {
        assert_eq!(
            at_end("\\ti"),
            Context::MetaCommand {
                prefix: "\\ti".into()
            }
        );
        assert_eq!(
            at_end("\\"),
            Context::MetaCommand {
                prefix: "\\".into()
            }
        );
    }

    #[test]
    fn a_backslash_argument_completes_relations() {
        assert_eq!(
            at_end("\\d ev"),
            Context::MetaArg {
                prefix: "ev".into()
            }
        );
    }

    #[test]
    fn bare_input_falls_back_to_keywords() {
        assert_eq!(
            at_end("SEL"),
            Context::Keyword {
                prefix: "SEL".into()
            }
        );
    }

    #[test]
    fn analyze_respects_a_mid_line_cursor() {
        let line = "SELECT * FROM ev WHERE x = 1";
        // Cursor right after "ev".
        let cursor = line.find(" WHERE").unwrap();
        assert!(matches!(analyze(line, cursor), Context::Relation { .. }));
    }

    // ---------------------------- table refs ------------------------------

    #[test]
    fn finds_a_single_table() {
        let refs = table_refs("SELECT * FROM events");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "events");
        assert_eq!(refs[0].alias, None);
        assert_eq!(refs[0].schema, None);
    }

    #[test]
    fn finds_an_alias_with_and_without_as() {
        let refs = table_refs("SELECT * FROM events e");
        assert_eq!(refs[0].alias.as_deref(), Some("e"));

        let refs = table_refs("SELECT * FROM events AS ev");
        assert_eq!(refs[0].alias.as_deref(), Some("ev"));
    }

    #[test]
    fn does_not_mistake_a_keyword_for_an_alias() {
        let refs = table_refs("SELECT * FROM events WHERE x = 1");
        assert_eq!(refs[0].name, "events");
        assert_eq!(refs[0].alias, None, "WHERE is not an alias");

        let refs = table_refs("SELECT * FROM events ORDER BY x");
        assert_eq!(refs[0].alias, None, "ORDER is not an alias");

        let refs = table_refs("SELECT * FROM a JOIN b ON a.x = b.x");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].alias, None, "JOIN is not an alias");
        assert_eq!(refs[1].alias, None, "ON is not an alias");
    }

    #[test]
    fn finds_multiple_tables_across_joins() {
        let refs = table_refs("SELECT * FROM events e JOIN users u ON u.user_id = e.user_id");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "events");
        assert_eq!(refs[0].alias.as_deref(), Some("e"));
        assert_eq!(refs[1].name, "users");
        assert_eq!(refs[1].alias.as_deref(), Some("u"));
    }

    #[test]
    fn handles_comma_separated_tables() {
        let refs = table_refs("SELECT * FROM events e, users u WHERE e.user_id = u.user_id");
        // The comma form only reliably yields the first; that is enough to be
        // useful and never wrong about what it does report.
        assert!(!refs.is_empty());
        assert_eq!(refs[0].name, "events");
    }

    #[test]
    fn splits_a_schema_qualified_name() {
        let refs = table_refs("SELECT * FROM analytics.daily_active_users d");
        assert_eq!(refs[0].schema.as_deref(), Some("analytics"));
        assert_eq!(refs[0].name, "daily_active_users");
        assert_eq!(refs[0].alias.as_deref(), Some("d"));
    }

    #[test]
    fn handles_quoted_identifiers() {
        let refs = table_refs(r#"SELECT * FROM "Weird Name" w"#);
        // The quoted name splits on whitespace, so only the first word survives;
        // what matters is that it does not panic or produce garbage.
        assert!(!refs.is_empty());
    }

    #[test]
    fn finds_the_target_of_update_and_insert() {
        assert_eq!(table_refs("UPDATE events SET x = 1")[0].name, "events");
        assert_eq!(
            table_refs("INSERT INTO events (a) VALUES (1)")[0].name,
            "events"
        );
    }

    #[test]
    fn returns_nothing_when_there_is_no_from_clause() {
        assert!(table_refs("SELECT 1").is_empty());
        assert!(table_refs("").is_empty());
    }

    // ------------------------- qualifier matching -------------------------

    #[test]
    fn a_qualifier_matches_an_alias() {
        let t = TableRef {
            schema: None,
            name: "events".into(),
            alias: Some("e".into()),
        };
        assert!(t.matches_qualifier("e"));
        assert!(t.matches_qualifier("E"), "case-insensitive");
        // With an alias present, the bare name no longer qualifies — same as SQL.
        assert!(!t.matches_qualifier("events"));
    }

    #[test]
    fn a_qualifier_matches_the_name_when_there_is_no_alias() {
        let t = TableRef {
            schema: None,
            name: "events".into(),
            alias: None,
        };
        assert!(t.matches_qualifier("events"));
        assert!(!t.matches_qualifier("e"));
    }

    // ---------------------------- candidates ------------------------------

    #[test]
    fn keyword_candidates_filter_by_prefix() {
        let items = keyword_candidates("SEL");
        assert!(items.iter().any(|i| i.value == "SELECT"));
        assert!(!items.iter().any(|i| i.value.starts_with("FROM")));
    }

    #[test]
    fn keyword_case_follows_what_was_typed() {
        assert_eq!(keyword_candidates("sel")[0].value, "select");
        assert_eq!(keyword_candidates("SEL")[0].value, "SELECT");
        assert_eq!(keyword_candidates("Sel")[0].value, "SELECT");
    }

    #[test]
    fn meta_candidates_filter_by_prefix() {
        let items = meta_candidates("\\d");
        let values: Vec<&str> = items.iter().map(|i| i.value.as_str()).collect();
        assert!(values.contains(&"\\d"));
        assert!(values.contains(&"\\dt"));
        assert!(values.contains(&"\\dn"));
        assert!(!values.contains(&"\\timing"));
    }

    #[test]
    fn every_offered_meta_command_is_one_the_parser_implements() {
        // Completion and the parser are two lists of the same commands, so they
        // can drift — and the failure mode is the worst kind: Tab suggests a
        // command that then answers "invalid command".
        for cmd in META_COMMANDS {
            // `\d` and `\i` change meaning with an argument; probe the form
            // that exercises the parser rather than the bare word.
            let probe = match *cmd {
                "\\i" => "\\i some_query".to_string(),
                other => other.to_string(),
            };
            match crate::repl::meta::parse(&probe) {
                Some(crate::repl::meta::MetaCommand::Unknown(c)) => {
                    panic!("completion offers {cmd}, but the parser rejects it as {c}")
                }
                None => panic!("completion offers {cmd}, but it does not parse as a meta-command"),
                _ => {}
            }
        }
    }

    // -------------------------- common prefix -----------------------------

    /// A keyword completion; the common-prefix tests only read `value`, so the
    /// kind and detail are arbitrary.
    ///
    /// # Arguments
    /// * `value` — `&str`: the candidate text, the only field that matters.
    ///
    /// # Returns
    /// `Completion` — kind [`CompletionKind::Keyword`], detail `None`.
    fn kw(value: &str) -> Completion {
        Completion {
            value: value.into(),
            kind: CompletionKind::Keyword,
            detail: None,
        }
    }

    #[test]
    fn common_prefix_of_one_item_is_the_item() {
        assert_eq!(common_prefix(&[kw("events")]), "events");
    }

    #[test]
    fn common_prefix_finds_the_shared_start() {
        assert_eq!(
            common_prefix(&[kw("events"), kw("event_properties")]),
            "event"
        );
    }

    #[test]
    fn common_prefix_is_empty_when_nothing_is_shared() {
        assert_eq!(common_prefix(&[kw("events"), kw("users")]), "");
        assert_eq!(common_prefix(&[]), "");
    }

    #[test]
    fn common_prefix_is_case_insensitive_but_keeps_the_first_casing() {
        assert_eq!(common_prefix(&[kw("SELECT"), kw("select")]), "SELECT");
    }

    // ----------------------------- tokenising -----------------------------

    #[test]
    fn token_at_finds_the_word_under_the_cursor() {
        let (before, prefix, start) = token_at("SELECT * FROM ev", 16);
        assert_eq!(prefix, "ev");
        assert_eq!(before, "SELECT * FROM ");
        assert_eq!(start, 14);
    }

    #[test]
    fn token_at_handles_an_empty_prefix() {
        let (_, prefix, start) = token_at("SELECT * FROM ", 14);
        assert_eq!(prefix, "");
        assert_eq!(start, 14);
    }

    #[test]
    fn token_at_clamps_an_out_of_range_cursor() {
        let (_, prefix, _) = token_at("abc", 999);
        assert_eq!(prefix, "abc");
    }
}
