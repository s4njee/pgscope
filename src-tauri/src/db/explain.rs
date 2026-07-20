//! EXPLAIN plan capture and analysis.
//!
//! Postgres emits a nested JSON document; this turns it into a tree the UI can
//! render, and computes the two things that actually make a plan readable:
//! **self time** (a node's own cost, excluding its children) and **row
//! misestimation** (what the planner expected versus what it got). Finding the
//! slowest node by *total* time is useless — the root always wins.

use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Client;

use crate::error::{AppError, Result};

/// Detail rows shown under a node, in the order psql's text output uses them.
/// Anything not listed here is dropped rather than flooding the card.
const DETAIL_KEYS: &[&str] = &[
    "Relation Name",
    "Alias",
    "Index Name",
    "Scan Direction",
    "Join Type",
    "Index Cond",
    "Recheck Cond",
    "Filter",
    "Hash Cond",
    "Merge Cond",
    "Join Filter",
    "Sort Key",
    "Sort Method",
    "Sort Space Used",
    "Sort Space Type",
    "Group Key",
    "Rows Removed by Filter",
    "Rows Removed by Join Filter",
    "Rows Removed by Index Recheck",
    "Heap Fetches",
    "Workers Planned",
    "Workers Launched",
    "Shared Hit Blocks",
    "Shared Read Blocks",
    "Temp Read Blocks",
    "Temp Written Blocks",
    "Subplan Name",
    "Parent Relationship",
];

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanNode {
    /// Stable id for React keys and expand/collapse state.
    pub id: String,
    pub node_type: String,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<f64>,
    pub plan_width: Option<i64>,

    // Present only for EXPLAIN ANALYZE.
    pub actual_startup_time: Option<f64>,
    pub actual_total_time: Option<f64>,
    pub actual_rows: Option<f64>,
    pub actual_loops: Option<f64>,

    /// Wall time attributable to this node alone: its own total across all
    /// loops, minus everything its children accounted for. This is what
    /// identifies the real bottleneck.
    pub self_time_ms: Option<f64>,
    /// Cost attributable to this node alone, for plans without ANALYZE.
    pub self_cost: Option<f64>,
    /// actual_rows*loops ÷ plan_rows. >1 means the planner under-estimated.
    pub row_ratio: Option<f64>,

    /// True when this node ran inside a Gather, i.e. its `actual_loops` counts
    /// **parallel workers** rather than sequential iterations.
    ///
    /// The distinction matters for reading `self_time_ms`: for a nested loop's
    /// inner side, loops × per-loop time is real wall time; for a parallel
    /// worker it is CPU time summed across workers, which can legitimately
    /// exceed the query's wall-clock execution time. The UI labels these
    /// differently rather than showing a number that looks impossible.
    pub parallel: bool,

    pub details: Vec<[String; 2]>,
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResult {
    pub plan: PlanNode,
    pub planning_time_ms: Option<f64>,
    pub execution_time_ms: Option<f64>,
    /// True when the statement was actually executed (EXPLAIN ANALYZE).
    pub analyzed: bool,
    /// Whether the run was wrapped in a rolled-back transaction.
    pub rolled_back: bool,
    /// The statement explained, for the header.
    pub sql: String,
    /// Largest `self_time_ms` in the tree, so the UI can scale its bars.
    pub max_self_time_ms: Option<f64>,
    /// Largest `self_cost`, likewise.
    pub max_self_cost: Option<f64>,
}

/// Read a numeric plan field, treating absent and non-numeric alike.
///
/// Costs and row counts are fractional, and the timing fields are missing
/// entirely without ANALYZE, so `None` is the normal case rather than an error.
///
/// # Arguments
/// * `v` — `&Value`: one `Plan` object, or the top-level result object.
/// * `key` — `&str`: the Postgres field name, spelled as EXPLAIN emits it
///   (`"Total Cost"`, `"Actual Loops"`) — exact match, no normalisation.
///
/// # Returns
/// `Option<f64>` — the value, or `None` when the key is absent or not a number.
fn as_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

/// Render a detail value without JSON quoting; arrays join with `, `.
///
/// # Arguments
/// * `v` — `&Value`: the raw detail value; arrays recurse element-wise.
///
/// # Returns
/// `String` — display text, empty for `Value::Null`.
fn detail_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(detail_string)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Convert one `Plan` object and its subtree into a [`PlanNode`].
///
/// `path` is the dotted child index of this node, which becomes its `id` — a
/// position-derived key is stable across re-renders without needing the plan to
/// carry one. Derived fields are left empty; `annotate` fills them afterwards.
///
/// A plan missing `Node Type` yields "Unknown" rather than failing: a partially
/// recognised tree is still worth showing.
///
/// # Arguments
/// * `v` — `&Value`: one `Plan` object from the EXPLAIN document.
/// * `path` — `&str`: dotted child index of this node (`"0"`, `"0.1.0"`), used
///   verbatim as the node's `id`; children append `.{i}`.
///
/// # Returns
/// `PlanNode` — the node and its whole subtree, with `self_time_ms`,
/// `self_cost`, `row_ratio` and `parallel` still at their defaults.
fn parse_node(v: &Value, path: &str) -> PlanNode {
    let mut node = PlanNode {
        id: path.to_string(),
        node_type: v
            .get("Node Type")
            .and_then(|x| x.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        startup_cost: as_f64(v, "Startup Cost"),
        total_cost: as_f64(v, "Total Cost"),
        plan_rows: as_f64(v, "Plan Rows"),
        plan_width: v.get("Plan Width").and_then(|x| x.as_i64()),
        actual_startup_time: as_f64(v, "Actual Startup Time"),
        actual_total_time: as_f64(v, "Actual Total Time"),
        actual_rows: as_f64(v, "Actual Rows"),
        actual_loops: as_f64(v, "Actual Loops"),
        ..Default::default()
    };

    for key in DETAIL_KEYS {
        if let Some(value) = v.get(*key) {
            let text = detail_string(value);
            if !text.is_empty() && text != "0" {
                node.details.push([key.to_string(), text]);
            }
        }
    }

    if let Some(Value::Array(children)) = v.get("Plans") {
        node.children = children
            .iter()
            .enumerate()
            .map(|(i, child)| parse_node(child, &format!("{path}.{i}")))
            .collect();
    }

    node
}

/// Flag every node below a Gather, where `actual_loops` counts workers.
///
/// The Gather itself runs once in the leader, so it is not marked; everything
/// beneath it is.
///
/// # Arguments
/// * `node` — `&mut PlanNode`: subtree root; its `parallel` flag is overwritten.
/// * `under_gather` — `bool`: whether an ancestor was a Gather. Pass `false` at
///   the plan root.
///
/// # Returns
/// `()` — sets `parallel` on `node` and every descendant in place.
fn mark_parallel(node: &mut PlanNode, under_gather: bool) {
    node.parallel = under_gather;
    let opens_parallel = node.node_type.starts_with("Gather");
    for child in node.children.iter_mut() {
        mark_parallel(child, under_gather || opens_parallel);
    }
}

/// Total wall time a node consumed, across every loop it ran.
///
/// Postgres reports per-loop averages, so a node executed 1000 times as the
/// inner side of a nested loop shows a tiny `Actual Total Time` while actually
/// dominating the query.
///
/// # Arguments
/// * `node` — `&PlanNode`: read only; a missing `actual_loops` counts as 1.
///
/// # Returns
/// `Option<f64>` — milliseconds across all loops, or `None` without ANALYZE.
fn total_time(node: &PlanNode) -> Option<f64> {
    let t = node.actual_total_time?;
    let loops = node.actual_loops.unwrap_or(1.0);
    Some(t * loops)
}

/// Fill in `self_time_ms`, `self_cost`, and `row_ratio` bottom-up.
///
/// # Arguments
/// * `node` — `&mut PlanNode`: subtree root; recurses into children first.
///
/// # Returns
/// `(Option<f64>, Option<f64>)` — this node's loop-multiplied total time and its
/// `total_cost`, which the parent subtracts to get its own share. `None` in
/// either slot means the parent keeps its full figure.
fn annotate(node: &mut PlanNode) -> (Option<f64>, Option<f64>) {
    let mut children_time = 0.0;
    let mut children_have_time = false;
    let mut children_cost = 0.0;
    let mut children_have_cost = false;

    for child in node.children.iter_mut() {
        let (t, c) = annotate(child);
        if let Some(t) = t {
            children_time += t;
            children_have_time = true;
        }
        if let Some(c) = c {
            children_cost += c;
            children_have_cost = true;
        }
    }

    let my_total = total_time(node);
    node.self_time_ms = my_total.map(|t| {
        let self_t = if children_have_time {
            t - children_time
        } else {
            t
        };
        // Parallel workers and rounding can push this slightly negative.
        self_t.max(0.0)
    });

    node.self_cost = node.total_cost.map(|c| {
        let self_c = if children_have_cost {
            c - children_cost
        } else {
            c
        };
        self_c.max(0.0)
    });

    node.row_ratio = match (node.actual_rows, node.plan_rows) {
        // Compare like with like: actual rows are per-loop, as are plan rows.
        (Some(actual), Some(planned)) if planned > 0.0 => Some(actual / planned),
        _ => None,
    };

    (my_total, node.total_cost)
}

/// Largest `self_time_ms` anywhere in the tree, for scaling the UI's bars.
///
/// Must run after `annotate`. Nodes under a Gather contribute CPU time summed
/// across workers, so this maximum can exceed the query's execution time.
///
/// # Arguments
/// * `node` — `&PlanNode`: subtree root, already annotated.
///
/// # Returns
/// `Option<f64>` — the maximum, or `None` when no node in the subtree has a
/// self time (a plan without ANALYZE).
fn max_self_time(node: &PlanNode) -> Option<f64> {
    let mut best = node.self_time_ms;
    for child in &node.children {
        if let Some(c) = max_self_time(child) {
            best = Some(best.map_or(c, |b: f64| b.max(c)));
        }
    }
    best
}

/// Largest `self_cost` in the tree — the cost-side counterpart to
/// [`max_self_time`], used to scale bars for plans without ANALYZE.
///
/// # Arguments
/// * `node` — `&PlanNode`: subtree root, already annotated.
///
/// # Returns
/// `Option<f64>` — the maximum, or `None` when no node carries a cost.
fn max_self_cost(node: &PlanNode) -> Option<f64> {
    let mut best = node.self_cost;
    for child in &node.children {
        if let Some(c) = max_self_cost(child) {
            best = Some(best.map_or(c, |b: f64| b.max(c)));
        }
    }
    best
}

/// Parse the document `EXPLAIN (FORMAT JSON)` returns.
///
/// # Arguments
/// * `json` — `&str`: the raw EXPLAIN document, a one-element array.
/// * `sql` — `&str`: the statement that was explained, carried through to the
///   result's header; not re-parsed or validated.
/// * `analyzed` — `bool`: whether ANALYZE was requested, recorded as-is.
/// * `rolled_back` — `bool`: whether the run was wrapped in a rolled-back
///   transaction, recorded as-is.
///
/// # Returns
/// `Result<ExplainResult>` — the annotated tree with its scaling maxima. `Err`
/// (`AppError::Invalid`) when the text is not JSON, the array is empty, or the
/// element has no `Plan` key.
pub fn parse_explain(
    json: &str,
    sql: &str,
    analyzed: bool,
    rolled_back: bool,
) -> Result<ExplainResult> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| AppError::Invalid(format!("could not parse EXPLAIN output: {e}")))?;

    // The document is a one-element array wrapping the plan.
    let first = root
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| AppError::Invalid("EXPLAIN returned no plan".into()))?;

    let plan_value = first
        .get("Plan")
        .ok_or_else(|| AppError::Invalid("EXPLAIN output has no Plan node".into()))?;

    let mut plan = parse_node(plan_value, "0");
    mark_parallel(&mut plan, false);
    annotate(&mut plan);

    Ok(ExplainResult {
        planning_time_ms: as_f64(first, "Planning Time"),
        execution_time_ms: as_f64(first, "Execution Time"),
        max_self_time_ms: max_self_time(&plan),
        max_self_cost: max_self_cost(&plan),
        plan,
        analyzed,
        rolled_back,
        sql: sql.to_string(),
    })
}

