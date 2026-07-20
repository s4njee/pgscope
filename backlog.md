# pgscope — Feature Backlog

Ideas for where the app goes after v0.1. Unlike [tasks.md](tasks.md) (the executed build plan),
nothing here is committed work — this is a menu, roughly ordered within each theme by
value-for-effort. Sizes are the same scale as tasks.md: S ≤ ½ day · M = 1–2 days · L = 3–5 days ·
XL = a week+.

Items marked ⬆ were explicitly deferred in [plan.md §10](plan.md) and already have architectural
seams waiting for them.

---

## 1. Query & editor

- ~~**Query editor tabs** ⬆ · XL~~ — **shipped.** CodeMirror 6 tabs in the toolbar's segmented
  control: PostgreSQL highlighting themed from the design tokens, schema-aware autocomplete fed by
  the explorer's introspection, `⌘↵` run-at-cursor / `⌘⇧↵` run-all, per-statement result tabs, and a
  dedicated write-capable connection separate from both the browse pool and the terminal. Saved
  queries now open here instead of the psql prompt. Statement boundaries come from the REPL's lexer
  over IPC so the two surfaces can't drift.
  *Still open from the original idea:* a split/side-by-side layout.
- ~~**Autocomplete in the terminal** · L~~ — **shipped.** `Tab` completes in the psql pane with
  psql's two-stage behaviour (insert the unambiguous part, list candidates on a second press).
  Context-aware: relations after `FROM`/`JOIN`/`UPDATE`, columns after `SELECT`/`WHERE`/`GROUP BY`,
  meta-commands after a backslash. Resolves table aliases, so `u.` in a join offers the right
  table's columns. Runs on the session's own connection rather than cached introspection, so it
  sees that session's `search_path` and temp tables.
  *Still open from the original idea:* function-name and schema-name completion, and completing
  column names inside `INSERT INTO t (…)` column lists.
- ~~**Save editor tab to disk** · S~~ — **shipped.** `⌘S` saves in place when the tab came from a
  file, otherwise asks for a name and adopts the created file; `⌘⇧S` always asks. A new
  `save_query_at` command writes by path, with a canonicalisation check so the frontend cannot use
  it to write outside the saved-queries directory. Naming uses an in-app modal because
  `window.prompt` is a no-op in a Tauri webview.
  *Still open from the original idea:* a close-with-unsaved-changes confirmation, and opening `.sql`
  files from outside the saved-queries directory.
- ~~**EXPLAIN visualizer** ⬆ · L~~ — **shipped.** `⌘E` / `⌘⇧E` in the editor render a collapsible
  plan tree with self-time bars, misestimation badges, and seq-scan flags. EXPLAIN ANALYZE runs
  inside a rolled-back transaction so explaining a DELETE can't destroy data. Loop-aware and
  parallel-aware: inner-loop nodes are multiplied by their loop count, and nodes under a Gather are
  labelled as CPU-across-workers rather than showing a time that exceeds the query's own.
  *Still open from the original idea:* an "explain this" action on the data grid's footer SQL, and
  surfacing plans for statements run in the psql pane.
- **SQL formatting** · S — A `\format`-style action (and ⌘⇧F) that pretty-prints the current
  input buffer before running it.
- ~~**Saved-query management UI** ⬆ · M~~ — **shipped.** Right-click the sidebar list for rename,
  duplicate, move-to-folder, delete (behind a confirmation with cancel focused), and new folder;
  right-click a folder to rename it. Folders are plain subdirectories up to four deep, so the tree
  is a filesystem view rather than a second source of truth; empty ones are listed separately since
  a folder that stays invisible until filled isn't usable. Renaming a folder is one atomic directory
  rename that also works when empty, and it reports each query's before/after path so open editor
  tabs follow their files — a deleted query detaches its tab and marks it dirty instead of
  discarding unsaved edits. Every webview-supplied path is canonicalised and required to resolve
  under the saved-queries directory, compared by whole path components so `saved_queries_evil`
  can't pass on a string prefix.
  *Deliberately not done:* drag-to-reorder. It needs an order file beside the queries, which goes
  stale the moment a `.sql` is added outside the app — the exact second-source-of-truth problem the
  folders-are-directories design avoids. Sorting is alphabetical, folders first.
  *Still open from the original idea:* dragging a query between folders (the menu does it today).
- **Query parameters** · M — Detect `:name` / `$1` placeholders in a saved query and prompt for
  values in a small design-token modal before running.

## 2. Data grid

- ~~**Inline row expansion** · M~~ — **shipped.** A `⤢` on truncated/`json` cells (or double-click)
  opens a panel under the row; `Esc` closes. Re-fetches uncapped (4MB ceiling) so the 8KB grid cap
  no longer hides the value, pretty-prints JSON server-side with `jsonb_pretty`, and highlights with
  a tokeniser rather than a regex so braces and digits inside strings aren't miscoloured. Locates
  the row by primary key, falling back to page position with a visible `located by position` note.
  *Still open from the original idea:* collapsible sub-trees for deep JSON, and a
  jsonpath/key filter within a large document.
