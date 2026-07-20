# pgscope — Implementation Plan

A desktop PostgreSQL explorer built with **Rust + Tauri 2**, implementing the high-fidelity design in
`design_handoff_postgres_explorer/` (to be vendored into this repo under `design/`). The design is the
source of truth for look & feel: schema/table tree sidebar, paged data grid, column/index/stats details
panel, FK relationships canvas, and a docked psql-style terminal, in a dark macOS-style window.

- Design spec: `design/README.md` (tokens, layout metrics, per-surface specs — all values verbatim)
- Design markup: `design/Postgres Explorer.dc.html` (inline-styled reference markup + sample data)
- Note: the design HTML references a `support.js` that is not included in the bundle, so it does not
  fully render standalone. Treat the README + markup as the spec rather than opening it live.

Fidelity target: **pixel-perfect** on colors, typography, spacing, and layout. Sample data in the mock
(48.2M-row `events` table etc.) is illustrative; all content is wired to a real PostgreSQL connection.

---

## 1. Decisions taken

Made to keep momentum; each is revisitable. Alternatives listed so a swap is cheap early.

| # | Decision | Rationale | Alternative |
|---|----------|-----------|-------------|
| D1 | **Frontend: React 18 + TypeScript + Vite** | Mainstream Tauri pairing, best tooling; design maps 1:1 to components. | Leptos/Dioxus for all-Rust — viable, but slower iteration on a hifi UI |
| D2 | **Terminal = custom psql-style REPL in Rust** (not an embedded `psql` binary) | No external dependency on `psql` being installed; full control over prompt/coloring to match the design; meta-command subset is tractable. | Spawn real `psql` via `portable-pty` + xterm.js — full psql semantics, but ANSI/PTY plumbing and styling fights the design |
| D3 | **Postgres driver: `tokio-postgres` + `deadpool-postgres`** (rustls TLS) | The app runs *arbitrary* user SQL — compile-time-checked `sqlx` buys nothing here; deadpool gives pooling + per-connection session options. | `sqlx` |
| D4 | **Text-protocol pipeline**: all result values fetched as text (`simple_query`, or `::text` casts for grid) | Uniform rendering for *any* column type (enums, domains, extensions) without `FromSql` impls per type; identical to how psql itself displays data. | Binary protocol + typed decoding — precise but a long tail of type support |
| D5 | **Fonts bundled** (`@fontsource/ibm-plex-mono` 400/500/600) | Desktop app must work offline; no Google Fonts request at runtime. Same rendering as the design. | Runtime Google Fonts link (as in mock) |
| D6 | **Window resizable**, default 1400×880, min 1180×720 | The mock's fixed size + scrolling "desk" is presentation chrome; a real app window resizes. Center grid column is `1fr` so layout flexes; fixed sidebar (236px) / details (266px) / terminal heights per design. | Fixed 1400×880 window |
| D7 | **Titlebar**: macOS → native overlay traffic lights (`titleBarStyle: Overlay`, `hiddenTitle`, traffic-light inset ≈ (14, 15)); Windows/Linux → `decorations: false` + custom-drawn lights wired to close/min/maximize | Native behavior on the primary platform, pixel-fidelity elsewhere. Custom 42px titlebar div with `data-tauri-drag-region` on all platforms. | Custom lights everywhere |
| D8 | **Browse connections are read-only** (`default_transaction_read_only=on`, `statement_timeout=30s` via connection options); the terminal connection is unrestricted | Grid/introspection can never mutate data; the terminal is the intentional escape hatch, exactly like psql. | One shared unrestricted pool |
| D9 | **Passwords in OS keychain** (`keyring` crate); profiles on disk hold no secrets | Table-stakes for a DB client. | Encrypted file store |
| D10 | **Package manager: pnpm** (npm works too) | Fast, standard with Vite/Tauri templates. | npm/yarn |

---

## 2. Architecture

### 2.1 Process model

