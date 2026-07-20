# pgscope — Epics & Stories

Backlog for the plan in [plan.md](plan.md). Section references (§) point there.

**Priority** · P0 = required for the design-complete app · P1 = required for a real, daily-usable
client · P2 = polish/stretch.
**Size** · S ≤ ½ day · M = 1–2 days · L = 3–5 days.
Check a box when the story's acceptance criteria pass. Stories within an epic are roughly ordered.

| Milestone | Epics |
|---|---|
| M0 scaffold | E1, E11.1 |
| M1 static shell | E2 |
| M2 db core | E3 |
| M3 explorer wired | E4, E5 |
| M4 relationships | E6 |
| M5 terminal | E7 |
| M6 product polish | E8, E9, E10, E11 |

---

## E1 — Scaffold & design system  *(M0)*

Goal: a running Tauri 2 + React/TS app with the full design token system and dev DB fixture.
Depends on: —

- [x] **E1.1 · P0 · S — Init repo & Tauri scaffold**
  `git init`; create-tauri-app (React + TS + Vite, pnpm); `.gitignore`; README stub.
  *Accept:* `pnpm tauri dev` opens a window; `cargo check` and `tsc --noEmit` pass.

- [x] **E1.2 · P0 · S — Vendor design handoff**
  Copy `design_handoff_postgres_explorer/` → `design/` so the spec is versioned with the code.
  *Accept:* `design/README.md` + `design/Postgres Explorer.dc.html` committed; plan/tasks reference them.

- [x] **E1.3 · P0 · S — Window configuration**
  1400×880 default, min 1180×720, resizable; macOS `titleBarStyle: Overlay` + `hiddenTitle` +
  traffic-light inset ≈ (14,15); `decorations: false` on Windows/Linux (D7).
  *Accept:* window opens at spec size; on macOS native lights sit inside a 42px custom bar area.

- [x] **E1.4 · P0 · M — Design tokens & base styles**
  `theme/tokens.css` with every color/spacing token from `design/README.md` (§6 naming);
  `theme/base.css` with scrollbar styles, `::selection`, blink keyframes.
  *Accept:* tokens render in a temporary swatch page; names match §6; no hex literals in components later.

- [x] **E1.5 · P0 · S — Bundle IBM Plex Mono**
  `@fontsource/ibm-plex-mono` 400/500/600, preloaded; no network font request.
  *Accept:* devtools shows zero requests to fonts.googleapis.com; glyphs match design weights.

- [x] **E1.6 · P0 · M — Dev fixture database**
  `dev/docker-compose.yml` (postgres:16) + `dev/seed.sql` reproducing the design's schema: 7 public
  tables incl. `events` with the exact columns/indexes, 3 views, `analytics` schema, all 5 FK edges;
  `dev/generate.sql` synthetic rows + `ANALYZE` (§7).
  *Accept:* `docker compose up` + seed yields a DB where `\d events` shows the design's columns and
  indexes; row estimates are non-zero.

- [x] **E1.7 · P1 · S — DataProvider seam**
  TS interface for everything the UI consumes (tree, tableMeta, page, fkGraph, repl, history, saved);
  `MockProvider` returning the design's sample data verbatim (rows, cols, terminal transcript).
  *Accept:* provider swappable via context; mock data matches `design/*.html` sample content.

---

## E2 — Static hifi UI shell  *(M1)*

Goal: every design surface pixel-perfect on mock data; the three mock interactions work.
Depends on: E1

- [x] **E2.1 · P0 · M — App layout frame**
  Titlebar / toolbar / body (sidebar · main · details) / terminal dock skeleton with exact heights,
  borders, backgrounds; independent scroll areas (sidebar, grid, details, terminal).
  *Accept:* structure matches design at 1400×880; center pane flexes on resize; no double scrollbars.

