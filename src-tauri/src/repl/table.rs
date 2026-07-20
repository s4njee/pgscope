//! psql-compatible "aligned" output formatter.
//!
//! This module is deliberately self-contained: it depends only on `unicode-width`
//! and has no knowledge of the rest of the crate. Everything it renders is driven
//! by already-stringified values, so it can be golden-tested without a database.
//!
//! All behaviour here was derived from, and is golden-tested against, the stdout of
//! a real `psql` 18.3 client talking to PostgreSQL 18.3 (`\pset border 1`, the
//! default). See the `tests` module at the bottom for the exact capture procedure.
//!
//! Notable places where real psql differs from a naive reading of the spec:
//!
//! * **Header centering** puts the *extra* space on the right:
//!   `left = pad / 2`, `right = pad - left`.
//! * **The last column is not right-padded.** psql omits both the fill and the
//!   closing space for the final column of a data row (but *not* of the header
//!   line, which keeps its full trailing padding).
//! * **Embedded newlines are not rendered literally.** psql wraps the value onto
//!   additional physical lines within the same column and marks every non-final
//!   line with a `+` in the position of the cell's closing space. The plan
//!   (§5.7) assumed literal rendering; real psql does the `+` wrap, and that is
//!   what is implemented here.
//! * **Expanded mode prints no `(N rows)` footer** unless the result is empty,
//!   in which case it prints `(0 rows)` and nothing else.
//! * **Zero-column result sets** render as `--` followed by the footer, with no
//!   header line and no per-row lines at all.

use unicode_width::UnicodeWidthStr;

/// A result set to render, with all values already converted to display strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    /// One entry per row; `None` = SQL NULL. Rows shorter than `columns` are
    /// treated as if the missing trailing cells were NULL.
    pub rows: Vec<Vec<Option<String>>>,
}

/// Formatting knobs mirroring the `\pset` options pgscope supports.
///
/// `Default` is psql's default: aligned (not expanded) output with NULL shown
/// as the empty string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatOptions {
    /// `\x` — expanded record display.
    pub expanded: bool,
    /// How a SQL NULL is rendered. psql's default is the empty string.
    pub null_display: String,
}

/// Render like psql's default "aligned" output (or expanded output when
/// `opts.expanded` is set). The returned string always ends in a newline.
///
/// psql additionally emits a blank line between consecutive result sets; that
/// separator is the REPL's responsibility, not the formatter's.
pub fn format_aligned(rs: &ResultSet, opts: &FormatOptions) -> String {
    if opts.expanded {
        format_expanded(rs, opts)
    } else {
        format_normal(rs, opts)
    }
}

/// Render a command tag result, e.g. `INSERT 0 1` / `SET` / `CREATE TABLE`.
pub fn format_command_tag(tag: &str) -> String {
    let mut s = String::with_capacity(tag.len() + 1);
    s.push_str(tag);
    s.push('\n');
    s
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// `n` spaces of padding. Callers compute `n` by subtracting display widths, so
/// it is already saturated at zero.
fn spaces(n: usize) -> String {
    " ".repeat(n)
}

/// The display text for one cell, substituting `null_display` for NULL (and for
/// cells missing from a short row).
fn cell_text<'a>(row: &'a [Option<String>], j: usize, opts: &'a FormatOptions) -> &'a str {
    match row.get(j) {
        Some(Some(v)) => v.as_str(),
        _ => opts.null_display.as_str(),
    }
}

/// A cell's physical lines. Always at least one element (possibly empty).
fn cell_lines<'a>(row: &'a [Option<String>], j: usize, opts: &'a FormatOptions) -> Vec<&'a str> {
    cell_text(row, j, opts).split('\n').collect()
}

/// `^-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$`, hand-rolled so the crate needs no
/// regex dependency.
fn is_numeric_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0usize;

    if i < b.len() && b[i] == b'-' {
        i += 1;
    }

    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == int_start {
        return false;
    }

    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return false;
        }
    }

    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }

    i == b.len()
}

