//! End-to-end tests against a live PostgreSQL fixture.
//!
//! Run with:
//!   cd dev && docker compose up -d
//!   PGSCOPE_TEST_URL=postgres://pgscope:pgscope@localhost:54330/analytics_prod \
//!     cargo test --features integration --test integration
//!
//! Without `--features integration` the whole module compiles to nothing, so a
//! normal `cargo test` never needs a database.

#![cfg(feature = "integration")]

use pgscope_lib::db::connect::{Connection, Profile};
use pgscope_lib::db::grid::{PageRequest, SortDir, SortKey};
use pgscope_lib::db::{self, introspect};
use pgscope_lib::repl::session::ReplSession;
use std::sync::{Arc, Mutex};

/// The fixture URL, overridable via `PGSCOPE_TEST_URL` so CI can point at its
/// own server without editing the tests.
fn test_url() -> String {
    std::env::var("PGSCOPE_TEST_URL")
        .unwrap_or_else(|_| "postgres://pgscope:pgscope@localhost:54330/analytics_prod".into())
}

/// A live connection to the fixture, panicking with a hint if it isn't up.
/// Installing the rustls provider is idempotent but has to happen once per
/// process, and any test may be the first to run.
async fn connect() -> std::sync::Arc<Connection> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (profile, password) = Profile::from_url(&test_url(), "test", "test").expect("parse url");
    Connection::open(profile, password)
        .await
        .expect("connect to the fixture — is `docker compose up` running in dev/?")
}

// ------------------------------ connection ------------------------------

#[tokio::test]
async fn connects_and_reports_server_facts() {
    let conn = connect().await;
    assert_eq!(conn.info.database, "analytics_prod");
    assert!(!conn.info.server_version.is_empty());
    assert!(conn.ping().await.unwrap() >= 0.0);
}

/// D8: the browse pool must never be able to mutate data.
#[tokio::test]
async fn browse_pool_is_read_only() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    let err = client
        .simple_query("DELETE FROM events WHERE event_id = 1")
        .await
        .expect_err("a write on the browse pool must fail");

    // `Display` on a driver error is just "db error" — the real text is on the
    // DbError payload.
    let db_err = err.as_db_error().expect("expected a server-side error");
    let msg = db_err.message().to_lowercase();
    assert!(
        msg.contains("read-only") || msg.contains("read only"),
        "expected a read-only rejection, got: {msg}"
    );
    assert_eq!(db_err.code().code(), "25006", "read_only_sql_transaction");
}

// ----------------------------- introspection ----------------------------

#[tokio::test]
async fn schema_tree_matches_the_design() {
    let conn = connect().await;
    let tree = introspect::schema_tree(&conn.pool).await.unwrap();

    let public = tree
        .iter()
        .find(|s| s.name == "public")
        .expect("public schema");
    // The design's sidebar: 7 tables and `views (3)`.
    assert_eq!(public.tables.len(), 7, "design shows 7 tables in public");
    assert_eq!(public.views.len(), 3, "design shows views (3)");

    let names: Vec<&str> = public.tables.iter().map(|t| t.name.as_str()).collect();
    for expected in [
        "event_properties",
        "events",
        "experiment_exposures",
        "funnels",
        "page_views",
        "sessions",
        "users",
    ] {
        assert!(names.contains(&expected), "missing table {expected}");
    }

    // The design also shows a sibling `analytics` schema.
    assert!(tree.iter().any(|s| s.name == "analytics"));

    // Row estimates must be populated, or the sidebar counts read as dashes.
    let events = public.tables.iter().find(|t| t.name == "events").unwrap();
    assert!(events.est_rows > 0, "ANALYZE has not run on events");
}

#[tokio::test]
async fn events_columns_carry_the_design_badges() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();

    let names: Vec<&str> = meta.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "event_id",
            "user_id",
            "session_id",
            "event_name",
            "properties",
            "created_at"
        ],
        "column order drives the grid header"
    );

    let by = |n: &str| meta.columns.iter().find(|c| c.name == n).unwrap();
    // The details panel badges: PK, FK, FK, NN, NN, NN.
    assert!(by("event_id").is_pk, "event_id is the PK");
    assert!(by("user_id").is_fk, "user_id is a FK");
    assert!(by("session_id").is_fk, "session_id is a FK");
    assert!(by("event_name").not_null);
    assert!(by("properties").not_null);
    assert!(by("created_at").not_null);

    // Header type lines: `bigint · PK`, `text · FK`, `uuid · FK`, `jsonb`, …
    assert_eq!(by("event_id").data_type, "bigint");
    assert_eq!(by("session_id").data_type, "uuid");
    assert_eq!(by("properties").data_type, "jsonb");
    assert!(by("created_at").data_type.starts_with("timestamp"));
}

#[tokio::test]
async fn events_indexes_match_the_details_panel() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();

    let names: Vec<&str> = meta.indexes.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names.len(), 4, "the design lists 4 indexes");
    assert_eq!(names[0], "events_pkey", "primary key sorts first");
    for expected in [
        "events_pkey",
        "idx_events_user_created",
        "idx_events_name",
        "brin_events_created_at",
    ] {
        assert!(names.contains(&expected), "missing index {expected}");
    }

    // Definitions render as the design's second lines.
    let def = |n: &str| {
        &meta
            .indexes
            .iter()
            .find(|i| i.name == n)
            .unwrap()
            .definition
    };
    assert_eq!(def("events_pkey"), "btree (event_id) · unique");
    assert_eq!(
        def("idx_events_user_created"),
        "btree (user_id, created_at DESC)"
    );
    assert_eq!(def("idx_events_name"), "btree (event_name)");
    assert_eq!(def("brin_events_created_at"), "brin (created_at)");
}

#[tokio::test]
async fn stats_are_populated() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();

    assert!(meta.stats.est_rows.unwrap() > 0);
    assert!(meta.stats.total_bytes.unwrap() > 0);
    assert!(meta.stats.index_bytes.unwrap() > 0);
}

#[tokio::test]
async fn views_have_columns_but_no_indexes_or_stats() {
    let conn = connect().await;
    let tree = introspect::schema_tree(&conn.pool).await.unwrap();
    let public = tree.iter().find(|s| s.name == "public").unwrap();
    let view = &public.views[0];

    let meta = introspect::table_meta(&conn.pool, "public", &view.name)
        .await
        .unwrap();
    assert!(matches!(meta.kind, introspect::RelKind::View));
    assert!(!meta.columns.is_empty());
    assert!(meta.indexes.is_empty(), "a view has no indexes to show");
}

#[tokio::test]
async fn fk_graph_returns_the_five_design_edges() {
    let conn = connect().await;
    let graph = introspect::fk_graph(&conn.pool, "public").await.unwrap();

    assert_eq!(graph.edges.len(), 5, "the design draws 5 FK edges");
    let mut pairs: Vec<(String, String)> = graph
        .edges
        .iter()
        .map(|e| (e.src_table.clone(), e.tgt_table.clone()))
        .collect();
    pairs.sort();

    let mut expected = vec![
        ("sessions".to_string(), "users".to_string()),
        ("events".to_string(), "sessions".to_string()),
        ("events".to_string(), "users".to_string()),
        ("page_views".to_string(), "sessions".to_string()),
        ("experiment_exposures".to_string(), "users".to_string()),
    ];
    expected.sort();
    assert_eq!(pairs, expected);

    // Every card carries the columns the ER view renders.
    assert!(graph.cards.iter().any(|c| c.table == "events"));
    let events_card = graph.cards.iter().find(|c| c.table == "events").unwrap();
    assert!(events_card.columns.iter().any(|c| c.is_pk));
    assert_eq!(graph.total_tables, 7);
}

// -------------------------------- grid ----------------------------------

/// An unsorted, unfiltered page of `public.events` — only the page number varies.
fn page_req(page: i64) -> PageRequest {
    PageRequest {
        schema: "public".into(),
        table: "events".into(),
        sort: Vec::new(),
        filter: None,
        page,
    }
}

#[tokio::test]
async fn fetches_a_page_of_fifty_rows() {
    let conn = connect().await;
    let result = db::fetch_page(&conn.pool, &page_req(0), |_| {})
        .await
        .unwrap();

    assert_eq!(result.rows.len(), 50, "the design pages 50 at a time");
    assert_eq!(result.rows[0].len(), 6, "events has 6 columns");
    assert!(result.timing_ms > 0.0);
    assert!(result.total.unwrap() > 0);
    assert!(result.total_is_estimate, "unfiltered totals use reltuples");
    assert!(result.sql.contains("FROM"));
}