- [x] **E2.2 · P0 · S — Titlebar**
  Traffic lights (custom on non-mac), centered title, connection pill (green dot, `connected · 12ms`);
  drag region.
  *Accept:* visually identical to design; window drags by the bar; lights close/min/zoom on non-mac.

- [x] **E2.3 · P0 · M — Toolbar**
  Breadcrumb with ▸ separators; Data|Relationships segmented control wired to `ui.activeTab`;
  filter input (static placeholder); `↻ Refresh` ghost + `+ New query` accent buttons with hover states.
  *Accept:* tab switching flips main area; all styles/hovers per design.

- [x] **E2.4 · P0 · M — Sidebar (DATABASE / SAVED QUERIES / HISTORY)**
  Tree rows with carets, amber ◆ root, table square glyphs, right-aligned counts, selected-row accent
  treatment, section dividers, hover states; mock content verbatim.
  *Accept:* pixel-match against design incl. selected `events` row (48.2M accent count).

- [x] **E2.5 · P0 · L — Data grid (static)**
  CSS-grid header (two-line cells, right borders) + 16 mock rows with per-column colors, ellipsis
  columns, selected `signup` row tint, hover row bg; paging footer with pager chips, counts, SQL + ms.
  *Accept:* pixel-match; column template `44 110 104 150 132 1fr 215`; footer disabled/hover states.

- [x] **E2.6 · P0 · M — Details panel (static)**
  Header + TABLE badge; COLUMNS with PK/FK/NN badges; INDEXES name+definition pairs; STATS pairs;
  `ui.showDetails` toggle removes the pane.
  *Accept:* pixel-match on all three sections; toggle reflows the grid.

- [x] **E2.7 · P0 · M — Relationships canvas (static)**
  Dot-grid background; 5 absolutely-positioned cards at the design's coordinates; highlighted `events`
  card (accent border, tinted header, glow); SVG edges + endpoint dots; caption.
  *Accept:* pixel-match at design coordinates; switching tabs preserves state.

- [x] **E2.8 · P0 · M — Terminal pane (static)**
  Header (`psql`, session label, `Timing on`, `clear`, `▾ collapse`); `<pre>` scrollback with the
  canned transcript (prompt/dim/body coloring); blinking block cursor; collapsed 28px bar; toggle.
  *Accept:* pixel-match incl. blink timing (1.1s step-end); collapse/expand works from both affordances.

- [x] **E2.9 · P0 · S — Visual sign-off pass**
  Side-by-side screenshot comparison vs the design for: Data tab, Relationships tab, collapsed
  terminal, details hidden. Fix discrepancies.
  *Accept:* screenshots in `design/checks/`; no visible deltas at 100% zoom.

---

## E3 — Postgres core & connection lifecycle  *(M2)*

Goal: real connectivity with safe defaults; introspection API complete.
Depends on: E1

- [x] **E3.1 · P0 · M — Connection config & TLS**
  Profile struct → `tokio_postgres::Config`; rustls TLS (`disable`/`prefer`/`require`);
  `application_name` set; connect + error mapping (`AppError`).
  *Accept:* connects to dev fixture and to a TLS-required server; auth/refused/TLS errors surface
  as typed messages, not panics.

- [x] **E3.2 · P0 · M — Browse pool (read-only)**
  deadpool pool (max 4) with `default_transaction_read_only=on`, `statement_timeout=30s` (D8).
  *Accept:* `INSERT` on a browse connection fails with read-only error; long query aborts at 30s;
  pool survives a killed backend (recycles).

- [x] **E3.3 · P0 · S — connect/disconnect/ping commands + status events**
  IPC per §2.3; pinger task every 15s emits `connection:status` with latency.
  *Accept:* UI store receives status/latency; disconnect stops the pinger and clears state.

- [x] **E3.4 · P0 · M — Introspection: schema tree**
  `schemaTree()` from §4.2 queries; compact-count formatting data (raw est_rows over IPC).
  *Accept:* dev fixture returns public (7 tables + est rows), views group (3), analytics schema;
  `reltuples = -1` handled.

