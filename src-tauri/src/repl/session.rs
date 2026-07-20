use serde::Serialize;
use tokio_postgres::{Client, SimpleQueryMessage};

use super::catalog;
use super::lexer;
use super::meta::{self, MetaCommand};
use super::table::{format_aligned, format_command_tag, FormatOptions, ResultSet};
use crate::db::connect::{connect_client, Profile};
use crate::error::{AppError, Result};
use crate::store::{self, Paths};

/// Per-statement output cap (plan.md §5.7). A `SELECT *` on a large table must
/// not push megabytes of text into the webview.
const MAX_ROWS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    Prompt,
    Body,
    Dim,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub text: String,
    pub kind: SegmentKind,
}

impl Segment {
    /// Query output proper — result tables and command tags.
    ///
    /// Already formatted for a monospaced pane, so the text must reach the
    /// webview with its whitespace intact.
    pub fn body(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SegmentKind::Body,
        }
    }
    /// Commentary the terminal emits about itself rather than the server:
    /// timings, `\timing on` acknowledgements, truncation notices.
    pub fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SegmentKind::Dim,
        }
    }
    /// A failure, server-side or local. Styled distinctly but not fatal — the
    /// session stays usable.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SegmentKind::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplOutput {
    pub segments: Vec<Segment>,
    pub prompt: String,
    pub incomplete: bool,
    pub timing: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplSessionInfo {
    pub session_id: String,
    pub prompt: String,
    pub timing: bool,
    pub expanded: bool,
}

/// One terminal session: a dedicated, unrestricted client plus the psql-ish
/// state that persists between submissions.
pub struct ReplSession {
    pub id: String,
    client: Client,
    profile: Profile,
    password: Option<String>,
    database: String,
    is_superuser: bool,
    /// Where saved queries live, so `\i` can find one by name.
    paths: Paths,
    /// Lines accumulated since the last complete statement.
    buffer: String,
    pub timing: bool,
    pub expanded: bool,
}