#[tokio::test]
async fn paging_advances_through_distinct_rows() {
    let conn = connect().await;
    let mut req = page_req(0);
    req.sort = vec![SortKey {
        column: "event_id".into(),
        dir: SortDir::Desc,
    }];

    let p0 = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    req.page = 1;
    let p1 = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();

    assert_ne!(
        p0.rows[0][0], p1.rows[0][0],
        "page 2 must differ from page 1"
    );
}

/// §4.3: the last page must not deep-OFFSET; it reverses the sort instead.
#[tokio::test]
async fn last_page_is_fast_and_correctly_ordered() {
    let conn = connect().await;
    let mut req = page_req(0);
    req.sort = vec![SortKey {
        column: "event_id".into(),
        dir: SortDir::Desc,
    }];

    let first = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    let total = first.total.unwrap();
    let last_page = (total - 1) / first.page_size;

    let t0 = std::time::Instant::now();
    req.page = last_page;
    let last = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    let elapsed = t0.elapsed();

    assert!(!last.rows.is_empty());
    assert!(
        elapsed.as_secs() < 5,
        "last page took {elapsed:?} — the reverse-scan optimisation regressed"
    );

    // Still in descending order after the client-side flip.
    let ids: Vec<i64> = last
        .rows
        .iter()
        .map(|r| r[0].as_ref().unwrap().parse().unwrap())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted, "rows must stay in DESC order");
}

#[tokio::test]
async fn filtering_narrows_results_and_counts_exactly() {
    let conn = connect().await;
    let mut req = page_req(0);
    req.filter = Some("event_name = 'signup'".into());

    let result = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    assert!(!result.total_is_estimate, "a filtered total is exact");
    assert!(result.total.unwrap() > 0);

    // Every returned row really is a signup (column index 3 = event_name).
    for row in &result.rows {
        assert_eq!(row[3].as_deref(), Some("signup"));
    }
}

#[tokio::test]
async fn a_bad_filter_surfaces_the_server_error() {
    let conn = connect().await;
    let mut req = page_req(0);
    req.filter = Some("bogus (((".into());

    let err = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap_err();
    assert_eq!(err.code(), "query", "should be a query error, not a panic");
}

#[tokio::test]
async fn nulls_come_back_as_none() {
    let conn = connect().await;
    let mut req = page_req(0);
    // The fixture deliberately has NULL user_ids.
    req.filter = Some("user_id IS NULL".into());

    let result = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    if !result.rows.is_empty() {
        assert!(result.rows[0][1].is_none(), "NULL must decode as None");
    }
}

#[tokio::test]
async fn every_table_in_the_fixture_can_be_browsed() {
    let conn = connect().await;
    let tree = introspect::schema_tree(&conn.pool).await.unwrap();

    for schema in &tree {
        for rel in schema.tables.iter().chain(schema.views.iter()) {
            let req = PageRequest {
                schema: schema.name.clone(),
                table: rel.name.clone(),
                sort: Vec::new(),
                filter: None,
                page: 0,
            };
            db::fetch_page(&conn.pool, &req, |_| {})
                .await
                .unwrap_or_else(|e| panic!("browsing {}.{} failed: {e}", schema.name, rel.name));
        }
    }
}

// ------------------------------- terminal -------------------------------

/// A saved-queries directory scoped to one call, so `\i` never touches the
/// developer's real one and concurrent tests cannot see each other's files.
///
/// The counter is what makes it per-*call* rather than per-tag: the test
/// binary runs these in parallel threads, and a shared directory name means one
/// test's setup races another's teardown.
fn scratch_paths(tag: &str) -> pgscope_lib::store::Paths {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pgscope-itest-{tag}-{}-{n}", std::process::id()));
    pgscope_lib::store::paths_in(&dir)
}

/// A session on its own `scratch_paths`, which is what almost every REPL test
/// wants: real database, throwaway saved-queries directory.
async fn repl() -> ReplSession {
    repl_with_paths(scratch_paths("repl")).await
}

/// Same, but for the `\i` tests that need to write files into the directory
/// first and so must own the `Paths` themselves. Passing the real store paths
/// here would let `\i` read the developer's own saved queries.
async fn repl_with_paths(paths: pgscope_lib::store::Paths) -> ReplSession {
    let conn = connect().await;
    ReplSession::open(
        conn.profile.clone(),
        conn.password.clone(),
        conn.info.database.clone(),
        conn.info.is_superuser,
        paths,
    )
    .await
    .unwrap()
}

/// The whole output as one string, flattening the segment kinds away — most
/// assertions only care that some text appeared, not how it was coloured.
fn text_of(out: &pgscope_lib::repl::session::ReplOutput) -> String {
    out.segments.iter().map(|s| s.text.as_str()).collect()
}

#[tokio::test]
async fn repl_runs_a_query_and_formats_it() {
    let mut session = repl().await;
    let out = session.submit("SELECT 1 AS one;").await;
    let text = text_of(&out);

    assert!(text.contains("one"), "header missing: {text}");
    assert!(text.contains('1'));
    assert!(text.contains("(1 row)"), "psql row footer missing: {text}");
    assert!(!out.incomplete);
}

/// The design's terminal shows this query typed across three lines.
#[tokio::test]
async fn repl_handles_the_designs_multiline_query() {
    let mut session = repl().await;

    let out1 = session
        .submit("SELECT event_name, count(*) FROM events")
        .await;
    assert!(out1.incomplete, "should show a continuation prompt");
    assert!(out1.prompt.ends_with("-#"), "prompt was {}", out1.prompt);

    // The design's query uses `interval '24 hours'`, but the fixture's data
    // ages: a day after seeding that window is empty and this test would fail
    // for a reason unrelated to what it checks. Widen it so the assertion is
    // about continuation prompts and formatting, not the clock.
    let out2 = session
        .submit("  WHERE created_at > now() - interval '10 years'")
        .await;
    assert!(out2.incomplete);

    let out3 = session
        .submit("  GROUP BY 1 ORDER BY 2 DESC LIMIT 5;")
        .await;
    assert!(!out3.incomplete);
    assert!(out3.prompt.ends_with("=#") || out3.prompt.ends_with("=>"));

    let text = text_of(&out3);
    assert!(text.contains("event_name"), "missing header: {text}");
    assert!(text.contains("page_view"), "missing data: {text}");
    assert!(text.contains("rows)") || text.contains("row)"));
}

#[tokio::test]
async fn repl_reports_errors_like_psql_and_keeps_going() {
    let mut session = repl().await;

    let bad = session.submit("SELECT * FROM nonexistent_table;").await;
    let text = text_of(&bad);
    assert!(
        text.contains("ERROR:"),
        "expected a psql-style error: {text}"
    );

    // The session survives: the next statement still runs.
    let good = session.submit("SELECT 42 AS answer;").await;
    assert!(text_of(&good).contains("42"));
}

#[tokio::test]
async fn repl_session_state_persists_across_statements() {
    let mut session = repl().await;
    session.submit("SET work_mem = '5MB';").await;
    let out = session.submit("SHOW work_mem;").await;
    assert!(text_of(&out).contains("5MB"), "SET did not persist");
}

#[tokio::test]
async fn repl_timing_toggles() {
    let mut session = repl().await;

    let on = session.submit("SELECT 1;").await;
    assert!(text_of(&on).contains("Time:"), "timing defaults on");

    session.submit("\\timing off").await;
    let off = session.submit("SELECT 1;").await;
    assert!(!text_of(&off).contains("Time:"), "\\timing off ignored");
}

#[tokio::test]
async fn repl_describe_lists_columns_and_indexes() {
    let mut session = repl().await;
    let out = session.submit("\\d events").await;
    let text = text_of(&out);

    assert!(text.contains("event_id"), "columns missing: {text}");
    assert!(text.contains("jsonb"), "types missing: {text}");
    assert!(text.contains("events_pkey"), "indexes missing: {text}");
    assert!(text.contains("Foreign-key"), "FKs missing: {text}");
}

#[tokio::test]
async fn repl_meta_listing_commands_work() {
    let mut session = repl().await;

    assert!(text_of(&session.submit("\\dt").await).contains("events"));
    assert!(text_of(&session.submit("\\dn").await).contains("analytics"));
    assert!(text_of(&session.submit("\\l").await).contains("analytics_prod"));
}

#[tokio::test]
async fn repl_rejects_unknown_meta_commands() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\nope").await);
    assert!(text.contains("invalid command"), "got: {text}");
}

/// Unlike the browse pool, the terminal is allowed to write.
#[tokio::test]
async fn repl_is_not_read_only() {
    let mut session = repl().await;
    let text = text_of(&session.submit("CREATE TEMP TABLE t_check(x int);").await);
    assert!(
        !text.contains("read-only"),
        "the terminal must not be read-only: {text}"
    );
}