- [x] **E3.5 · P0 · M — Introspection: tableMeta**
  Columns (PK/FK/NN flags), indexes (method/unique + display definition derived from indexdef), stats
  (est rows, total/index bytes, last_autovacuum).
  *Accept:* `events` in fixture returns the design's 6 columns with correct badges and 4 indexes with
  the design's definition strings; missing stats → None.

- [x] **E3.6 · P0 · S — Introspection: fkGraph**
  §4.2 FK query + per-table column lists for cards.
  *Accept:* fixture returns exactly the 5 design edges with correct src/tgt columns.

- [x] **E3.7 · P1 · S — Query cancellation plumbing**
  CancelToken registry for grid + repl statements; `cancelGrid`/`replCancel` commands.
  *Accept:* a `pg_sleep(60)` via grid path is cancellable in <1s; backend logs no orphaned tasks.

- [x] **E3.8 · P1 · S — Dev auto-connect**
  `PGSCOPE_DEV_URL` env: dev builds connect on launch, bypassing the (not yet built) modal.
  *Accept:* `pnpm tauri dev` against docker fixture reaches connected state with zero clicks.

---

## E4 — Schema explorer wiring  *(M3)*

Goal: sidebar + breadcrumb live against the real DB.
Depends on: E2, E3

- [x] **E4.1 · P0 · M — Live database tree**
  Replace mock tree with `schemaTree()`; expansion state persisted; compact count formatter
  (48.2M / 910K / 214); views + foreign schemas collapsed by default.
  *Accept:* fixture renders like the design; expanding `views` lists the 3 views; re-launch restores
  expansion.

- [x] **E4.2 · P0 · S — Table selection flow**
  Click table → selected styling, breadcrumb update, grid + details load; selection survives tab
  switches.
  *Accept:* selecting `sessions` updates breadcrumb `analytics_prod ▸ public ▸ sessions`, grid and
  details refresh; selected row styled per design.

- [x] **E4.3 · P1 · S — Titlebar reflects connection**
  Title `pgscope — {db}@{host}:{port}`; pill states connected/connecting/lost with latency updates.
  *Accept:* killing docker flips pill to `disconnected` within one ping interval; restart recovers.

- [x] **E4.4 · P2 · S — Tree refresh**
  Refetch tree on `↻ Refresh` and after reconnect.
  *Accept:* creating a table in psql then Refresh shows it without app restart.

---

## E5 — Data grid & details panel  *(M3)*

Goal: real paged browsing with sort/filter/timing, and a live details panel.
Depends on: E3, E4

- [x] **E5.1 · P0 · M — Page query builder (Rust)**
  Ident quoting, `::text` projection, sort validation, raw filter passthrough, LIMIT/OFFSET; unit
  tests incl. hostile identifiers (`weird"name`, mixed case).
  *Accept:* builder tests green; generated SQL matches footer display string.

- [x] **E5.2 · P0 · M — fetchPage command + grid wiring**
  Execute on browse pool, measure ms, per-cell 8KB cap; grid renders real rows with §4.4 color rules;
  loading/error/empty states.
  *Accept:* fixture `events` browses at 50 rows/page; jsonb amber, FKs dim, text accent-light;
  footer shows real SQL + green ms.

- [x] **E5.3 · P0 · M — Paging controls & totals**
  `⇤ ← → ⇥` with disabled states; reversed-query last page (§4.3); totals: reltuples unfiltered,
  timed `count(*)` filtered (5s cap → `≥ n`, `⇥` disabled).
  *Accept:* on a 500k-row fixture table, `⇥` returns in <1s and shows the true tail; `rows a–b of N`
  formats with grouping.

- [x] **E5.4 · P0 · S — Column sorting**
  Header click toggles asc/desc; `↓`/`↑` indicator beside name; resets to page 1.
  *Accept:* sorting `created_at` flips order; indicator matches design typography.

