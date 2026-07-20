//! Deciding when typed input forms a complete statement.
//!
//! psql shows a continuation prompt until the buffer ends with a `;` that is
//! genuinely a terminator — not one inside a string, an identifier, a comment,
//! or a dollar-quoted body. This module implements that judgement, which is the
//! whole basis of the `=#` vs `-#` prompt distinction.

/// What the lexer was in the middle of when the input ran out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// A complete statement (or empty input): show the normal `=#` prompt.
    None,
    /// Mid-statement: `-#`.
    Statement,
    /// Inside `'...'`.
    SingleQuote,
    /// Inside `"..."`.
    DoubleQuote,
    /// Inside `$tag$ ... $tag$`.
    DollarQuote,
    /// Inside a `/* ... */` block comment.
    BlockComment,
}

impl Pending {
    /// Whether the buffer can be sent to the server as it stands.
    ///
    /// Empty input counts as complete — it is [`Pending::None`] too, so callers
    /// that gate on this must handle the nothing-to-run case themselves.
    ///
    /// # Arguments
    /// * `&self` — `&Pending`: the state [`scan`] left off in.
    ///
    /// # Returns
    /// `bool` — true only for [`Pending::None`]; every other variant means
    /// something is still open.
    pub fn is_complete(&self) -> bool {
        matches!(self, Pending::None)
    }

    /// The prompt suffix psql shows for this state, given a base prompt.
    /// psql distinguishes these; we render the common ones.
    ///
    /// # Arguments
    /// * `&self` — `&Pending`: the state to render a marker for.
    ///
    /// # Returns
    /// `char` — the character the prompt ends with: `=`, `-`, `'`, `"`, `$`
    /// or `*`.
    pub fn prompt_marker(&self) -> char {
        match self {
            Pending::None => '=',
            Pending::Statement => '-',
            Pending::SingleQuote => '\'',
            Pending::DoubleQuote => '"',
            Pending::DollarQuote => '$',
            Pending::BlockComment => '*',
        }
    }
}