```
┌────────────────────────────  Tauri app  ────────────────────────────┐
│                                                                     │
│  WebView (React + TS)                Rust core (tokio)              │
│  ┌───────────────────────┐  invoke   ┌───────────────────────────┐  │
│  │ UI components         │──────────▶│ tauri::commands           │  │
│  │ zustand stores        │◀──────────│  ├─ db::pool   (browse,   │  │
│  │ ipc client (typed)    │  events   │  │   read-only, pooled)   │  │
│  └───────────────────────┘           │  ├─ db::introspect        │  │
│                                      │  ├─ db::grid  (page SQL)  │  │
│   renders exactly the design;        │  ├─ repl::session (dedic. │  │
│   all data via IPC, no direct        │  │   client, unrestricted)│  │
│   network from the webview           │  ├─ store:: profiles/     │  │
│                                      │  │   history/saved (fs)   │  │
│                                      │  └─ secrets:: keyring     │  │
│                                      └────────────┬──────────────┘  │
└───────────────────────────────────────────────────┼─────────────────┘
                                                    │ TCP (+TLS via rustls)
                                              PostgreSQL server
```

Connection roles per profile:
- **browse pool** (deadpool, max ~4): introspection + grid paging. Read-only + statement timeout (D8).
- **repl client** (dedicated `tokio_postgres::Client`): one per terminal session ("session 1" in the
  design; multi-session is future work). Holds session state (`\timing`, `SET`s survive).
- **pinger task**: `SELECT 1` every 15s on the browse pool; emits latency events for the titlebar pill.

### 2.2 Backend module layout (`src-tauri/src/`)

```
main.rs / lib.rs      app setup, state, plugin registration
error.rs              AppError (thiserror) → serialized { code, message, detail } for the UI
state.rs              AppState { active: Option<Connection>, repl_sessions, config_paths }
db/
  connect.rs          profile → tokio-postgres Config (TLS, options), pool build, ping
  introspect.rs       schema tree, columns, indexes, stats, FK graph (SQL in §4)
  grid.rs             safe ident quoting, page query builder, count strategy, cancellation
  format.rs           value display rules (truncation caps, NULL handling)
repl/
  session.rs          per-session client, input buffer, prompt state, cancel token
  meta.rs             \d \dt \dn \l \timing \x \? parser + renderers
  table.rs            psql "aligned" table formatter (widths, alignment, (N rows))
store/
  profiles.rs         profiles.json (no secrets) in app_config_dir
  history.rs          history.jsonl append/read (terminal commands)
  saved.rs            saved_queries/*.sql listing + read
secrets.rs            keyring get/set/delete (service "pgscope", account = profile id)
commands.rs           #[tauri::command] wrappers, thin — logic lives in modules above
```

### 2.3 IPC contract

Commands (all async; errors serialize as `AppError`):

```ts
// connection lifecycle
connect(profileId: string): Promise<ConnectionInfo>      // { db, host, port, user, serverVersion, superuser }
disconnect(): Promise<void>
ping(): Promise<number>                                  // ms
listProfiles(): Promise<Profile[]>
saveProfile(p: Profile, password?: string): Promise<void>
deleteProfile(id: string): Promise<void>

// explorer
schemaTree(): Promise<SchemaNode[]>                      // schemas → tables/views with est row counts
tableMeta(schema: string, table: string): Promise<TableMeta>  // columns, indexes, stats
fkGraph(schema: string): Promise<FkGraph>                // cards (table + columns) and edges

// grid
fetchPage(req: PageRequest): Promise<PageResult>
// PageRequest { schema, table, sortCol?, sortDir?, filter?, page, pageSize: 50, mode: 'first'|'prev'|'next'|'last' }
// PageResult  { columns: ColumnMeta[], rows: (string|null)[][], timingMs, totalEst, totalExact?, sql }
cancelGrid(): Promise<void>

// terminal
replOpen(): Promise<ReplSession>                          // { sessionId, prompt: "analytics_prod=#" }
replExec(sessionId: string, input: string): Promise<ReplOutput>
// ReplOutput { segments: Segment[], prompt: string, timingMs? }
// Segment { text: string, kind: 'prompt'|'body'|'dim'|'error' }   // maps to design colors
replCancel(sessionId: string): Promise<void>

// sidebar extras
historyList(): Promise<HistoryItem[]>                     // { input, firstKeyword, at }
savedQueries(): Promise<SavedQuery[]>                     // { name, path, content }
```