- [x] **E5.5 · P0 · M — Filter input**
  Enter applies input as `WHERE` clause (page 1), Esc clears; SQL errors → red footer message, last
  good rows retained.
  *Accept:* `event_name = 'signup'` filters; `bogus (((` shows the Postgres error inline; clearing
  restores unfiltered browse.

- [x] **E5.6 · P0 · M — Live details panel**
  Wire COLUMNS/INDEXES/STATS to `tableMeta`; size formatter (`12 GB`), relative time (`41 min ago`);
  badge precedence PK>FK>NN; VIEW badge for views.
  *Accept:* fixture `events` panel matches design content structure with real values; view selection
  shows VIEW badge and no stats crash.

- [x] **E5.7 · P1 · S — Refresh action**
  `↻` re-runs current page + tableMeta; spinner on button while in flight.
  *Accept:* inserting a row via terminal then Refresh shows it (on first page with matching sort).

- [x] **E5.8 · P1 · S — Row selection & copy**
  Click selects row (design tint); `⌘C` copies TSV; click-away deselects.
  *Accept:* clipboard holds tab-separated values incl. NULL as empty.

- [x] **E5.9 · P2 · M — Adaptive column widths**
  Type-class width table (§5.4) applied to arbitrary tables; json columns get `1fr`.
  *Accept:* `users`/`sessions`/`funnels` all render sensibly without horizontal squish at 1400px.

---

## E6 — Relationships graph  *(M4)*

Goal: real FK diagram for any schema, matching the design's aesthetics.
Depends on: E3, E4

- [x] **E6.1 · P0 · M — Live cards from fkGraph**
  TableCard component fed by real columns (name/type + PK/FK tags); selected table gets the
  highlighted treatment.
  *Accept:* fixture renders 5+ cards with correct tags; selected `events` styled per design.

- [x] **E6.2 · P0 · M — Auto-layout**
  Deterministic grid layout (§5.6): selection + FK-neighborhood first, FK-degree ordering, 3 columns,
  cap ~9 cards; caption `{schema} schema · {n} of {m} tables · FK graph`.
  *Accept:* fixture layout is stable across runs and readable; caption counts correct.

- [x] **E6.3 · P0 · S — SVG edges**
  Straight lines between nearest card-edge midpoints, `#3a5f80` 1.5px, 3px accent endpoint dots;
  edges recompute from card positions.
  *Accept:* all fixture FKs drawn; endpoints touch card borders, not centers.

- [x] **E6.4 · P1 · M — Drag to reposition (persisted)**
  Pointer-drag cards; edges follow live; positions saved per schema and restored.
  *Accept:* rearranged layout survives app restart; reset available (double-click canvas / context).

- [x] **E6.5 · P1 · S — Card click → select table**
  Clicking a card selects it (details/breadcrumb update, highlight moves).
  *Accept:* selecting `sessions` card then switching to Data shows sessions rows.

---

## E7 — psql terminal  *(M5)*

Goal: a genuinely usable psql-style REPL matching the design's rendering.
Depends on: E3

- [x] **E7.1 · P0 · M — Repl session backend**
  Dedicated client per session; `replOpen`/`replExec`; prompt computed from db + superuser
  (`=#`/`=>`); session survives across UI tab switches.
  *Accept:* `SELECT 1;` returns a formatted result; `SET` persists across statements (session state).

- [x] **E7.2 · P0 · M — Statement gathering & continuation**
  Minimal lexer: terminator `;` outside strings/dollar-quotes; multi-line buffer; continuation prompt
  `{db}-#`; multi-statement input via `simple_query`.
  *Accept:* the design's 3-line GROUP BY query entered line-by-line shows `-#` prompts then one
  result; `SELECT 'a;b';` is not split; `$$ … ; … $$` not split.