- ~~**Column resize & reorder** ⬆ · M~~ — **shipped.** Drag a header's right edge to resize
  (double-click it to reset), drag the header to reorder with a drop indicator, right-click for
  reset-this-width / reset-layout. Persisted per table in `grid_layout.json`, sibling to
  `er_layout.json`. A 4px threshold separates drag from click, so click-to-sort still works; the
  header became a `div` to host the resize grip, with Enter/Space and `aria-sort` wired back.
  Reordering is a view permutation only — each column carries its canonical index so row lookups
  can't drift to the display position — and a stale saved order skips dropped columns and *appends*
  unknown ones, since applying it naively makes a newly added column invisible.
  *Still open from the original idea:* auto-fit a column to its content, and a global "compact /
  comfortable" density toggle.
- ~~**Multi-column sort** · S~~ — **shipped.** Shift-click appends a column to the sort (cycling
  asc → desc → removed); the header shows each key's direction and ordinal. `PageRequest` now carries
  a `sort: Vec<SortKey>` list, every column is validated against introspection, duplicates are
  rejected, and the reverse-scan last page flips every key. The WHERE/ORDER builders are now shared
  between the grid and cell expansion so the two cannot disagree about row order — which is what
  "located by position" depends on.
  *Still open from the original idea:* `NULLS FIRST/LAST` control, and persisting a table's sort
  across sessions.
- ~~**Cell/row context menu** · M~~ — **shipped.** Right-click a cell for: copy cell, expand cell,
  copy row as JSON/CSV/INSERT, and filter to/away from the value (`IS NULL` / `IS NOT NULL` for null
  cells). Literals and row formats are generated in Rust beside `quote_literal` so escaping has one
  implementation and knows each column's type; new predicates AND onto the existing filter with it
  parenthesised. Round-trip tested: generated predicates are fed back through the grid and must
  select the row they came from.
  *Still open from the original idea:* copy several selected rows at once, and a header context menu
  for column-level actions.
- **Keyset pagination** · M — For sorted browses on an indexed column, page by `WHERE col > last`
  instead of OFFSET — the reverse-scan trick fixed the *last* page; this fixes pages 500–999,999
  in between.
- **Editable cells (opt-in)** · XL — Double-click to edit, generating an UPDATE preview that runs
  on the *terminal's* connection (the browse pool stays read-only by design, D8). Needs a PK,
  a confirmation affordance, and a visible "this will write" treatment. High value, high care.
- **Export page/result to CSV / JSON / Markdown** ⬆ · M — From the grid footer and from terminal
  results. Rust side streams via the existing text pipeline; file save via Tauri dialog plugin.

## 3. Schema & relationships

- **ER graph auto-layout upgrade** · M — Optional force-directed layout for schemas with >9
  connected tables (the current grid + FK-degree ordering caps at `MAX_CARDS`), plus pan/zoom on
  the canvas and a "fit" button.
- **Column-anchored FK edges** · M — Draw edges from the specific FK column row to the target PK
  row (the data is already in `FkEdge.srcColumns`/`tgtColumns`), not just card edge midpoints.
- **DDL viewer** · M — A "Definition" section in the details panel (or `\d+`-style REPL output)
  showing `pg_get_viewdef` for views and a reconstructed `CREATE TABLE` for tables, in a
  copyable block.
- **Table search / jump** · S — ⌘P fuzzy-finder over all schemas/tables/views, reusing the tree
  cache. The sidebar scales poorly past a few hundred tables without it.
- **Row-count refresh action** · S — Right-click a table → `ANALYZE` (via the terminal
  connection) to fix the `—` shown for never-analyzed tables.
- **Schema diff** · XL — Compare two connections (or two schemas) and render added/removed/changed
  tables, columns, and indexes. Builds directly on `introspect.rs`; the hard part is presentation.

## 4. Terminal

- **Multiple sessions** ⬆ · M — The backend already keys sessions by id and the design labels the
  pane `session 1`; add tabs in the terminal header, each with its own connection and scrollback.
- ~~**Resizable pane** ⬆ · S~~ — **shipped.** Drag the pane's top edge, or focus the handle for
  ↑/↓ (Home/End for the extremes, double-click to reset). Height persists next to
  `terminalCollapsed` and is clamped to 70% of the window — on rehydration too, so a height saved
  on a large monitor can't leave the pane unusable on a laptop, and on live window resize. Pointer
  events with capture, and pointermove writes the height straight to the DOM node so a long
  scrollback isn't re-rendered at pointer-event rate.
  *Still open from the original idea:* the same treatment for the sidebar and details panel.