Events (backend → UI): `connection:status` `{ state: 'connected'|'connecting'|'lost', latencyMs? }`.

`ColumnMeta = { name, dataType, isPk, isFk, notNull }` — drives header type lines, badges, and cell
coloring. Types are shared by hand-written TS mirrors of the serde structs (option: `tauri-specta`
codegen later).

### 2.4 Frontend structure (`src/`)

```
theme/tokens.css        every design token as a CSS custom property (§6)
theme/base.css          scrollbars, ::selection, @font-face (bundled Plex Mono), blink keyframes
lib/ipc.ts              typed invoke wrappers + event subscriptions
lib/format.ts           compact counts (48.2M), thousands grouping, relative time (41 min ago), ms
state/                  zustand stores:
  connection.ts         status, latency, profile, ConnectionInfo
  ui.ts                 activeTab ('data'|'relationships'), showDetails, terminalCollapsed
  explorer.ts           schema tree, expanded nodes, selected table
  grid.ts               page, sort, filter, rows, timing, totals, loading/error
  terminal.ts           scrollback segments, input, cursor, command history, running
components/
  Titlebar/             traffic lights (platform-adaptive), title, connection pill
  Toolbar/              breadcrumb, Data|Relationships segmented tabs, filter input,
                        Refresh ghost button, + New query accent button
  Sidebar/              Section headers, DatabaseTree, SavedQueries, History
  DataGrid/             HeaderRow, Rows (CSS grid, per-type cell colors), PagingFooter
  DetailsPanel/         ColumnsList (badges), IndexesList, StatsList
  Relationships/        Canvas (dot grid), TableCard, EdgesSvg, caption
  Terminal/             Header, Scrollback (styled <pre>), InputLine (blink cursor), CollapsedBar
  ConnectModal/         net-new surface (§5.8), styled with the same tokens
App.tsx                 window layout per design (titlebar / toolbar / body / terminal)
```

A thin `DataProvider` seam (mock vs Tauri IPC) lets M1 build the entire static UI against the mock's
sample data, then swap to real IPC without touching components.

---

## 3. Milestones

| M | Deliverable | Definition of done |
|---|-------------|--------------------|
| **M0** | Scaffold & design system | `pnpm tauri dev` opens a 1400×880 window; tokens.css + bundled fonts in place; CI runs fmt/clippy/tsc/tests |
| **M1** | Static hifi shell | Every design surface rendered pixel-perfect from mock data behind `DataProvider`; tab switching, details toggle, terminal collapse work; side-by-side visual check vs design done |
| **M2** | DB core | Connect via profile (dev: env `PGSCOPE_DEV_URL`); browse pool (read-only) + repl client; introspection queries return real data; latency pill live |
| **M3** | Explorer wired | Sidebar tree, breadcrumb, grid paging/sorting/filter, details panel, footer timings — all real; refresh works |
| **M4** | Relationships | FK graph from catalog, auto-layout, selected-table highlight, caption; drag-to-reposition (persisted) |
| **M5** | Terminal | psql-style REPL: SQL exec, aligned output, continuation prompts, meta-command subset, \timing, cancel, scrollback, history capture |
| **M6** | Product polish | Connect modal + profiles + keychain; saved queries & history panels wired; keyboard shortcuts; error states; packaging (`tauri build`) for macOS with icon |