/// Run EXPLAIN against a statement.
///
/// `analyze` really executes the statement, so the run is wrapped in a
/// transaction that is always rolled back. Without that, "explain" on an
/// UPDATE or DELETE would quietly modify data — the one thing a user inspecting
/// a query definitely does not expect. The rollback is reported back to the UI
/// so it can say so.
///
/// # Arguments
/// * `client` — `&Client`: the connection the statement runs on; when `analyze`
///   is set it also carries the BEGIN/ROLLBACK, so it must be the same session.
/// * `sql` — `&str`: the statement, interpolated verbatim after `EXPLAIN (…)`.
/// * `analyze` — `bool`: `true` really executes the statement.
///
/// # Returns
/// `Result<ExplainResult>` — the parsed plan. `Err` on any protocol failure, on
/// a failed rollback (reported even when the EXPLAIN itself succeeded), or when
/// the output is empty or unparseable.
pub async fn explain(client: &Client, sql: &str, analyze: bool) -> Result<ExplainResult> {
    let options = if analyze {
        "ANALYZE, BUFFERS, VERBOSE, COSTS, FORMAT JSON"
    } else {
        "VERBOSE, COSTS, FORMAT JSON"
    };
    let query = format!("EXPLAIN ({options}) {sql}");

    let json = if analyze {
        client.batch_execute("BEGIN").await?;
        let result = fetch_explain_json(client, &query).await;
        // Roll back whether or not the statement succeeded.
        let rollback = client.batch_execute("ROLLBACK").await;
        result.and_then(|json| rollback.map(|_| json).map_err(Into::into))?
    } else {
        fetch_explain_json(client, &query).await?
    };

    parse_explain(&json, sql, analyze, analyze)
}