- ~~**Broader meta-command coverage** · M~~ — **shipped.** The `\d` family (`\dt \dv \dm \di \ds
  \df \dn \du \dx \l`, combinable as `\dtv`) with psql's `S` and `+` modifiers, plus `\conninfo`,
  `\encoding`, and `\i` running a saved query. Every listing takes a psql name pattern, compiled to
  the same anchored POSIX regexes psql builds rather than a `LIKE` approximation — so `*`/`?`
  wildcards, `schema.name` qualification, and quoted-means-case-sensitive all behave. Catalog SQL is
  built by pure functions and unit-tested as strings, then every form is executed against the
  fixture, because a string test cannot catch a column that does not exist.
  *Still open from the original idea:* `\copy` (client-side COPY is a protocol feature, not a
  listing) and `\e` opening an editor tab — both now report *why* they're unavailable rather than
  "invalid command".
- **Search scrollback** · S — ⌘F within the pane; highlight matches in the segment renderer.
- **Result-set click-through** · M — Render table names in `\dt`-style output as clickable —
  selecting the table in the explorer. Segments already carry structured kinds; add a `link` kind.
- **`\watch` support** · M — Re-run the last statement on an interval with in-place output
  replacement, plus a visible stop affordance.

## 5. Connections & environments

- **Environment badges** · S — Tag profiles dev/staging/prod; prod gets an amber titlebar accent
  and a confirm-before-write prompt in the terminal. Cheap insurance against the classic mistake.
- **SSH tunnels** · L — Tunnel-per-profile (host/user/key) so remote databases work without
  manual `ssh -L`. Rust-side via `russh`; keychain already stores secrets per profile id.
- **Multiple simultaneous connections** ⬆ · XL — Sidebar shows several database roots at once;
  every store keys state by connection id. The biggest structural change on this list — most
  stores currently assume one active connection.
- **Connection health page** · M — Click the latency pill → mini panel: server version, uptime,
  connection counts by state from `pg_stat_activity`, cache hit ratio.
- **Read-only profile flag** · S — Per-profile toggle that applies the browse pool's
  `default_transaction_read_only=on` to the *terminal* connection too, for browsing prod safely.

## 6. Observability & DBA tools

- **Activity view** · L — A live `pg_stat_activity` table (new sidebar section or tab): running
  queries, states, wait events, with a cancel button wired to the existing CancelToken plumbing.
- **Table bloat & index usage panel** · M — Extend the STATS section: seq vs index scans,
  unused-index warnings, dead-tuple ratio — the queries are standard catalog fare.
- **Lock inspector** · M — Who blocks whom (`pg_locks` joined to activity), rendered as a small
  dependency list; pairs naturally with the activity view.
- **Slow-query capture** · M — If `pg_stat_statements` is installed, a "Top queries" panel with
  mean/total time and calls; degrade gracefully when the extension is absent.

## 7. Platform & distribution

- **Windows smoke test + parity pass** · M — The one gap tasks.md acknowledges: run the
  non-macOS titlebar path on real Windows, fix the inevitable paper cuts (drag region,
  double-click maximize, snap layouts).
- **Signing & notarisation** · S (given credentials) — The release workflow already reads
  `APPLE_*` secrets; this is an account, not code.
- **Auto-update** ⬆ · M — `tauri-plugin-updater` against GitHub releases; the tag-triggered
  release workflow already produces the artifacts.
- **Light theme** ⬆ · L — The token system makes it mechanical (`tokens.css` is the single source
  of truth), but a good light palette for this design is real design work, not a find-replace.
- **Homebrew cask** · S — `brew install --cask pgscope` once releases are signed.

## 8. Quality-of-life

- **Command palette** · M — ⌘K (move terminal-clear to ⌘⇧K): every action — tables, saved
  queries, tab switches, connect — searchable in one place.
- **History search & pinning** · S — Filter box above the HISTORY section; pin favourites to the
  top. `history.jsonl` already persists everything needed.
- **Per-table default sort memory** · S — Remember last sort column/direction per table alongside
  the ER layout positions.
- **Drag-and-drop .sql files** · S — Drop a file on the window → contents into the terminal input
  (or future editor tab).
- **Row detail sidebar** · M — Selecting a row optionally swaps the details panel to a
  field-by-field record view (like `\x` for the grid), with jsonb pretty-printed.

---

## Suggested next slice ("v0.2")

If the goal is the biggest daily-driver jump for roughly two weeks of work:

1. Save editor tab to disk (⌘S) — S · closes the obvious gap left by the editor
2. Table search / jump (3.4) — S
3. Cell context menu + copy/export (2.4, 2.7) — M+M
4. Inline jsonb expansion (2.1) — M
5. Terminal autocomplete (1.2) — L
6. Environment badges + read-only profiles (5.1, 5.5) — S+S
7. Windows smoke test (7.1) — M
8. Saved-query management (1.5) — M

That slice touches every surface a user hits daily, closes the Windows gap, and defers the two
structural epics (editor tabs, multi-connection) until the smaller wins prove which one users
actually pull toward.
