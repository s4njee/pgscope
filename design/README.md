# Handoff: PostgreSQL Explorer with psql Command Pane ("pgscope")

## Overview
A desktop database client UI for exploring a PostgreSQL analytics database: a schema/table tree sidebar, a table data grid with paging, a column/index details panel, an ER-style relationships view, and a docked psql terminal pane at the bottom. Designed as a macOS-style desktop app window.

## About the Design Files
The files in this bundle are **design references created in HTML** — a static high-fidelity mockup showing intended look and behavior, not production code to copy directly. Your task is to **recreate this design in the target codebase's existing environment** (React, Electron, Tauri, etc.) using its established patterns and libraries — or, if no environment exists yet, choose the most appropriate framework and implement the design there.

`Postgres Explorer.dc.html` is the design source. All styling is inline; every value below is taken verbatim from it.

## Fidelity
**High-fidelity (hifi).** Colors, typography, spacing, and layout are final. Recreate pixel-perfectly. The data (tables, rows, terminal output) is sample content — production wiring is up to the implementer. Interactions beyond tab switching and terminal collapse are static/mocked in the design.

## Design Tokens

### Colors
- Desk background: `#0e1116`, with `radial-gradient(1200px 700px at 30% 0%, #1a2029 0%, #0e1116 60%)`
- Window background: `#0c0f15`
- Titlebar / footer / grid header: `#12161e`, `#10141b`
- Panel (sidebar, details, toolbar): `#0f131b`
- Panel raised / chip bg: `#161c26`, hover `#141924` / `#1a212e`
- Terminal background: `#0a0d12`
- Border primary: `#1f2633`; border faint: `#1a212e`, row divider `#131824`; card border `#263041`
- Text primary: `#cdd6e4`; secondary: `#8b99b0`; dim: `#6b7a92`; faint: `#4a566b`; extra-faint: `#3d475a`
- Terminal body text: `#aebacd`
- Accent (Postgres blue): `#4e9cd8`; accent light: `#7cb9e8`, `#9ecdf0`
- Accent backgrounds: `rgba(78,156,216,0.13–0.18)`; accent borders: `rgba(78,156,216,0.3–0.5)`
- ER graph line: `#3a5f80` (1.5px), endpoints 3px circles `#4e9cd8`
- Amber (jsonb values, PK badges, db icon): `#d9b26a`; PK badge bg `rgba(217,178,106,0.15)`
- Success green (status dot, timings): `#4ec98a`
- Traffic lights: `#ff5f57`, `#febc2e`, `#28c840`
- Selection: `rgba(78,156,216,0.35)`

### Typography
Single family: **IBM Plex Mono** (Google Fonts; weights 400/500/600). Monospace everywhere.
- Window title / breadcrumb / tree items / grid cells: 12px (title & grid header names 500–600 weight)
- Grid cell text: 11.5px; header column names 11.5px/600; header type line 9.5px faint
- Section headers (DATABASE, COLUMNS…): 10px / 600 / letter-spacing 0.08em / `#4a566b`, uppercase
- Buttons/toolbar labels: 11px; tab labels 11.5px/500
- Badges (PK/FK/NN, TABLE): 8.5–9.5px, padding 1–2px 4–7px, radius 3–4px
- Terminal: 12px, line-height 1.5
- History items: 11px; connection pill: 10.5px

### Spacing & shape
- Window: 1400×880px, radius 12px, shadow `0 0 0 1px rgba(255,255,255,0.07), 0 24px 80px rgba(0,0,0,0.6)`; must not shrink (flex-shrink:0), desk scrolls
- Desk padding: 36px, content centered
- Titlebar 42px; toolbar 40px; grid footer 32px; terminal header 30px; collapsed terminal bar 28px
- Sidebar 236px; details panel 266px; terminal body 212px
- Row height: ~26px (padding 5px 10px); tree items padding 4px 14px, indent steps 14px
- Radii: buttons/inputs 6px; tab group 7px (inner 5px); ER cards 7px; pills 20px; badges 3–4px

