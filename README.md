# pgscope

A desktop PostgreSQL explorer with a docked psql pane — schema tree, paged data
grid, column/index/stats panel, foreign-key relationship graph, and an
integrated psql-style terminal.

Built with **Rust + Tauri 2** and a **React + TypeScript** frontend, implementing
the high-fidelity design in [`design/`](design/).

![The Data tab: schema tree, paged grid, and the details panel](docs/screenshots/data-tab.png)

## Quick start

```bash
pnpm install

# Bring up the dev fixture (a seeded analytics database)
cd dev && docker compose up -d && cd ..

# Run the app against it
PGSCOPE_DEV_URL=postgres://pgscope:pgscope@localhost:54330/analytics_prod pnpm tauri dev
```

Without `PGSCOPE_DEV_URL` the app opens its connect dialog; profiles are saved to
disk and passwords go to the OS keychain.

### Demo mode (no database, no Rust)

```bash
pnpm dev          # http://localhost:1425
```

Outside the Tauri shell there is no `invoke` to call, so `lib/ipc.ts` serves every
command from the in-memory fixtures in [`src/lib/demo.ts`](src/lib/demo.ts)
instead — a seeded analytics database mirroring [`dev/seed.sql`](dev/seed.sql),
with ~7,000 deterministically generated rows. This drives the *real* components
and stores, not a mock-up of them, which is why the screenshots below are taken
from it:

```bash
node scripts/screenshots.mjs      # regenerates docs/screenshots/
```

The capture script drives Chrome through the app and fails loudly if a button it
clicks has been renamed — a screenshot of a mock-up would instead go stale in
silence.

## Design

`design/README.md` is the visual specification — colors, typography, spacing, and
per-surface layout — and is the source of truth for the UI. Every token lives in
[`src/theme/tokens.css`](src/theme/tokens.css); components reference those
variables rather than raw hex values.