Sequencing: M0 → M1 → M2 → {M3, M4, M5 in any order, M3 first recommended} → M6.

---

## 4. PostgreSQL integration

### 4.1 Connection

- `tokio_postgres::Config` from profile: host, port, db, user, password (keychain), `sslmode`
  (prefer/require/disable) via `tokio-postgres-rustls`.
- Browse pool options: `options=-c default_transaction_read_only=on -c statement_timeout=30s -c application_name=pgscope`.
- Repl client: `application_name=pgscope-repl`, no forced read-only, no default timeout.
- Reconnect: pool recycles dead connections automatically; the repl session detects a dropped client,
  prints psql-style `server closed the connection unexpectedly` (error segment), and reconnects on the
  next submit, printing a dim `-- reconnected` notice.
- Cancellation: keep each running statement's `CancelToken`; `cancelGrid`/`replCancel`
  (and terminal Ctrl+C) call `cancel_query`.

### 4.2 Introspection SQL (browse pool)

Schemas with object counts (sidebar tree; `views (3)`, `analytics (schema)` rows):

```sql
SELECT n.nspname                                            AS schema,
       count(*) FILTER (WHERE c.relkind IN ('r','p'))       AS tables,
       count(*) FILTER (WHERE c.relkind IN ('v','m'))       AS views
FROM pg_namespace n
LEFT JOIN pg_class c ON c.relnamespace = n.oid
WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'
GROUP BY n.nspname ORDER BY n.nspname;
```

Tables in a schema with row estimates (sidebar counts `48.2M`, `214`, …):

```sql
SELECT c.relname AS name, c.relkind, c.reltuples::bigint AS est_rows
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m')
ORDER BY c.relkind, c.relname;
```

`reltuples = -1` (never analyzed, PG13+): show `—` in the sidebar and `n/a` in stats.

Columns with PK/FK/NN flags (grid header type lines, details panel badges):

```sql
SELECT a.attname                                    AS name,
       format_type(a.atttypid, a.atttypmod)         AS data_type,
       a.attnotnull                                 AS not_null,
       COALESCE(bool_or(con.contype = 'p'), false)  AS is_pk,
       COALESCE(bool_or(con.contype = 'f'), false)  AS is_fk
FROM pg_attribute a
LEFT JOIN pg_constraint con
       ON con.conrelid = a.attrelid AND a.attnum = ANY (con.conkey)
      AND con.contype IN ('p','f')
WHERE a.attrelid = $1::regclass AND a.attnum > 0 AND NOT a.attisdropped
GROUP BY a.attnum, a.attname, a.atttypid, a.atttypmod, a.attnotnull
ORDER BY a.attnum;
```

Indexes (details panel `btree (user_id, created_at DESC)` lines — derived by stripping the
`CREATE [UNIQUE] INDEX … USING ` prefix from `pg_get_indexdef`, appending `· unique` when applicable):

```sql
SELECT ci.relname AS name, am.amname AS method, i.indisunique, i.indisprimary,
       pg_get_indexdef(i.indexrelid) AS definition
FROM pg_index i
JOIN pg_class ci ON ci.oid = i.indexrelid
JOIN pg_am    am ON am.oid = ci.relam
WHERE i.indrelid = $1::regclass
ORDER BY i.indisprimary DESC, ci.relname;
```

Stats (details panel STATS block):

```sql
SELECT c.reltuples::bigint               AS est_rows,
       pg_total_relation_size(c.oid)     AS total_bytes,
       pg_indexes_size(c.oid)            AS index_bytes,
       s.last_autovacuum
FROM pg_class c
LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid
WHERE c.oid = $1::regclass;
```

FK graph (relationships tab):