## Screens / Views

### Window chrome (always visible)
- **Titlebar (42px)**: traffic lights (12px circles, 8px gap) · centered title `pgscope — analytics_prod@localhost:5432` (12px/500 `#8b99b0`) · right connection pill (7px green dot + `connected · 12ms`).
- **Toolbar (40px)**: breadcrumb `analytics_prod ▸ public ▸ events` (last segment `#cdd6e4`/600, separators `#3d475a`) · segmented tab control (Data | Relationships) in a `#12161e` bordered pill; active tab bg `rgba(78,156,216,0.18)`, text `#9ecdf0`; inactive text `#6b7a92` · spacer · mock filter input 260px (`⌕ WHERE event_name = …`, faint) · `↻ Refresh` ghost button · `+ New query` accent button (accent bg/border, `#7cb9e8` text).

### Sidebar (236px, `#0f131b`, right border)
Three sections separated by 1px dividers (`#1f2633`, 12px 14px margins):
1. **DATABASE** — tree: `◆ analytics_prod` (amber diamond) → `public` (`7 tables` count right-aligned faint) → 7 tables each with an 8px square outline glyph and right-aligned row count: event_properties 2.1M, **events 48.2M (selected)**, experiment_exposures 910K, funnels 214, page_views 31.8M, sessions 6.4M, users 1.2M. Collapsed nodes: `views (3)`, `analytics (schema)`. Selected row: bg `rgba(78,156,216,0.13)` + 2px left accent border, text `#cdd6e4`/500, count `#7cb9e8`. Carets ▾/▸ 9px faint.
2. **SAVED QUERIES** — amber ▪ glyph + name + right `.sql`: dau_last_30d, funnel_signup_activate, top_events_hourly.
3. **HISTORY** — truncated one-liners, first keyword in accent: `\d events · 2m`, `SELECT event_name, count(*)… · 9m`, `\timing on · 12m`, `EXPLAIN ANALYZE SELECT… · 31m`.
Hover state on all items: bg `#141924`.

