//! psql backslash commands.
//!
//! The informational half of psql's set (plan.md §5.7): the `\d` family with
//! psql's own name patterns and its `S`/`+` modifiers, plus the session
//! commands that the terminal header exposes. Anything else gets psql's
//! "invalid command" message rather than silently doing nothing.

use super::pattern::{self, NamePattern};

/// What a `\d…` command is listing.
///
/// `Relations` carries the `relkind` values to match, so the combined forms
/// psql allows (`\dtv` = tables *and* views) fall out of accumulating letters
/// rather than needing a variant each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectClass {
    Relations(Vec<char>),
    Functions,
    Roles,
    Extensions,
    Schemas,
    Databases,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSpec {
    pub what: ObjectClass,
    pub pattern: NamePattern,
    /// `+` — extra columns (size, description, definition).
    pub verbose: bool,
    /// `S` — include system objects.
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaCommand {
    /// Any `\d…` / `\l` listing.
    List(ListSpec),
    /// `\d <name>` — describe one relation.
    Describe { name: String, verbose: bool },
    /// `\timing [on|off]`; None toggles.
    Timing(Option<bool>),
    /// `\x [on|off]`; None toggles.
    Expanded(Option<bool>),
    /// `\conninfo` — who and where am I connected as.
    ConnInfo,
    /// `\encoding` — the client encoding (always UTF8 here).
    Encoding,
    /// `\i <name>` — run a saved query.
    Include(String),
    /// `\?` — help.
    Help,
    /// `\q` — quit (the pane persists; we print a note).
    Quit,
    /// Recognised but unsupported, with a clear message.
    Unsupported(String),
    /// Anything else.
    Unknown(String),
}

/// Relation-listing letters and the `relkind` values they select.
fn relkinds_for(letter: char) -> Option<&'static [char]> {
    Some(match letter {
        // Partitioned tables are tables as far as a user is concerned.
        't' => &['r', 'p'],
        'v' => &['v'],
        'm' => &['m'],
        's' => &['S'],
        'i' => &['i', 'I'],
        'E' => &['f'],
        _ => return None,
    })
}

/// What bare `\d` lists: everything except indexes, as in psql.
const DEFAULT_RELKINDS: &[char] = &['r', 'p', 'v', 'm', 'S', 'f'];

/// Decode the letters after `\d` into a class plus modifiers.
///
/// Returns None for any letter we don't implement, so the caller can report it
/// as an invalid command instead of quietly listing the wrong thing.
fn parse_d_suffix(suffix: &str) -> Option<(Option<ObjectClass>, bool, bool)> {
    let mut relkinds: Vec<char> = Vec::new();
    let mut class: Option<ObjectClass> = None;
    let mut verbose = false;
    let mut system = false;

    for c in suffix.chars() {
        match c {
            '+' => verbose = true,
            'S' => system = true,
            'n' | 'f' | 'x' | 'u' | 'g' => {
                // These name a whole class, so they cannot be combined with
                // each other or with the relation letters.
                if class.is_some() || !relkinds.is_empty() {
                    return None;
                }
                class = Some(match c {
                    'n' => ObjectClass::Schemas,
                    'f' => ObjectClass::Functions,
                    'x' => ObjectClass::Extensions,
                    // psql treats \du and \dg as the same list post-8.1.
                    _ => ObjectClass::Roles,
                });
            }
            _ => {
                let kinds = relkinds_for(c)?;
                if class.is_some() {
                    return None;
                }
                relkinds.extend_from_slice(kinds);
            }
        }
    }

    let what = match class {
        Some(c) => Some(c),
        None if relkinds.is_empty() => None,
        None => Some(ObjectClass::Relations(relkinds)),
    };
    Some((what, verbose, system))
}

