//! Catalog queries behind the `\d` family.
//!
//! Building the SQL is separated from running it so the interesting part — which
//! filters a given `\dtS+ public.*` turns into — is testable without a server.
//! The queries follow psql's own column headings, because the aligned formatter
//! and any muscle memory the user has both key off them.

use super::meta::{ListSpec, ObjectClass};
use super::pattern::NamePattern;
use crate::db::grid::quote_literal;

/// The predicate that hides system catalogs, applied unless `S` was given.
const NOT_SYSTEM: &str = "n.nspname <> 'pg_catalog' AND n.nspname <> 'information_schema' \
                          AND n.nspname !~ '^pg_toast'";

/// Join predicates into a `WHERE` block, or nothing at all when there are none.
///
/// Returning the empty string for an empty slice is what lets every builder
/// below interpolate this unconditionally after its `FROM`, instead of tracking
/// whether any filter survived.
///
/// # Arguments
/// * `clauses` — `&[String]`: already-formed SQL predicates, each safe to
///   interpolate; empty means no filtering at all.
///
/// # Returns
/// `String` — a `"\nWHERE …"` block joined by `AND`, or the empty string when
/// `clauses` is empty.
fn and_all(clauses: &[String]) -> String {
    if clauses.is_empty() {
        String::new()
    } else {
        format!("\nWHERE {}", clauses.join("\n  AND "))
    }
}

/// A single-name object's pattern clause (roles, extensions, schemas,
/// databases) — these have no schema to qualify, so only the name half applies.
///
/// # Arguments
/// * `pattern` — `&NamePattern`: the parsed `\d…` argument; its schema half is
///   ignored here.
/// * `col` — `&str`: the SQL column holding the name, e.g. `"r.rolname"`.
///
/// # Returns
/// `Vec<String>` — one regex-match predicate, or empty when the pattern names
/// nothing.
fn name_only(pattern: &NamePattern, col: &str) -> Vec<String> {
    match &pattern.name {
        Some(n) => vec![format!("{col} ~ {}", quote_literal(n))],
        None => Vec::new(),
    }
}

/// The SQL for one `\d…` listing.
///
/// # Arguments
/// * `spec` — `&ListSpec`: the parsed command — object class, pattern, and the
///   `S` / `+` modifiers.
///
/// # Returns
/// `String` — a complete, ready-to-run `SELECT` with no bind parameters.
pub fn list_sql(spec: &ListSpec) -> String {
    match &spec.what {
        ObjectClass::Relations(kinds) => relations_sql(spec, kinds),
        ObjectClass::Functions => functions_sql(spec),
        ObjectClass::Roles => roles_sql(spec),
        ObjectClass::Extensions => extensions_sql(spec),
        ObjectClass::Schemas => schemas_sql(spec),
        ObjectClass::Databases => databases_sql(spec),
    }
}

/// `\dt`, `\dv`, `\di`, `\ds` and friends — anything living in `pg_class`.
///
/// `kinds` is the set of `relkind` characters the command selects, and it also
/// shapes the output: indexes bring an extra "Table" column, and `+` adds a
/// size that is suppressed for relations with no storage.
///
/// The kind characters are interpolated as literals, so they must come from the
/// meta-command table rather than from user input.
///
/// # Arguments
/// * `spec` — `&ListSpec`: supplies the pattern and the `S` / `+` modifiers.
/// * `kinds` — `&[char]`: `relkind` characters, interpolated raw, so they must
///   be trusted constants rather than user input.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_class`, ordered by schema then name.
fn relations_sql(spec: &ListSpec, kinds: &[char]) -> String {
    let kind_list = kinds
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(",");

    let mut clauses = vec![format!("c.relkind IN ({kind_list})")];
    if !spec.system {
        clauses.push(NOT_SYSTEM.to_string());
    }
    clauses.extend(spec.pattern.clauses(
        "n.nspname",
        "c.relname",
        Some("pg_catalog.pg_table_is_visible(c.oid)"),
    ));

    // Listing indexes without saying what they index is close to useless, so
    // the extra column appears exactly when indexes are in scope.
    let index_col = if kinds.contains(&'i') {
        ",\n       COALESCE(ct.relname, '') AS \"Table\""
    } else {
        ""
    };
    let index_join = if kinds.contains(&'i') {
        "\nLEFT JOIN pg_index idx ON idx.indexrelid = c.oid\
         \nLEFT JOIN pg_class ct ON ct.oid = idx.indrelid"
    } else {
        ""
    };

    let verbose_cols = if spec.verbose {
        // Views and composite types have no storage; asking for their size is
        // an error rather than a zero, so the CASE is load-bearing.
        ",\n       CASE WHEN c.relkind IN ('r','p','m','i','I','S','t')\
         \n            THEN pg_size_pretty(pg_total_relation_size(c.oid)) ELSE '' END AS \"Size\",\
         \n       COALESCE(obj_description(c.oid, 'pg_class'), '') AS \"Description\""
    } else {
        ""
    };

    format!(
        "SELECT n.nspname AS \"Schema\",\n       \
         c.relname AS \"Name\",\n       \
         CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned table'\n            \
         WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view'\n            \
         WHEN 'i' THEN 'index' WHEN 'I' THEN 'partitioned index'\n            \
         WHEN 'S' THEN 'sequence' WHEN 'f' THEN 'foreign table'\n            \
         ELSE c.relkind::text END AS \"Type\",\n       \
         pg_get_userbyid(c.relowner) AS \"Owner\"{index_col}{verbose_cols}\n\
         FROM pg_class c\n\
         JOIN pg_namespace n ON n.oid = c.relnamespace{index_join}{}\n\
         ORDER BY 1, 2",
        and_all(&clauses)
    )
}