impl ReplSession {
    /// Open a terminal session on its own connection.
    ///
    /// Deliberately *not* the browse pool: this client carries no read-only or
    /// statement-timeout options, so the terminal can write and can run long
    /// statements. `is_superuser` and `database` only decorate the prompt; the
    /// password is kept so the session can silently reconnect after the server
    /// drops it.
    pub async fn open(
        profile: Profile,
        password: Option<String>,
        database: String,
        is_superuser: bool,
        paths: Paths,
    ) -> Result<Self> {
        let client = connect_client(&profile, password.as_deref(), "pgscope-repl", None).await?;
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            client,
            profile,
            password,
            database,
            is_superuser,
            paths,
            buffer: String::new(),
            // The design shows "Timing on" in the header.
            timing: true,
            expanded: false,
        })
    }

    /// The psql prompt for the current buffer state.
    ///
    /// Two independent characters, as in psql:
    ///  - the *marker* says what is open — `=` for a fresh statement, `-` mid
    ///    statement, `'`/`"`/`$`/`*` inside a string, identifier, dollar quote,
    ///    or block comment;
    ///  - the *tail* says who you are — `#` for a superuser, `>` otherwise, and
    ///    it does not change while a statement is being continued.
    ///
    /// So: `analytics_prod=#`, `analytics_prod-#`, `analytics_prod=>`.
    pub fn prompt(&self) -> String {
        let pending = lexer::scan(&self.buffer);
        let marker = pending.prompt_marker();
        let tail = if self.is_superuser { '#' } else { '>' };
        format!("{}{}{}", self.database, marker, tail)
    }

    /// The session's identity and toggle state, for the UI to render a freshly
    /// opened or restored terminal without having to submit anything.
    pub fn info(&self) -> ReplSessionInfo {
        ReplSessionInfo {
            session_id: self.id.clone(),
            prompt: self.prompt(),
            timing: self.timing,
            expanded: self.expanded,
        }
    }

    /// Wrap rendered segments with the state the UI needs afterwards.
    ///
    /// Prompt and `incomplete` are recomputed from the buffer at this moment, so
    /// this must be called *after* the buffer has been consumed or extended —
    /// otherwise the terminal shows a continuation prompt for a statement it
    /// just ran.
    fn output(&self, segments: Vec<Segment>) -> ReplOutput {
        ReplOutput {
            segments,
            prompt: self.prompt(),
            incomplete: !lexer::scan(&self.buffer).is_complete(),
            timing: self.timing,
            expanded: self.expanded,
        }
    }

    /// Tab completions for `line` at `cursor`, resolved against this session's
    /// own connection.
    pub async fn complete(
        &self,
        line: &str,
        cursor: usize,
    ) -> Result<super::complete::CompletionResult> {
        super::complete::complete(&self.client, line, cursor).await
    }

    /// The client's cancellation handle, so a running statement can be aborted
    /// from another task.
    pub fn cancel_token(&self) -> tokio_postgres::CancelToken {
        self.client.cancel_token()
    }

    /// Handle one submitted line.
    pub async fn submit(&mut self, line: &str) -> ReplOutput {
        // Meta-commands only apply at the start of a statement, as in psql.
        if self.buffer.trim().is_empty() {
            if let Some(cmd) = meta::parse(line) {
                self.buffer.clear();
                return self.run_meta(cmd).await;
            }
        }

        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);

        if !lexer::scan(&self.buffer).is_complete() {
            // Continuation: nothing to run yet.
            return self.output(Vec::new());
        }

        let sql = std::mem::take(&mut self.buffer);
        let trimmed = sql.trim().to_string();
        if trimmed.is_empty() {
            return self.output(Vec::new());
        }

        let segments = self.run_sql(&trimmed).await;
        self.output(segments)
    }

    /// Execute one complete statement and render whatever comes back.
    ///
    /// Uses the simple query protocol, so `sql` may contain several
    /// semicolon-separated statements in a single implicit transaction, and all
    /// values arrive as text. Errors become error segments rather than an `Err`:
    /// a failed statement is normal terminal output, not a failed command.
    ///
    /// The reported time is the client-side round trip, so it includes network
    /// latency — psql measures the same way.
    async fn run_sql(&mut self, sql: &str) -> Vec<Segment> {
        let t0 = std::time::Instant::now();
        let result = self.client.simple_query(sql).await;
        let elapsed = t0.elapsed().as_secs_f64() * 1000.0;

        let mut segments = match result {
            Ok(messages) => render_messages(messages, self.expanded),
            Err(e) => {
                let err: AppError = e.into();
                // A dropped connection is worth calling out explicitly, since
                // the next submit will transparently reconnect.
                if matches!(err, AppError::Connect(_)) {
                    self.reconnect_notice().await
                } else {
                    render_error(&err)
                }
            }
        };

        if self.timing {
            segments.push(Segment::dim(format!("Time: {elapsed:.3} ms\n")));
        }
        segments
    }

    /// Replace a dead client so the session survives a server restart.
    async fn reconnect_notice(&mut self) -> Vec<Segment> {
        let mut segments = vec![Segment::error(
            "server closed the connection unexpectedly\n\
             \tThis probably means the server terminated abnormally\n\
             \tbefore or while processing the request.\n",
        )];

        match connect_client(
            &self.profile,
            self.password.as_deref(),
            "pgscope-repl",
            None,
        )
        .await
        {
            Ok(client) => {
                self.client = client;
                // Session state (SET, \timing on the server side) is gone.
                segments.push(Segment::dim("-- reconnected; session state was reset\n"));
            }
            Err(e) => segments.push(Segment::error(format!("{e}\n"))),
        }
        segments
    }

    /// Dispatch one backslash command.
    ///
    /// Some of these are purely local (`\timing`, `\x`, `\?`), some become
    /// catalog queries. Either way the buffer has already been cleared by the
    /// caller — a meta-command never continues a statement.
    async fn run_meta(&mut self, cmd: MetaCommand) -> ReplOutput {
        let segments = match cmd {
            MetaCommand::Timing(v) => {
                self.timing = v.unwrap_or(!self.timing);
                vec![Segment::dim(format!(
                    "Timing is {}.\n",
                    if self.timing { "on" } else { "off" }
                ))]
            }
            MetaCommand::Expanded(v) => {
                self.expanded = v.unwrap_or(!self.expanded);
                vec![Segment::dim(format!(
                    "Expanded display is {}.\n",
                    if self.expanded { "on" } else { "off" }
                ))]
            }
            MetaCommand::Help => vec![Segment::dim(format!("{}\n", meta::HELP_TEXT))],
            MetaCommand::Quit => vec![Segment::dim(
                "Use the window controls to close pgscope; the session stays open.\n",
            )],
            MetaCommand::Unsupported(c) => {
                vec![Segment::error(meta::unsupported_message(&c))]
            }
            MetaCommand::Unknown(c) => {
                vec![Segment::error(format!("{}\n", meta::unknown_message(&c)))]
            }
            MetaCommand::List(spec) => self.list(&catalog::list_sql(&spec)).await,
            MetaCommand::ConnInfo => self.query_to_segments(catalog::CONNINFO_SQL).await,
            MetaCommand::Encoding => vec![Segment::body("UTF8\n")],
            MetaCommand::Include(name) => self.include(&name).await,
            MetaCommand::Describe { name, verbose } => self.describe(&name, verbose).await,
        };
        self.output(segments)
    }

    /// `\i <name>`: run a saved query.
    ///
    /// psql's `\i` takes a path; here it takes a saved-query name, because the
    /// saved-queries directory is the only place the app writes `.sql` and
    /// accepting arbitrary paths would let the terminal read any file on disk.
    async fn include(&mut self, name: &str) -> Vec<Segment> {
        let queries = match store::load_saved_queries(&self.paths) {
            Ok(q) => q,
            Err(e) => return vec![Segment::error(format!("{e}\n"))],
        };
        let Some(query) = queries.iter().find(|q| q.name == name) else {
            let mut text = format!("no saved query named {name:?}\n");
            if !queries.is_empty() {
                let names: Vec<&str> = queries.iter().map(|q| q.name.as_str()).collect();
                text.push_str(&format!("available: {}\n", names.join(", ")));
            }
            return vec![Segment::error(text)];
        };

        let mut segments = vec![Segment::dim(format!("-- {}.sql\n", query.name))];
        // Run the file as SQL only. Meta-commands inside a saved query are not
        // interpreted, which also means `\i` cannot recurse into itself.
        for statement in lexer::split_statements(&query.content) {
            let sql = statement.trim();
            if sql.is_empty() {
                continue;
            }
            segments.extend(self.run_sql(sql).await);
        }
        segments
    }

    /// Run internally-generated SQL and render it like any other output.
    ///
    /// For catalog queries this module builds, never for user input — the SQL
    /// arrives already interpolated, so nothing untrusted may reach here.
    async fn query_to_segments(&mut self, sql: &str) -> Vec<Segment> {
        match self.client.simple_query(sql).await {
            Ok(messages) => render_messages(messages, self.expanded),
            Err(e) => render_error(&e.into()),
        }
    }

    /// Render a `\d` sub-section, or nothing at all when the query matched no
    /// rows.
    ///
    /// `render_messages` always produces output for a successful query — a
    /// header, a rule and `(0 rows)` — so an `is_empty` check can never suppress
    /// an empty section. A table with no indexes would otherwise get an
    /// "Indexes:" heading over an empty frame, which psql does not print.
    async fn section(&mut self, title: &str, sql: &str) -> Vec<Segment> {
        let Ok(messages) = self.client.simple_query(sql).await else {
            return Vec::new();
        };
        if !messages
            .iter()
            .any(|m| matches!(m, SimpleQueryMessage::Row(_)))
        {
            return Vec::new();
        }
        let mut out = vec![Segment::dim(format!("\n{title}\n"))];
        out.extend(render_messages(messages, self.expanded));
        out
    }

    /// Run a `\d`-family listing.
    ///
    /// Split from [`Self::query_to_segments`] only for the empty case: psql says
    /// "Did not find any relations." instead of drawing an empty frame, and with
    /// patterns in play that message is the answer rather than an absence of one.
    async fn list(&mut self, sql: &str) -> Vec<Segment> {
        match self.client.simple_query(sql).await {
            Ok(messages) => {
                let any_rows = messages
                    .iter()
                    .any(|m| matches!(m, SimpleQueryMessage::Row(_)));
                if any_rows {
                    render_messages(messages, self.expanded)
                } else {
                    vec![Segment::dim("Did not find any matching objects.\n")]
                }
            }
            Err(e) => render_error(&e.into()),
        }
    }

    /// `\d <table>`: columns, then indexes, then foreign keys — psql's layout.
    /// `\d+` adds storage and per-column comments.
    async fn describe(&mut self, name: &str, verbose: bool) -> Vec<Segment> {
        let lit = crate::db::grid::quote_literal(name);

        let verbose_cols = if verbose {
            ",
                    CASE a.attstorage WHEN 'p' THEN 'plain' WHEN 'e' THEN 'external'
                         WHEN 'm' THEN 'main' WHEN 'x' THEN 'extended' ELSE '' END AS \"Storage\",
                    COALESCE(col_description(a.attrelid, a.attnum), '') AS \"Description\""
        } else {
            ""
        };

        let columns_sql = format!(
            "SELECT a.attname AS \"Column\",
                    format_type(a.atttypid, a.atttypmod) AS \"Type\",
                    CASE WHEN a.attnotnull THEN 'not null' ELSE '' END AS \"Nullable\",
                    COALESCE(pg_get_expr(d.adbin, d.adrelid), '') AS \"Default\"{verbose_cols}
             FROM pg_attribute a
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             WHERE a.attrelid = {lit}::regclass AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum"
        );

        let mut segments = vec![Segment::dim(format!("Table \"{name}\"\n"))];
        match self.client.simple_query(&columns_sql).await {
            Ok(messages) => segments.extend(render_messages(messages, self.expanded)),
            Err(e) => return render_error(&e.into()),
        }

        let index_sql = format!(
            "SELECT ci.relname AS \"Index\", pg_get_indexdef(i.indexrelid) AS \"Definition\"
             FROM pg_index i
             JOIN pg_class ci ON ci.oid = i.indexrelid
             WHERE i.indrelid = {lit}::regclass
             ORDER BY i.indisprimary DESC, ci.oid"
        );
        segments.extend(self.section("Indexes:", &index_sql).await);

        let fk_sql = format!(
            "SELECT con.conname AS \"Constraint\",
                    pg_get_constraintdef(con.oid) AS \"Definition\"
             FROM pg_constraint con
             WHERE con.conrelid = {lit}::regclass AND con.contype = 'f'
             ORDER BY con.conname"
        );
        segments.extend(self.section("Foreign-key constraints:", &fk_sql).await);

        segments
    }
}