/// Parse a line as a meta-command. Returns None if it isn't one.
pub fn parse(input: &str) -> Option<MetaCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('\\') {
        return None;
    }

    // Split into the command word and the rest of the line. The rest is kept
    // whole rather than tokenised, so a quoted pattern containing a space
    // (`\dt "my table"`) survives.
    let (cmd, rest) = match trimmed.find(char::is_whitespace) {
        Some(i) => (&trimmed[..i], trimmed[i..].trim()),
        None => (trimmed, ""),
    };

    // psql strips a trailing `;` from meta-commands and their arguments.
    let cmd = cmd.trim_end_matches(';');
    let rest = rest.trim_end_matches(';').trim();
    let arg = if rest.is_empty() { None } else { Some(rest) };

    let flag = || -> Option<bool> {
        match rest.to_ascii_lowercase().as_str() {
            "on" | "true" | "1" => Some(true),
            "off" | "false" | "0" => Some(false),
            _ => None,
        }
    };

    // The `\l` family shares the listing shape but has no letters to decode.
    let databases = |suffix: &str| {
        MetaCommand::List(ListSpec {
            what: ObjectClass::Databases,
            pattern: pattern::parse(rest),
            verbose: suffix.contains('+'),
            system: false,
        })
    };

    Some(match cmd {
        "\\timing" => MetaCommand::Timing(flag()),
        "\\x" => MetaCommand::Expanded(flag()),
        "\\conninfo" => MetaCommand::ConnInfo,
        "\\encoding" => MetaCommand::Encoding,
        "\\?" | "\\h" | "\\help" => MetaCommand::Help,
        "\\q" | "\\quit" => MetaCommand::Quit,
        "\\c" | "\\connect" | "\\copy" | "\\e" | "\\edit" | "\\o" | "\\!" => {
            MetaCommand::Unsupported(cmd.to_string())
        }
        "\\i" | "\\include" => match arg {
            Some(name) => MetaCommand::Include(name.to_string()),
            None => MetaCommand::Unknown("\\i".into()),
        },
        "\\l" | "\\l+" | "\\list" | "\\list+" => databases(cmd),
        _ if cmd.starts_with("\\d") => {
            let Some((what, verbose, system)) = parse_d_suffix(&cmd[2..]) else {
                return Some(MetaCommand::Unknown(cmd.to_string()));
            };
            match (what, arg) {
                // Bare `\d name` is the odd one out: no letters means describe
                // that relation rather than list a class of them.
                (None, Some(name)) => MetaCommand::Describe {
                    name: name.to_string(),
                    verbose,
                },
                (what, _) => MetaCommand::List(ListSpec {
                    what: what.unwrap_or_else(|| ObjectClass::Relations(DEFAULT_RELKINDS.to_vec())),
                    pattern: pattern::parse(rest),
                    verbose,
                    system,
                }),
            }
        }
        other => MetaCommand::Unknown(other.to_string()),
    })
}

/// psql's message for an unrecognised backslash command.
pub fn unknown_message(cmd: &str) -> String {
    format!("invalid command {cmd}\nTry \\? for help.")
}

/// The note shown for commands psql has but pgscope does not, so the reason is
/// specific rather than a generic "invalid command".
pub fn unsupported_message(cmd: &str) -> String {
    let why = match cmd {
        "\\c" | "\\connect" => "use the connection pill to switch databases",
        "\\copy" => "client-side COPY is not implemented; server-side COPY works",
        "\\e" | "\\edit" => "press ⌘N to open a query editor tab instead",
        "\\o" => "output redirection to a file is not implemented",
        "\\!" => "shell escapes are deliberately not available",
        _ => "not supported in pgscope",
    };
    format!("{cmd} is not supported in pgscope — {why}.\n")
}

pub const HELP_TEXT: &str = "\
General
  \\q                     quit psql (in pgscope, closes the session)
  \\?                     show this help
  \\conninfo              show the current connection
  \\i      NAME           run a saved query by name

Informational  (S = also system objects, + = more detail)
  \\d[S+]                 list tables, views, matviews, sequences
  \\d[S+]  NAME           describe table, view, index, or sequence
  \\dt[S+] [PATTERN]      list tables
  \\dv[S+] [PATTERN]      list views
  \\dm[S+] [PATTERN]      list materialized views
  \\di[S+] [PATTERN]      list indexes
  \\ds[S+] [PATTERN]      list sequences
  \\df[S+] [PATTERN]      list functions
  \\dn[S+] [PATTERN]      list schemas
  \\du[+]  [PATTERN]      list roles
  \\dx[+]  [PATTERN]      list extensions
  \\l[+]   [PATTERN]      list databases