/// `\df` — functions, procedures, aggregates and window functions alike, since
/// `pg_proc` holds them all and `prokind` is reported as a column rather than
/// used as a filter.
///
/// Sorted by argument types as well as name so overloads group predictably.
///
/// # Arguments
/// * `spec` — `&ListSpec`: supplies the pattern and the `S` / `+` modifiers.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_proc`.
fn functions_sql(spec: &ListSpec) -> String {
    let mut clauses = Vec::new();
    if !spec.system {
        clauses.push(NOT_SYSTEM.to_string());
    }
    clauses.extend(spec.pattern.clauses(
        "n.nspname",
        "p.proname",
        Some("pg_catalog.pg_function_is_visible(p.oid)"),
    ));

    let verbose_cols = if spec.verbose {
        ",\n       l.lanname AS \"Language\",\n       \
         COALESCE(obj_description(p.oid, 'pg_proc'), '') AS \"Description\""
    } else {
        ""
    };
    let verbose_join = if spec.verbose {
        "\nLEFT JOIN pg_language l ON l.oid = p.prolang"
    } else {
        ""
    };

    format!(
        "SELECT n.nspname AS \"Schema\",\n       \
         p.proname AS \"Name\",\n       \
         pg_get_function_result(p.oid) AS \"Result data type\",\n       \
         pg_get_function_arguments(p.oid) AS \"Argument data types\",\n       \
         CASE p.prokind WHEN 'a' THEN 'agg' WHEN 'w' THEN 'window'\n            \
         WHEN 'p' THEN 'proc' ELSE 'func' END AS \"Type\"{verbose_cols}\n\
         FROM pg_proc p\n\
         JOIN pg_namespace n ON n.oid = p.pronamespace{verbose_join}{}\n\
         ORDER BY 1, 2, 4",
        and_all(&clauses)
    )
}

/// `\du` — roles, with their flags folded into psql's single "Attributes"
/// column.
///
/// `S` here means the `pg_`-prefixed predefined roles rather than a system
/// schema, since roles are cluster-wide and have no namespace.
///
/// # Arguments
/// * `spec` — `&ListSpec`: only the name half of its pattern is used; `S`
///   controls the `pg_`-prefixed roles.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_roles`.
fn roles_sql(spec: &ListSpec) -> String {
    let mut clauses = Vec::new();
    if !spec.system {
        // The `pg_` roles are Postgres's own predefined ones, noise in a list
        // of "who has an account here".
        clauses.push("r.rolname !~ '^pg_'".to_string());
    }
    clauses.extend(name_only(&spec.pattern, "r.rolname"));

    let verbose_cols = if spec.verbose {
        ",\n       COALESCE(shobj_description(r.oid, 'pg_authid'), '') AS \"Description\""
    } else {
        ""
    };

    format!(
        "SELECT r.rolname AS \"Role name\",\n       \
         array_to_string(ARRAY(SELECT a FROM unnest(ARRAY[\n         \
         CASE WHEN r.rolsuper THEN 'Superuser' END,\n         \
         CASE WHEN NOT r.rolinherit THEN 'No inheritance' END,\n         \
         CASE WHEN r.rolcreaterole THEN 'Create role' END,\n         \
         CASE WHEN r.rolcreatedb THEN 'Create DB' END,\n         \
         CASE WHEN NOT r.rolcanlogin THEN 'Cannot login' END,\n         \
         CASE WHEN r.rolreplication THEN 'Replication' END,\n         \
         CASE WHEN r.rolconnlimit >= 0 THEN 'Connections: ' || r.rolconnlimit END\n       \
         ]) AS a WHERE a IS NOT NULL), ', ') AS \"Attributes\"{verbose_cols}\n\
         FROM pg_roles r{}\n\
         ORDER BY 1",
        and_all(&clauses)
    )
}