### Data tab (default)
**Grid** (fills remaining width):
- Header row on `#10141b`: grid-template-columns `44px 110px 104px 150px 132px 1fr 215px` (#, event_id, user_id, session_id, event_name ↓, properties, created_at). Two lines per header cell: name (11.5px/600) + type (`bigint · PK`, `text · FK`, `uuid · FK`, `text`, `jsonb`, `timestamptz` — 9.5px faint). 1px right borders `#1a212e`.
- 16 sample rows, same column template. Cell colors: row # `#3d475a`, event_id `#8b99b0`, user_id/session_id/created_at `#6b7a92`, event_name `#7cb9e8`, properties (jsonb) `#d9b26a`. session_id and properties truncate with ellipsis. One selected row (the `signup` row) bg `rgba(78,156,216,0.10)`. Row hover `#12161f`. Bottom borders `#131824`.
- Sample data: descending event_ids from 48213904, users like `u_8f3c21`, uuid sessions, event names (page_view, click, signup, session_start, feature_used, purchase, experiment_view), short jsonb like `{"path": "/pricing"}`, timestamps `2026-07-18 09:41:22.114+00` descending.
- **Paging footer (32px)**: pager buttons `⇤ ← → ⇥` (bordered 4px-radius chips; back buttons disabled/faint, forward buttons hoverable) · `rows 1–50 of 48,213,904` · right: `SELECT * FROM events ORDER BY created_at DESC — 11.8 ms` (timing in green).

**Details panel** (266px, left border, toggleable):
- Header: `events` (13px/600) + `TABLE` accent badge.
- **COLUMNS**: 6 rows — name (11.5px `#cdd6e4`), type right-aligned faint, badge: event_id bigint PK (amber), user_id text FK (blue), session_id uuid FK (blue), event_name/properties/created_at NN (gray `rgba(107,122,146,0.15)`/`#6b7a92`).
- **INDEXES**: 4 entries, name (11px `#8b99b0`) over definition (9.5px faint): events_pkey `btree (event_id) · unique`; idx_events_user_created `btree (user_id, created_at DESC)`; idx_events_name `btree (event_name)`; brin_events_created_at `brin (created_at)`.
- **STATS**: label/value pairs — est. rows 48,213,904 · total size 12 GB · indexes 3.1 GB · last autovacuum 41 min ago.

### Relationships tab
Canvas on `#0c0f15` with 24px dot grid (`radial-gradient(#1a212e 1px, transparent 1px)`).
- 5 absolutely positioned table cards (208px wide; events 224px): users (70,60), sessions (392,40), events (726,140), page_views (392,330), experiment_exposures (70,330). Card: `#10141b`, border `#263041`, radius 7px, shadow `0 8px 24px rgba(0,0,0,0.4)`; header bar `#161c26` with table name in `#7cb9e8` 12px/600; column rows: name / type (right, faint) / PK (amber 9px) or FK (blue 9px) tags.
- `events` card is highlighted: accent border `rgba(78,156,216,0.5)`, accent-tinted header, outer glow ring.
- FK edges as straight SVG lines (`#3a5f80`, 1.5px) with 3px accent circle endpoints: users→sessions, sessions→events, users→events, sessions→page_views, users→experiment_exposures.
- Caption bottom-right: `public schema · 5 of 7 tables · FK graph` (10px faint).

### psql terminal pane (bottom dock)
- **Header (30px, `#10141b`)**: `psql` (10.5px/600) · `analytics_prod · session 1` faint · right: `Timing on` (green) · `clear` · `▾ collapse` (both faint, hover → `#8b99b0`).
- **Body**: `<pre>` 212px tall, scrollable, bg `#0a0d12`, 12px / 1.5 `#aebacd`. Prompt `analytics_prod=#` and continuation `analytics_prod-#` in accent `#4e9cd8`; result table header/separator, `(5 rows)` and `Time: 428.116 ms` in dim `#6b7a92`; row data in body color. Canned content: a `SELECT event_name, count(*)` GROUP BY query over last 24h with 5 result rows (page_view 1842311 … purchase 4310).
- Blinking block cursor after the final prompt: 7×14px accent rect, `blink 1.1s step-end infinite` (50% duty).
- **Collapsed state**: 28px bar (`psql · analytics_prod` + `▴ expand`), whole bar clickable.

## Interactions & Behavior
- Tab control switches main area between Data and Relationships (simple state; no animation).
- Terminal `▾ collapse` / collapsed bar click toggles the pane (no animation in mock; a ~150ms height ease is fine).
- Hover states as listed (rows, tree items, buttons, header links). Cursor: pointer on all interactive elements.
- Everything else (paging, search, refresh, saved queries, history) is static in the mock — wire to real behavior as appropriate.
- Grid and sidebar scroll independently; terminal body scrolls; window is fixed-size.

## State Management
Mock state (map to real app state):
- `activeTab`: 'data' | 'relationships' (default 'data')
- `showDetails`: boolean (default true) — details panel visibility
- `terminalCollapsed`: boolean (default false)
Real implementation additionally needs: selected table, page/offset, selected row, query history list, connection status, terminal session buffer.

## Assets
None. No images or icon fonts — glyphs are unicode text characters (▸ ▾ ◆ ▪ ⌕ ↻ ⇤ ← → ⇥ ▴) and small CSS shapes (traffic-light circles, table-square outlines, cursor block). ER lines are trivial inline SVG. Font loaded from Google Fonts (IBM Plex Mono 400/500/600).

## Files
- `Postgres Explorer.dc.html` — the full design (open in a browser to view). Markup is in the `<x-dc>` template with all styles inline; sample data and tab/terminal state live in the `Component` class at the bottom of the file.