```sql
SELECT con.conname,
       sc.relname AS src_table, tc.relname AS tgt_table,
       (SELECT string_agg(a.attname, ', ' ORDER BY k.ord)
          FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
          JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum) AS src_columns,
       (SELECT string_agg(a.attname, ', ' ORDER BY k.ord)
          FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
          JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum) AS tgt_columns
FROM pg_constraint con
JOIN pg_class sc ON sc.oid = con.conrelid
JOIN pg_namespace sn ON sn.oid = sc.relnamespace
JOIN pg_class tc ON tc.oid = con.confrelid
WHERE con.contype = 'f' AND sn.nspname = $1;
```

### 4.3 Grid paging

- Query shape: `SELECT "c1"::text, "c2"::text, … FROM "schema"."table" [WHERE <user filter verbatim>]
  [ORDER BY "col" ASC|DESC] LIMIT 50 OFFSET <n>`.
  - Identifiers quoted by doubling `"` (never user-interpolated unquoted). Sort column must be one of
    the introspected columns; direction is an enum. The **filter string is intentionally raw SQL** —
    this is a DB client; the session it runs on is read-only + time-limited (D8).
- **Last page** (`⇥`): never deep-OFFSET on 48M rows — run the query with the sort **reversed**,
  `LIMIT 50`, and re-reverse client-side.
- **Totals**: unfiltered → `reltuples` estimate, displayed grouped (`48,213,904`) like the design.
  Filtered → `SELECT count(*)` with the filter under a 5s timeout; on timeout show `≥ 50·page` and
  disable `⇥`.
- Footer text = actual executed SQL (ellipsized) + round-trip ms measured in Rust, green per design.
- Guards: per-cell display cap 8 KB (append `…`); page size fixed at 50 (matches `rows 1–50 of …`).

### 4.4 Cell color rules (generalizing the mock's `events` palette)

| Column class | Color token |
|---|---|
| row number gutter | `--text-xfaint` `#3d475a` |
| PK column / other numerics | `--text-secondary` `#8b99b0` |
| FK columns, uuid, timestamps/dates | `--text-dim` `#6b7a92` |
| text/enum (non-FK) | `--accent-light` `#7cb9e8` |
| json/jsonb | `--amber` `#d9b26a` |
| NULL (any type) | `--text-faint` `#4a566b`, rendered as `NULL` |

This reproduces the mock's events table exactly (event_id PK secondary, user_id/session_id FK dim,
event_name accent-light, properties amber, created_at dim) and behaves sensibly for any table.

---

## 5. Feature specs by surface

Layout metrics, colors, and typography for every surface are in `design/README.md`; this section only
specifies *behavior* and where the real app extends the mock.

### 5.1 Titlebar (42px)
- Title: `pgscope — {db}@{host}:{port}`; `pgscope — not connected` before first connect.
- Connection pill: green dot + `connected · {latency}ms` (updated by pinger); amber `connecting…`;
  red `disconnected`. Click → open Connect modal.
- Whole bar is a drag region; traffic lights per D7.

### 5.2 Toolbar (40px)
- Breadcrumb `db ▸ schema ▸ table` reflects the current selection (last segment bright/600).
- Segmented control switches Data | Relationships (`ui.activeTab`), styles per design, no animation.
- Filter input (260px): placeholder `⌕ WHERE event_name = …`; typed text is the SQL after `WHERE`.
  Enter applies (grid reloads page 1), Esc clears. On SQL error: red footer message, grid keeps last
  good rows.
- `↻ Refresh`: re-runs current page + refreshes tableMeta/stats.
- `+ New query`: expands terminal if collapsed and focuses its input.