/// A column is numeric (and therefore right-aligned) when it has at least one
/// non-NULL value and every non-NULL value is a numeric literal. An all-NULL or
/// empty column is treated as text, matching psql's left alignment for the
/// common `text` case.
fn numeric_columns(rs: &ResultSet) -> Vec<bool> {
    (0..rs.columns.len())
        .map(|j| {
            let mut saw_value = false;
            for row in &rs.rows {
                if let Some(Some(v)) = row.get(j) {
                    saw_value = true;
                    if !is_numeric_literal(v) {
                        return false;
                    }
                }
            }
            saw_value
        })
        .collect()
}

/// psql's `(N rows)` trailer, singular for exactly one.
fn footer(n: usize) -> String {
    if n == 1 {
        "(1 row)\n".to_string()
    } else {
        format!("({} rows)\n", n)
    }
}

/// The default bordered table: centered headers, a `-+-` rule, then one block of
/// physical lines per row.
///
/// Widths are measured in terminal columns via `unicode-width`, not bytes or
/// chars, so CJK and combining marks align. Numeric columns — those whose every
/// non-NULL value parses as a number — are right-aligned, which means a single
/// non-numeric value flips the whole column back to left.
///
/// A result with no rows still prints the header and the rule; only the footer
/// distinguishes it.
fn format_normal(rs: &ResultSet, opts: &FormatOptions) -> String {
    let ncols = rs.columns.len();
    let mut out = String::new();

    // psql renders a zero-column result as a bare `--` rule plus the footer.
    if ncols == 0 {
        out.push_str("--\n");
        out.push_str(&footer(rs.rows.len()));
        return out;
    }

    // Column widths: max display width across the header and every physical
    // line of every value.
    let mut widths: Vec<usize> = rs.columns.iter().map(|c| width(c)).collect();
    for row in &rs.rows {
        for (j, w) in widths.iter_mut().enumerate() {
            for line in cell_lines(row, j, opts) {
                *w = (*w).max(width(line));
            }
        }
    }

    let numeric = numeric_columns(rs);

    // Header — names centered, extra padding space to the right. Unlike data
    // rows the header keeps the trailing padding of its final column.
    for (j, name) in rs.columns.iter().enumerate() {
        if j > 0 {
            out.push('|');
        }
        let pad = widths[j].saturating_sub(width(name));
        let left = pad / 2;
        out.push(' ');
        out.push_str(&spaces(left));
        out.push_str(name);
        out.push_str(&spaces(pad - left));
        out.push(' ');
    }
    out.push('\n');

    // Separator.
    for (j, w) in widths.iter().enumerate() {
        if j > 0 {
            out.push('+');
        }
        out.push_str(&"-".repeat(w + 2));
    }
    out.push('\n');

    // Data rows.
    for row in &rs.rows {
        let cells: Vec<Vec<&str>> = (0..ncols).map(|j| cell_lines(row, j, opts)).collect();
        let nlines = cells.iter().map(|c| c.len()).max().unwrap_or(1).max(1);

        for i in 0..nlines {
            for j in 0..ncols {
                if j > 0 {
                    out.push('|');
                }
                let content = cells[j].get(i).copied().unwrap_or("");
                let has_more = i + 1 < cells[j].len();
                let last = j + 1 == ncols;
                let pad = widths[j].saturating_sub(width(content));

                // Non-final lines of a wrapped cell are marked with `+` where the
                // closing space would go. The final column has no closing space.
                let trail = if has_more {
                    Some('+')
                } else if !last {
                    Some(' ')
                } else {
                    None
                };

                out.push(' ');
                if numeric[j] {
                    out.push_str(&spaces(pad));
                    out.push_str(content);
                } else {
                    out.push_str(content);
                    if trail.is_some() {
                        out.push_str(&spaces(pad));
                    }
                }
                if let Some(c) = trail {
                    out.push(c);
                }
            }
            out.push('\n');
        }
    }

    out.push_str(&footer(rs.rows.len()));
    out
}