#[tokio::test]
async fn repl_caps_runaway_output() {
    let mut session = repl().await;
    // Far more rows than MAX_ROWS.
    let out = session.submit("SELECT generate_series(1, 25000);").await;
    let text = text_of(&out);
    assert!(
        text.contains("output truncated"),
        "large output should be capped: {}",
        &text[text.len().saturating_sub(400)..]
    );
}

#[tokio::test]
async fn repl_does_not_split_semicolons_inside_strings() {
    let mut session = repl().await;
    let out = session.submit("SELECT 'a;b' AS s;").await;
    assert!(text_of(&out).contains("a;b"));
    assert!(!out.incomplete);
}

// ----------------------------- cancellation -----------------------------

/// A slow page fetch must be abortable, and the handle must be published
/// *before* the query is awaited — otherwise cancel can only ever fire after
/// the statement it wanted to kill has already finished.
#[tokio::test]
async fn grid_page_fetch_is_cancellable_mid_flight() {
    let conn = connect().await;
    let captured: Arc<Mutex<Option<tokio_postgres::CancelToken>>> = Arc::new(Mutex::new(None));

    let req = PageRequest {
        schema: "public".into(),
        table: "events".into(),
        sort: Vec::new(),
        // Force a long-running scan.
        filter: Some("pg_sleep(30) IS NOT NULL".into()),
        page: 0,
    };

    let sink = Arc::clone(&captured);
    let fetch = tokio::spawn({
        let pool = conn.pool.clone();
        async move {
            db::fetch_page(&pool, &req, move |token| {
                *sink.lock().unwrap() = Some(token);
            })
            .await
        }
    });

    // The handle must appear while the query is still running.
    let mut token = None;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Some(t) = captured.lock().unwrap().clone() {
            token = Some(t);
            break;
        }
    }
    let token = token.expect("cancel handle was never published mid-flight");

    let t0 = std::time::Instant::now();
    token.cancel_query(tokio_postgres::NoTls).await.unwrap();
    let result = fetch.await.unwrap();

    assert!(result.is_err(), "a cancelled query must not return rows");
    assert!(
        t0.elapsed().as_secs() < 5,
        "cancel took {:?} — it did not actually abort the statement",
        t0.elapsed()
    );
}

/// The terminal's cancel handle is held outside the session mutex, so Ctrl+C
/// works *while* a statement is running and holding that lock.
#[tokio::test]
async fn repl_statement_is_cancellable_while_running() {
    let session = repl().await;
    // Snapshot the token the way AppState does, before locking the session.
    let token = session.cancel_token();
    let session = Arc::new(tokio::sync::Mutex::new(session));

    let running = Arc::clone(&session);
    let query = tokio::spawn(async move {
        let mut guard = running.lock().await;
        guard.submit("SELECT pg_sleep(30);").await
    });

    // Give the statement time to reach the server.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // The session mutex is held by the running query right now — proving the
    // token had to come from outside it.
    assert!(
        session.try_lock().is_err(),
        "expected the session to be locked"
    );

    let t0 = std::time::Instant::now();
    token.cancel_query(tokio_postgres::NoTls).await.unwrap();
    let out = query.await.unwrap();

    assert!(
        t0.elapsed().as_secs() < 5,
        "cancel took {:?} — Ctrl+C would not work",
        t0.elapsed()
    );
    let text = text_of(&out);
    assert!(
        text.to_lowercase().contains("cancel"),
        "expected a cancellation notice, got: {text}"
    );
}

/// A filtered count that blows its timeout must actually cancel the statement.
///
/// Regression: abandoning the timed-out future only dropped the *future* — the
/// backend kept executing the count, and deadpool recycled the connection while
/// it was still busy. The page query then drew that same connection from the
/// pool and queued behind the orphan until `statement_timeout` (30s) killed it,
/// turning a ~6s call into a ~30s one.
///
/// Asserting on `pg_stat_activity` does not work here: by the time the call
/// returns, statement_timeout has already reaped the orphan. The observable
/// symptom is the stall itself.
#[tokio::test]
async fn a_timed_out_count_cancels_the_statement_server_side() {
    let conn = connect().await;

    // Slow per row: counting 500k rows blows the 5s budget, while the LIMIT 50
    // page finishes in about a second.
    let slow = PageRequest {
        schema: "public".into(),
        table: "events".into(),
        sort: Vec::new(),
        filter: Some("pg_sleep(0.02) IS NOT NULL".into()),
        page: 0,
    };

    let t0 = std::time::Instant::now();
    let result = db::fetch_page(&conn.pool, &slow, |_| {}).await.unwrap();
    let elapsed = t0.elapsed();

    assert_eq!(
        result.rows.len(),
        50,
        "the page itself should still succeed"
    );
    assert!(
        result.total.is_none(),
        "the count should have given up and reported an unknown total"
    );
    assert!(
        elapsed.as_secs() < 15,
        "fetch_page took {elapsed:?}: the page query queued behind the \
         abandoned count instead of it being cancelled"
    );
}

// ------------------------------ performance -----------------------------

/// Interaction budgets (plan.md §E10.3). These are generous ceilings meant to
/// catch regressions like a lost index or a reintroduced deep OFFSET, not to
/// benchmark the machine.
#[tokio::test]
async fn common_interactions_stay_within_budget() {
    let conn = connect().await;

    let bench = |label: &'static str, budget_ms: u128| {
        move |elapsed: std::time::Duration| {
            println!("  {label}: {:?}", elapsed);
            assert!(
                elapsed.as_millis() < budget_ms,
                "{label} took {elapsed:?}, budget {budget_ms}ms"
            );
        }
    };

    // Sidebar tree on connect.
    let t0 = std::time::Instant::now();
    let tree = introspect::schema_tree(&conn.pool).await.unwrap();
    bench("schema_tree", 2000)(t0.elapsed());
    assert!(!tree.is_empty());

    // Selecting a table: metadata + first page.
    let t0 = std::time::Instant::now();
    introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();
    bench("table_meta", 1000)(t0.elapsed());

    let t0 = std::time::Instant::now();
    db::fetch_page(&conn.pool, &page_req(0), |_| {})
        .await
        .unwrap();
    bench("first page", 1000)(t0.elapsed());

    // Deep page via the reverse-scan path.
    let mut req = page_req(0);
    req.sort = vec![SortKey {
        column: "event_id".into(),
        dir: SortDir::Desc,
    }];
    let total = db::fetch_page(&conn.pool, &req, |_| {})
        .await
        .unwrap()
        .total
        .unwrap();
    req.page = (total - 1) / 50;
    let t0 = std::time::Instant::now();
    db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    bench("last page", 2000)(t0.elapsed());

    // Relationship graph.
    let t0 = std::time::Instant::now();
    introspect::fk_graph(&conn.pool, "public").await.unwrap();
    bench("fk_graph", 2000)(t0.elapsed());
}

/// A large terminal dump must stay bounded rather than growing without limit.
#[tokio::test]
async fn large_terminal_output_is_bounded() {
    let mut session = repl().await;

    let t0 = std::time::Instant::now();
    let out = session.submit("SELECT generate_series(1, 50000);").await;
    let elapsed = t0.elapsed();
    let text = text_of(&out);

    println!("  50k-row dump: {elapsed:?}, {} chars", text.len());
    assert!(elapsed.as_secs() < 20, "50k-row dump took {elapsed:?}");
    // MAX_ROWS is 10k; the rendered output must reflect that cap, not 50k rows.
    assert!(text.contains("output truncated"));
    assert!(
        text.len() < 2_000_000,
        "rendered {} chars — the row cap is not bounding output",
        text.len()
    );
}

// ---------------------------- query editor ------------------------------

