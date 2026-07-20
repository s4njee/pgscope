use deadpool_postgres::Pool;
use serde::{Deserialize, Serialize};

use super::grid::{quote_ident, quote_literal};
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelKind {
    Table,
    View,
}

impl RelKind {
    /// Collapse `pg_class.relkind` into the two categories the tree draws.
    ///
    /// Materialised views count as views: browsing one is a read of stored
    /// rows, but it has no writable-table affordances. Partitioned tables (`p`)
    /// fall through to `Table` along with ordinary ones.
    ///
    /// # Arguments
    /// * `c` — `i8`: the raw `pg_class.relkind` byte as Postgres returns it
    ///   (`"char"`), reinterpreted as an ASCII character here.
    ///
    /// # Returns
    /// `Self` — `View` for `v`/`m`, `Table` for everything else, including
    /// unrecognised kinds.
    fn from_relkind(c: i8) -> Self {
        match c as u8 as char {
            'v' | 'm' => Self::View,
            _ => Self::Table,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub name: String,
    pub kind: RelKind,
    /// `reltuples`; -1 means "never analyzed" and the UI shows a dash.
    pub est_rows: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaNode {
    pub name: String,
    pub tables: Vec<Relation>,
    pub views: Vec<Relation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    pub is_pk: bool,
    pub is_fk: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexMeta {
    pub name: String,
    /// Display line, e.g. `btree (user_id, created_at DESC) · unique`.
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableStats {
    pub est_rows: Option<i64>,
    pub total_bytes: Option<i64>,
    pub index_bytes: Option<i64>,
    /// Seconds since the last autovacuum; the UI formats it as "41 min ago".
    pub last_autovacuum_secs: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMeta {
    pub schema: String,
    pub name: String,
    pub kind: RelKind,
    pub columns: Vec<ColumnMeta>,
    pub indexes: Vec<IndexMeta>,
    pub stats: TableStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkEdge {
    pub name: String,
    pub src_table: String,
    pub tgt_table: String,
    pub src_columns: Vec<String>,
    pub tgt_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkCard {
    pub table: String,
    pub columns: Vec<ColumnMeta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkGraph {
    pub schema: String,
    pub cards: Vec<FkCard>,
    pub edges: Vec<FkEdge>,
    /// Total relations in the schema, for the "5 of 7 tables" caption.
    pub total_tables: usize,
}

/// User schemas with their table/view counts, and the relations in each.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool; one connection is held for the
///   whole walk.
///
/// # Returns
/// `Result<Vec<SchemaNode>>` — schemas in name order, `pg_*` and
/// `information_schema` excluded, each with tables and views separated and named
/// in order. `Err` if any catalogue query fails.
pub async fn schema_tree(pool: &Pool) -> Result<Vec<SchemaNode>> {
    let client = pool.get().await?;

    let schema_rows = client
        .query(
            "SELECT n.nspname AS schema
             FROM pg_namespace n
             WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'
             ORDER BY n.nspname",
            &[],
        )
        .await?;

    let mut out = Vec::with_capacity(schema_rows.len());
    for srow in schema_rows {
        let schema: String = srow.get("schema");

        let rel_rows = client
            .query(
                "SELECT c.relname AS name, c.relkind, c.reltuples::bigint AS est_rows
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m')
                 ORDER BY c.relname",
                &[&schema],
            )
            .await?;

        let mut tables = Vec::new();
        let mut views = Vec::new();
        for r in rel_rows {
            let rel = Relation {
                name: r.get("name"),
                kind: RelKind::from_relkind(r.get::<_, i8>("relkind")),
                est_rows: r.get("est_rows"),
            };
            match rel.kind {
                RelKind::Table => tables.push(rel),
                RelKind::View => views.push(rel),
            }
        }

        out.push(SchemaNode {
            name: schema,
            tables,
            views,
        });
    }

    Ok(out)
}

/// A `schema.table` reference safe to interpolate into a `::regclass` cast.
///
/// # Arguments
/// * `schema` — `&str`: raw, unquoted schema name.
/// * `table` — `&str`: raw, unquoted table name.
///
/// # Returns
/// `String` — a single-quoted SQL literal whose contents are the two
/// double-quoted identifiers, e.g. `'"public"."events"'`; safe to interpolate
/// directly.
fn regclass_literal(schema: &str, table: &str) -> String {
    quote_literal(&format!("{}.{}", quote_ident(schema), quote_ident(table)))
}

/// Columns of one relation in `attnum` order — the physical order `SELECT *`
/// would return.
///
/// Dropped columns still occupy an `attnum` and are excluded. `is_pk`/`is_fk`
/// are aggregated because a column can appear in several constraints; they say
/// only that the column participates, not that it is the whole key.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool.
/// * `schema` — `&str`: raw schema name; quoted via `regclass_literal`.
/// * `table` — `&str`: raw table name; likewise quoted.
///
/// # Returns
/// `Result<Vec<ColumnMeta>>` — live columns in `attnum` order. `Err` if the
/// relation doesn't resolve as a `regclass` or the query fails.
pub async fn columns(pool: &Pool, schema: &str, table: &str) -> Result<Vec<ColumnMeta>> {
    let client = pool.get().await?;
    let sql = format!(
        "SELECT a.attname AS name,
                format_type(a.atttypid, a.atttypmod) AS data_type,
                a.attnotnull AS not_null,
                COALESCE(bool_or(con.contype = 'p'), false) AS is_pk,
                COALESCE(bool_or(con.contype = 'f'), false) AS is_fk
         FROM pg_attribute a
         LEFT JOIN pg_constraint con
                ON con.conrelid = a.attrelid AND a.attnum = ANY (con.conkey)
               AND con.contype IN ('p','f')
         WHERE a.attrelid = {}::regclass AND a.attnum > 0 AND NOT a.attisdropped
         GROUP BY a.attnum, a.attname, a.atttypid, a.atttypmod, a.attnotnull
         ORDER BY a.attnum",
        regclass_literal(schema, table)
    );

    let rows = client.query(sql.as_str(), &[]).await?;
    Ok(rows
        .into_iter()
        .map(|r| ColumnMeta {
            name: r.get("name"),
            data_type: r.get("data_type"),
            not_null: r.get("not_null"),
            is_pk: r.get("is_pk"),
            is_fk: r.get("is_fk"),
        })
        .collect())
}

/// Turn `CREATE [UNIQUE] INDEX x ON t USING btree (a, b DESC)` into the
/// details-panel line `btree (a, b DESC) · unique`.
///
/// # Arguments
/// * `definition` — `&str`: the full `pg_get_indexdef` text; returned unchanged
///   if it has no ` USING ` clause.
/// * `is_unique` — `bool`: `pg_index.indisunique`, which appends the ` · unique`
///   suffix.
///
/// # Returns
/// `String` — the access method and key list, with the suffix when unique.
fn index_display(definition: &str, is_unique: bool) -> String {
    let body = definition
        .split_once(" USING ")
        .map(|(_, rest)| rest.trim())
        .unwrap_or(definition)
        .to_string();
    if is_unique {
        format!("{body} · unique")
    } else {
        body
    }
}

/// Indexes in the order the details panel lists them: the primary key first,
/// then the rest in creation order (`oid`).
///
/// Creation order — not alphabetical — is what the design shows: pkey,
/// idx_events_user_created, idx_events_name, brin_events_created_at. Sorting by
/// name would put `brin_…` second.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool.
/// * `schema` — `&str`: raw schema name; quoted via `regclass_literal`.
/// * `table` — `&str`: raw table name; likewise quoted.
///
/// # Returns
/// `Result<Vec<IndexMeta>>` — primary key first, then creation order; empty for
/// a relation with no indexes. `Err` if the relation doesn't resolve or the
/// query fails.
pub async fn indexes(pool: &Pool, schema: &str, table: &str) -> Result<Vec<IndexMeta>> {
    let client = pool.get().await?;
    let sql = format!(
        "SELECT ci.relname AS name, i.indisunique, i.indisprimary,
                pg_get_indexdef(i.indexrelid) AS definition
         FROM pg_index i
         JOIN pg_class ci ON ci.oid = i.indexrelid
         WHERE i.indrelid = {}::regclass
         ORDER BY i.indisprimary DESC, ci.oid",
        regclass_literal(schema, table)
    );

    let rows = client.query(sql.as_str(), &[]).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let definition: String = r.get("definition");
            IndexMeta {
                name: r.get("name"),
                definition: index_display(&definition, r.get::<_, bool>("indisunique")),
            }
        })
        .collect())
}

/// Size and freshness figures for the details panel.
///
/// All of it comes from the statistics collector, so all of it is an estimate:
/// `reltuples` is whatever the last ANALYZE saw (-1 if never), and the sizes
/// are on-disk totals including TOAST and indexes. Cheap by design — an exact
/// `count(*)` would scan the table.
///
/// A relation with no `pg_stat_user_tables` row (a view, or one never touched)
/// yields all-`None` rather than an error.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool.
/// * `schema` — `&str`: raw schema name; quoted via `regclass_literal`.
/// * `table` — `&str`: raw table name; likewise quoted.
///
/// # Returns
/// `Result<TableStats>` — every field `None` when the relation has no stats row;
/// `est_rows` may be -1, meaning never analyzed. `Err` if the relation doesn't
/// resolve or the query fails.
pub async fn stats(pool: &Pool, schema: &str, table: &str) -> Result<TableStats> {
    let client = pool.get().await?;
    let sql = format!(
        "SELECT c.reltuples::bigint AS est_rows,
                pg_total_relation_size(c.oid)::bigint AS total_bytes,
                pg_indexes_size(c.oid)::bigint AS index_bytes,
                EXTRACT(EPOCH FROM (now() - s.last_autovacuum))::float8 AS autovacuum_age
         FROM pg_class c
         LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid
         WHERE c.oid = {}::regclass",
        regclass_literal(schema, table)
    );

    let row = client.query_opt(sql.as_str(), &[]).await?;
    Ok(match row {
        Some(r) => TableStats {
            est_rows: r.get("est_rows"),
            total_bytes: r.get("total_bytes"),
            index_bytes: r.get("index_bytes"),
            last_autovacuum_secs: r.get("autovacuum_age"),
        },
        None => TableStats::default(),
    })
}

/// Everything the details panel shows for one relation.
///
/// Resolves the relkind first so views can skip the index and stats queries
/// entirely — they would return nothing useful. Errors if the relation doesn't
/// exist in this schema.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool; several checkouts happen in
///   sequence, not concurrently.
/// * `schema` — `&str`: raw schema name, bound as a parameter for the relkind
///   lookup.
/// * `table` — `&str`: raw table name, likewise bound.
///
/// # Returns
/// `Result<TableMeta>` — for a view, `indexes` is empty and `stats` is all
/// defaults, both skipped rather than queried. `Err` when no such relation
/// exists in the schema.
pub async fn table_meta(pool: &Pool, schema: &str, table: &str) -> Result<TableMeta> {
    let client = pool.get().await?;
    let kind_row = client
        .query_one(
            "SELECT c.relkind FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await?;
    let kind = RelKind::from_relkind(kind_row.get::<_, i8>("relkind"));
    drop(client);

    // Views have no indexes or table stats; skip those round-trips.
    let (columns, indexes, stats) = match kind {
        RelKind::Table => (
            columns(pool, schema, table).await?,
            indexes(pool, schema, table).await?,
            stats(pool, schema, table).await?,
        ),
        RelKind::View => (
            columns(pool, schema, table).await?,
            Vec::new(),
            TableStats::default(),
        ),
    };

    Ok(TableMeta {
        schema: schema.to_string(),
        name: table.to_string(),
        kind,
        columns,
        indexes,
        stats,
    })
}

/// Foreign-key relationships within one schema, as cards and edges for the ER
/// diagram.
///
/// Only tables touched by a constraint get a card, so `cards.len()` is normally
/// below `total_tables` — the caption reports both. Edges whose *target* lives
/// in another schema are still included, but that target has no card, since the
/// constraint is selected by the referencing side's namespace.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool; one connection for the two
///   catalogue queries, then one per card for its columns.
/// * `schema` — `&str`: raw schema name, bound as a parameter; scopes both the
///   constraints and the table count.
///
/// # Returns
/// `Result<FkGraph>` — cards only for tables in this schema that participate in
/// a foreign key, edges in constraint-name order, and `total_tables` counting
/// every table in the schema. `Err` if any query fails.
pub async fn fk_graph(pool: &Pool, schema: &str) -> Result<FkGraph> {
    let client = pool.get().await?;

    let edge_rows = client
        .query(
            "SELECT con.conname AS name,
                    sc.relname AS src_table,
                    tc.relname AS tgt_table,
                    (SELECT array_agg(a.attname ORDER BY k.ord)
                       FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
                       JOIN pg_attribute a
                         ON a.attrelid = con.conrelid AND a.attnum = k.attnum) AS src_columns,
                    (SELECT array_agg(a.attname ORDER BY k.ord)
                       FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
                       JOIN pg_attribute a
                         ON a.attrelid = con.confrelid AND a.attnum = k.attnum) AS tgt_columns
             FROM pg_constraint con
             JOIN pg_class sc ON sc.oid = con.conrelid
             JOIN pg_namespace sn ON sn.oid = sc.relnamespace
             JOIN pg_class tc ON tc.oid = con.confrelid
             WHERE con.contype = 'f' AND sn.nspname = $1
             ORDER BY con.conname",
            &[&schema],
        )
        .await?;

    let edges: Vec<FkEdge> = edge_rows
        .into_iter()
        .map(|r| FkEdge {
            name: r.get("name"),
            src_table: r.get("src_table"),
            tgt_table: r.get("tgt_table"),
            src_columns: r.get::<_, Vec<String>>("src_columns"),
            tgt_columns: r.get::<_, Vec<String>>("tgt_columns"),
        })
        .collect();

    let table_rows = client
        .query(
            "SELECT c.relname AS name
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relkind IN ('r','p')
             ORDER BY c.relname",
            &[&schema],
        )
        .await?;
    let all_tables: Vec<String> = table_rows.iter().map(|r| r.get("name")).collect();
    let total_tables = all_tables.len();
    drop(client);

    // Only tables that participate in a FK get a card — an isolated table has
    // nothing to show in a relationship graph.
    let mut connected: Vec<String> = Vec::new();
    for e in &edges {
        for t in [&e.src_table, &e.tgt_table] {
            if !connected.contains(t) {
                connected.push(t.clone());
            }
        }
    }
    connected.retain(|t| all_tables.contains(t));

    let mut cards = Vec::with_capacity(connected.len());
    for t in connected {
        cards.push(FkCard {
            columns: columns(pool, schema, &t).await?,
            table: t,
        });
    }

    Ok(FkGraph {
        schema: schema.to_string(),
        cards,
        edges,
        total_tables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_display_strips_the_create_prefix() {
        assert_eq!(
            index_display(
                "CREATE INDEX idx_events_user_created ON public.events USING btree (user_id, created_at DESC)",
                false
            ),
            "btree (user_id, created_at DESC)"
        );
    }

    #[test]
    fn index_display_marks_unique_indexes() {
        assert_eq!(
            index_display(
                "CREATE UNIQUE INDEX events_pkey ON public.events USING btree (event_id)",
                true
            ),
            "btree (event_id) · unique"
        );
    }

    #[test]
    fn index_display_handles_brin() {
        assert_eq!(
            index_display(
                "CREATE INDEX brin_events_created_at ON public.events USING brin (created_at)",
                false
            ),
            "brin (created_at)"
        );
    }

    #[test]
    fn index_display_falls_back_when_there_is_no_using_clause() {
        assert_eq!(
            index_display("something unexpected", false),
            "something unexpected"
        );
    }

    #[test]
    fn regclass_literal_is_injection_safe() {
        // A table named with a quote must not break out of the literal.
        let lit = regclass_literal("public", r#"ev"il"#);
        assert_eq!(lit, r#"'"public"."ev""il"'"#);
    }
}