- [x] **E7.3 · P0 · L — Aligned table formatter**
  psql "aligned" output: padded headers, `---+---` separator, numeric right-align heuristic, `(N
  rows)`; golden tests against captured psql output for ≥6 result shapes (empty, 1 col, wide, NULLs,
  unicode widths, negative numbers).
  *Accept:* fixture `SELECT event_name, count(*) … LIMIT 5` renders byte-identical to psql (mod
  trailing whitespace); goldens in repo.

- [x] **E7.4 · P0 · M — Scrollback & input line UX**
  Hidden input; typed text after prompt with blinking block cursor; Up/Down history; Enter submit;
  paste; auto-scroll to bottom on output; scrollback ring buffer (~200k chars).
  *Accept:* feels like a terminal for typing/history/paste; blink matches design; no caret artifacts.

- [x] **E7.5 · P0 · M — Meta-commands subset**
  `\d`, `\d <table>`, `\dt`, `\dn`, `\l`, `\timing [on|off]`, `\x`, `\?`, `\q`(notice); unknown →
  psql-style error. Rendering reuses introspection + formatter.
  *Accept:* `\d events` lists columns/indexes/FKs like psql's layout; `\timing on` then a query
  prints `Time: … ms` dim; `\x` flips to expanded records.

- [x] **E7.6 · P0 · S — Error rendering**
  `ERROR:  message` (+ `LINE`/`HINT` when present) in `#ff5f57`; session continues.
  *Accept:* syntax error renders like psql; next prompt appears; buffer cleared.

- [x] **E7.7 · P1 · S — Cancel with Ctrl+C**
  Running statement: Ctrl+C cancels via token, prints `^C` + cancellation error dim; idle: clears
  input buffer and reprints prompt (psql behavior).
  *Accept:* `SELECT pg_sleep(60);` cancels in <1s; UI never blocks while running.

- [x] **E7.8 · P1 · S — Output caps & clear**
  10k rows / ~5MB per statement cap with dim truncation notice; `clear` header action + `⌘K`.
  *Accept:* `SELECT * FROM events` (500k fixture rows) stays responsive and shows the notice.

- [x] **E7.9 · P1 · S — Header states & toggles**
  `Timing on` green indicator (click toggles, mirrors `\timing`); `▾ collapse`/expand with ~150ms
  ease; `+ New query` toolbar button focuses input (expands first).
  *Accept:* all header affordances behave; collapsed bar matches design.

- [x] **E7.10 · P1 · S — Reconnect on dropped session**
  Detect dead client; print psql-style connection-lost error; transparently reconnect on next submit
  with dim `-- reconnected` notice.
  *Accept:* docker restart mid-session: next query succeeds after the notice; `\timing` state reset
  documented (or restored).

---

## E8 — Saved queries & history  *(M6)*

Goal: the sidebar's lower sections backed by real persistence.
Depends on: E7 (history source), E2

- [x] **E8.1 · P0 · S — History persistence**
  Append every submitted terminal input to `history.jsonl` (input, ts); load last N on launch.
  *Accept:* entries survive restart; file capped (e.g. 1k entries, oldest dropped).

- [x] **E8.2 · P0 · S — History panel wiring**
  Render with first-keyword accent + relative age (`· 2m`), live-updating; click inserts into
  terminal input (no auto-run).
  *Accept:* matches design typography; clicking `\d events` puts it at the prompt ready to edit.

- [x] **E8.3 · P0 · S — Saved queries listing**
  `saved_queries/*.sql` in app data dir; seed the three design examples on first run; click inserts
  content into terminal input.
  *Accept:* files added externally appear after Refresh; missing dir auto-created.

- [x] **E8.4 · P2 · M — Save current query**
  Affordance to save the last executed terminal statement as a named `.sql`.
  *Accept:* saved file appears in the panel and on disk; name collisions handled.

---

## E9 — Connection manager  *(M6)*