/// Scan `input` and report what remains open at the end.
///
/// # Arguments
/// * `input` — `&str`: the whole buffer typed so far, not just the last line.
///
/// # Returns
/// `Pending` — what was still open when the input ran out;
/// [`Pending::None`] for a terminated statement or for empty input.
pub fn scan(input: &str) -> Pending {
    let b: Vec<char> = input.chars().collect();
    let n = b.len();
    let mut i = 0;
    // Whether we've seen non-whitespace since the last terminating `;`.
    let mut has_content = false;

    while i < n {
        let c = b[i];

        // Line comment: skip to end of line.
        if c == '-' && i + 1 < n && b[i + 1] == '-' {
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: nests in Postgres.
        if c == '/' && i + 1 < n && b[i + 1] == '*' {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if b[i] == '/' && i + 1 < n && b[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && i + 1 < n && b[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                return Pending::BlockComment;
            }
            continue;
        }

        // Single-quoted string; '' is an escaped quote.
        if c == '\'' {
            i += 1;
            loop {
                if i >= n {
                    return Pending::SingleQuote;
                }
                if b[i] == '\'' {
                    if i + 1 < n && b[i + 1] == '\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            has_content = true;
            continue;
        }

        // Quoted identifier; "" is an escaped quote.
        if c == '"' {
            i += 1;
            loop {
                if i >= n {
                    return Pending::DoubleQuote;
                }
                if b[i] == '"' {
                    if i + 1 < n && b[i + 1] == '"' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            has_content = true;
            continue;
        }

        // Dollar quoting: $tag$ ... $tag$ (tag may be empty).
        if c == '$' {
            if let Some((tag, after)) = read_dollar_tag(&b, i) {
                let close: Vec<char> = tag.chars().collect();
                let mut j = after;
                loop {
                    if j >= n {
                        return Pending::DollarQuote;
                    }
                    if b[j] == '$' && b[j..].starts_with(close.as_slice()) {
                        j += close.len();
                        break;
                    }
                    j += 1;
                }
                i = j;
                has_content = true;
                continue;
            }
        }

        if c == ';' {
            has_content = false;
            i += 1;
            continue;
        }

        if !c.is_whitespace() {
            has_content = true;
        }
        i += 1;
    }

    if has_content {
        Pending::Statement
    } else {
        Pending::None
    }
}

/// If a `$` at `start` opens a dollar-quote, return its full tag (`$tag$`) and
/// the index just past it.
///
/// # Arguments
/// * `b` — `&[char]`: the buffer as characters, not bytes.
/// * `start` — `usize`: index of the `$`; asserted to actually be one.
///
/// # Returns
/// `Option<(String, usize)>` — the tag including both `$`s and the index just
/// past it; `None` when this `$` is something else, such as a `$1` placeholder.
fn read_dollar_tag(b: &[char], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(b[start], '$');
    let mut j = start + 1;
    let mut tag = String::from("$");
    while j < b.len() {
        let c = b[j];
        if c == '$' {
            tag.push('$');
            return Some((tag, j + 1));
        }
        // Tags are identifier-ish; anything else means this isn't a dollar quote
        // (e.g. the `$1` of a parameter placeholder).
        if c.is_alphanumeric() || c == '_' {
            tag.push(c);
            j += 1;
        } else {
            return None;
        }
    }
    None
}

/// One statement's position within a buffer, in **character** offsets.
///
/// The query editor uses these to find the statement under the cursor and to
/// highlight what it is about to run, so the boundaries have to agree exactly
/// with the ones the REPL uses — hence sharing this lexer rather than
/// reimplementing it in TypeScript.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRange {
    /// Offset of the first character of the statement.
    pub start: usize,
    /// Offset one past the last character, including the terminating `;`.
    pub end: usize,
    /// The statement text, trimmed, without its terminating `;`.
    pub text: String,
}

/// Locate every statement in `input`.
///
/// Leading whitespace and comments between statements are attributed to the
/// statement that follows them, so clicking anywhere in a buffer lands inside
/// some range as long as there is a statement after the cursor.
///
/// # Arguments
/// * `input` — `&str`: the editor buffer; may hold any number of statements.
///
/// # Returns
/// `Vec<StatementRange>` — ranges in buffer order, in character offsets;
/// empty for blank or semicolon-only input.
pub fn statement_ranges(input: &str) -> Vec<StatementRange> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut consumed = 0usize;

    for idx in 0..chars.len() {
        if chars[idx] != ';' {
            continue;
        }
        let prefix: String = chars[consumed..=idx].iter().collect();
        if !scan(&prefix).is_complete() {
            continue;
        }
        push_range(&chars, consumed, idx + 1, &mut out);
        consumed = idx + 1;
    }

    // A trailing statement with no terminator still counts — that's what the
    // editor runs when you hit ⌘↵ without typing the final semicolon.
    if consumed < chars.len() {
        push_range(&chars, consumed, chars.len(), &mut out);
    }

    out
}

/// Trim the slice `[from, to)` down to its non-blank extent and record it.
///
/// # Arguments
/// * `chars` — `&[char]`: the whole buffer, indexed in character offsets.
/// * `from` — `usize`: inclusive start of the candidate slice.
/// * `to` — `usize`: exclusive end, including any terminating `;`.
/// * `out` — `&mut Vec<StatementRange>`: accumulator the range is pushed onto.
///
/// # Returns
/// `()` — pushes one range onto `out`, or nothing at all when the slice trims
/// away to blank or to a bare `;`.
fn push_range(chars: &[char], from: usize, to: usize, out: &mut Vec<StatementRange>) {
    let mut start = from;
    while start < to && chars[start].is_whitespace() {
        start += 1;
    }
    let mut end = to;
    while end > start && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    if start >= end {
        return;
    }

    let raw: String = chars[start..end].iter().collect();
    let text = raw.trim_end_matches(';').trim().to_string();
    if text.is_empty() {
        return;
    }
    out.push(StatementRange { start, end, text });
}

/// The statement containing `cursor` (a character offset), if any.
///
/// A cursor resting just past a statement's `;` still selects that statement —
/// the common case of typing a query, ending it, and immediately running.
///
/// # Arguments
/// * `input` — `&str`: the editor buffer.
/// * `cursor` — `usize`: a character offset, which may sit past the end.
///
/// # Returns
/// `Option<StatementRange>` — the containing statement, else the next one
/// after the cursor, else the last; `None` only when the buffer holds none.
pub fn statement_at(input: &str, cursor: usize) -> Option<StatementRange> {
    let ranges = statement_ranges(input);
    ranges
        .iter()
        .find(|r| cursor >= r.start && cursor <= r.end)
        .or_else(|| ranges.iter().find(|r| r.start >= cursor))
        .or_else(|| ranges.last())
        .cloned()
}

/// Split a buffer into individual statements, dropping the terminating `;`.
/// Used to report per-statement results the way psql does.
///
/// # Arguments
/// * `input` — `&str`: the buffer to split.
///
/// # Returns
/// `Vec<String>` — trimmed statement texts in order, each without its `;`;
/// an unterminated trailing statement is kept.
pub fn split_statements(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut consumed = 0;

    for (idx, c) in chars.iter().enumerate() {
        current.push(*c);
        if *c == ';' {
            // A `;` terminates only if everything before it is complete.
            let prefix: String = chars[consumed..=idx].iter().collect();
            if scan(&prefix).is_complete() {
                let stmt = current.trim().trim_end_matches(';').trim().to_string();
                if !stmt.is_empty() {
                    out.push(stmt);
                }
                current.clear();
                consumed = idx + 1;
            }
        }
    }

    let tail = current.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_complete() {
        assert_eq!(scan(""), Pending::None);
        assert_eq!(scan("   \n  "), Pending::None);
    }

    #[test]
    fn a_terminated_statement_is_complete() {
        assert_eq!(scan("SELECT 1;"), Pending::None);
    }

    #[test]
    fn an_unterminated_statement_continues() {
        assert_eq!(scan("SELECT 1"), Pending::Statement);
        assert_eq!(
            scan("SELECT event_name, count(*) FROM events"),
            Pending::Statement
        );
    }

    /// The design's terminal shows exactly this query entered across three
    /// lines, with `-#` continuation prompts on lines 2 and 3.
    #[test]
    fn the_designs_multiline_query_shows_continuation_then_completes() {
        let l1 = "SELECT event_name, count(*) FROM events";
        assert_eq!(scan(l1), Pending::Statement);

        let l2 = format!("{l1}\n  WHERE created_at > now() - interval '24 hours'");
        assert_eq!(scan(&l2), Pending::Statement);

        let l3 = format!("{l2}\n  GROUP BY 1 ORDER BY 2 DESC LIMIT 5;");
        assert_eq!(scan(&l3), Pending::None);
    }

    #[test]
    fn a_semicolon_inside_a_string_does_not_terminate() {
        assert_eq!(scan("SELECT 'a;b'"), Pending::Statement);
        assert_eq!(scan("SELECT 'a;b';"), Pending::None);
        assert_eq!(split_statements("SELECT 'a;b';").len(), 1);
    }

    #[test]
    fn an_unclosed_string_is_reported() {
        assert_eq!(scan("SELECT 'abc"), Pending::SingleQuote);
    }

    #[test]
    fn escaped_quotes_do_not_close_the_string() {
        assert_eq!(scan("SELECT 'it''s'"), Pending::Statement);
        assert_eq!(scan("SELECT 'it''s"), Pending::SingleQuote);
    }

    #[test]
    fn quoted_identifiers_are_tracked() {
        assert_eq!(scan(r#"SELECT * FROM "weird;name""#), Pending::Statement);
        assert_eq!(scan(r#"SELECT * FROM "unclosed"#), Pending::DoubleQuote);
    }

    #[test]
    fn dollar_quoted_bodies_are_not_split() {
        let f = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN; RETURN 1; END; $$ LANGUAGE plpgsql;";
        assert_eq!(scan(f), Pending::None);
        assert_eq!(split_statements(f).len(), 1, "must stay one statement");
    }

    #[test]
    fn an_unclosed_dollar_quote_continues() {
        assert_eq!(scan("SELECT $$ abc ; def"), Pending::DollarQuote);
    }

    #[test]
    fn tagged_dollar_quotes_work() {
        assert_eq!(scan("SELECT $tag$ a;b $tag$;"), Pending::None);
        assert_eq!(scan("SELECT $tag$ a;b "), Pending::DollarQuote);
    }

    #[test]
    fn parameter_placeholders_are_not_dollar_quotes() {
        assert_eq!(scan("SELECT $1;"), Pending::None);
    }

    #[test]
    fn line_comments_are_skipped() {
        assert_eq!(scan("SELECT 1 -- a ; comment\n;"), Pending::None);
        assert_eq!(scan("-- just a comment"), Pending::None);
    }

    #[test]
    fn block_comments_are_skipped_and_nest() {
        assert_eq!(scan("SELECT /* ; */ 1;"), Pending::None);
        assert_eq!(scan("SELECT /* /* ; */ */ 1;"), Pending::None);
        assert_eq!(scan("SELECT /* unclosed"), Pending::BlockComment);
    }

    #[test]
    fn splits_multiple_statements() {
        let stmts = split_statements("SELECT 1; SELECT 2;");
        assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn keeps_a_trailing_unterminated_statement() {
        let stmts = split_statements("SELECT 1; SELECT 2");
        assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
    }

    // ---------------------- statement ranges ----------------------

    #[test]
    fn ranges_cover_each_statement() {
        let sql = "SELECT 1;\nSELECT 2;";
        let ranges = statement_ranges(sql);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].text, "SELECT 1");
        assert_eq!(ranges[1].text, "SELECT 2");
        // Offsets must index back into the original buffer.
        assert_eq!(&sql[ranges[0].start..ranges[0].end], "SELECT 1;");
        assert_eq!(&sql[ranges[1].start..ranges[1].end], "SELECT 2;");
    }

    #[test]
    fn ranges_include_an_unterminated_trailing_statement() {
        let ranges = statement_ranges("SELECT 1;\nSELECT 2");
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[1].text, "SELECT 2");
    }

    #[test]
    fn ranges_do_not_split_inside_strings_or_dollar_quotes() {
        assert_eq!(statement_ranges("SELECT 'a;b';").len(), 1);
        assert_eq!(
            statement_ranges(
                "CREATE FUNCTION f() RETURNS int AS $$ BEGIN; END; $$ LANGUAGE plpgsql;"
            )
            .len(),
            1
        );
    }

    #[test]
    fn ranges_ignore_blank_and_semicolon_only_input() {
        assert!(statement_ranges("").is_empty());
        assert!(statement_ranges("   \n\t ").is_empty());
        assert!(statement_ranges(";;;").is_empty());
    }

    #[test]
    fn ranges_skip_leading_whitespace() {
        let sql = "\n\n   SELECT 1;";
        let ranges = statement_ranges(sql);
        assert_eq!(ranges.len(), 1);
        assert!(sql[ranges[0].start..].starts_with("SELECT"));
    }

    #[test]
    fn statement_at_finds_the_one_under_the_cursor() {
        let sql = "SELECT 1;\nSELECT 2;";
        assert_eq!(statement_at(sql, 3).unwrap().text, "SELECT 1");
        assert_eq!(statement_at(sql, 13).unwrap().text, "SELECT 2");
    }

    #[test]
    fn statement_at_claims_a_cursor_resting_on_the_semicolon() {
        // Typing "SELECT 1;" then running immediately leaves the cursor at
        // offset 9, one past the `;`.
        let sql = "SELECT 1;";
        assert_eq!(statement_at(sql, 9).unwrap().text, "SELECT 1");
    }

    #[test]
    fn statement_at_between_statements_picks_the_next_one() {
        let sql = "SELECT 1;\n\n\nSELECT 2;";
        // Cursor in the blank run between the two.
        assert_eq!(statement_at(sql, 11).unwrap().text, "SELECT 2");
    }

    #[test]
    fn statement_at_past_the_end_picks_the_last() {
        let sql = "SELECT 1;\n\n";
        assert_eq!(statement_at(sql, sql.len()).unwrap().text, "SELECT 1");
    }

    #[test]
    fn statement_at_returns_none_for_an_empty_buffer() {
        assert!(statement_at("", 0).is_none());
        assert!(statement_at("   ", 2).is_none());
    }

    #[test]
    fn prompt_markers_match_psql() {
        assert_eq!(Pending::None.prompt_marker(), '=');
        assert_eq!(Pending::Statement.prompt_marker(), '-');
        assert_eq!(Pending::SingleQuote.prompt_marker(), '\'');
    }
}
