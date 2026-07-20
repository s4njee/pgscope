pub mod cell;
pub mod connect;
pub mod explain;
pub mod grid;
pub mod introspect;
pub mod query;
pub mod rowfmt;

use std::time::Duration;

use deadpool_postgres::Pool;

use crate::error::{AppError, Result};
use grid::{
    build_count_query, build_page_query, cap_cell, display_sql, PageRequest, PageResult, PAGE_SIZE,
};

/// How long a filtered `count(*)` may run before we give up and show an
/// approximate total. Keeping this short matters: on a large table an
/// unindexed filter can otherwise stall the footer for minutes.
const COUNT_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the total row count for the footer and paging maths.
///
/// Unfiltered browsing uses `reltuples`, which is free but approximate — the
/// design's "of 48,213,904" is exactly this number. A filter forces a real
/// `count(*)`, bounded by `COUNT_TIMEOUT`; on timeout we return `None` and the
/// UI degrades to "≥ n" with the last-page jump disabled.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool; the `count(*)` runs on a
///   connection taken from it.
/// * `req` — `&PageRequest`: supplies schema, table, and the filter that decides
///   estimate vs exact.
///
/// # Returns
/// `Result<(Option<i64>, bool)>` — the count and whether it is an estimate.
/// `None` for the count means the exact `count(*)` hit `COUNT_TIMEOUT`. `Err`
/// means the introspection or count query itself failed.
async fn resolve_total(pool: &Pool, req: &PageRequest) -> Result<(Option<i64>, bool)> {
    let has_filter = req
        .filter
        .as_deref()
        .map(|f| !f.trim().is_empty())
        .unwrap_or(false);

    if !has_filter {
        let stats = introspect::stats(pool, &req.schema, &req.table).await?;
        // reltuples is -1 on a never-analyzed table.
        let est = stats.est_rows.filter(|n| *n >= 0);
        return Ok((est, true));
    }

    let client = pool.get().await?;
    let sql = build_count_query(req);
    let cancel = client.cancel_token();

    match tokio::time::timeout(COUNT_TIMEOUT, client.query_one(sql.as_str(), &[])).await {
        Ok(Ok(row)) => Ok((Some(row.get::<_, i64>(0)), false)),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => {
            // Giving up on the *future* doesn't stop the server: the statement
            // keeps running and the connection goes back to the pool still
            // busy, so the very next query queues behind it. Cancel for real,
            // then drain the aborted query off this connection before it is
            // recycled.
            let _ = cancel.cancel_query(tokio_postgres::NoTls).await;
            let _ = client.simple_query("").await;
            Ok((None, false))
        }
    }
}

/// Fetch one page of a table for the data grid.
///
/// `on_token` receives the statement's cancellation handle before the query
/// runs, so a caller can abort a slow page fetch.
///
/// # Arguments
/// * `pool` — `&Pool`: the read-only browse pool.
/// * `req` — `&PageRequest`: table identity plus page index, sort, and raw
///   user-supplied filter text.
/// * `on_token` — `F: FnOnce(tokio_postgres::CancelToken)`: called once with the
///   statement's cancellation handle before the page query is issued.
///
/// # Returns
/// `Result<PageResult>` — the page's cells (each capped by `cap_cell`), timing,
/// and total. `Err` is `AppError::Invalid` when the relation has no visible
/// columns, otherwise a pool or query failure.
pub async fn fetch_page<F>(pool: &Pool, req: &PageRequest, on_token: F) -> Result<PageResult>
where
    F: FnOnce(tokio_postgres::CancelToken),
{
    let cols = introspect::columns(pool, &req.schema, &req.table).await?;
    if cols.is_empty() {
        return Err(AppError::Invalid(format!(
            "{}.{} has no visible columns",
            req.schema, req.table
        )));
    }
    let col_names: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();

    let (total, total_is_estimate) = resolve_total(pool, req).await?;
    let query = build_page_query(req, &col_names, total)?;

    let client = pool.get().await?;
    on_token(client.cancel_token());
    let t0 = std::time::Instant::now();
    let rows = client.query(query.sql.as_str(), &[]).await?;
    let timing_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Everything was projected as ::text, so every column decodes as
    // Option<String> regardless of its real type.
    let mut out: Vec<Vec<Option<String>>> = rows
        .iter()
        .map(|r| {
            (0..col_names.len())
                .map(|i| r.get::<_, Option<String>>(i).map(cap_cell))
                .collect()
        })
        .collect();

    if query.reverse_rows {
        out.reverse();
    }

    Ok(PageResult {
        rows: out,
        timing_ms,
        total,
        total_is_estimate,
        page: req.page,
        page_size: PAGE_SIZE,
        sql: display_sql(&query.sql, 120),
    })
}