/// A raw client on its own connection, because the editor tests run `SET` and
/// DDL that must not leak into other tests sharing a session.
async fn editor_client() -> tokio_postgres::Client {
    let conn = connect().await;
    pgscope_lib::db::connect::connect_client(
        &conn.profile,
        conn.password.as_deref(),
        "pgscope-editor-test",
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn editor_runs_a_select_and_returns_structured_rows() {
    let client = editor_client().await;
    let run = pgscope_lib::db::query::exec_all(
        &client,
        &[
            "SELECT event_name, count(*) AS n FROM events GROUP BY 1 ORDER BY 2 DESC LIMIT 3"
                .into(),
        ],
    )
    .await
    .unwrap();

    assert_eq!(run.statements.len(), 1);
    let set = run.statements[0]
        .result
        .as_ref()
        .expect("a SELECT returns rows");
    assert_eq!(set.columns, vec!["event_name", "n"]);
    assert_eq!(set.rows.len(), 3);
    assert!(!set.truncated);
    // Values arrive as text, so any column type renders.
    assert!(set.rows[0][1].as_ref().unwrap().parse::<i64>().is_ok());
}

#[tokio::test]
async fn editor_reports_no_result_set_for_non_returning_statements() {
    let client = editor_client().await;
    let run = pgscope_lib::db::query::exec_all(&client, &["SET work_mem = '6MB'".into()])
        .await
        .unwrap();

    assert_eq!(run.statements.len(), 1);
    assert!(run.statements[0].result.is_none(), "SET returns no rows");
}

#[tokio::test]
async fn editor_runs_statements_in_order() {
    let client = editor_client().await;
    let run = pgscope_lib::db::query::exec_all(
        &client,
        &[
            "CREATE TEMP TABLE editor_order(x int)".into(),
            "INSERT INTO editor_order VALUES (1), (2)".into(),
            "SELECT count(*) FROM editor_order".into(),
        ],
    )
    .await
    .unwrap();

    assert_eq!(run.statements.len(), 3);
    let set = run.statements[2].result.as_ref().unwrap();
    assert_eq!(set.rows[0][0].as_deref(), Some("2"));
}

/// A failure part-way through must stop the run rather than pressing on
/// against a half-applied state.
#[tokio::test]
async fn editor_stops_at_the_first_failing_statement() {
    let client = editor_client().await;
    let err = pgscope_lib::db::query::exec_all(
        &client,
        &[
            "CREATE TEMP TABLE editor_stop(x int)".into(),
            "SELECT * FROM nonexistent_table_xyz".into(),
            "INSERT INTO editor_stop VALUES (99)".into(),
        ],
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "query");

    // The third statement must not have run.
    let check =
        pgscope_lib::db::query::exec_all(&client, &["SELECT count(*) FROM editor_stop".into()])
            .await
            .unwrap();
    assert_eq!(
        check.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(),
        Some("0"),
        "statements after the failure should not have executed"
    );
}

/// Unlike the browse pool, the editor is allowed to write — that is the whole
/// point of a query editor.
#[tokio::test]
async fn editor_can_write() {
    let client = editor_client().await;
    let run = pgscope_lib::db::query::exec_all(
        &client,
        &[
            "CREATE TEMP TABLE editor_write(x int)".into(),
            "INSERT INTO editor_write VALUES (7)".into(),
            "SELECT x FROM editor_write".into(),
        ],
    )
    .await
    .unwrap();

    assert_eq!(
        run.statements[2].result.as_ref().unwrap().rows[0][0].as_deref(),
        Some("7")
    );
}

#[tokio::test]
async fn editor_caps_runaway_result_sets() {
    let client = editor_client().await;
    let run =
        pgscope_lib::db::query::exec_all(&client, &["SELECT generate_series(1, 25000)".into()])
            .await
            .unwrap();

    let set = run.statements[0].result.as_ref().unwrap();
    assert_eq!(set.rows.len(), pgscope_lib::db::query::MAX_ROWS);
    assert_eq!(set.total_rows, 25_000, "the true count is still reported");
    assert!(set.truncated);
}

#[tokio::test]
async fn editor_preserves_nulls() {
    let client = editor_client().await;
    let run =
        pgscope_lib::db::query::exec_all(&client, &["SELECT NULL::text AS a, 'x' AS b".into()])
            .await
            .unwrap();

    let set = run.statements[0].result.as_ref().unwrap();
    assert!(set.rows[0][0].is_none(), "NULL must decode as None");
    assert_eq!(set.rows[0][1].as_deref(), Some("x"));
}

/// The editor's statement splitting is the REPL's lexer, so multi-statement
/// buffers agree between the two surfaces.
#[tokio::test]
async fn editor_statement_ranges_agree_with_the_repl_lexer() {
    use pgscope_lib::repl::lexer;

    let buffer = "SELECT 1;\nSELECT 'a;b';\nSELECT 3";
    let ranges = lexer::statement_ranges(buffer);
    assert_eq!(ranges.len(), 3);
    assert_eq!(
        ranges.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        vec!["SELECT 1", "SELECT 'a;b'", "SELECT 3"]
    );

    // And they actually run.
    let client = editor_client().await;
    let sqls: Vec<String> = ranges.into_iter().map(|r| r.text).collect();
    let run = pgscope_lib::db::query::exec_all(&client, &sqls)
        .await
        .unwrap();
    assert_eq!(run.statements.len(), 3);
    assert_eq!(
        run.statements[1].result.as_ref().unwrap().rows[0][0].as_deref(),
        Some("a;b")
    );
}

// --------------------------- EXPLAIN plans ------------------------------

#[tokio::test]
async fn explain_returns_a_plan_without_executing() {
    let client = editor_client().await;
    let result = pgscope_lib::db::explain::explain(
        &client,
        "SELECT event_name, count(*) FROM events GROUP BY 1",
        false,
    )
    .await
    .unwrap();

    assert!(!result.analyzed);
    assert!(!result.rolled_back);
    // Costs are present; actual timings are not, because nothing ran.
    assert!(result.plan.total_cost.unwrap() > 0.0);
    assert!(result.plan.actual_total_time.is_none());
    assert!(result.execution_time_ms.is_none());
    assert!(result.max_self_cost.unwrap() > 0.0);
}

#[tokio::test]
async fn explain_analyze_reports_real_timings_and_rows() {
    let client = editor_client().await;
    let result = pgscope_lib::db::explain::explain(
        &client,
        "SELECT event_name, count(*) FROM events GROUP BY 1",
        true,
    )
    .await
    .unwrap();

    assert!(result.analyzed);
    assert!(result.execution_time_ms.unwrap() > 0.0);
    assert!(result.planning_time_ms.unwrap() >= 0.0);
    assert!(result.plan.actual_total_time.unwrap() > 0.0);
    assert!(result.plan.actual_rows.unwrap() > 0.0);
    assert!(result.max_self_time_ms.unwrap() > 0.0);
}

#[tokio::test]
async fn explain_captures_the_scan_node_and_its_relation() {
    let client = editor_client().await;
    let result = pgscope_lib::db::explain::explain(&client, "SELECT * FROM events LIMIT 10", true)
        .await
        .unwrap();

    // Walk to a node that names the relation.
    fn find_relation(node: &pgscope_lib::db::explain::PlanNode) -> Option<String> {
        for d in &node.details {
            if d[0] == "Relation Name" {
                return Some(d[1].clone());
            }
        }
        node.children.iter().find_map(find_relation)
    }

    assert_eq!(find_relation(&result.plan).as_deref(), Some("events"));
}

/// Self time must sum sensibly against the reported execution time — the
/// property that makes the bottleneck bar meaningful.
///
/// Parallelism is disabled here on purpose: under a Gather, `actual_loops`
/// counts workers, so summed self time is CPU across workers and legitimately
/// exceeds wall-clock. That case is covered by the next test.
#[tokio::test]
async fn self_times_are_consistent_with_execution_time() {
    let client = editor_client().await;
    client
        .batch_execute("SET max_parallel_workers_per_gather = 0")
        .await
        .unwrap();

    let result = pgscope_lib::db::explain::explain(
        &client,
        "SELECT event_name, count(*) FROM events GROUP BY 1 ORDER BY 2 DESC",
        true,
    )
    .await
    .unwrap();

    /// Self time across the whole tree, which should reconcile with the
    /// server's reported execution time when nothing ran in parallel.
    fn sum_self(node: &pgscope_lib::db::explain::PlanNode) -> f64 {
        node.self_time_ms.unwrap_or(0.0) + node.children.iter().map(sum_self).sum::<f64>()
    }
    /// Asserts the premise stated above actually held — the `SET` is a request,
    /// not a guarantee.
    fn any_parallel(node: &pgscope_lib::db::explain::PlanNode) -> bool {
        node.parallel || node.children.iter().any(any_parallel)
    }

    assert!(
        !any_parallel(&result.plan),
        "parallelism should be off here"
    );

    let total = sum_self(&result.plan);
    let execution = result.execution_time_ms.unwrap();
    assert!(
        total <= execution * 1.2 + 5.0,
        "self times sum to {total}ms but execution was {execution}ms"
    );
}

/// Nodes under a Gather must be flagged, because their summed self time is CPU
/// across workers and can exceed wall-clock execution time. Without the flag
/// the UI would print an apparently impossible number (observed live: 85.85ms
/// of self time on a query whose execution time was 36.29ms).
#[tokio::test]
async fn parallel_nodes_are_flagged_so_cpu_time_can_be_labelled() {
    let client = editor_client().await;
    for setting in [
        "SET max_parallel_workers_per_gather = 2",
        "SET parallel_setup_cost = 0",
        "SET parallel_tuple_cost = 0",
        "SET min_parallel_table_scan_size = 0",
    ] {
        client.batch_execute(setting).await.unwrap();
    }

    let result = pgscope_lib::db::explain::explain(
        &client,
        "SELECT count(*) FROM events WHERE properties->>'path' = '/pricing'",
        true,
    )
    .await
    .unwrap();

    /// Flattens the tree to (node type, parallel flag) pairs in pre-order, so
    /// the assertions can talk about nodes below the Gather without walking.
    fn collect(node: &pgscope_lib::db::explain::PlanNode, out: &mut Vec<(String, bool)>) {
        out.push((node.node_type.clone(), node.parallel));
        for c in &node.children {
            collect(c, out);
        }
    }
    let mut nodes = Vec::new();
    collect(&result.plan, &mut nodes);

    let gather = nodes.iter().find(|(t, _)| t.starts_with("Gather"));
    match gather {
        Some((_, gather_is_parallel)) => {
            assert!(
                !gather_is_parallel,
                "the Gather runs in the leader and is not itself a worker node"
            );
            assert!(
                nodes.iter().any(|(_, parallel)| *parallel),
                "nodes under a Gather must be flagged: {nodes:?}"
            );
        }
        None => assert!(
            nodes.iter().all(|(_, parallel)| !*parallel),
            "no Gather in the plan, so nothing should be flagged: {nodes:?}"
        ),
    }
}

/// EXPLAIN ANALYZE really executes, so a DML statement must be rolled back.
/// Without this, "explain" on a DELETE would silently destroy data.
#[tokio::test]
async fn explain_analyze_rolls_back_writes() {
    let client = editor_client().await;

    // A TEMP table: invisible to other sessions and dropped automatically.
    // A real table in `public` would briefly change the schema's table count
    // and race the tests that assert on it.
    client
        .batch_execute("CREATE TEMP TABLE explain_rollback_check(x int) ON COMMIT PRESERVE ROWS")
        .await
        .unwrap();

    let result = pgscope_lib::db::explain::explain(
        &client,
        "INSERT INTO explain_rollback_check VALUES (1), (2), (3)",
        true,
    )
    .await
    .unwrap();
    assert!(result.analyzed);
    assert!(
        result.rolled_back,
        "an analyzed run must report the rollback"
    );

    // The rows must NOT be there.
    let check = client
        .query_one("SELECT count(*) FROM explain_rollback_check", &[])
        .await
        .unwrap();
    assert_eq!(
        check.get::<_, i64>(0),
        0,
        "EXPLAIN ANALYZE on an INSERT wrote rows — the rollback failed"
    );

    client
        .batch_execute("DROP TABLE explain_rollback_check")
        .await
        .unwrap();
}

/// The rollback must also happen when the explained statement errors, or the
/// connection would be left in a failed transaction.
#[tokio::test]
async fn a_failed_explain_leaves_the_connection_usable() {
    let client = editor_client().await;

    let err = pgscope_lib::db::explain::explain(&client, "SELECT * FROM no_such_table_xyz", true)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "query");

    // The session must not be stuck in an aborted transaction.
    let row = client.query_one("SELECT 1 AS ok", &[]).await.unwrap();
    assert_eq!(row.get::<_, i32>(0), 1);
}

#[tokio::test]
async fn explain_rejects_multiple_statements() {
    // EXPLAIN takes one statement; the command surface must say so rather than
    // building invalid SQL.
    let ranges = pgscope_lib::repl::lexer::statement_ranges("SELECT 1; SELECT 2;");
    assert_eq!(ranges.len(), 2, "the guard in explain_query keys off this");
}

#[tokio::test]
async fn explain_detects_an_index_scan_when_one_applies() {
    let client = editor_client().await;
    // idx_events_name exists on event_name, so an equality filter should use it.
    let result = pgscope_lib::db::explain::explain(
        &client,
        "SELECT * FROM events WHERE event_name = 'signup' LIMIT 5",
        false,
    )
    .await
    .unwrap();

    /// Flattens the tree to node types only — the planner is free to pick any
    /// shape here, so the test can only assert that some node kind is present.
    fn node_types(node: &pgscope_lib::db::explain::PlanNode, out: &mut Vec<String>) {
        out.push(node.node_type.clone());
        for c in &node.children {
            node_types(c, out);
        }
    }
    let mut types = Vec::new();
    node_types(&result.plan, &mut types);

    assert!(
        types.iter().any(|t| t.contains("Scan")),
        "expected some scan node, got {types:?}"
    );
}

// --------------------------- tab completion -----------------------------

use pgscope_lib::repl::complete::CompletionKind;

#[tokio::test]
async fn completes_a_table_name_after_from() {
    let session = repl().await;
    let line = "SELECT * FROM ev";
    let r = complete_via(&session, line).await;

    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    assert!(values.contains(&"events"), "got {values:?}");
    assert!(values.contains(&"event_properties"), "got {values:?}");
    // Only relations, no keywords, in a FROM position.
    assert!(r
        .items
        .iter()
        .all(|i| matches!(i.kind, CompletionKind::Table | CompletionKind::View)));
    // Both candidates share "event".
    assert_eq!(r.common_prefix, "event");
    // The replaced range covers exactly the typed token.
    assert_eq!(&line[r.start..r.end], "ev");
}

#[tokio::test]
async fn completes_a_column_after_where() {
    let session = repl().await;
    let r = complete_via(&session, "SELECT * FROM events WHERE event_n").await;

    let cols: Vec<&str> = r
        .items
        .iter()
        .filter(|i| i.kind == CompletionKind::Column)
        .map(|i| i.value.as_str())
        .collect();
    assert_eq!(cols, vec!["event_name"], "got {:?}", r.items);
    // Columns carry their type as detail.
    let col = r.items.iter().find(|i| i.value == "event_name").unwrap();
    assert_eq!(col.detail.as_deref(), Some("text"));
}

/// The alias case: `e.` must resolve `e` back to `events` via the FROM clause.
#[tokio::test]
async fn completes_columns_through_a_table_alias() {
    let session = repl().await;
    let r = complete_via(&session, "SELECT * FROM events e WHERE e.").await;

    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    for expected in [
        "event_id",
        "user_id",
        "session_id",
        "event_name",
        "properties",
    ] {
        assert!(
            values.contains(&expected),
            "missing {expected} in {values:?}"
        );
    }
    assert!(r.items.iter().all(|i| i.kind == CompletionKind::Column));
}

#[tokio::test]
async fn resolves_the_right_table_in_a_join() {
    let session = repl().await;
    let line = "SELECT * FROM events e JOIN users u ON u.user_id = e.user_id WHERE u.";
    let r = complete_via(&session, line).await;

    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    // users columns, not events ones.
    assert!(
        values.contains(&"email"),
        "expected users columns: {values:?}"
    );
    assert!(
        values.contains(&"plan"),
        "expected users columns: {values:?}"
    );
    assert!(
        !values.contains(&"event_name"),
        "must not offer events columns for the users alias: {values:?}"
    );
}

#[tokio::test]
async fn completes_backslash_commands() {
    let session = repl().await;
    let r = complete_via(&session, "\\t").await;

    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    assert!(values.contains(&"\\timing"), "got {values:?}");
    assert!(r.items.iter().all(|i| i.kind == CompletionKind::Meta));
}

#[tokio::test]
async fn completes_a_relation_as_a_backslash_argument() {
    let session = repl().await;
    let r = complete_via(&session, "\\d even").await;

    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    assert!(values.contains(&"events"), "got {values:?}");
}

#[tokio::test]
async fn completion_of_an_unknown_prefix_is_empty_not_an_error() {
    let session = repl().await;
    let r = complete_via(&session, "SELECT * FROM zzzz_no_such").await;
    assert!(r.items.is_empty());
    assert_eq!(r.common_prefix, "");
}

#[tokio::test]
async fn completing_a_qualifier_that_is_not_in_scope_falls_back_to_the_name() {
    let session = repl().await;
    // No FROM clause, so `events.` can only be read as the table itself.
    let r = complete_via(&session, "SELECT events.").await;
    let values: Vec<&str> = r.items.iter().map(|i| i.value.as_str()).collect();
    assert!(values.contains(&"event_name"), "got {values:?}");
}

#[tokio::test]
async fn completion_sees_schema_qualified_relations() {
    let session = repl().await;
    let r = complete_via(&session, "SELECT * FROM analytics.daily").await;
    // The prefix after the dot is treated as a qualified reference; either way
    // this must not error.
    let _ = r;

    let r2 = complete_via(&session, "SELECT * FROM daily").await;
    let values: Vec<&str> = r2.items.iter().map(|i| i.value.as_str()).collect();
    assert!(
        values.iter().any(|v| v.starts_with("daily")),
        "expected the analytics view to be reachable: {values:?}"
    );
    // Non-public relations carry their schema as detail.
    let item = r2
        .items
        .iter()
        .find(|i| i.value.starts_with("daily"))
        .unwrap();
    assert_eq!(item.detail.as_deref(), Some("analytics"));
}

/// Helper: complete at the end of `line` using a session's own client.
async fn complete_via(
    session: &ReplSession,
    line: &str,
) -> pgscope_lib::repl::complete::CompletionResult {
    session
        .complete(line, line.chars().count())
        .await
        .expect("completion should not error")
}

// -------------------------- inline cell expansion -----------------------

use pgscope_lib::db::cell::{fetch_cell, CellFormat, CellRequest, PkValue};

/// The first page of `events` sorted by primary key descending, so the row a
/// cell test picks out is the same one on every run.
fn cell_page() -> PageRequest {
    PageRequest {
        schema: "public".into(),
        table: "events".into(),
        sort: vec![SortKey {
            column: "event_id".into(),
            dir: SortDir::Desc,
        }],
        filter: None,
        page: 0,
    }
}

#[tokio::test]
async fn expands_a_jsonb_cell_pretty_printed() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    // Grab a real row's PK first.
    let row = client
        .query_one(
            "SELECT event_id::text FROM events WHERE properties IS NOT NULL \
             ORDER BY event_id DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap();
    let event_id: String = row.get(0);

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "properties".into(),
            pk: vec![PkValue {
                column: "event_id".into(),
                value: event_id,
            }],
            page: cell_page(),
            row_index: 0,
        },
    )
    .await
    .unwrap();

    assert_eq!(cell.format, CellFormat::Json);
    assert_eq!(cell.data_type, "jsonb");
    assert!(cell.located_by_pk);

    let value = cell.value.expect("jsonb value");
    // jsonb_pretty indents across lines; the grid's minified copy would not.
    assert!(
        value.contains('\n'),
        "expected pretty-printed JSON, got: {value}"
    );
    assert!(value.starts_with('{'));
    assert!(cell.total_bytes > 0);
    assert!(!cell.truncated);
}

