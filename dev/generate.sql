-- pgscope dev fixture — synthetic data
--
-- Shapes are lifted from design/Postgres Explorer.dc.html: `u_8f3c21`-style user
-- ids, uuid session ids, the seven event names, and the exact jsonb payloads the
-- grid shows (`{"path": "/pricing"}`, `{"btn": "cta_hero"}`, …).
--
-- Volumes are sized to exercise paging (and make `⇥` last-page performance a real
-- measurement) while seeding in well under a couple of minutes:
--   users 50k · sessions 200k · events 500k · page_views 300k
--   experiment_exposures 40k · event_properties 100k · funnels 214
--
-- Deterministic: setseed() + generate_series, so every `docker compose down -v && up`
-- produces byte-identical data (modulo now()).

\set ON_ERROR_STOP on

SET client_min_messages = warning;
SET synchronous_commit = off;

SELECT setseed(0.42);

\timing on

-- ---------------------------------------------------------------------------
-- users (50k)
--
-- The design shows ids like u_8f3c21 — `u_` plus 6 lowercase hex chars. The nine
-- ids that literally appear in the mock are inserted first so the fixture
-- contains them verbatim; the rest come from an injective i -> hex map
-- (i * 331 < 2^24 for i <= 50k, and 331 is coprime with 2^24, so no wraparound
-- and no collisions among generated ids).
-- ---------------------------------------------------------------------------

INSERT INTO public.users (user_id, email, plan, created_at)
SELECT uid,
       uid || '@example.com',
       (ARRAY['free', 'pro', 'team'])[1 + (ord % 3)],
       now() - (ord * interval '3 hours')
FROM unnest(ARRAY[
    'u_8f3c21', 'u_2d91be', 'u_c07f44', 'u_51aa02', 'u_7b30ff',
    'u_e44b19', 'u_09e7d3', 'u_a1c8e0', 'u_5fd2c7'
]) WITH ORDINALITY AS d(uid, ord);

INSERT INTO public.users (user_id, email, plan, created_at)
SELECT 'u_' || lpad(to_hex(i * 331), 6, '0'),
       'user' || i || '@' || (ARRAY['example.com', 'acme.io', 'globex.dev', 'initech.co'])[1 + (i % 4)],
       -- Plan mix roughly 70/22/8 free/pro/team.
       CASE WHEN r < 0.70 THEN 'free' WHEN r < 0.92 THEN 'pro' ELSE 'team' END,
       now() - (power(1.0 - i::float8 / 50000, 1.5) * interval '540 days')
FROM (SELECT i, random() AS r FROM generate_series(1, 50000) i OFFSET 0) g
ON CONFLICT (user_id) DO NOTHING;

-- Stable 1..N pick tables so every FK reference is guaranteed to resolve
-- (cheaper and safer than re-deriving ids and hoping they exist).
CREATE TEMP TABLE u_pick AS
SELECT row_number() OVER (ORDER BY user_id) AS n, user_id FROM public.users;
CREATE UNIQUE INDEX ON u_pick (n);
ANALYZE u_pick;

-- ---------------------------------------------------------------------------
-- sessions (200k) — uuid PK, FK to users
-- ---------------------------------------------------------------------------

INSERT INTO public.sessions (session_id, user_id, device, started_at)
SELECT md5('sess:' || g.i)::uuid,
       u.user_id,
       -- Device mix ~ 52% mobile / 40% desktop / 8% tablet, matching the
       -- `{"device": "mobile"}` / `{"device": "desktop"}` rows in the mock.
       CASE WHEN g.r2 < 0.52 THEN 'mobile' WHEN g.r2 < 0.92 THEN 'desktop' ELSE 'tablet' END,
       now() - ((1.0 - g.i::float8 / 200000) * interval '30 days')
FROM (SELECT i, random() AS r1, random() AS r2
      FROM generate_series(1, 200000) i OFFSET 0) g
JOIN u_pick u ON u.n = 1 + floor(g.r1 * 50000)::int;

CREATE TEMP TABLE s_pick AS
SELECT row_number() OVER (ORDER BY session_id) AS n, session_id, user_id FROM public.sessions;
CREATE UNIQUE INDEX ON s_pick (n);
ANALYZE s_pick;