Goal: first-run and multi-profile connect flow with secure secrets.
Depends on: E3

- [x] **E9.1 · P0 · M — Connect modal UI**
  Design-token-styled modal (§5.8): fields name/host/port/database/user/password/sslmode; Connect +
  Save; inline error area; shown on first run and via pill click.
  *Accept:* visually consistent with the app; tab order and Enter-to-connect work; errors readable.

- [x] **E9.2 · P0 · M — Profiles + keychain**
  `profiles.json` (no secrets) + `keyring` for passwords; list/create/edit/delete; last-used
  auto-connect on launch.
  *Accept:* password absent from all files (verified by grep); delete removes keychain entry;
  auto-connect lands in connected state.

- [x] **E9.3 · P1 · S — Disconnect / switch profile**
  Pill menu or modal action: disconnect (clears state to modal) and switch between profiles.
  *Accept:* switching fixture→other DB rebuilds tree/grid/terminal session cleanly; no stale data
  flashes.

---

## E10 — Hardening & polish  *(M6)*

Goal: daily-driver quality.
Depends on: E4–E9

- [x] **E10.1 · P1 · S — Keyboard shortcuts**
  §5.9 set: ⌘R ⌘F ⌘K ⌘J ⌘I ⌘T ⌘1/⌘2 (+ menu entries on macOS).
  *Accept:* all bound and discoverable in the app menu; no conflicts with webview defaults.

- [x] **E10.2 · P1 · M — Global error & empty states audit**
  Consistent surfaces: grid footer errors, terminal errors, tree failures (banner), lost-connection
  behavior in every pane; no unhandled promise rejections.
  *Accept:* scripted failure walkthrough (kill db, bad filter, bad SQL, huge output) shows designed
  states everywhere; console clean.

- [x] **E10.3 · P1 · S — Performance pass**
  Profile: tab switch, 50-row render, terminal 10k-row dump, tree with 500 tables. Fix jank; memoize
  rows; verify ring buffer.
  *Accept:* interactions <16ms script time in profile on target hardware; no unbounded memory growth
  in a 30-min session.

- [x] **E10.4 · P1 · S — Window-state persistence**
  Remember size/position (+ `ui` toggles: details, terminal collapsed, active tab) across launches.
  *Accept:* relaunch restores geometry and toggles.

- [x] **E10.5 · P2 · S — Relative-time & latency refresh**
  History ages and `last autovacuum` re-render on an interval; pill latency smoothing.
  *Accept:* `· 2m` becomes `· 3m` without interaction.

- [x] **E10.6 · P2 · M — Windows/Linux titlebar parity** — *Linux verified; Windows still untested*
  Custom traffic lights wired to window ops; drag region.
  Smoke-tested on Linux via `dev/linux-smoke/` — an Ubuntu 24.04 container that builds the app for
  Linux and runs it under Xvfb. Verified: the app builds and launches, connects to the fixture,
  renders the full UI, and draws its own traffic lights (screenshot: `design/checks/linux-titlebar.png`);
  clicking the red light terminates the app, so the controls are really wired, not just painted.
  **Fixed here:** `titleBarStyle`/`hiddenTitle` are macOS-only keys, so on Windows/Linux `decorations`
  stayed at its `true` default and the OS title bar would have stacked on top of our custom 42px bar.
  `lib.rs` now calls `set_decorations(false)` on non-macOS.
  **Not verified:** minimize/maximize are window-manager operations and are no-ops under a bare Xvfb
  server, so the container cannot exercise them; and Windows has not been run at all.
  *Accept:* smoke-tested on one Windows or Linux machine; controls behave natively enough.

---

## E11 — CI & packaging  *(starts M0, finishes M6)*

Goal: green pipeline and installable builds.
Depends on: E1 (E11.1), then all