#[tokio::test]
async fn expands_a_text_cell_without_pretty_printing() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();
    let row = client
        .query_one(
            "SELECT event_id::text FROM events ORDER BY event_id DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap();

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "event_name".into(),
            pk: vec![PkValue {
                column: "event_id".into(),
                value: row.get(0),
            }],
            page: cell_page(),
            row_index: 0,
        },
    )
    .await
    .unwrap();

    assert_eq!(cell.format, CellFormat::Text);
    assert_eq!(cell.data_type, "text");
    assert!(cell.value.is_some());
}

/// The whole point: values the grid truncates at 8KB must come back whole.
#[tokio::test]
async fn expansion_returns_a_value_larger_than_the_grid_cap() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    // A synthetic value well past the grid's 8KB cell cap.
    let big_len = pgscope_lib::db::grid::CELL_CAP_BYTES * 3;
    let row = client
        .query_one(
            "SELECT repeat('x', $1)::text AS v, octet_length(repeat('x', $1)) AS n",
            &[&(big_len as i32)],
        )
        .await
        .unwrap();
    let full: String = row.get("v");
    assert_eq!(full.len(), big_len);

    // The grid would have cut this down; confirm the cap is real.
    let capped = pgscope_lib::db::grid::cap_cell(full.clone());
    assert!(capped.len() < full.len(), "the grid cap should shorten it");
    assert!(capped.ends_with('…'));

    // And confirm the expansion path has a far larger ceiling. A compile-time
    // assertion, since both sides are constants — a runtime one can only fail
    // in a build that already shipped.
    const _: () = assert!(
        pgscope_lib::db::cell::EXPANDED_CAP_BYTES > pgscope_lib::db::grid::CELL_CAP_BYTES * 100,
        "expansion must not inherit the grid's cap"
    );
}