-- ---------------------------------------------------------------------------
-- events (500k)
--
-- event_id runs up to 48213904 — the exact top id in the design's grid — so the
-- first page of `ORDER BY created_at DESC` looks like the mock.
--
-- event_name weights mirror the ratios in the design's terminal output
--   page_view 1842311 · session_start 409112 · click 322480 · signup 12055 · purchase 4310
-- normalised to cumulative thresholds. feature_used and experiment_view sit
-- below purchase, which is why the mock's `LIMIT 5` cuts them off.
--
-- created_at is spread evenly over the last 10 days and is monotonic with
-- event_id (so id order == time order, as in the mock). 10 days / 500k rows puts
-- ~1.7s between adjacent events, matching the sub-second-to-second gaps the mock's
-- first grid page shows, while still leaving ~50k events inside the last 24 hours
-- for the terminal's GROUP BY query. (A front-loaded ramp was tried first; any
-- exponent > 1 collapses the newest rows into microseconds of each other, which
-- makes the first page look wrong.)
-- ---------------------------------------------------------------------------

INSERT INTO public.events (event_id, user_id, session_id, event_name, properties, created_at)
SELECT g.event_id,
       CASE WHEN g.r5 < 0.005 THEN NULL ELSE u.user_id END,    -- ~0.5% anonymous
       CASE WHEN g.r5 > 0.997 THEN NULL ELSE s.session_id END, -- ~0.3% sessionless
       g.event_name,
       CASE g.event_name
         WHEN 'page_view' THEN
           jsonb_build_object('path', (ARRAY[
             '/pricing', '/docs/psql', '/', '/signup', '/changelog',
             '/dashboards/12', '/checkout', '/login'
           ])[1 + floor(g.r2 * 8)::int])
         WHEN 'click' THEN
           jsonb_build_object('btn', (ARRAY[
             'cta_hero', 'nav_pricing', 'signup_submit', 'docs_search'
           ])[1 + floor(g.r2 * 4)::int])
         WHEN 'session_start' THEN
           jsonb_build_object('device', (ARRAY['mobile', 'desktop', 'tablet'])[1 + floor(g.r2 * 3)::int])
         WHEN 'signup' THEN
           jsonb_build_object('plan', (ARRAY['pro', 'team', 'free'])[1 + floor(g.r2 * 3)::int],
                              'ref',  (ARRAY['hn', 'google', 'twitter', 'direct'])[1 + floor(g.r3 * 4)::int])
         WHEN 'purchase' THEN
           (ARRAY[
             '{"sku": "team_annual", "mrr": 79}'::jsonb,
             '{"sku": "pro_monthly", "mrr": 29}'::jsonb,
             '{"sku": "team_monthly", "mrr": 99}'::jsonb,
             '{"sku": "pro_annual", "mrr": 24}'::jsonb
           ])[1 + floor(g.r2 * 4)::int]
         WHEN 'feature_used' THEN
           jsonb_build_object('feature', (ARRAY[
             'query_editor', 'saved_query', 'export_csv', 'er_diagram'
           ])[1 + floor(g.r2 * 4)::int])
         ELSE
           jsonb_build_object('exp', (ARRAY[
             'onboarding_v2', 'pricing_page_v3', 'nav_redesign'
           ])[1 + floor(g.r2 * 3)::int],
                              'variant', (ARRAY['a', 'b'])[1 + floor(g.r3 * 2)::int])
       END,
       g.created_at
FROM (
    SELECT 47713904 + i AS event_id,
           CASE WHEN r1 < 0.71000 THEN 'page_view'
                WHEN r1 < 0.86780 THEN 'session_start'
                WHEN r1 < 0.99230 THEN 'click'
                WHEN r1 < 0.99690 THEN 'signup'
                WHEN r1 < 0.99856 THEN 'purchase'
                WHEN r1 < 0.99966 THEN 'feature_used'
                ELSE 'experiment_view' END AS event_name,
           r2, r3, r4, r5,
           now() - ((1.0 - i::float8 / 500000) * interval '10 days') AS created_at
    FROM (SELECT i, random() AS r1, random() AS r2, random() AS r3,
                 random() AS r4, random() AS r5
          FROM generate_series(1, 500000) i OFFSET 0) q
) g
JOIN s_pick s ON s.n = 1 + floor(g.r4 * 200000)::int
JOIN u_pick u ON u.n = 1 + floor(g.r3 * 50000)::int;

-- ---------------------------------------------------------------------------
-- page_views (300k) — FK to sessions
-- ---------------------------------------------------------------------------

INSERT INTO public.page_views (view_id, session_id, path, referrer)
SELECT g.i,
       s.session_id,
       (ARRAY['/pricing', '/docs/psql', '/', '/signup', '/changelog',
              '/dashboards/12', '/checkout', '/login', '/blog', '/docs/api'
       ])[1 + floor(g.r2 * 10)::int],
       -- ~30% of page views have no referrer (direct traffic) -> exercises NULL cells.
       CASE WHEN g.r3 < 0.30 THEN NULL
            ELSE (ARRAY['https://news.ycombinator.com/', 'https://www.google.com/',
                        'https://twitter.com/', 'https://github.com/',
                        'https://reddit.com/r/postgres'])[1 + floor(g.r3 * 5)::int]
       END
