//! psql's name patterns for the `\d` family.
//!
//! `\dt user*`, `\df public.*`, `\di "MixedCase"` — psql turns each of these
//! into a pair of anchored POSIX regexes (one for the schema, one for the name)
//! and compares them with `~`. Reimplemented here rather than approximated with
//! `LIKE`, because the two disagree in ways a user would notice: psql's `*` is
//! not SQL's `%`, an unquoted pattern folds to lower case, and a quoted one
//! does not.
//!
//! See psql's `processSQLNamePattern`; this is that behaviour minus the parts
//! that only matter for its `--single-line` and encoding paths.

use crate::db::grid::quote_literal;

/// A pattern split into its schema and object halves.
///
/// `schema` is `None` when the user wrote no dot — which in psql means "search
/// path only", not "every schema", and the SQL builder must honour that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePattern {
    pub schema: Option<String>,
    pub name: Option<String>,
}

/// Characters that are regex metacharacters and must be escaped when they
/// appear literally in a pattern. `*` and `?` are deliberately absent: outside
/// quotes they are the wildcards, and inside quotes they are added by the
/// quoting branch instead.
const REGEX_SPECIALS: &[char] = &[
    '|', '*', '+', '?', '(', ')', '[', ']', '{', '}', '.', '^', '$', '\\',
];

/// Append one character as a regex literal, escaping it if it would otherwise
/// be a metacharacter.
///
/// Callers reach here only for text that is meant literally — wildcards are
/// translated by the parser before it gets this far.
fn push_escaped(out: &mut String, c: char) {
    if REGEX_SPECIALS.contains(&c) {
        out.push('\\');
    }
    out.push(c);
}