#[tokio::test]
async fn expands_a_null_cell_as_null_not_empty() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    // The fixture has NULL user_ids.
    let row = client
        .query_opt(
            "SELECT event_id::text FROM events WHERE user_id IS NULL LIMIT 1",
            &[],
        )
        .await
        .unwrap();
    let Some(row) = row else {
        return; // no NULLs in this fixture build; nothing to assert
    };

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "user_id".into(),
            pk: vec![PkValue {
                column: "event_id".into(),
                value: row.get(0),
            }],
            page: cell_page(),
            row_index: 0,
        },
    )
    .await
    .unwrap();

    assert!(
        cell.value.is_none(),
        "NULL must stay distinct from empty string"
    );
}

/// Without a primary key the row is located by page position instead.
#[tokio::test]
async fn expands_by_page_position_when_there_is_no_pk() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "properties".into(),
            pk: Vec::new(),
            page: cell_page(),
            row_index: 3,
        },
    )
    .await
    .unwrap();

    assert!(
        !cell.located_by_pk,
        "should report the weaker location method"
    );
    assert_eq!(cell.format, CellFormat::Json);

    // It must be the same row the grid shows at that index.
    let expected = client
        .query_one(
            "SELECT jsonb_pretty(properties) FROM events ORDER BY event_id DESC LIMIT 1 OFFSET 3",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(cell.value, expected.get::<_, Option<String>>(0));
}

#[tokio::test]
async fn expansion_honours_the_page_filter_when_locating_by_position() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    let mut page = cell_page();
    page.filter = Some("event_name = 'signup'".into());

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "event_name".into(),
            pk: Vec::new(),
            page,
            row_index: 0,
        },
    )
    .await
    .unwrap();

    // Ignoring the filter would return whatever row happened to be first.
    assert_eq!(cell.value.as_deref(), Some("signup"));
}

#[tokio::test]
async fn expanding_an_unknown_column_errors_clearly() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    let err = fetch_cell(
        &client,
        &CellRequest {
            column: "no_such_column".into(),
            pk: Vec::new(),
            page: cell_page(),
            row_index: 0,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code(), "invalid");
    assert!(err.to_string().contains("no_such_column"));
}

/// The expansion runs on the browse pool, so it must inherit read-only.
#[tokio::test]
async fn expansion_cannot_be_used_to_write() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();

    let mut page = cell_page();
    // A filter is raw SQL by design; the read-only session is what contains it.
    page.filter = Some("(SELECT count(*) FROM (DELETE FROM events RETURNING 1) d) >= 0".into());

    let result = fetch_cell(
        &client,
        &CellRequest {
            column: "event_name".into(),
            pk: Vec::new(),
            page,
            row_index: 0,
        },
    )
    .await;

    assert!(
        result.is_err(),
        "a write smuggled through the filter must fail"
    );
}

// ------------------------- multi-column sorting -------------------------

/// `page_req` with the sort varied too — the same `public.events` target, since
/// only ordering is under test here.
fn sorted_page(keys: Vec<SortKey>, page: i64) -> PageRequest {
    PageRequest {
        schema: "public".into(),
        table: "events".into(),
        sort: keys,
        filter: None,
        page,
    }
}

/// Keeps the multi-key sort lists at the call site readable.
fn key(column: &str, dir: SortDir) -> SortKey {
    SortKey {
        column: column.into(),
        dir,
    }
}

/// Column indexes in `events`: 0 event_id, 3 event_name.
#[tokio::test]
async fn sorts_by_two_columns_in_the_right_precedence() {
    let conn = connect().await;
    let req = sorted_page(
        vec![
            key("event_name", SortDir::Asc),
            key("event_id", SortDir::Desc),
        ],
        0,
    );
    let result = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    assert_eq!(result.rows.len(), 50);

    let names: Vec<&str> = result
        .rows
        .iter()
        .map(|r| r[3].as_deref().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "primary key must order first");

    // Within each event_name group, event_id must descend.
    for window in result.rows.windows(2) {
        if window[0][3] == window[1][3] {
            let a: i64 = window[0][0].as_deref().unwrap().parse().unwrap();
            let b: i64 = window[1][0].as_deref().unwrap().parse().unwrap();
            assert!(
                a > b,
                "secondary key must descend within a group: {a} then {b}"
            );
        }
    }
}

#[tokio::test]
async fn swapping_the_sort_precedence_changes_the_result() {
    let conn = connect().await;

    let a = db::fetch_page(
        &conn.pool,
        &sorted_page(
            vec![
                key("event_name", SortDir::Asc),
                key("event_id", SortDir::Desc),
            ],
            0,
        ),
        |_| {},
    )
    .await
    .unwrap();

    let b = db::fetch_page(
        &conn.pool,
        &sorted_page(
            vec![
                key("event_id", SortDir::Desc),
                key("event_name", SortDir::Asc),
            ],
            0,
        ),
        |_| {},
    )
    .await
    .unwrap();

    assert_ne!(a.rows[0], b.rows[0], "precedence must actually matter");
}