/// `\x` output: one `-[ RECORD n ]-` block per row, column names down the left.
///
/// The value field is sized across the entire result set rather than per record,
/// so every record rule comes out the same length — psql's behaviour, and the
/// reason a single wide value widens every block.
fn format_expanded(rs: &ResultSet, opts: &FormatOptions) -> String {
    // Expanded mode prints a footer only for an empty result set.
    if rs.rows.is_empty() {
        return "(0 rows)\n".to_string();
    }

    let ncols = rs.columns.len();
    let name_w = rs.columns.iter().map(|c| width(c)).max().unwrap_or(0);

    // Value field width is computed across the *whole* result set, not per
    // record, so every `-[ RECORD n ]` rule has the same length.
    let mut value_w = 0usize;
    for row in &rs.rows {
        for j in 0..ncols {
            for line in cell_lines(row, j, opts) {
                value_w = value_w.max(width(line));
            }
        }
    }

    let mut out = String::new();
    for (idx, row) in rs.rows.iter().enumerate() {
        // Rule: dashes as wide as `name | value`, with a `+` where the `|` of the
        // data lines sits, then the `-[ RECORD n ]` label stamped over the front.
        // The label wins where the two overlap.
        let label = format!("-[ RECORD {} ]", idx + 1);
        let label_len = label.chars().count();
        let total = (name_w + 3 + value_w).max(label_len);
        let mut rule: Vec<char> = vec!['-'; total];
        if name_w + 1 < total {
            rule[name_w + 1] = '+';
        }
        for (k, c) in label.chars().enumerate() {
            rule[k] = c;
        }
        out.extend(rule);
        out.push('\n');

        for j in 0..ncols {
            let lines = cell_lines(row, j, opts);
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    let name = &rs.columns[j];
                    out.push_str(name);
                    out.push_str(&spaces(name_w.saturating_sub(width(name))));
                } else {
                    out.push_str(&spaces(name_w));
                }
                out.push_str(" | ");
                out.push_str(line);
                if i + 1 < lines.len() {
                    out.push('+');
                }
                out.push('\n');
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    // ── How the goldens were produced ──────────────────────────────────────
    //
    // Every `const` below is a *machine-generated* verbatim copy of the stdout
    // of real psql 18.3 (Homebrew client) talking to PostgreSQL 18.3, captured
    // with default `\pset` settings (format aligned, border 1, null '') via:
    //
    //     psql -h <host> -p <port> -U <user> -d <db> -q -f probe.sql
    //
    // where probe.sql interleaves `\echo ===CASE <name>` markers with the SQL
    // quoted in each constant's doc comment. A script then split the capture on
    // those markers and emitted the `concat!` literals below, so no golden was
    // hand-transcribed. The only edit applied is dropping the single blank line
    // psql emits *between* result sets: that is inter-result spacing, not part
    // of the table, and it is the REPL's job to add it.
    //
    // The DESIGN golden also matches the `<pre>` block of the psql terminal pane
    // in `design/Postgres Explorer.dc.html` (~line 258) and `design/README.md`,
    // modulo the two trailing spaces on the header line, which the design HTML
    // drops but real psql emits. `design_matches_mock` asserts that explicitly.

    // -- aligned mode -------------------------------------------------------

    /// SELECT * FROM (VALUES ('page_view',1842311),('session_start',409112),
    ///                       ('click',322480),('signup',12055),('purchase',4310))
    ///        AS t(event_name,count);
    const DESIGN: &str = concat!(
        "  event_name   |  count  \n",
        "---------------+---------\n",
        " page_view     | 1842311\n",
        " session_start |  409112\n",
        " click         |  322480\n",
        " signup        |   12055\n",
        " purchase      |    4310\n",
        "(5 rows)\n",
    );
    /// SELECT * FROM (VALUES ('x'::text,1)) AS t(event_name,count) WHERE false;
    const EMPTY: &str = concat!(
        " event_name | count \n",
        "------------+-------\n",
        "(0 rows)\n",
    );
    /// SELECT * FROM (VALUES ('alpha'),('bee'),('c')) AS t(name);
    const SINGLE_COL: &str = concat!(
        " name  \n",
        "-------\n",
        " alpha\n",
        " bee\n",
        " c\n",
        "(3 rows)\n",
    );
    /// SELECT * FROM (VALUES ('a'::text,1),(NULL,2),('ccc',NULL)) AS t(name,n);
    const NULLS: &str = concat!(
        " name | n \n",
        "------+---\n",
        " a    | 1\n",
        "      | 2\n",
        " ccc  |  \n",
        "(3 rows)\n",
    );
    /// \pset null '[null]'
    /// SELECT * FROM (VALUES ('a'::text),(NULL)) AS t(name);
    const CUSTOM_NULL: &str = concat!(
        "  name  \n",
        "--------\n",
        " a\n",
        " [null]\n",
        "(2 rows)\n",
    );
    /// SELECT * FROM (VALUES ('日本語',1),('ab',22),('한국어테스트',333)) AS t(label,n);
    const UNICODE: &str = concat!(
        "    label     |  n  \n",
        "--------------+-----\n",
        " 日本語       |   1\n",
        " ab           |  22\n",
        " 한국어테스트 | 333\n",
        "(3 rows)\n",
    );
    /// SELECT * FROM (VALUES (-5),(1000),(-12345)) AS t(delta);
    const NEGATIVES: &str = concat!(
        " delta  \n",
        "--------\n",
        "     -5\n",
        "   1000\n",
        " -12345\n",
        "(3 rows)\n",
    );
    /// SELECT * FROM (VALUES ('a'::text,1.5),('b',-2.25),('c',30.0)) AS t(k,v);
    const FLOATS: &str = concat!(
        " k |   v   \n",
        "---+-------\n",
        " a |   1.5\n",
        " b | -2.25\n",
        " c |  30.0\n",
        "(3 rows)\n",
    );
    /// SELECT * FROM (VALUES (E'line1\nline2','after')) AS t(txt,other);
    const NEWLINE_MID: &str = concat!(
        "  txt  | other \n",
        "-------+-------\n",
        " line1+| after\n",
        " line2 | \n",
        "(1 row)\n",
    );
    /// SELECT * FROM (VALUES ('a',E'x\ny'),('b','wwwwww')) AS t(p,qqqq);
    const NEWLINE_LAST: &str = concat!(
        " p |  qqqq  \n",
        "---+--------\n",
        " a | x     +\n",
        "   | y\n",
        " b | wwwwww\n",
        "(2 rows)\n",
    );
    /// SELECT * FROM (VALUES (E'1\n22'::text,'z')) AS t(n,o);
    const NEWLINE_NUMERIC: &str = concat!(
        " n  | o \n",
        "----+---\n",
        " 1 +| z\n",
        " 22 | \n",
        "(1 row)\n",
    );
    /// SELECT * FROM (VALUES (1),(2)) AS t(a_very_long_column_name_here);
    const WIDEHDR: &str = concat!(
        " a_very_long_column_name_here \n",
        "------------------------------\n",
        "                            1\n",
        "                            2\n",
        "(2 rows)\n",
    );
    /// SELECT * FROM (VALUES ('abcd')) AS t(ab);
    const EVENPAD: &str = concat!(
        "  ab  \n",
        "------\n",
        " abcd\n",
        "(1 row)\n",
    );
    /// SELECT * FROM (VALUES ('abcde')) AS t(ab);
    const ODDPAD: &str = concat!(
        "  ab   \n",
        "-------\n",
        " abcde\n",
        "(1 row)\n",
    );
    /// SELECT * FROM (VALUES (NULL::text,'xyz'),(NULL,'q')) AS t(a,b);
    const ALL_NULL: &str = concat!(
        " a |  b  \n",
        "---+-----\n",
        "   | xyz\n",
        "   | q\n",
        "(2 rows)\n",
    );
    /// SELECT;
    const ZERO_COL: &str = concat!(
        "--\n",
        "(1 row)\n",
    );

    // -- expanded mode (\x) -------------------------------------------------

    /// SELECT * FROM (VALUES ('page_view',1842311),('session_start',409112))
    ///        AS t(event_name,count);
    const EXPANDED: &str = concat!(
        "-[ RECORD 1 ]-------------\n",
        "event_name | page_view\n",
        "count      | 1842311\n",
        "-[ RECORD 2 ]-------------\n",
        "event_name | session_start\n",
        "count      | 409112\n",
    );
    /// SELECT * FROM (VALUES ('short','a_much_longer_value_here'))
    ///        AS t(a_long_column_name,b);
    const EXPANDED_LONG: &str = concat!(
        "-[ RECORD 1 ]------+-------------------------\n",
        "a_long_column_name | short\n",
        "b                  | a_much_longer_value_here\n",
    );
    /// SELECT * FROM (VALUES ('a'::text,NULL::int),(NULL,2)) AS t(name,n);
    const EXPANDED_NULLS: &str = concat!(
        "-[ RECORD 1 ]\n",
        "name | a\n",
        "n    | \n",
        "-[ RECORD 2 ]\n",
        "name | \n",
        "n    | 2\n",
    );
    /// SELECT * FROM (VALUES ('日本語','x')) AS t(label,other);
    const EXPANDED_UNICODE: &str = concat!(
        "-[ RECORD 1 ]-\n",
        "label | 日本語\n",
        "other | x\n",
    );
    /// SELECT * FROM (VALUES ('x'::text,1)) AS t(event_name,count) WHERE false;
    const EXPANDED_EMPTY: &str = "(0 rows)\n";
    /// SELECT * FROM (VALUES ('a',E'x\ny')) AS t(p,q);
    const EXPANDED_NEWLINE: &str = concat!(
        "-[ RECORD 1 ]\n",
        "p | a\n",
        "q | x+\n",
        "  | y\n",
    );

    // -- helpers ------------------------------------------------------------

    /// Build a `ResultSet` from `Option` cells (`None` = SQL NULL).
    fn rs(columns: &[&str], rows: &[&[Option<&str>]]) -> ResultSet {
        ResultSet {
            columns: columns.iter().map(|s| s.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| c.map(|s| s.to_string())).collect())
                .collect(),
        }
    }

    /// Build a `ResultSet` where every cell is non-NULL.
    fn vals(columns: &[&str], rows: &[&[&str]]) -> ResultSet {
        ResultSet {
            columns: columns.iter().map(|s| s.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| Some(c.to_string())).collect())
                .collect(),
        }
    }

    /// Default aligned output — psql's `\x off`.
    fn plain() -> FormatOptions {
        FormatOptions::default()
    }

    /// Default output with `\x on`, leaving every other option at its default.
    fn expanded() -> FormatOptions {
        FormatOptions {
            expanded: true,
            ..FormatOptions::default()
        }
    }

    /// The exact result the design mock shows, so the rendered block can be
    /// compared against it character for character.
    fn design_rs() -> ResultSet {
        vals(
            &["event_name", "count"],
            &[
                &["page_view", "1842311"],
                &["session_start", "409112"],
                &["click", "322480"],
                &["signup", "12055"],
                &["purchase", "4310"],
            ],
        )
    }

    // -- (1) the design's exact query result --------------------------------

    #[test]
    fn design_query_result() {
        assert_eq!(format_aligned(&design_rs(), &plain()), DESIGN);
    }

    /// The mock in `design/Postgres Explorer.dc.html` shows the same block with
    /// the header line's trailing whitespace stripped. Compare modulo that.
    #[test]
    fn design_matches_mock() {
        const MOCK: &str = concat!(
            "  event_name   |  count\n",
            "---------------+---------\n",
            " page_view     | 1842311\n",
            " session_start |  409112\n",
            " click         |  322480\n",
            " signup        |   12055\n",
            " purchase      |    4310\n",
            "(5 rows)\n",
        );
        let rstrip = |s: &str| {
            s.lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(
            rstrip(&format_aligned(&design_rs(), &plain())),
            rstrip(MOCK)
        );
    }

    // -- (2) empty result ---------------------------------------------------

    #[test]
    fn empty_result() {
        let r = rs(&["event_name", "count"], &[]);
        assert_eq!(format_aligned(&r, &plain()), EMPTY);
    }

    // -- (3) single column --------------------------------------------------

    #[test]
    fn single_column() {
        let r = vals(&["name"], &[&["alpha"], &["bee"], &["c"]]);
        assert_eq!(format_aligned(&r, &plain()), SINGLE_COL);
    }

    // -- (4) NULLs ----------------------------------------------------------

    #[test]
    fn nulls() {
        let r = rs(
            &["name", "n"],
            &[
                &[Some("a"), Some("1")],
                &[None, Some("2")],
                &[Some("ccc"), None],
            ],
        );
        assert_eq!(format_aligned(&r, &plain()), NULLS);
    }

    #[test]
    fn custom_null_display() {
        let r = rs(&["name"], &[&[Some("a")], &[None]]);
        let opts = FormatOptions {
            expanded: false,
            null_display: "[null]".to_string(),
        };
        assert_eq!(format_aligned(&r, &opts), CUSTOM_NULL);
    }

    /// An all-NULL column has no non-NULL value to prove it numeric, so it is
    /// left-aligned (psql decides by type; pgscope decides by content).
    #[test]
    fn all_null_column_is_text() {
        let r = rs(&["a", "b"], &[&[None, Some("xyz")], &[None, Some("q")]]);
        assert_eq!(format_aligned(&r, &plain()), ALL_NULL);
    }

    // -- (5) unicode / CJK width -------------------------------------------

    #[test]
    fn unicode_widths() {
        let r = vals(
            &["label", "n"],
            &[
                &["日本語", "1"],
                &["ab", "22"],
                &["한국어테스트", "333"],
            ],
        );
        assert_eq!(format_aligned(&r, &plain()), UNICODE);
    }

    // -- (6) negative numbers ----------------------------------------------

    #[test]
    fn negative_numbers() {
        let r = vals(&["delta"], &[&["-5"], &["1000"], &["-12345"]]);
        assert_eq!(format_aligned(&r, &plain()), NEGATIVES);
    }

    #[test]
    fn decimals_are_numeric() {
        let r = vals(&["k", "v"], &[&["a", "1.5"], &["b", "-2.25"], &["c", "30.0"]]);
        assert_eq!(format_aligned(&r, &plain()), FLOATS);
    }

    // -- (8) embedded newlines ---------------------------------------------

    #[test]
    fn newline_in_middle_column() {
        let r = vals(&["txt", "other"], &[&["line1\nline2", "after"]]);
        assert_eq!(format_aligned(&r, &plain()), NEWLINE_MID);
    }

    /// The final column *is* padded on its continuation lines (so the `+` lands
    /// in the right place) but not on its final line.
    #[test]
    fn newline_in_last_column() {
        let r = vals(&["p", "qqqq"], &[&["a", "x\ny"], &["b", "wwwwww"]]);
        assert_eq!(format_aligned(&r, &plain()), NEWLINE_LAST);
    }

    /// A wrapped value never matches the numeric pattern, so its column stays
    /// left-aligned.
    #[test]
    fn newline_defeats_numeric_detection() {
        let r = vals(&["n", "o"], &[&["1\n22", "z"]]);
        assert_eq!(format_aligned(&r, &plain()), NEWLINE_NUMERIC);
    }

    // -- header padding / footer plurality ---------------------------------

    #[test]
    fn header_wider_than_values() {
        let r = vals(&["a_very_long_column_name_here"], &[&["1"], &["2"]]);
        assert_eq!(format_aligned(&r, &plain()), WIDEHDR);
    }

    #[test]
    fn header_centering_even_padding() {
        let r = vals(&["ab"], &[&["abcd"]]);
        assert_eq!(format_aligned(&r, &plain()), EVENPAD);
    }

    /// Odd padding puts the extra space on the right.
    #[test]
    fn header_centering_odd_padding() {
        let r = vals(&["ab"], &[&["abcde"]]);
        assert_eq!(format_aligned(&r, &plain()), ODDPAD);
    }

    #[test]
    fn footer_plurality() {
        assert!(format_aligned(&vals(&["a"], &[&["1"]]), &plain()).ends_with("(1 row)\n"));
        assert!(format_aligned(&vals(&["a"], &[&["1"], &["2"]]), &plain()).ends_with("(2 rows)\n"));
        assert!(format_aligned(&rs(&["a"], &[]), &plain()).ends_with("(0 rows)\n"));
    }

    // -- zero columns -------------------------------------------------------

    #[test]
    fn zero_columns() {
        let one = ResultSet {
            columns: vec![],
            rows: vec![vec![]],
        };
        assert_eq!(format_aligned(&one, &plain()), ZERO_COL);

        let none = ResultSet {
            columns: vec![],
            rows: vec![],
        };
        assert_eq!(format_aligned(&none, &plain()), "--\n(0 rows)\n");
    }

    // -- (7) expanded mode --------------------------------------------------

    #[test]
    fn expanded_basic() {
        let r = vals(
            &["event_name", "count"],
            &[&["page_view", "1842311"], &["session_start", "409112"]],
        );
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED);
    }

    /// When the name field is wider than the `-[ RECORD n ]` label, psql draws
    /// the `+` divider in the rule.
    #[test]
    fn expanded_rule_has_divider_plus() {
        let r = vals(
            &["a_long_column_name", "b"],
            &[&["short", "a_much_longer_value_here"]],
        );
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED_LONG);
    }

    #[test]
    fn expanded_nulls() {
        let r = rs(&["name", "n"], &[&[Some("a"), None], &[None, Some("2")]]);
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED_NULLS);
    }

    #[test]
    fn expanded_unicode() {
        let r = vals(&["label", "other"], &[&["日本語", "x"]]);
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED_UNICODE);
    }

    /// Expanded mode prints a footer only when the result is empty.
    #[test]
    fn expanded_empty() {
        let r = rs(&["event_name", "count"], &[]);
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED_EMPTY);
    }

    #[test]
    fn expanded_newline() {
        let r = vals(&["p", "q"], &[&["a", "x\ny"]]);
        assert_eq!(format_aligned(&r, &expanded()), EXPANDED_NEWLINE);
    }

    // -- command tags -------------------------------------------------------

    /// Captured from psql: `SET search_path = public;` -> `SET`,
    /// `CREATE TEMP TABLE zz(a int);` -> `CREATE TABLE`,
    /// `INSERT INTO zz VALUES (1);` -> `INSERT 0 1`.
    #[test]
    fn command_tags() {
        assert_eq!(format_command_tag("SET"), "SET\n");
        assert_eq!(format_command_tag("CREATE TABLE"), "CREATE TABLE\n");
        assert_eq!(format_command_tag("INSERT 0 1"), "INSERT 0 1\n");
    }

    // -- misc ---------------------------------------------------------------

    /// Truncation is an upstream concern; the formatter never shortens a value.
    #[test]
    fn long_values_are_not_truncated() {
        let long = "x".repeat(500);
        let r = vals(&["v"], &[&[long.as_str()]]);
        let out = format_aligned(&r, &plain());
        assert!(out.contains(&long));
        assert_eq!(out.lines().nth(1).unwrap().len(), 502);
        assert!(out.ends_with("(1 row)\n"));
    }

    #[test]
    fn numeric_literal_recognition() {
        for s in [
            "0", "-0", "42", "-42", "1.5", "-2.25", "30.0", "1e10", "1E10", "1e+10", "1e-10",
            "-1.5e-10",
        ] {
            assert!(is_numeric_literal(s), "expected numeric: {s:?}");
        }
        for s in [
            "", "-", ".", "1.", ".5", "1e", "1e+", "abc", "1 ", " 1", "1,000", "0x1f", "1.2.3",
            "+1", "NaN", "Infinity", "1\n2",
        ] {
            assert!(!is_numeric_literal(s), "expected non-numeric: {s:?}");
        }
    }

    /// Rows shorter than `columns` are treated as trailing NULLs, not a panic.
    #[test]
    fn short_rows_are_tolerated() {
        let r = ResultSet {
            columns: vec!["a".into(), "b".into()],
            rows: vec![vec![Some("1".into())]],
        };
        assert_eq!(format_aligned(&r, &plain()), " a | b \n---+---\n 1 | \n(1 row)\n");
    }
}