/// `\dx` — installed extensions and the versions actually in place, which may
/// lag the newest available on disk.
///
/// # Arguments
/// * `spec` — `&ListSpec`: only the name half of its pattern and `+` matter;
///   `S` has no effect here.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_extension`.
fn extensions_sql(spec: &ListSpec) -> String {
    // Installed extensions are never "system objects" in the sense S controls,
    // so only the pattern narrows this list.
    let clauses = name_only(&spec.pattern, "e.extname");
    let verbose_cols = if spec.verbose {
        ",\n       n.nspname AS \"Schema\""
    } else {
        ""
    };
    format!(
        "SELECT e.extname AS \"Name\",\n       \
         e.extversion AS \"Version\",\n       \
         COALESCE(obj_description(e.oid, 'pg_extension'), '') AS \"Description\"{verbose_cols}\n\
         FROM pg_extension e\n\
         JOIN pg_namespace n ON n.oid = e.extnamespace{}\n\
         ORDER BY 1",
        and_all(&clauses)
    )
}

/// `\dn` — schemas in the current database. Without `S` this hides
/// `information_schema` and everything `pg_`, including the toast namespaces.
///
/// # Arguments
/// * `spec` — `&ListSpec`: only the name half of its pattern is used, since a
///   schema has no schema of its own.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_namespace`.
fn schemas_sql(spec: &ListSpec) -> String {
    let mut clauses = Vec::new();
    if !spec.system {
        clauses.push("n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'".to_string());
    }
    clauses.extend(name_only(&spec.pattern, "n.nspname"));

    let verbose_cols = if spec.verbose {
        ",\n       COALESCE(obj_description(n.oid, 'pg_namespace'), '') AS \"Description\""
    } else {
        ""
    };
    format!(
        "SELECT n.nspname AS \"Name\",\n       \
         pg_get_userbyid(n.nspowner) AS \"Owner\"{verbose_cols}\n\
         FROM pg_namespace n{}\n\
         ORDER BY 1",
        and_all(&clauses)
    )
}

/// `\l` — databases in the cluster, template databases excluded unconditionally
/// (psql's `\l` hides them too, and `S` doesn't govern them).
///
/// # Arguments
/// * `spec` — `&ListSpec`: only the name half of its pattern and `+` matter;
///   `S` has no effect here.
///
/// # Returns
/// `String` — a complete `SELECT` over `pg_database`.
fn databases_sql(spec: &ListSpec) -> String {
    let mut clauses = vec!["NOT d.datistemplate".to_string()];
    clauses.extend(name_only(&spec.pattern, "d.datname"));

    let verbose_cols = if spec.verbose {
        // A database we cannot connect to has no size we are allowed to read;
        // psql prints the error text there, we print nothing.
        ",\n       CASE WHEN has_database_privilege(d.datname, 'CONNECT')\n            \
         THEN pg_size_pretty(pg_database_size(d.datname)) ELSE '' END AS \"Size\",\n       \
         COALESCE(shobj_description(d.oid, 'pg_database'), '') AS \"Description\""
    } else {
        ""
    };

    format!(
        "SELECT d.datname AS \"Name\",\n       \
         pg_get_userbyid(d.datdba) AS \"Owner\",\n       \
         pg_encoding_to_char(d.encoding) AS \"Encoding\"{verbose_cols}\n\
         FROM pg_database d{}\n\
         ORDER BY 1",
        and_all(&clauses)
    )
}