Intentional deviations from the mock are listed in [plan.md §6](plan.md#6-design-fidelity-plan):
a resizable window instead of a fixed 1400×880 desk, native macOS traffic
lights, bundled fonts, and short type aliases (`timestamptz` rather than
Postgres's canonical `timestamp with time zone`, which overflows the ER cards).

## Architecture

The webview never talks to PostgreSQL directly. All database access lives in the
Rust core behind Tauri commands:

| Connection | Purpose | Session options |
|---|---|---|
| **browse pool** (deadpool, max 4) | introspection + grid paging | `default_transaction_read_only=on`, `statement_timeout=30s` |
| **repl client** (one per terminal session) | the psql pane | unrestricted — this is the intentional escape hatch |

Browsing therefore *cannot* mutate data; writes go through the terminal, exactly
as they would in psql. Values are fetched as text so any column type — enums,
domains, extension types, arrays — renders without a per-type decoder.

See [plan.md](plan.md) for the full architecture and [tasks.md](tasks.md) for the
epic/story backlog.

```
src/                  React + TypeScript frontend
  theme/              design tokens, base styles, component CSS
  lib/                IPC client, formatters, cell-colour rules, ER layout
  state/              zustand stores (connection, explorer, grid, terminal, ui)
  components/         one directory-level component per design surface
src-tauri/src/
  db/                 connection, introspection, grid query builder
  repl/               lexer, meta-commands, psql output formatter, session
  store.rs            profiles, history, saved queries, ER layout (on disk)
  secrets.rs          passwords (OS keychain only)
  commands.rs         Tauri command surface
dev/                  dockerised fixture reproducing the design's database
```

## Development

```bash
pnpm tauri dev          # run the app
pnpm typecheck          # tsc --noEmit
pnpm test               # vitest
```

```bash
cd src-tauri
cargo test --lib        # unit tests, no database needed
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### Integration tests

These exercise the real database paths — introspection, paging, cancellation,
read-only enforcement, and the psql REPL — against the dev fixture:

```bash
cd dev && docker compose up -d && cd ../src-tauri
PGSCOPE_TEST_URL=postgres://pgscope:pgscope@localhost:54330/analytics_prod \
  cargo test --features integration --test integration
```

Without `--features integration` the suite compiles to nothing, so a plain
`cargo test` never needs a database.

### Linux smoke test

macOS gets the native traffic lights overlaid on the custom titlebar; every other
platform draws its own and turns window decorations off. To exercise that path
from a Mac, `dev/linux-smoke/` builds the app for Linux in a container and runs
it under Xvfb:

```bash
docker build -f dev/linux-smoke/Dockerfile -t pgscope-linux-smoke .
docker run --rm \
  -v "$PWD":/src:ro -v /tmp/pgscope-linux-out:/out \
  -e PGHOST=host.docker.internal -e PGPORT=54330 \
  -e PGSCOPE_DEV_URL=postgres://pgscope:pgscope@host.docker.internal:54330/analytics_prod \
  --add-host=host.docker.internal:host-gateway \
  pgscope-linux-smoke
```

It writes a screenshot and window geometry to `/tmp/pgscope-linux-out/`, and
asserts that clicking the drawn close button actually terminates the app.
Minimize and maximize are window-manager operations and are no-ops under a bare
Xvfb server, so they are not asserted; Windows is untested.

## Keyboard shortcuts

| | |
|---|---|
| `⌘N` | new query tab |
| `⌘↵` | run the statement at the cursor (or the selection) |
| `⌘⇧↵` | run every statement in the tab |
| `⌘S` | save the tab (in place, or prompt for a name) |
| `⌘⇧S` | save the tab as a new file |
| `⌘E` | show the query plan (EXPLAIN) |
| `⌘⇧E` | run and show the real plan (EXPLAIN ANALYZE) |
| `⌘R` | refresh the current page and metadata |
| `⌘F` | focus the filter box |
| `⌘K` | clear the terminal |
| `⌘J` | collapse / expand the terminal |
| `⌘T` | focus the terminal prompt |
| `⌘I` | toggle the details panel |
| `⌘1` / `⌘2` | Data / Relationships |
| `Tab` | complete the word at the cursor (in the terminal) |
| `Ctrl+C` | cancel the running statement (in the terminal) |

## Context menu

Right-clicking a cell offers: copy the cell, expand it inline, copy the whole
row as JSON / CSV / INSERT, and filter to (or away from) the cell's value.

Filter-to-value composes rather than replaces — a new predicate is ANDed onto
whatever filter is already applied, with the existing one parenthesised.
Without those parens, adding `c = 3` to `a = 1 OR b = 2` would bind as
`a = 1 OR (b = 2 AND c = 3)`, quietly widening the result instead of narrowing
it.

Both the SQL literals and the row formats are generated **in Rust**, beside the
query builder's `quote_literal`, rather than in the frontend. Escaping rules
that exist in two places drift, and the copy that drifts emits SQL which looks
right and isn't. That also means the generator knows each column's real type,
so numbers stay bare, `jsonb` gets its `::jsonb` cast, NULL becomes the keyword
rather than the string `'NULL'`, and a NULL cell's filter uses `IS NULL` — `=
NULL` is never true and would silently match nothing.

Integration tests close the loop: every column of a real row generates a
predicate, which is fed back through the grid and must select that row, and a
generated INSERT is executed inside a rolled-back transaction to prove it parses
and binds.

## Sorting

Clicking a header sorts by that column, toggling direction if it was already
sorted. **Shift-click adds a column to the sort** rather than replacing it, so
several terms build up an ORDER BY; the header shows each key's direction and,
once there is more than one, its position. Shift-clicking a key cycles it
asc → desc → removed, so the interaction undoes itself.

The last-page optimisation reverses the sort and takes the tail rather than
deep-OFFSETing. With several keys it has to flip **every** term — a partial flip
would silently reorder rows within their groups, which is the quiet way a
multi-column tail goes wrong. `the_multi_column_last_page_is_correctly_ordered`
checks both the group order and the true final row against an independent query.

## Column width & order

Drag a header's right edge to resize; double-click that edge to reset the column
to its type-derived default. Drag the header itself to reorder, with a rule
showing where the column will land. Right-click a header to reset one width or
the whole table's layout. Both live in `grid_layout.json`, keyed `schema.table`,
sibling to `er_layout.json`.

A press only becomes a drag past a 4px threshold — the two gestures start
identically, so without it every click-to-sort would also register as a
zero-distance reorder. The header is a `div` rather than a `button` because a
button cannot contain the independently-pressable resize grip; Enter and Space
are wired back up explicitly, along with `aria-sort`.

Two things here are easy to get quietly wrong:

**Reordering is a view permutation only.** Row values arrive positionally
against the *unreordered* columns, so `orderColumns` returns each column paired
with its canonical index and the grid indexes rows with that. Using the display
position would show the wrong data under the right header — correct until the
first time someone moves a column.

**A saved order is stale by construction.** It names columns, and the table can
gain or lose one between sessions. Names that no longer exist are skipped, and
columns the order has never seen are appended rather than dropped: applying a
saved order naively makes a newly added column invisible with nothing on screen
to explain it.

Widths are clamped in two places with deliberately different ranges. The
frontend's 56–1200px is the UX range for a drag; the backend's wider 32–2000px
is a persistence guard that must never reject a legitimate value the frontend
produced. A column dragged to zero width takes its own grip with it, so that
failure is unrecoverable from the UI — hence a floor rather than a warning.

## Inspecting values

The grid caps every cell at 8KB so a page of wide `jsonb` doesn't ship megabytes
to the webview — but those capped values are exactly the ones worth reading.
Hovering a truncated or `json` cell reveals a `⤢`; clicking it (or double-clicking
the cell) opens a panel under the row, `Esc` closes it.

The panel re-fetches the value rather than reusing the grid's copy, so it is
uncapped up to 4MB, and Postgres pretty-prints JSON via `jsonb_pretty` rather
than the app guessing at formatting. Highlighting uses a tokeniser, not a regex:
a regex colours braces and digits *inside* string values, which jsonb is full of
(URLs, embedded JSON, timestamps).

The row is located by primary key when the table has one. Without a PK it falls
back to the row's position in the page — correct as long as the ordering is
stable, and the panel says `located by position` so that caveat is visible
rather than silent.

## Relationships

The Relationships tab draws the schema's foreign keys as a graph, laid out by
FK degree so the most-referenced tables land centrally. Cards are draggable and
positions persist per schema in `er_layout.json`.

![The Relationships tab: foreign-key graph for the public schema](docs/screenshots/relationships.png)

Large schemas are capped at `MAX_CARDS` and the caption says how many tables are
shown, so a partial graph never reads as a complete one.

## Query editor

`+ New query` (or `⌘N`) opens an editor tab, which appears in the toolbar's
segmented control alongside Data and Relationships — so "what's in the main
area" stays a single control. Clicking a saved query opens it in a tab rather
than the psql prompt, and reopening the same file focuses its existing tab.

![The query editor: run-at-cursor, per-statement result tabs, and schema-aware completion](docs/screenshots/query-editor.png)

The editor is CodeMirror 6 with PostgreSQL syntax highlighting themed from
`tokens.css`, and autocomplete fed by the introspection the explorer has already
fetched — table names from the schema tree, column names for the selected table.

`⌘↵` runs the statement under the cursor; `⌘⇧↵` runs the whole buffer. "Which
statement is under the cursor" is answered by the same lexer the psql pane uses
(`repl::lexer`), exposed over IPC, so the two surfaces can't disagree about
where statements begin and end — including inside strings, dollar-quoted bodies,
and nested comments.

Runs use a dedicated connection, separate from both the read-only browse pool
(the editor must be able to write) and the terminal's client (so an editor run
can't clobber `SET`s made in the psql pane). Multi-statement runs stop at the
first error rather than pressing on against a half-applied state, and each
result set gets its own tab in the results pane.

### Saving

`⌘S` writes a tab to `saved_queries/`. A tab opened from a file saves back to
that file; a new tab asks for a name and then adopts the file it created, so the
next `⌘S` goes in place. `⌘⇧S` always asks, which is how you make a copy. The
sidebar's SAVED QUERIES list refreshes when a new file appears.

### Managing saved queries

Right-click in the SAVED QUERIES list: rename, duplicate, move to a folder,
delete (behind a confirmation, with the cancel button focused), or create a
folder. Right-click a folder to rename it, or the empty area for "New folder".

Queries live in folders up to four levels deep, written as ordinary
subdirectories of `saved_queries/` — the backend reports each query's name as
its `/`-separated path, so the tree is a filesystem view rather than a second
source of truth. Empty folders are listed separately, because a "New folder"
that stays invisible until something is moved into it isn't usable. A folder
disappears when its last query leaves it.

Renaming a folder is a real directory rename, not N file moves: it's atomic, and
it works on an empty folder. It returns each query's before/after path so any
open editor tab follows the file it was opened from. Deleting a query detaches
its tab and marks it dirty rather than closing it — losing unsaved edits because
a file vanished from the sidebar would be worse than an orphaned tab.

Every path here comes from the webview, so the Rust side never trusts one.
`store` canonicalises it and requires it to resolve *under* the saved-queries
directory; the check compares whole path components, so a sibling directory
named `saved_queries_evil` cannot pass by sharing a string prefix. Names are
sanitised segment by segment, so `..` becomes a folder with a silly name rather
than a way upward. Without these, the commands would be arbitrary-file
read/write/delete primitives reachable from the frontend.

Not implemented: drag-to-reorder. Ordering is alphabetical with folders first.
Manual ordering would need an order file next to the queries — a second source
of truth that goes stale the moment a `.sql` is added or removed outside the
app, which is the thing the folders-are-directories design avoids.

Note that `window.prompt` does not work in a Tauri webview — WKWebView only
shows one if the host implements the text-input panel delegate, which wry does
not, so it returns null immediately. Naming therefore goes through an in-app
modal (`state/prompt.ts`), which also restores focus to the editor on close.

## Query plans

`⌘E` shows the planner's plan without running anything. `⌘⇧E` runs the statement
and shows the real one — **inside a transaction that is always rolled back**, so
inspecting an `UPDATE` or `DELETE` cannot modify data. The plan header says when
that happened.

The tree renders each node with the two things that actually make a plan
readable:

- **Self time** — a node's own time, excluding its children. Ranking by *total*
  time is useless because the root always wins; self time finds the real cost
  centre, and the bar on each node scales against the heaviest one.
- **Row misestimation** — actual versus planned rows. A badly wrong estimate is
  the usual root cause behind a bad plan, so nodes off by 10× or more are
  badged.

Two subtleties the display handles explicitly. A node on the inner side of a
nested loop reports a *per-loop average*, so its true cost is that times the
loop count — easy to miss reading raw EXPLAIN output. And under a Gather,
`loops` counts parallel workers rather than iterations, which makes the summed
figure CPU time across workers; those nodes are labelled `cpu` and read
`× N workers`, because otherwise a node can appear to take longer than the
entire query did.

![A query plan: misestimation badge, seq-scan flag, and CPU-across-workers labelling](docs/screenshots/query-plan.png)

## psql pane

A psql-compatible REPL implemented in Rust — no `psql` binary required. It
supports multi-line statements with continuation prompts, `\timing`, `\x`, and
the informational backslash commands. Output is rendered with psql's own
aligned-table format, verified by golden tests captured from real psql 18 output.

The pane is resizable: drag its top edge, or focus the handle and use
↑/↓ (Home/End for the extremes, double-click to reset). The height is clamped to
70% of the window and re-clamped when the window shrinks, so the grid above can
never be squeezed out of existence — including by a height persisted from a
larger monitor.

![The psql pane: aligned output and meta-commands](docs/screenshots/psql-pane.png)

### Meta-commands

```
\d \dt \dv \dm \di \ds \df \dn \du \dx \l    listings, each taking a pattern
\d NAME                                      describe a relation
\i NAME                                      run a saved query
\conninfo  \encoding  \timing  \x  \?        session
```

Every listing accepts psql's `S` and `+` modifiers (`\dtS+` = tables including
system ones, with sizes and comments) and psql's name patterns:

| pattern | means |
| --- | --- |
| `\dt ev*` | tables in the search path starting with `ev` |
| `\dt public.*` | every table in `public`, search path ignored |
| `\dt "MixedCase"` | quoted, so case-sensitive |
| `\dt EVENTS` | unquoted, so folded to `events` |

Patterns become anchored POSIX regexes exactly as psql builds them
(`repl::pattern`), not `LIKE` approximations — `\dt vent` finds nothing, because
psql anchors both ends. `\i` takes a saved-query name rather than a path, so the
terminal cannot be used to read arbitrary files, and it does not interpret
meta-commands inside the file, which is also what stops it recursing.

The catalog SQL is built by pure functions in `repl::catalog` and unit-tested as
strings, but every listing form is *also* executed against the fixture in the
integration suite — string assertions cannot catch a column that does not exist.

### Tab completion

The psql pane completes on `Tab`, in psql's two stages: one press inserts as
much as is unambiguous, a second press lists the candidates in columns.

What it offers depends on where the cursor is. After `FROM`/`JOIN`/`UPDATE` it
offers relations; after `SELECT`/`WHERE`/`GROUP BY` it offers columns of the
tables in scope plus keywords; after a backslash it offers meta-commands, and
after `\d ` a relation name. Keyword case follows what you typed.

The useful case is qualified references: `u.` completes the columns of whatever
`u` was aliased to earlier in the statement, which means the FROM clause is
parsed for aliases. In
`SELECT * FROM events e JOIN users u ON … WHERE u.` it offers `users` columns
and not `events` ones.

Completion runs on the session's own connection, so it sees that session's
`search_path` and any temp tables it created. The context analysis is a pure
function (`repl::complete::analyze`) with the alias parsing tested separately
from anything that needs a database.

## Status

The Data, Relationships, and terminal surfaces are implemented and wired to a
live database. See [tasks.md](tasks.md) for what is done and what remains.
Tagged releases publish a zipped macOS app and a Windows NSIS installer.
The macOS archive is currently unsigned; code signing and notarisation remain
future release-hardening work.