### 5.3 Sidebar (236px)
- **DATABASE** tree: db root (amber ◆) → schemas (`public` expanded by default, others collapsed with
  counts: `views (3)` groups the schema's views; sibling schemas like `analytics (schema)`) → tables
  with 8px square glyph + compact est counts right-aligned (`48.2M` formatter: <1000 raw; K/M/B, 1dp).
- Selection: click loads grid + details + breadcrumb; selected style = accent-tinted bg + 2px left
  accent border per design. Carets toggle expansion; expanded state persisted in `explorer` store.
- **SAVED QUERIES**: lists `saved_queries/*.sql` from app data dir (amber ▪, name, `.sql` right).
  Click → insert content into terminal input (not auto-run). Management UI is future work.
- **HISTORY**: last N terminal inputs, first keyword accent-colored, relative age (`· 2m`). Click →
  insert into terminal input. Persisted to `history.jsonl`.

### 5.4 Data grid
- Columns from `tableMeta`; header shows name (+ sort arrow `↓`/`↑` on the sorted column) over
  `type[ · PK| · FK]` faint line. Click header toggles sort (single column, v1).
- Column widths by type class, defaulting to the design's values for equivalents:
  row gutter 44, int/bigint 110, short text 104, uuid 150, name-ish text 132, json `1fr` (min 200),
  timestamptz 215, fallback 140. Column resize is future work.
- Row click selects (accent-tinted bg per design). Hover per design. Cmd+C copies selected row as TSV.
- Paging footer: `⇤ ← → ⇥` chips (disabled = faint, no hover), `rows {a}–{b} of {total}`, right side
  = executed SQL + green ms.
- States: loading (dim shimmer on rows area), error (footer red text), empty (`0 rows` centered dim).

### 5.5 Details panel (266px, toggleable)
- Header: table name + `TABLE` / `VIEW` accent badge.
- COLUMNS: all columns (scrolls); badge precedence PK (amber) > FK (blue) > NN (gray); type
  right-aligned faint.
- INDEXES: name + derived definition line (§4.2).
- STATS: est. rows (grouped), total size / indexes (`pg_size_pretty`-style formatter in Rust or TS),
  last autovacuum as relative time; `n/a` fallbacks.
- Toggle: `⌘I` and/or a toolbar affordance sets `ui.showDetails` (default true).

### 5.6 Relationships tab
- Data: `fkGraph(schema)` — cards for tables (name + columns w/ type, PK/FK tags), edges for FKs.
- Layout: deterministic grid seeded from the design's geometry — card width 208 (selected 224),
  x steps of ~322 starting at 70, y steps of ~270 starting at 60, 3 columns. Ordering: selected table
  and its FK neighborhood first, then by FK degree desc. Cap ~9 cards; caption bottom-right
  `{schema} schema · {shown} of {total} tables · FK graph`.
- Edges: straight SVG lines `#3a5f80` 1.5px between nearest card-edge midpoints, 3px accent endpoint
  circles.
- Selected table: accent border + tinted header + glow ring per design; clicking a card selects that
  table (updates breadcrumb/details; grid loads on switching back to Data).
- Drag cards to reposition; positions persisted per schema (app data). (M4, P1.)

### 5.7 psql terminal (bottom dock)
- Header: `psql` + `{db} · session 1` · right: `Timing on` (green, visible when enabled; click
  toggles), `clear` (clears scrollback, `⌘K`), `▾ collapse`. Collapsed 28px bar per design; whole bar
  expands. Optional ~150ms height ease.
- Input model: hidden input overlaying the `<pre>`; typed text renders after the prompt with the
  blinking 7×14px block cursor. Up/Down = command history; Enter submits; Ctrl+C cancels a running
  query (prints `^C`); paste supported.
- Prompting: `{db}=#` (superuser) / `{db}=>` (otherwise) in accent; continuation `{db}-#` when the
  buffer has an unterminated statement (no `;` outside quotes/dollar-quotes — minimal lexer; the
  in-string `'#` prompt variants of real psql are out of scope).
- Execution: buffer sent via `simple_query` (multi-statement OK, values arrive as text). Each result
  rendered by the aligned formatter. **Verified against real psql 18**, which differs from this
  plan's initial assumptions in several ways: embedded newlines wrap *within* the column with a `+`
  continuation marker rather than breaking alignment; header names centre with the extra space on the
  right; the last column of a data row gets no trailing padding while the header line keeps it;
  expanded mode prints no `(N rows)` footer; and numeric alignment is decided by column *type*, so an
  all-NULL column is treated as text. Details: header + `---+---` separator in dim, rows in body color, numerics
  right-aligned (right-align when all non-null values match `^-?[0-9.eE+]+$`), `(N rows)` dim,
  `Time: {ms} ms` dim when `\timing` on. Errors as `ERROR:  {message}` in `#ff5f57`.
- Meta-commands (v1 subset): `\d` (list relations), `\d <table>` (columns/indexes/FKs),
  `\dt`, `\dn`, `\l`, `\timing [on|off]`, `\x` (expanded output), `\?` (help), `\q` (prints note —
  the pane persists). Unknown: `invalid command \foo. Try \? for help.`
- Protection: per-statement output cap 10k rows / ~5 MB with dim `-- output truncated (10000 of N
  rows)` notice; scrollback ring buffer ~200k chars. Every submitted input appended to history store.

### 5.8 Connect modal (net-new; not in the mock)
- Shown on first run / disconnect / pill click. Same tokens: panel `#0f131b`, border `#1f2633`,
  radius 12, IBM Plex Mono.
- Fields: name, host, port, database, user, password, sslmode; Save (profile → JSON, password →
  keychain) and Connect. Profile list with last-used auto-connect on launch.
- Errors surfaced inline (auth/refused/TLS), never as raw panics.

### 5.9 Keyboard shortcuts (M6)
`⌘R` refresh · `⌘F` focus filter · `⌘K` clear terminal · `⌘J` toggle terminal · `⌘I` toggle details ·
`⌘T` focus terminal input · `⌘1/⌘2` Data/Relationships.

---

## 6. Design fidelity plan

- `theme/tokens.css`: every color/size from `design/README.md` as a CSS variable, named semantically
  (`--bg-window: #0c0f15; --bg-panel: #0f131b; --bg-raised: #161c26; --border: #1f2633;
  --border-faint: #1a212e; --row-divider: #131824; --card-border: #263041; --text: #cdd6e4;
  --text-secondary: #8b99b0; --text-dim: #6b7a92; --text-faint: #4a566b; --text-xfaint: #3d475a;
  --accent: #4e9cd8; --accent-light: #7cb9e8; --accent-lighter: #9ecdf0; --amber: #d9b26a;
  --green: #4ec98a; --term-bg: #0a0d12; --term-text: #aebacd; --er-line: #3a5f80; …`) plus the
  accent alpha backgrounds/borders, traffic light colors, selection color.
- `theme/base.css`: webkit scrollbar styling from the mock (10px, `#232b3a` thumb, 2px `#0c0f15`
  border), `::selection`, `@keyframes blink` (1.1s step-end, 50% duty), font-face for bundled
  Plex Mono 400/500/600.
- Sub-pixel font sizes (11.5/9.5px) and exact paddings/heights copied verbatim from the spec; grid
  uses the same `grid-template-columns` approach as the mock.
- **Known deviations (intentional):** resizable window instead of fixed desk (D6); native macOS
  traffic lights (D7); bundled fonts (D5); real scroll/data behavior everywhere the mock is static.
- **Type-name aliasing:** Postgres reports canonical names (`timestamp with time zone`), the design
  labels columns with short aliases (`timestamptz`). The long forms also overflow the 208px ER cards
  and truncate column names, so `shortType()` abbreviates for display; the full name stays in the
  `title` tooltip.
- **Index ordering:** the details panel lists indexes in *creation* order (pkey, user_created, name,
  brin), not alphabetically — sorting by name would put `brin_…` second. The query orders by
  `indisprimary DESC, oid`.
- Verification: run the app beside a faithful static re-render of the design at 1400×880 and compare
  screenshots per surface (Data tab, Relationships tab, collapsed terminal) before calling M1 done.

---

## 7. Dev fixture

To develop against realistic data (and demo the app looking like the design):

- `dev/docker-compose.yml`: `postgres:16` on port **54330** (54329 collided with an
  unrelated container on the dev machine; override with `PGSCOPE_DEV_PORT`), db `analytics_prod`.
- `dev/seed.sql`: recreates the design's world — `public` schema with `users`, `sessions`, `events`
  (bigint PK, text FK user_id, uuid FK session_id, event_name text, properties jsonb, created_at
  timestamptz + the four indexes from the details panel), `page_views`, `experiment_exposures`,
  `event_properties`, `funnels`; 3 views; an `analytics` schema; FKs matching the ER diagram
  (users→sessions, sessions→events, users→events, sessions→page_views, users→experiment_exposures).
- `dev/generate.sql` (or a small Rust bin): a few hundred thousand synthetic rows with the mock's
  shapes (`u_8f3c21` users, uuid sessions, event names page_view/click/signup/…, jsonb like
  `{"path": "/pricing"}`), then `ANALYZE`.
- `PGSCOPE_DEV_URL=postgres://…` env var: dev builds auto-connect to it (skips the modal) until M6.

---

## 8. Testing & verification

- **Rust unit**: ident quoting, page-SQL builder (incl. reversed last-page), aligned-table formatter
  (golden tests against real psql output strings), meta-command parser, statement-terminator lexer
  (quotes/dollar-quoting), value truncation, compact/relative formatters.
- **Rust integration** (against dockerized seed DB, behind a feature flag / CI service): introspection
  queries return expected columns/indexes/FKs; read-only session rejects writes; cancellation works.
- **Frontend**: vitest + testing-library for stores and pure components (formatters, badge precedence,
  pager state); mock `DataProvider`.
- **E2E (optional, M6+)**: tauri-driver/WebdriverIO smoke — connect to seed DB, click table, page,
  run terminal query.
- **Visual**: manual side-by-side per §6; screenshots archived in `design/checks/`.
- Every milestone's DoD includes: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`,
  `tsc --noEmit`, `vitest run` green in CI (GitHub Actions, macOS runner).

---

## 9. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Huge tables: deep OFFSET, `count(*)` cost | reltuples estimates; reversed-order last page; 5s count timeout; statement_timeout=30s on browse |
| Arbitrary types break rendering | Text-protocol pipeline (D4) — everything is a string by the time it leaves Postgres |
| Runaway queries freeze UI | All commands async; cancel tokens wired to Ctrl+C / cancel buttons; output caps |
| psql fidelity rabbit hole (`\` commands, prompt states) | Explicit v1 subset (§5.7); formatter golden-tested against real psql output; everything else prints a clear "not supported" |
| Webview perf on large scrollback/grids | 50-row pages; scrollback ring buffer; cell/output caps; virtualization only if profiling demands it |
| Cross-platform titlebar quirks | D7 platform split; Windows/Linux treated as secondary until M6 |
| Secrets handling | keyring only; profiles.json never stores passwords; no telemetry |

---

## 10. Out of scope (v1)

Multiple simultaneous connections/windows; editing data (grid stays read-only — writes go through the
terminal or a query editor tab); multiple terminal sessions; EXPLAIN visualizer;
CSV/JSON export; column resize/reorder; saved-query management UI; auto-update; Windows/Linux polish
beyond "works"; light theme.

---

## 11. Repo layout (target)

```
pgscope/
├── plan.md · tasks.md
├── design/                      # vendored handoff (README.md, Postgres Explorer.dc.html)
├── dev/                         # docker-compose.yml, seed.sql, generate.sql
├── src/                         # React + TS frontend (§2.4)
├── src-tauri/                   # Rust core (§2.2), tauri.conf.json, icons/
├── .github/workflows/ci.yml
├── package.json · pnpm-lock.yaml · vite.config.ts · tsconfig.json
└── README.md
```