/// `\conninfo` — psql prints this from its own connection struct; we ask the
/// server so it reflects what actually happened (a `search_path` set by the
/// profile, a role changed by `SET ROLE`).
pub const CONNINFO_SQL: &str = "\
SELECT current_database() AS \"Database\",
       current_user AS \"User\",
       COALESCE(inet_server_addr()::text, 'local socket') AS \"Host\",
       COALESCE(inet_server_port()::text, '') AS \"Port\",
       current_setting('server_version') AS \"Server version\",
       current_setting('search_path') AS \"Search path\"";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::meta::{self, MetaCommand};

    /// Parses `input` and renders its SQL, failing the test if it wasn't a
    /// listing — every caller here cares about the SQL, not the command variant.
    ///
    /// # Arguments
    /// * `input` — `&str`: raw meta-command text as typed, backslash included.
    ///
    /// # Returns
    /// `String` — the rendered SQL; panics rather than returning when `input`
    /// is not a listing command.
    fn sql_for(input: &str) -> String {
        match meta::parse(input) {
            Some(MetaCommand::List(spec)) => list_sql(&spec),
            other => panic!("expected a list for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn dt_selects_only_table_relkinds() {
        let sql = sql_for("\\dt");
        assert!(sql.contains("c.relkind IN ('r','p')"), "{sql}");
    }

    #[test]
    fn system_objects_are_hidden_unless_s_is_given() {
        assert!(sql_for("\\dt").contains("pg_catalog"));
        // With S, the exclusion must be gone entirely — not merely widened.
        let with_s = sql_for("\\dtS");
        assert!(
            !with_s.contains("n.nspname <> 'pg_catalog'"),
            "S should drop the system filter:\n{with_s}"
        );
    }

    #[test]
    fn a_bare_listing_is_limited_to_the_search_path() {
        // This is the difference between `\dt` and "every table in the cluster".
        assert!(sql_for("\\dt").contains("pg_table_is_visible"));
    }

    #[test]
    fn a_qualified_pattern_replaces_the_visibility_filter() {
        let sql = sql_for("\\dt other.*");
        assert!(!sql.contains("pg_table_is_visible"), "{sql}");
        assert!(sql.contains("n.nspname ~ '^(other)$'"), "{sql}");
    }

    #[test]
    fn a_name_pattern_becomes_an_anchored_regex_comparison() {
        let sql = sql_for("\\dt ev*");
        assert!(sql.contains("c.relname ~ '^(ev.*)$'"), "{sql}");
    }

    #[test]
    fn verbose_adds_size_but_guards_relations_without_storage() {
        let sql = sql_for("\\dt+");
        assert!(sql.contains("pg_size_pretty"), "{sql}");
        // A view has no relfilenode; sizing it unconditionally would error.
        assert!(sql.contains("CASE WHEN c.relkind IN"), "{sql}");
        assert!(!sql_for("\\dt").contains("pg_size_pretty"));
    }

    #[test]
    fn listing_indexes_also_names_the_indexed_table() {
        let sql = sql_for("\\di");
        assert!(sql.contains("AS \"Table\""), "{sql}");
        assert!(sql.contains("pg_index"), "{sql}");
        // Other listings must not carry the join along.
        assert!(!sql_for("\\dt").contains("pg_index"));
    }

    #[test]
    fn functions_use_their_own_visibility_function() {
        let sql = sql_for("\\df");
        assert!(sql.contains("pg_function_is_visible"), "{sql}");
        assert!(sql.contains("pg_get_function_arguments"), "{sql}");
    }

    #[test]
    fn roles_hide_the_predefined_pg_roles_by_default() {
        assert!(sql_for("\\du").contains("r.rolname !~ '^pg_'"));
        assert!(!sql_for("\\duS").contains("r.rolname !~ '^pg_'"));
    }

    #[test]
    fn single_name_objects_ignore_a_schema_qualifier() {
        // `pg_roles` has no schema column; emitting `n.nspname` for it would be
        // a SQL error rather than an empty result.
        let sql = sql_for("\\du admin*");
        assert!(!sql.contains("nspname"), "{sql}");
        assert!(sql.contains("r.rolname ~ '^(admin.*)$'"), "{sql}");
    }

    #[test]
    fn databases_always_exclude_templates() {
        assert!(sql_for("\\l").contains("NOT d.datistemplate"));
    }

    #[test]
    fn database_sizes_are_guarded_by_connect_privilege() {
        let sql = sql_for("\\l+");
        assert!(sql.contains("has_database_privilege"), "{sql}");
    }

    #[test]
    fn every_listing_form_produces_a_where_clause_that_parses_positionally() {
        // Cheap structural guard: no listing should ever emit a dangling
        // `WHERE` or a doubled `AND`, which is how string-built SQL usually
        // breaks when a filter becomes conditional.
        for input in [
            "\\d",
            "\\dt",
            "\\dtS",
            "\\dt+",
            "\\di",
            "\\ds",
            "\\dv",
            "\\dm",
            "\\df",
            "\\df+",
            "\\dn",
            "\\du",
            "\\dx",
            "\\l",
            "\\l+",
            "\\dt public.*",
            "\\dx pg*",
        ] {
            let sql = sql_for(input);
            assert!(!sql.contains("WHERE\n"), "{input}: dangling WHERE\n{sql}");
            assert!(!sql.contains("AND AND"), "{input}: doubled AND\n{sql}");
            assert!(!sql.contains("WHERE ORDER"), "{input}: empty WHERE\n{sql}");
            assert!(
                sql.trim_end().ends_with("ORDER BY 1") || sql.contains("ORDER BY 1, 2"),
                "{input}: no ordering\n{sql}"
            );
        }
    }
}