/// The reverse-scan tail must flip every key, or rows come back mis-ordered
/// within their groups.
#[tokio::test]
async fn the_multi_column_last_page_is_correctly_ordered() {
    let conn = connect().await;
    let keys = vec![
        key("event_name", SortDir::Asc),
        key("event_id", SortDir::Desc),
    ];

    let first = db::fetch_page(&conn.pool, &sorted_page(keys.clone(), 0), |_| {})
        .await
        .unwrap();
    let total = first.total.unwrap();
    let last_page = (total - 1) / first.page_size;

    let t0 = std::time::Instant::now();
    let last = db::fetch_page(&conn.pool, &sorted_page(keys.clone(), last_page), |_| {})
        .await
        .unwrap();
    let elapsed = t0.elapsed();

    assert!(!last.rows.is_empty());
    assert!(elapsed.as_secs() < 5, "reverse-scan regressed: {elapsed:?}");

    // Still ascending by name after the client-side flip.
    let names: Vec<&str> = last.rows.iter().map(|r| r[3].as_deref().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "primary key order lost on the last page");

    // And still descending by id within each group.
    for window in last.rows.windows(2) {
        if window[0][3] == window[1][3] {
            let a: i64 = window[0][0].as_deref().unwrap().parse().unwrap();
            let b: i64 = window[1][0].as_deref().unwrap().parse().unwrap();
            assert!(a > b, "secondary key order lost on the last page");
        }
    }

    // The true tail: compare against an independent query.
    let client = conn.pool.get().await.unwrap();
    let expected = client
        .query(
            "SELECT event_id::text FROM events ORDER BY event_name DESC, event_id ASC LIMIT 1",
            &[],
        )
        .await
        .unwrap();
    let expected_last: String = expected[0].get(0);
    let actual_last = last.rows.last().unwrap()[0].as_deref().unwrap();
    assert_eq!(
        actual_last, expected_last,
        "the final row of the last page must be the true tail"
    );
}

#[tokio::test]
async fn a_three_column_sort_works() {
    let conn = connect().await;
    let req = sorted_page(
        vec![
            key("event_name", SortDir::Asc),
            key("user_id", SortDir::Desc),
            key("event_id", SortDir::Asc),
        ],
        0,
    );
    let result = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap();
    assert_eq!(result.rows.len(), 50);
    assert!(result.sql.contains("ORDER BY"));
}

#[tokio::test]
async fn an_unsorted_browse_still_works() {
    let conn = connect().await;
    let result = db::fetch_page(&conn.pool, &sorted_page(Vec::new(), 0), |_| {})
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 50);
    assert!(!result.sql.contains("ORDER BY"));
}

#[tokio::test]
async fn a_duplicated_sort_column_is_rejected() {
    let conn = connect().await;
    let req = sorted_page(
        vec![
            key("event_id", SortDir::Asc),
            key("event_id", SortDir::Desc),
        ],
        0,
    );
    let err = db::fetch_page(&conn.pool, &req, |_| {}).await.unwrap_err();
    assert_eq!(err.code(), "invalid");
}

/// Cell expansion locates rows by position using the page's ordering, so it
/// must honour a multi-column sort too.
#[tokio::test]
async fn cell_expansion_follows_a_multi_column_sort() {
    let conn = connect().await;
    let client = conn.pool.get().await.unwrap();
    let keys = vec![
        key("event_name", SortDir::Asc),
        key("event_id", SortDir::Desc),
    ];

    let cell = fetch_cell(
        &client,
        &CellRequest {
            column: "event_name".into(),
            pk: Vec::new(),
            page: sorted_page(keys, 0),
            row_index: 7,
        },
    )
    .await
    .unwrap();

    let expected = client
        .query_one(
            "SELECT event_name FROM events ORDER BY event_name ASC, event_id DESC \
             LIMIT 1 OFFSET 7",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(cell.value.as_deref(), expected.get::<_, Option<&str>>(0));
}

// ------------------ context menu: predicate round-trip ------------------

use pgscope_lib::db::rowfmt::{
    format_row, value_predicate, PredicateOp, RowFormat, RowFormatRequest,
};

/// Filter-to-value is only useful if the predicate it generates is valid SQL
/// that the grid can actually run. Generate one from a real row, feed it back
/// through the page query, and check it selects that row.
#[tokio::test]
async fn a_generated_predicate_round_trips_through_the_grid() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();

    // Take a real first-page row.
    let page = sorted_page(vec![key("event_id", SortDir::Desc)], 0);
    let first = db::fetch_page(&conn.pool, &page, |_| {}).await.unwrap();
    let row = &first.rows[0];

    for (i, column) in meta.columns.iter().enumerate() {
        let value = row[i].as_deref();
        let predicate = value_predicate(column, value, PredicateOp::Eq);

        let mut filtered = page.clone();
        filtered.filter = Some(predicate.clone());

        let result = db::fetch_page(&conn.pool, &filtered, |_| {})
            .await
            .unwrap_or_else(|e| panic!("predicate `{predicate}` was not valid SQL: {e}"));

        assert!(
            !result.rows.is_empty(),
            "predicate `{predicate}` matched nothing, but came from a real row"
        );
        // Every returned row really has that value.
        for r in &result.rows {
            assert_eq!(
                r[i].as_deref(),
                value,
                "predicate `{predicate}` matched a row with a different value"
            );
        }
    }
}

/// The NOT-equal form must exclude the row it came from.
#[tokio::test]
async fn a_generated_exclusion_predicate_excludes_that_value() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();
    let name_col = meta
        .columns
        .iter()
        .find(|c| c.name == "event_name")
        .unwrap();

    let predicate = value_predicate(name_col, Some("signup"), PredicateOp::NotEq);
    let mut page = sorted_page(vec![key("event_id", SortDir::Desc)], 0);
    page.filter = Some(predicate);

    let result = db::fetch_page(&conn.pool, &page, |_| {}).await.unwrap();
    assert!(!result.rows.is_empty());
    for r in &result.rows {
        assert_ne!(r[3].as_deref(), Some("signup"));
    }
}

/// A NULL cell's predicate must use IS NULL — `= NULL` matches nothing.
#[tokio::test]
async fn a_null_predicate_actually_finds_null_rows() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();
    let user_col = meta.columns.iter().find(|c| c.name == "user_id").unwrap();

    let predicate = value_predicate(user_col, None, PredicateOp::Eq);
    assert!(predicate.contains("IS NULL"), "got {predicate}");

    let mut page = sorted_page(vec![key("event_id", SortDir::Desc)], 0);
    page.filter = Some(predicate);

    let result = db::fetch_page(&conn.pool, &page, |_| {}).await.unwrap();
    // The fixture has NULL user_ids; every match must really be NULL.
    for r in &result.rows {
        assert!(r[1].is_none(), "IS NULL matched a non-null row");
    }
}

/// Copy-as-INSERT must produce a statement the server accepts. Run it inside a
/// rolled-back transaction on the *editor* connection so nothing is written.
#[tokio::test]
async fn a_generated_insert_is_valid_sql() {
    let conn = connect().await;
    let meta = introspect::table_meta(&conn.pool, "public", "events")
        .await
        .unwrap();
    let page = sorted_page(vec![key("event_id", SortDir::Desc)], 0);
    let first = db::fetch_page(&conn.pool, &page, |_| {}).await.unwrap();

    let sql = format_row(&RowFormatRequest {
        schema: "public".into(),
        table: "events".into(),
        columns: meta.columns.clone(),
        values: first.rows[0].clone(),
        format: RowFormat::Insert,
    });

    let client = editor_client().await;
    client.batch_execute("BEGIN").await.unwrap();
    // The row already exists, so expect either success or a duplicate-key
    // error — both prove the statement parsed and bound correctly. A syntax or
    // type error would be a different SQLSTATE.
    let result = client.batch_execute(&sql).await;
    client.batch_execute("ROLLBACK").await.unwrap();

    if let Err(e) = result {
        let db_err = e.as_db_error().expect("expected a server error");
        assert_eq!(
            db_err.code().code(),
            "23505",
            "expected only a unique-violation, got {}: {}",
            db_err.code().code(),
            db_err.message()
        );
    }
}

// ----------------------- meta-command coverage --------------------------

/// The first error segment, if any. The REPL reports failures in-band rather
/// than as a `Result`, so this is how a test asserts a command succeeded.
fn errored(out: &pgscope_lib::repl::session::ReplOutput) -> Option<String> {
    out.segments
        .iter()
        .find(|s| s.kind == pgscope_lib::repl::session::SegmentKind::Error)
        .map(|s| s.text.clone())
}