/// EXPLAIN FORMAT JSON returns the whole document as one text cell, though a
/// long plan may arrive split across rows.
///
/// # Arguments
/// * `client` — `&Client`: the same session as any surrounding transaction.
/// * `query` — `&str`: the complete `EXPLAIN (…) …` text, run through
///   `simple_query` so it carries no parameters.
///
/// # Returns
/// `Result<String>` — the concatenated JSON document. `Err` on a protocol
/// failure, or `AppError::Invalid` when the output is blank.
async fn fetch_explain_json(client: &Client, query: &str) -> Result<String> {
    let rows = client.simple_query(query).await?;
    let mut out = String::new();
    for msg in rows {
        if let tokio_postgres::SimpleQueryMessage::Row(row) = msg {
            if let Some(text) = row.get(0) {
                out.push_str(text);
            }
        }
    }
    if out.trim().is_empty() {
        return Err(AppError::Invalid("EXPLAIN returned no output".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative EXPLAIN ANALYZE document: a Limit over a Sort over a
    /// HashAggregate over a Seq Scan, with the Seq Scan doing the real work and
    /// badly under-estimating its row count.
    const ANALYZED: &str = r#"[
      {
        "Plan": {
          "Node Type": "Limit",
          "Startup Cost": 1000.0,
          "Total Cost": 1010.0,
          "Plan Rows": 5,
          "Plan Width": 40,
          "Actual Startup Time": 300.0,
          "Actual Total Time": 300.5,
          "Actual Rows": 5,
          "Actual Loops": 1,
          "Plans": [
            {
              "Node Type": "Sort",
              "Startup Cost": 900.0,
              "Total Cost": 950.0,
              "Plan Rows": 7,
              "Plan Width": 40,
              "Actual Startup Time": 299.0,
              "Actual Total Time": 299.5,
              "Actual Rows": 7,
              "Actual Loops": 1,
              "Sort Key": ["count(*) DESC"],
              "Sort Method": "quicksort",
              "Plans": [
                {
                  "Node Type": "Seq Scan",
                  "Relation Name": "events",
                  "Alias": "events",
                  "Startup Cost": 0.0,
                  "Total Cost": 800.0,
                  "Plan Rows": 100,
                  "Plan Width": 12,
                  "Actual Startup Time": 0.2,
                  "Actual Total Time": 250.0,
                  "Actual Rows": 50000,
                  "Actual Loops": 1,
                  "Filter": "(created_at > now())",
                  "Rows Removed by Filter": 1234
                }
              ]
            }
          ]
        },
        "Planning Time": 0.35,
        "Execution Time": 301.2
      }
    ]"#;

    /// The `ANALYZED` fixture parsed once per test — a real EXPLAIN payload, so
    /// the shape and timing assertions all describe the same plan.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `ExplainResult` — the parsed fixture; panics if parsing fails.
    fn parsed() -> ExplainResult {
        parse_explain(ANALYZED, "SELECT 1", true, true).unwrap()
    }

    #[test]
    fn parses_the_tree_shape() {
        let r = parsed();
        assert_eq!(r.plan.node_type, "Limit");
        assert_eq!(r.plan.children.len(), 1);
        assert_eq!(r.plan.children[0].node_type, "Sort");
        assert_eq!(r.plan.children[0].children[0].node_type, "Seq Scan");
        assert!(r.plan.children[0].children[0].children.is_empty());
    }

    #[test]
    fn captures_timings_and_costs() {
        let r = parsed();
        assert_eq!(r.planning_time_ms, Some(0.35));
        assert_eq!(r.execution_time_ms, Some(301.2));
        assert_eq!(r.plan.total_cost, Some(1010.0));
        assert_eq!(r.plan.actual_rows, Some(5.0));
        assert!(r.analyzed);
    }

    #[test]
    fn ids_are_unique_and_stable() {
        let r = parsed();
        assert_eq!(r.plan.id, "0");
        assert_eq!(r.plan.children[0].id, "0.0");
        assert_eq!(r.plan.children[0].children[0].id, "0.0.0");
        // Re-parsing gives the same ids, so expand state survives a re-run.
        assert_eq!(parsed().plan.children[0].id, "0.0");
    }

    #[test]
    fn self_time_excludes_children() {
        let r = parsed();
        let limit = &r.plan;
        let sort = &limit.children[0];
        let scan = &sort.children[0];

        // Limit: 300.5 total - 299.5 in Sort = 1.0 of its own.
        assert!((limit.self_time_ms.unwrap() - 1.0).abs() < 1e-6);
        // Sort: 299.5 - 250.0 = 49.5.
        assert!((sort.self_time_ms.unwrap() - 49.5).abs() < 1e-6);
        // A leaf keeps all of its time.
        assert!((scan.self_time_ms.unwrap() - 250.0).abs() < 1e-6);
    }

    #[test]
    fn self_time_identifies_the_real_bottleneck() {
        let r = parsed();
        // The root has the largest *total* time but the Seq Scan is the actual
        // cost centre — the whole point of computing self time.
        assert_eq!(r.max_self_time_ms, Some(250.0));
        assert!(
            r.plan.self_time_ms.unwrap() < r.plan.children[0].children[0].self_time_ms.unwrap()
        );
    }

    #[test]
    fn self_time_accounts_for_loops() {
        // A node run 1000 times reports a per-loop average; the true cost is
        // 1000x that, which is exactly the case people miss reading plans.
        let json = r#"[{"Plan":{
            "Node Type":"Nested Loop","Total Cost":10.0,"Plan Rows":1,
            "Actual Total Time":500.0,"Actual Rows":1000,"Actual Loops":1,
            "Plans":[{"Node Type":"Index Scan","Total Cost":5.0,"Plan Rows":1,
                      "Actual Total Time":0.4,"Actual Rows":1,"Actual Loops":1000}]
        }}]"#;
        let r = parse_explain(json, "q", true, false).unwrap();
        let inner = &r.plan.children[0];

        // 0.4ms x 1000 loops = 400ms, not 0.4ms.
        assert!((inner.self_time_ms.unwrap() - 400.0).abs() < 1e-6);
        // The outer node keeps only 500 - 400 = 100ms.
        assert!((r.plan.self_time_ms.unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn self_time_never_goes_negative() {
        // Parallel workers can make children sum above the parent's total.
        let json = r#"[{"Plan":{
            "Node Type":"Gather","Total Cost":10.0,"Plan Rows":1,
            "Actual Total Time":100.0,"Actual Rows":1,"Actual Loops":1,
            "Plans":[{"Node Type":"Parallel Seq Scan","Total Cost":5.0,"Plan Rows":1,
                      "Actual Total Time":90.0,"Actual Rows":1,"Actual Loops":3}]
        }}]"#;
        let r = parse_explain(json, "q", true, false).unwrap();
        assert_eq!(r.plan.self_time_ms, Some(0.0));
    }

    #[test]
    fn row_ratio_flags_misestimation() {
        let r = parsed();
        let scan = &r.plan.children[0].children[0];
        // Planner expected 100 rows, got 50,000 — a 500x under-estimate.
        assert_eq!(scan.row_ratio, Some(500.0));
        // The Limit estimated exactly right.
        assert_eq!(r.plan.row_ratio, Some(1.0));
    }

    #[test]
    fn row_ratio_is_absent_when_the_planner_expected_zero() {
        let json = r#"[{"Plan":{"Node Type":"Result","Plan Rows":0,"Actual Rows":5}}]"#;
        let r = parse_explain(json, "q", true, false).unwrap();
        assert_eq!(r.plan.row_ratio, None, "must not divide by zero");
    }

    #[test]
    fn collects_useful_details_and_drops_noise() {
        let r = parsed();
        let scan = &r.plan.children[0].children[0];
        let keys: Vec<&str> = scan.details.iter().map(|d| d[0].as_str()).collect();

        assert!(keys.contains(&"Relation Name"));
        assert!(keys.contains(&"Filter"));
        assert!(keys.contains(&"Rows Removed by Filter"));
        // Costs are rendered as first-class fields, not detail rows.
        assert!(!keys.contains(&"Total Cost"));
    }

    #[test]
    fn renders_array_details_as_readable_text() {
        let r = parsed();
        let sort = &r.plan.children[0];
        let key = sort.details.iter().find(|d| d[0] == "Sort Key").unwrap();
        // Not JSON-quoted.
        assert_eq!(key[1], "count(*) DESC");
    }

    #[test]
    fn handles_a_plan_without_analyze() {
        let json = r#"[{"Plan":{
            "Node Type":"Seq Scan","Relation Name":"events",
            "Startup Cost":0.0,"Total Cost":800.0,"Plan Rows":100,"Plan Width":12
        }}]"#;
        let r = parse_explain(json, "q", false, false).unwrap();

        assert!(!r.analyzed);
        assert_eq!(r.plan.actual_total_time, None);
        assert_eq!(r.plan.self_time_ms, None);
        assert_eq!(r.max_self_time_ms, None);
        // Cost is still usable for the bars.
        assert_eq!(r.plan.self_cost, Some(800.0));
        assert_eq!(r.max_self_cost, Some(800.0));
    }

    #[test]
    fn self_cost_excludes_children() {
        let r = parsed();
        // Limit 1010 - Sort 950 = 60.
        assert!((r.plan.self_cost.unwrap() - 60.0).abs() < 1e-6);
        // Sort 950 - Seq Scan 800 = 150.
        assert!((r.plan.children[0].self_cost.unwrap() - 150.0).abs() < 1e-6);
    }

    #[test]
    fn marks_nodes_under_a_gather_as_parallel() {
        // Under a Gather, `Actual Loops` counts workers, so the multiplied
        // self time is CPU across workers rather than wall time — and can
        // legitimately exceed the query's execution time.
        let json = r#"[{"Plan":{
            "Node Type":"Gather","Total Cost":10.0,"Plan Rows":1,
            "Actual Total Time":36.0,"Actual Rows":44491,"Actual Loops":1,
            "Workers Launched":2,
            "Plans":[{"Node Type":"Parallel Seq Scan","Total Cost":5.0,"Plan Rows":1000,
                      "Actual Total Time":28.6,"Actual Rows":14830,"Actual Loops":3}]
        }}]"#;
        let r = parse_explain(json, "q", true, false).unwrap();

        // The Gather runs in the leader; only its children are parallel.
        assert!(!r.plan.parallel);
        assert!(r.plan.children[0].parallel);

        // The multiplied figure is still reported — it is the true total work —
        // but the flag lets the UI label it as CPU rather than wall time.
        let scan = &r.plan.children[0];
        assert!((scan.self_time_ms.unwrap() - 85.8).abs() < 0.1);
        assert!(
            scan.self_time_ms.unwrap() > r.execution_time_ms.unwrap_or(36.0),
            "this is the confusing case the flag exists to explain"
        );
    }

    #[test]
    fn parallel_marking_propagates_through_the_whole_subtree() {
        let json = r#"[{"Plan":{
            "Node Type":"Gather","Plan Rows":1,
            "Plans":[{"Node Type":"Nested Loop","Plan Rows":1,
                      "Plans":[{"Node Type":"Parallel Seq Scan","Plan Rows":1}]}]
        }}]"#;
        let r = parse_explain(json, "q", true, false).unwrap();
        assert!(!r.plan.parallel);
        assert!(r.plan.children[0].parallel, "direct child");
        assert!(r.plan.children[0].children[0].parallel, "grandchild too");
    }

    #[test]
    fn nodes_outside_a_gather_are_not_parallel() {
        let r = parsed();
        assert!(!r.plan.parallel);
        assert!(!r.plan.children[0].parallel);
        assert!(!r.plan.children[0].children[0].parallel);
    }

    #[test]
    fn rejects_malformed_output() {
        assert!(parse_explain("not json", "q", false, false).is_err());
        assert!(parse_explain("[]", "q", false, false).is_err());
        assert!(parse_explain(r#"[{"no plan here": 1}]"#, "q", false, false).is_err());
    }

    #[test]
    fn tolerates_unknown_node_fields() {
        let json = r#"[{"Plan":{"Node Type":"Custom Scan","Some Future Field":42}}]"#;
        let r = parse_explain(json, "q", false, false).unwrap();
        assert_eq!(r.plan.node_type, "Custom Scan");
    }
}