FROM (SELECT i, random() AS r1, random() AS r2, random() AS r3
      FROM generate_series(1, 300000) i OFFSET 0) g
JOIN s_pick s ON s.n = 1 + floor(g.r1 * 200000)::int;

-- ---------------------------------------------------------------------------
-- experiment_exposures (40k) — FK to users
-- ---------------------------------------------------------------------------

INSERT INTO public.experiment_exposures (exposure_id, user_id, experiment, variant, exposed_at)
SELECT g.i,
       u.user_id,
       (ARRAY['onboarding_v2', 'pricing_page_v3', 'nav_redesign',
              'terminal_autocomplete'])[1 + floor(g.r2 * 4)::int],
       (ARRAY['a', 'b', 'control'])[1 + floor(g.r3 * 3)::int],
       now() - ((1.0 - g.i::float8 / 40000) * interval '30 days')
FROM (SELECT i, random() AS r1, random() AS r2, random() AS r3
      FROM generate_series(1, 40000) i OFFSET 0) g
JOIN u_pick u ON u.n = 1 + floor(g.r1 * 50000)::int;

-- ---------------------------------------------------------------------------
-- event_properties (100k) — property catalogue; no FKs by design (§ seed.sql)
-- ---------------------------------------------------------------------------

INSERT INTO public.event_properties
    (property_id, event_name, key, value_type, sample_value, occurrences, first_seen, last_seen)
SELECT i,
       (ARRAY['page_view', 'click', 'signup', 'session_start',
              'feature_used', 'purchase', 'experiment_view'])[1 + (i % 7)],
       'prop_' || lpad(to_hex(i), 5, '0'),
       (ARRAY['string', 'number', 'boolean', 'object'])[1 + floor(r1 * 4)::int],
       CASE WHEN r1 < 0.25 THEN '/pricing'
            WHEN r1 < 0.50 THEN (1 + floor(r2 * 1000))::text
            WHEN r1 < 0.75 THEN CASE WHEN r2 < 0.5 THEN 'true' ELSE 'false' END
            ELSE NULL END,
       (1 + floor(r2 * 250000))::bigint,
       now() - (r1 * interval '540 days') - interval '1 day',
       now() - (r2 * interval '2 days')
FROM (SELECT i, random() AS r1, random() AS r2
      FROM generate_series(1, 100000) i OFFSET 0) g;

-- ---------------------------------------------------------------------------
-- funnels (214) — matching the count the design sidebar shows literally
-- ---------------------------------------------------------------------------

INSERT INTO public.funnels (funnel_id, name, steps, owner, window_days, is_active, created_at)
SELECT i,
       (ARRAY['signup_activate', 'trial_convert', 'checkout', 'onboarding',
              'docs_to_signup'])[1 + (i % 5)] || '_v' || (1 + i / 5),
       (ARRAY['page_view', 'signup', 'feature_used', 'purchase'])[1:(2 + (i % 3))],
       'u_' || lpad(to_hex(((i % 50) + 1) * 331), 6, '0'),
       (ARRAY[1, 7, 14, 30])[1 + (i % 4)],
       (i % 7) <> 0,
       now() - (i * interval '18 hours')
FROM generate_series(1, 214) i;

-- ---------------------------------------------------------------------------
-- analytics schema rollups
-- ---------------------------------------------------------------------------

INSERT INTO analytics.daily_active_users (day, active_users, new_users, events)
SELECT d::date,
       (12000 + floor(random() * 4000))::int,
       (300 + floor(random() * 200))::int,
       (900000 + floor(random() * 400000))::bigint
FROM generate_series(now() - interval '89 days', now(), interval '1 day') d;

INSERT INTO analytics.retention_cohorts (cohort_week, week_offset, cohort_size, retained)
SELECT c::date,
       w,
       sz,
       (sz * power(0.86, w))::int
FROM generate_series(now() - interval '25 weeks', now(), interval '1 week') c,
     generate_series(0, 11) w,
     LATERAL (SELECT (2000 + floor(random() * 1500))::int AS sz) z;

-- ---------------------------------------------------------------------------
-- Statistics — the sidebar row counts read pg_class.reltuples, which is -1
-- until the first ANALYZE. Without this the tree shows `—` everywhere.
-- ---------------------------------------------------------------------------

ANALYZE;

\timing off

SELECT relname,
       reltuples::bigint AS est_rows,
       pg_size_pretty(pg_total_relation_size(oid)) AS total_size
FROM pg_class
WHERE relnamespace IN ('public'::regnamespace, 'analytics'::regnamespace)
  AND relkind = 'r'
ORDER BY relname;