Patterns follow psql: * and ? are wildcards, a dot separates schema from name,
and double quotes make a name case-sensitive.

Formatting
  \\timing [on|off]       toggle timing of commands
  \\x      [on|off]       toggle expanded output";

#[cfg(test)]
mod tests {
    use super::*;

    /// The `ListSpec` a `\d`-family command parses to, failing the test on any
    /// other variant so the assertions can index into it directly.
    fn list(input: &str) -> ListSpec {
        match parse(input) {
            Some(MetaCommand::List(s)) => s,
            other => panic!("expected a list for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn plain_sql_is_not_a_meta_command() {
        assert!(parse("SELECT 1").is_none());
        assert!(parse("  select * from events;").is_none());
    }

    #[test]
    fn bare_d_lists_relations_but_not_indexes() {
        let s = list("\\d");
        assert_eq!(
            s.what,
            ObjectClass::Relations(DEFAULT_RELKINDS.to_vec()),
            "bare \\d matches psql: everything except indexes"
        );
        assert!(!s.verbose && !s.system);
    }

    #[test]
    fn d_with_a_name_describes_rather_than_lists() {
        // The design's history panel shows exactly this command.
        assert_eq!(
            parse("\\d events"),
            Some(MetaCommand::Describe {
                name: "events".into(),
                verbose: false
            })
        );
    }

    #[test]
    fn d_plus_with_a_name_is_a_verbose_describe() {
        assert_eq!(
            parse("\\d+ events"),
            Some(MetaCommand::Describe {
                name: "events".into(),
                verbose: true
            })
        );
    }

    #[test]
    fn strips_a_trailing_semicolon_from_command_and_argument() {
        assert_eq!(
            parse("\\d events;"),
            Some(MetaCommand::Describe {
                name: "events".into(),
                verbose: false
            })
        );
        assert!(matches!(parse("\\dt;"), Some(MetaCommand::List(_))));
    }

    #[test]
    fn each_listing_letter_selects_its_relkinds() {
        assert_eq!(list("\\dt").what, ObjectClass::Relations(vec!['r', 'p']));
        assert_eq!(list("\\dv").what, ObjectClass::Relations(vec!['v']));
        assert_eq!(list("\\dm").what, ObjectClass::Relations(vec!['m']));
        assert_eq!(list("\\ds").what, ObjectClass::Relations(vec!['S']));
        assert_eq!(list("\\di").what, ObjectClass::Relations(vec!['i', 'I']));
        assert_eq!(list("\\dn").what, ObjectClass::Schemas);
        assert_eq!(list("\\df").what, ObjectClass::Functions);
        assert_eq!(list("\\dx").what, ObjectClass::Extensions);
        assert_eq!(list("\\du").what, ObjectClass::Roles);
        assert_eq!(list("\\dg").what, ObjectClass::Roles);
        assert_eq!(list("\\l").what, ObjectClass::Databases);
        assert_eq!(list("\\list").what, ObjectClass::Databases);
    }

    #[test]
    fn letters_combine_the_way_psql_combines_them() {
        // `\dtv` is one list of tables and views, not an invalid command.
        assert_eq!(
            list("\\dtv").what,
            ObjectClass::Relations(vec!['r', 'p', 'v'])
        );
    }

    #[test]
    fn modifiers_parse_in_either_order_and_alongside_letters() {
        for input in ["\\dtS+", "\\dt+S", "\\dS+t"] {
            let s = list(input);
            assert_eq!(s.what, ObjectClass::Relations(vec!['r', 'p']), "{input}");
            assert!(s.verbose, "{input} should be verbose");
            assert!(s.system, "{input} should include system objects");
        }
    }

    #[test]
    fn a_pattern_is_parsed_and_attached_to_the_listing() {
        let s = list("\\dt public.ev*");
        assert_eq!(s.pattern.schema.as_deref(), Some("^(public)$"));
        assert_eq!(s.pattern.name.as_deref(), Some("^(ev.*)$"));
    }

    #[test]
    fn a_quoted_pattern_containing_a_space_survives_parsing() {
        // Tokenising on whitespace would truncate this to `"my` — the reason
        // the argument is kept as the rest of the line.
        let s = list("\\dt \"my table\"");
        assert_eq!(s.pattern.name.as_deref(), Some("^(my table)$"));
    }

    #[test]
    fn class_letters_cannot_be_combined_with_each_other() {
        // `\dfn` is meaningless — functions and schemas are different lists —
        // and must not silently resolve to one of them.
        assert_eq!(parse("\\dfn"), Some(MetaCommand::Unknown("\\dfn".into())));
        assert_eq!(parse("\\dft"), Some(MetaCommand::Unknown("\\dft".into())));
    }

    #[test]
    fn an_unimplemented_d_letter_is_an_invalid_command() {
        assert_eq!(parse("\\dz"), Some(MetaCommand::Unknown("\\dz".into())));
    }

    #[test]
    fn parses_timing_with_and_without_a_flag() {
        // The design's history shows `\timing on`.
        assert_eq!(parse("\\timing on"), Some(MetaCommand::Timing(Some(true))));
        assert_eq!(
            parse("\\timing off"),
            Some(MetaCommand::Timing(Some(false)))
        );
        assert_eq!(parse("\\timing"), Some(MetaCommand::Timing(None)));
        assert_eq!(parse("\\timing ON"), Some(MetaCommand::Timing(Some(true))));
    }

    #[test]
    fn parses_expanded_output() {
        assert_eq!(parse("\\x"), Some(MetaCommand::Expanded(None)));
        assert_eq!(parse("\\x on"), Some(MetaCommand::Expanded(Some(true))));
    }

    #[test]
    fn parses_help_quit_and_conninfo() {
        assert_eq!(parse("\\?"), Some(MetaCommand::Help));
        assert_eq!(parse("\\q"), Some(MetaCommand::Quit));
        assert_eq!(parse("\\conninfo"), Some(MetaCommand::ConnInfo));
        assert_eq!(parse("\\encoding"), Some(MetaCommand::Encoding));
    }

    #[test]
    fn include_needs_a_name() {
        assert_eq!(
            parse("\\i daily_active"),
            Some(MetaCommand::Include("daily_active".into()))
        );
        // Bare `\i` has nothing to run; psql errors, and so do we.
        assert_eq!(parse("\\i"), Some(MetaCommand::Unknown("\\i".into())));
    }

    #[test]
    fn commands_we_deliberately_lack_explain_themselves() {
        assert_eq!(
            parse("\\c otherdb"),
            Some(MetaCommand::Unsupported("\\c".into()))
        );
        assert_eq!(
            parse("\\copy"),
            Some(MetaCommand::Unsupported("\\copy".into()))
        );
        assert!(unsupported_message("\\copy").contains("server-side COPY"));
        assert!(unsupported_message("\\e").contains("⌘N"));
    }

    #[test]
    fn reports_unknown_commands_like_psql() {
        assert_eq!(parse("\\foo"), Some(MetaCommand::Unknown("\\foo".into())));
        assert_eq!(
            unknown_message("\\foo"),
            "invalid command \\foo\nTry \\? for help."
        );
    }

    #[test]
    fn every_command_in_the_help_text_actually_parses() {
        // Help that advertises a command we don't implement is worse than no
        // help, so the text is checked against the parser rather than trusted.
        for line in HELP_TEXT.lines() {
            let line = line.trim_start();
            let Some(word) = line.split_whitespace().next() else {
                continue;
            };
            if !word.starts_with('\\') {
                continue;
            }
            // Strip the `[S+]` / `[+]` modifier notation from the help spelling.
            let base: String = word.chars().take_while(|c| *c != '[').collect();
            let probe = if base == "\\i" || base == "\\d" {
                format!("{base} events")
            } else {
                base.clone()
            };
            match parse(&probe) {
                Some(MetaCommand::Unknown(c)) => {
                    panic!("help advertises {word}, but {probe:?} parses as unknown ({c})")
                }
                None => panic!("help advertises {word}, but {probe:?} is not a meta-command"),
                _ => {}
            }
        }
    }
}