/// Convert driver messages into rendered output.
///
/// `simple_query` interleaves row-description, row, and command-complete
/// messages; we group consecutive rows into one result set and render each the
/// way psql does, separated by blank lines.
fn render_messages(messages: Vec<SimpleQueryMessage>, expanded: bool) -> Vec<Segment> {
    let opts = FormatOptions {
        expanded,
        null_display: String::new(),
    };

    let mut segments = Vec::new();
    let mut current: Option<ResultSet> = None;
    let mut truncated_from: Option<usize> = None;
    let mut total_rows = 0usize;

    let flush = |current: &mut Option<ResultSet>,
                 segments: &mut Vec<Segment>,
                 truncated_from: &mut Option<usize>,
                 total_rows: &mut usize| {
        if let Some(rs) = current.take() {
            if !segments.is_empty() {
                segments.push(Segment::body("\n"));
            }
            segments.push(Segment::body(format_aligned(&rs, &opts)));
            if let Some(shown) = truncated_from.take() {
                segments.push(Segment::dim(format!(
                    "-- output truncated ({shown} of {} rows)\n",
                    *total_rows
                )));
            }
            *total_rows = 0;
        }
    };

    for msg in messages {
        match msg {
            SimpleQueryMessage::RowDescription(desc) => {
                flush(
                    &mut current,
                    &mut segments,
                    &mut truncated_from,
                    &mut total_rows,
                );
                current = Some(ResultSet {
                    columns: desc.iter().map(|c| c.name().to_string()).collect(),
                    rows: Vec::new(),
                });
            }
            SimpleQueryMessage::Row(row) => {
                total_rows += 1;
                if let Some(rs) = current.as_mut() {
                    if rs.rows.len() < MAX_ROWS {
                        rs.rows.push(
                            (0..row.len())
                                .map(|i| row.get(i).map(str::to_string))
                                .collect(),
                        );
                    } else if truncated_from.is_none() {
                        truncated_from = Some(MAX_ROWS);
                    }
                }
            }
            SimpleQueryMessage::CommandComplete(_) => {
                if current.is_some() {
                    flush(
                        &mut current,
                        &mut segments,
                        &mut truncated_from,
                        &mut total_rows,
                    );
                }
                // A tag with no preceding rows (INSERT/SET/CREATE) — the
                // driver doesn't expose the tag text, so report the row count
                // the way psql's tag would read for DML.
            }
            _ => {}
        }
    }

    flush(
        &mut current,
        &mut segments,
        &mut truncated_from,
        &mut total_rows,
    );

    if segments.is_empty() {
        segments.push(Segment::body(format_command_tag("OK")));
    }
    segments
}

/// Render a failure in psql's shape: `ERROR:  <message>`, then the server's
/// DETAIL/HINT lines if it supplied any.
///
/// Two spaces after the colon is psql's own alignment, not a typo.
fn render_error(err: &AppError) -> Vec<Segment> {
    let mut text = format!("ERROR:  {err}\n");
    if let Some(detail) = err.detail() {
        text.push_str(&detail);
        text.push('\n');
    }
    vec![Segment::error(text)]
}