/// Parse a psql name pattern into anchored regexes.
///
/// Returns `None` halves for parts the user left empty, so `\dt` (no pattern at
/// all) and `\dt public.*` are distinguishable from `\dt *`.
pub fn parse(pattern: &str) -> NamePattern {
    let mut schema: Option<String> = None;
    let mut current = String::new();
    let mut in_quotes = false;

    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c == '"' {
                // `""` inside quotes is one literal quote, as in an identifier.
                if chars.get(i + 1) == Some(&'"') {
                    push_escaped(&mut current, '"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
            } else {
                // Quoted text is literal, and keeps its case.
                push_escaped(&mut current, c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                // Only the first unquoted dot splits; a second one is part of
                // the name, matching psql's two-part-name limit.
                '.' if schema.is_none() => {
                    schema = Some(std::mem::take(&mut current));
                }
                '*' => current.push_str(".*"),
                '?' => current.push('.'),
                // An unquoted identifier folds to lower case in Postgres, so
                // the pattern has to fold too or `\dt Users` finds nothing.
                _ => {
                    for lower in c.to_lowercase() {
                        push_escaped(&mut current, lower);
                    }
                }
            }
        }
        i += 1;
    }

    let anchor = |s: String| {
        if s.is_empty() {
            None
        } else {
            Some(anchored(&s))
        }
    };

    NamePattern {
        schema: schema.and_then(anchor),
        name: anchor(current),
    }
}

/// Wrap a regex body so it must match the whole identifier. Without this,
/// `\dt user` would also list `users` — psql anchors, so we do.
fn anchored(body: &str) -> String {
    format!("^({body})$")
}

impl NamePattern {
    /// A `WHERE` fragment matching this pattern.
    ///
    /// `name_col` and `schema_col` are already-qualified column expressions.
    /// `visible` is a predicate that limits results to the session's
    /// `search_path` — applied only when the user gave no schema part, which is
    /// what makes `\dt` list "your" tables rather than every schema's.
    pub fn clauses(&self, schema_col: &str, name_col: &str, visible: Option<&str>) -> Vec<String> {
        let mut out = Vec::new();
        match &self.schema {
            Some(s) => out.push(format!("{schema_col} ~ {}", quote_literal(s))),
            None => {
                if let Some(v) = visible {
                    out.push(v.to_string());
                }
            }
        }
        if let Some(n) = &self.name {
            out.push(format!("{name_col} ~ {}", quote_literal(n)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name regex a pattern compiles to; the schema half is tested separately.
    fn name_of(p: &str) -> Option<String> {
        parse(p).name
    }

    #[test]
    fn a_plain_name_is_anchored() {
        // Without anchoring, `\dt user` would also match `users` — the single
        // most likely way a hand-rolled version of this goes wrong.
        assert_eq!(name_of("users"), Some("^(users)$".into()));
    }

    #[test]
    fn star_becomes_dot_star_and_question_becomes_dot() {
        assert_eq!(name_of("user*"), Some("^(user.*)$".into()));
        assert_eq!(name_of("user?"), Some("^(user.)$".into()));
    }

    #[test]
    fn unquoted_patterns_fold_to_lower_case() {
        // Unquoted identifiers fold in Postgres, so `\dt Users` must find the
        // table actually named `users`.
        assert_eq!(name_of("Users"), Some("^(users)$".into()));
        assert_eq!(name_of("USER*"), Some("^(user.*)$".into()));
    }

    #[test]
    fn quoted_patterns_keep_their_case() {
        assert_eq!(name_of("\"Users\""), Some("^(Users)$".into()));
    }

    #[test]
    fn a_star_inside_quotes_is_literal() {
        // Quoted text is a literal identifier fragment, so the wildcard has to
        // be escaped rather than expanded.
        assert_eq!(name_of("\"a*b\""), Some("^(a\\*b)$".into()));
    }

    #[test]
    fn doubled_quotes_are_one_literal_quote() {
        // `"` is not a regex metacharacter, so it passes through unescaped —
        // the doubling is identifier syntax, resolved here, not in the regex.
        assert_eq!(name_of("\"a\"\"b\""), Some("^(a\"b)$".into()));
    }

    #[test]
    fn a_dot_splits_schema_from_name() {
        let p = parse("public.events");
        assert_eq!(p.schema, Some("^(public)$".into()));
        assert_eq!(p.name, Some("^(events)$".into()));
    }

    #[test]
    fn a_dot_inside_quotes_does_not_split() {
        let p = parse("\"a.b\"");
        assert_eq!(p.schema, None);
        assert_eq!(p.name, Some("^(a\\.b)$".into()));
    }

    #[test]
    fn a_second_dot_belongs_to_the_name() {
        // Postgres names are at most two parts here; anything after the first
        // dot is the object name, dots and all.
        let p = parse("public.a.b");
        assert_eq!(p.schema, Some("^(public)$".into()));
        assert_eq!(p.name, Some("^(a\\.b)$".into()));
    }

    #[test]
    fn a_trailing_dot_means_any_name_in_that_schema() {
        let p = parse("public.");
        assert_eq!(p.schema, Some("^(public)$".into()));
        assert_eq!(p.name, None);
    }

    #[test]
    fn a_leading_dot_means_any_schema() {
        let p = parse(".events");
        assert_eq!(p.schema, None);
        assert_eq!(p.name, Some("^(events)$".into()));
    }

    #[test]
    fn regex_metacharacters_in_a_pattern_are_literal() {
        // A user typing a name that happens to contain `+` means the character,
        // not "one or more" — an unescaped one would be a silent wrong answer.
        assert_eq!(name_of("a+b"), Some("^(a\\+b)$".into()));
        assert_eq!(name_of("a(b)"), Some("^(a\\(b\\))$".into()));
        assert_eq!(name_of("a$b"), Some("^(a\\$b)$".into()));
        assert_eq!(name_of("a\\b"), Some("^(a\\\\b)$".into()));
    }

    #[test]
    fn an_empty_pattern_matches_everything() {
        let p = parse("");
        assert_eq!(p.schema, None);
        assert_eq!(p.name, None);
    }

    #[test]
    fn no_schema_part_falls_back_to_the_visibility_predicate() {
        // This is what makes bare `\dt` list the search path rather than every
        // schema in the database.
        let p = parse("events");
        let c = p.clauses("n.nspname", "c.relname", Some("pg_table_is_visible(c.oid)"));
        assert_eq!(c.len(), 2);
        assert_eq!(c[0], "pg_table_is_visible(c.oid)");
        assert!(c[1].contains("c.relname ~ '^(events)$'"));
    }

    #[test]
    fn an_explicit_schema_replaces_the_visibility_predicate() {
        // `\dt other.*` must reach outside the search path, or the qualified
        // form would be pointless.
        let p = parse("other.*");
        let c = p.clauses("n.nspname", "c.relname", Some("pg_table_is_visible(c.oid)"));
        assert!(!c.iter().any(|s| s.contains("visible")));
        assert!(c.iter().any(|s| s.contains("n.nspname ~ '^(other)$'")));
    }

    #[test]
    fn a_quote_in_a_pattern_cannot_break_out_of_the_literal() {
        // The regex goes into SQL as a literal; quoting is `quote_literal`'s
        // job, but a pattern full of quotes is exactly the input that would
        // expose a mistake there.
        let p = parse("\"a'b\"");
        let c = p.clauses("n.nspname", "c.relname", None);
        assert_eq!(c[0], "c.relname ~ '^(a''b)$'");
    }
}