- [x] **E11.1 · P0 · S — CI pipeline**
  GitHub Actions (macOS runner): `cargo fmt --check`, `clippy -D warnings`, `cargo test`,
  `tsc --noEmit`, `vitest run`; cache rust/pnpm.
  *Accept:* red on any failure; runtime <10 min warm.

- [x] **E11.2 · P1 · M — Integration tests vs dockerized fixture**
  Service container in CI running seed DB; Rust integration tests (introspection shapes, read-only
  enforcement, cancellation, formatter goldens vs psql).
  *Accept:* suite green in CI; runnable locally via `cargo test --features integration`.

- [x] **E11.3 · P1 · M — macOS packaging** — *unsigned; notarisation not executed*
  `tauri build`: app icon (accent/amber database mark), bundle id `dev.pgscope.app`, DMG.
  Signing/notarisation is wired into `.github/workflows/release.yml` via `APPLE_*` secrets but has
  not been run — no Apple Developer credentials on this machine, so the bundle is unsigned and needs
  a right-click → Open (or `xattr -dr com.apple.quarantine`) on first launch.
  *Accept:* DMG installs and launches on a clean macOS user account; icon renders in Dock.

- [x] **E11.4 · P2 · S — Release automation**
  Tag-triggered workflow producing artifacts + changelog stub.
  *Accept:* `v0.1.0` tag yields downloadable build from the Actions run.

---

## Suggested order

E1 → E2 (sign off fidelity early) → E3 → E4 → E5 → E7 → E6 → E8 → E9 → E10 → E11.2–E11.4.
E11.1 lands with E1. E6 and E7 are independent after E3/E4 and can run in parallel.

**P0 total:** 34 stories ≈ the design-complete, fixture-connected app (M0–M6 core).
**P1 adds:** 18 stories → daily-usable client. **P2:** 7 stories → stretch.

---

## Status

**69 of 69 stories complete.**

Caveats worth carrying forward, none of which are open work items but all of which are gaps in
*verification* rather than implementation:
- **Windows has never been run.** E10.6 was closed on a Linux smoke test (`dev/linux-smoke/`); the
  same code path serves Windows but nobody has executed it there.
- **Minimize/maximize are unverified.** They are window-manager operations and no-ops under the
  container's bare Xvfb server. Only *close* was asserted.
- **The macOS bundle is unsigned.** Notarisation is wired into the release workflow but has never
  run — no Apple Developer credentials available.

Verification as of the last pass:
- `cargo test --lib` — **88 passed** (query builder, statement lexer, meta-command parser, psql
  aligned-output goldens captured from real psql 18, storage, ER layout)
- `cargo test --features integration` — **31 passed** against the dockerised fixture (introspection
  shapes, paging incl. the reverse-scan last page, filtering, cancellation, read-only enforcement,
  the psql REPL, and interaction performance budgets)
- `vitest run` — **49 passed** (formatters, cell-colour rules, ER layout)
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` — clean
- `tsc --noEmit` — clean
- Visual check against the design: `design/checks/`

Three bugs were found and fixed during implementation:
1. **Terminal Ctrl+C never worked.** The cancel handle was read through the session mutex, which
   `submit` holds for the entire query — so a cancel could only fire once the statement it wanted to
   kill had already finished. Handles now live outside that mutex.
2. **A timed-out filtered `count(*)` poisoned the connection pool.** Abandoning the future left the
   statement running server-side, and deadpool recycled the busy connection; the next page query
   queued behind it until `statement_timeout` (30s) reaped it, turning a ~6s call into a ~31s one.
   The timeout now issues a real cancel. Regression test:
   `a_timed_out_count_cancels_the_statement_server_side`.
3. **Windows/Linux would have shown two title bars.** `titleBarStyle: Overlay` and `hiddenTitle` are
   macOS-only config keys; elsewhere they are ignored and `decorations` stays `true`, so the OS's own
   title bar would have rendered above the custom 42px one. `lib.rs` now disables decorations on
   non-macOS. Found by auditing E10.6 rather than by a test.