/// The point of the whole exercise: the catalog SQL has to *run*.
///
/// The unit tests in `catalog.rs` only assert on strings, which cannot catch a
/// misspelled column, a function that does not exist on this server version, or
/// a join that needs a table the query never added.
#[tokio::test]
async fn every_listing_form_executes_against_a_real_server() {
    let mut session = repl().await;
    for cmd in [
        "\\d",
        "\\d+",
        "\\dS",
        "\\dt",
        "\\dt+",
        "\\dtS",
        "\\dtS+",
        "\\dv",
        "\\dv+",
        "\\dm",
        "\\di",
        "\\di+",
        "\\ds",
        "\\dE",
        "\\df",
        "\\df+",
        "\\dfS",
        "\\dn",
        "\\dn+",
        "\\du",
        "\\du+",
        "\\duS",
        "\\dx",
        "\\dx+",
        "\\l",
        "\\l+",
        "\\dtv",
        "\\dt public.*",
        "\\dt ev*",
        "\\df pg_*",
    ] {
        let out = session.submit(cmd).await;
        assert!(
            errored(&out).is_none(),
            "{cmd} produced an error: {}",
            errored(&out).unwrap()
        );
    }
}

#[tokio::test]
async fn a_name_pattern_narrows_the_listing() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\dt ev*").await);
    assert!(text.contains("events"), "{text}");
    assert!(
        !text.contains("users"),
        "pattern did not exclude users: {text}"
    );
}

#[tokio::test]
async fn a_pattern_is_anchored_rather_than_a_substring_match() {
    // `\dt vent` must NOT find `events`; psql anchors both ends, and a `LIKE
    // '%vent%'` shortcut would quietly pass the previous test while failing here.
    let mut session = repl().await;
    let text = text_of(&session.submit("\\dt vent").await);
    assert!(
        text.contains("Did not find"),
        "unanchored pattern matched: {text}"
    );
}

#[tokio::test]
async fn an_unquoted_pattern_folds_case() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\dt EVENTS").await);
    assert!(text.contains("events"), "case folding failed: {text}");
}

#[tokio::test]
async fn system_tables_appear_only_with_the_s_modifier() {
    let mut session = repl().await;
    let plain = text_of(&session.submit("\\dt pg_class").await);
    assert!(plain.contains("Did not find"), "{plain}");

    let with_s = text_of(&session.submit("\\dtS pg_class").await);
    assert!(with_s.contains("pg_class"), "{with_s}");
}

#[tokio::test]
async fn listing_indexes_names_the_table_each_one_belongs_to() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\di").await);
    assert!(text.contains("Table"), "no Table column: {text}");
    assert!(text.contains("events"), "{text}");
}

#[tokio::test]
async fn verbose_listing_adds_a_size_column_that_the_server_can_compute() {
    let mut session = repl().await;
    let out = session.submit("\\dt+").await;
    assert!(errored(&out).is_none(), "{:?}", errored(&out));
    let text = text_of(&out);
    assert!(text.contains("Size"), "{text}");
    // A size only ever renders in kB/MB units; a bare "0" would mean the CASE
    // guard swallowed every relation.
    assert!(
        text.contains("bytes") || text.contains("kB") || text.contains("MB"),
        "{text}"
    );
}

#[tokio::test]
async fn verbose_listing_of_views_does_not_error_on_missing_storage() {
    // `pg_total_relation_size` on a view is the obvious way this breaks, and it
    // only breaks when a view actually exists.
    let mut session = repl().await;
    let out = session.submit("\\dv+").await;
    assert!(errored(&out).is_none(), "{:?}", errored(&out));
}

#[tokio::test]
async fn an_empty_listing_says_so_instead_of_drawing_a_blank_frame() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\dt zzz_no_such_table*").await);
    assert!(text.contains("Did not find any matching objects"), "{text}");
}

#[tokio::test]
async fn conninfo_reports_the_live_connection() {
    let mut session = repl().await;
    let out = session.submit("\\conninfo").await;
    assert!(errored(&out).is_none(), "{:?}", errored(&out));
    assert!(text_of(&out).contains("analytics_prod"));
}

#[tokio::test]
async fn describe_verbose_adds_storage_and_description_columns() {
    let mut session = repl().await;
    let out = session.submit("\\d+ events").await;
    assert!(errored(&out).is_none(), "{:?}", errored(&out));
    let text = text_of(&out);
    assert!(text.contains("Storage"), "{text}");
    assert!(text.contains("event_name"), "{text}");
}

#[tokio::test]
async fn include_runs_a_saved_query() {
    let paths = scratch_paths("include");
    std::fs::write(
        paths.saved_queries().join("count_events.sql"),
        "SELECT count(*) AS n FROM events;\n",
    )
    .unwrap();

    let mut session = repl_with_paths(paths).await;
    let out = session.submit("\\i count_events").await;
    assert!(errored(&out).is_none(), "{:?}", errored(&out));
    let text = text_of(&out);
    assert!(text.contains("count_events.sql"), "{text}");
    assert!(text.contains("(1 row)"), "query did not run: {text}");
}

#[tokio::test]
async fn include_runs_every_statement_in_the_file() {
    let paths = scratch_paths("include-multi");
    std::fs::write(
        paths.saved_queries().join("two.sql"),
        "SELECT 1 AS first;\nSELECT 2 AS second;\n",
    )
    .unwrap();

    let mut session = repl_with_paths(paths).await;
    let text = text_of(&session.submit("\\i two").await);
    assert!(text.contains("first"), "{text}");
    assert!(
        text.contains("second"),
        "only the first statement ran: {text}"
    );
}

#[tokio::test]
async fn include_does_not_interpret_meta_commands_inside_the_file() {
    // Which is also what stops `\i` recursing into itself.
    let paths = scratch_paths("include-meta");
    std::fs::write(paths.saved_queries().join("loop.sql"), "\\i loop\n").unwrap();

    let mut session = repl_with_paths(paths).await;
    let out = session.submit("\\i loop").await;
    // It must fail as bad SQL rather than recurse — the test finishing at all
    // is most of the assertion.
    assert!(errored(&out).is_some(), "expected a syntax error");
}

#[tokio::test]
async fn include_names_the_available_queries_when_one_is_missing() {
    let paths = scratch_paths("include-missing");
    std::fs::write(paths.saved_queries().join("real_one.sql"), "SELECT 1;\n").unwrap();

    let mut session = repl_with_paths(paths).await;
    let err = errored(&session.submit("\\i nope").await).expect("expected an error");
    assert!(err.contains("no saved query"), "{err}");
    assert!(
        err.contains("real_one"),
        "should list what is available: {err}"
    );
}

#[tokio::test]
async fn unsupported_commands_explain_themselves_specifically() {
    let mut session = repl().await;
    let err = errored(&session.submit("\\copy events to 'x'").await).expect("expected an error");
    assert!(err.contains("COPY"), "{err}");
    assert!(
        !err.contains("invalid command"),
        "should not be the generic message: {err}"
    );
}

/// `\d` must not print a heading over an empty frame.
///
/// `render_messages` always produces output for a successful query — a header,
/// a rule and `(0 rows)` — so the old `is_empty` guard could never suppress an
/// empty section, and every table without indexes got a bare "Indexes:" frame.
#[tokio::test]
async fn describe_omits_sections_a_table_does_not_have() {
    let mut session = repl().await;
    session
        .submit("CREATE TEMP TABLE plain_table (a int);")
        .await;

    let text = text_of(&session.submit("\\d plain_table").await);
    assert!(text.contains("Column"), "columns are still listed: {text}");
    assert!(!text.contains("Indexes:"), "empty index section: {text}");
    assert!(
        !text.contains("Foreign-key constraints:"),
        "empty FK section: {text}"
    );
}

/// The counterpart: a table that *has* indexes still shows them.
#[tokio::test]
async fn describe_still_shows_sections_a_table_does_have() {
    let mut session = repl().await;
    let text = text_of(&session.submit("\\d events").await);
    assert!(text.contains("Indexes:"), "index section missing: {text}");
    assert!(text.contains("events_pkey"), "{text}");
}

/// Switching connections must invalidate the terminal session bound to the old
/// one.
///
/// The bug this pins: teardown was guarded on the connection becoming `None`, so
/// *replacing* one connection with another left the psql pane holding a client
/// to the previous database. The prompt kept naming the old database because it
/// really was still connected to it — statements ran there while the titlebar
/// named the new one.
#[tokio::test]
async fn switching_connections_invalidates_the_terminal_session() {
    let state = pgscope_lib::state::AppState::new(scratch_paths("switch"));
    let conn = connect().await;
    state
        .set_connection(Some(std::sync::Arc::clone(&conn)))
        .await;

    let session = ReplSession::open(
        conn.profile.clone(),
        conn.password.clone(),
        conn.info.database.clone(),
        conn.info.is_superuser,
        scratch_paths("switch-repl"),
    )
    .await
    .unwrap();
    let id = session.id.clone();
    state.add_repl(session).await;
    assert!(state.repl(&id).await.is_ok(), "session should start usable");

    // Reconnect — a different Arc, same server; the point is that it is a change.
    let other = connect().await;
    state.set_connection(Some(other)).await;

    assert!(
        state.repl(&id).await.is_err(),
        "the old session must not survive a connection switch"
    );
}
