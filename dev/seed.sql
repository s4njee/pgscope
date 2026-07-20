-- pgscope dev fixture — schema DDL
--
-- Recreates the world depicted in design/README.md and design/Postgres Explorer.dc.html:
--   * public schema with exactly 7 tables and exactly 3 views
--   * an `analytics` sibling schema (sidebar shows `analytics (schema)`)
--   * `events` with exactly the 6 columns and 4 indexes the details panel lists
--   * exactly the 5 FK edges the relationships canvas draws
--
-- Runs as 01-seed.sql from docker-entrypoint-initdb.d; 02-generate.sql loads rows.

\set ON_ERROR_STOP on

SET client_min_messages = warning;

-- ---------------------------------------------------------------------------
-- Schemas
-- ---------------------------------------------------------------------------

CREATE SCHEMA IF NOT EXISTS analytics;

-- ===========================================================================
-- public: the 7 tables shown in the sidebar tree (alphabetical there:
-- event_properties, events, experiment_exposures, funnels, page_views,
-- sessions, users). Created here in dependency order.
-- ===========================================================================

-- users -- ER card: user_id text PK, email text, plan text, created_at timestamptz
CREATE TABLE public.users (
    user_id    text        PRIMARY KEY,
    email      text        NOT NULL,
    plan       text        NOT NULL,
    created_at timestamptz NOT NULL
);

-- sessions -- ER card: session_id uuid PK, user_id text FK, device text, started_at timestamptz
CREATE TABLE public.sessions (
    session_id uuid        PRIMARY KEY,
    user_id    text        NOT NULL REFERENCES public.users (user_id),  -- FK edge 1: users -> sessions
    device     text        NOT NULL,
    started_at timestamptz NOT NULL
);

-- events -- the selected table. Column list and types are verbatim from the
-- design's grid header and details panel COLUMNS block:
--   event_id bigint PK | user_id text FK | session_id uuid FK
--   event_name text NN | properties jsonb NN | created_at timestamptz NN
CREATE TABLE public.events (
    event_id   bigint      PRIMARY KEY,
    user_id    text        REFERENCES public.users (user_id),      -- FK edge 2: users -> events
    session_id uuid        REFERENCES public.sessions (session_id),-- FK edge 3: sessions -> events
    event_name text        NOT NULL,
    properties jsonb       NOT NULL,
    created_at timestamptz NOT NULL
);
-- NOTE: user_id / session_id are intentionally nullable. The details panel gives
-- them FK badges (not NN), and NN badges only to event_name/properties/created_at.

-- page_views -- ER card: view_id bigint PK, session_id uuid FK, path text, referrer text
CREATE TABLE public.page_views (
    view_id    bigint PRIMARY KEY,
    session_id uuid   REFERENCES public.sessions (session_id),     -- FK edge 4: sessions -> page_views
    path       text   NOT NULL,
    referrer   text
);

-- experiment_exposures -- ER card: exposure_id bigint PK, user_id text FK,
-- experiment text, variant text
CREATE TABLE public.experiment_exposures (
    exposure_id bigint      PRIMARY KEY,
    user_id     text        REFERENCES public.users (user_id),     -- FK edge 5: users -> experiment_exposures
    experiment  text        NOT NULL,
    variant     text        NOT NULL,
    exposed_at  timestamptz NOT NULL
);

-- event_properties -- appears in the sidebar but not in the ER view, so columns
-- are invented. Deliberately FK-free: the relationships canvas draws exactly 5
-- edges, and a natural events(event_name) reference would add a 6th.
CREATE TABLE public.event_properties (
    property_id  bigint      PRIMARY KEY,
    event_name   text        NOT NULL,
    key          text        NOT NULL,
    value_type   text        NOT NULL,
    sample_value text,
    occurrences  bigint      NOT NULL DEFAULT 0,
    first_seen   timestamptz NOT NULL,
    last_seen    timestamptz NOT NULL
);

-- funnels -- also sidebar-only (214 rows in the design). Invented columns, no FKs.
CREATE TABLE public.funnels (
    funnel_id   integer     PRIMARY KEY,
    name        text        NOT NULL UNIQUE,
    steps       text[]      NOT NULL,
    owner       text        NOT NULL,
    window_days integer     NOT NULL DEFAULT 7,
    is_active   boolean     NOT NULL DEFAULT true,
    created_at  timestamptz NOT NULL
);

-- ---------------------------------------------------------------------------
-- Indexes on events — EXACTLY the four the details panel lists, in its order:
--   events_pkey             btree (event_id) · unique   (implicit, from the PK)
--   idx_events_user_created btree (user_id, created_at DESC)
--   idx_events_name         btree (event_name)
--   brin_events_created_at  brin (created_at)
-- No other index on events, or the panel would show a fifth row.
-- ---------------------------------------------------------------------------

CREATE INDEX idx_events_user_created ON public.events USING btree (user_id, created_at DESC);
CREATE INDEX idx_events_name         ON public.events USING btree (event_name);
CREATE INDEX brin_events_created_at  ON public.events USING brin  (created_at);

-- Supporting indexes on the other tables (they have their own details panels;
-- only `events` has a design-mandated index list).
CREATE INDEX idx_sessions_user_started      ON public.sessions   USING btree (user_id, started_at DESC);
CREATE INDEX idx_page_views_session         ON public.page_views USING btree (session_id);
CREATE INDEX idx_exposures_experiment       ON public.experiment_exposures USING btree (experiment, variant);
CREATE INDEX idx_event_properties_name_key  ON public.event_properties USING btree (event_name, key);

-- ---------------------------------------------------------------------------
-- Views in public — exactly 3, so the sidebar's collapsed `views (3)` node is
-- literal. Names mirror the SAVED QUERIES entries in the design sidebar.
-- ---------------------------------------------------------------------------

CREATE VIEW public.dau_last_30d AS
SELECT date_trunc('day', e.created_at)::date AS day,
       count(DISTINCT e.user_id)             AS active_users,
       count(*)                              AS events
FROM public.events e
WHERE e.created_at > now() - interval '30 days'
GROUP BY 1
ORDER BY 1 DESC;

CREATE VIEW public.top_events_hourly AS
SELECT date_trunc('hour', e.created_at) AS hour,
       e.event_name,
       count(*) AS events
FROM public.events e
WHERE e.created_at > now() - interval '24 hours'
GROUP BY 1, 2
ORDER BY 1 DESC, 3 DESC;

CREATE VIEW public.funnel_signup_activate AS
SELECT u.plan,
       count(*) FILTER (WHERE e.event_name = 'signup')       AS signups,
       count(*) FILTER (WHERE e.event_name = 'feature_used') AS activations,
       count(*) FILTER (WHERE e.event_name = 'purchase')     AS purchases
FROM public.users u
LEFT JOIN public.events e ON e.user_id = u.user_id
GROUP BY 1
ORDER BY 2 DESC;

-- ---------------------------------------------------------------------------
-- analytics schema — a couple of rollup objects so the sidebar's
-- `analytics (schema)` node expands into something real.
-- ---------------------------------------------------------------------------

CREATE TABLE analytics.daily_active_users (
    day          date    PRIMARY KEY,
    active_users integer NOT NULL,
    new_users    integer NOT NULL,
    events       bigint  NOT NULL
);

CREATE TABLE analytics.retention_cohorts (
    cohort_week date    NOT NULL,
    week_offset integer NOT NULL,
    cohort_size integer NOT NULL,
    retained    integer NOT NULL,
    PRIMARY KEY (cohort_week, week_offset)
);

CREATE VIEW analytics.event_totals AS
SELECT e.event_name,
       count(*)                   AS total_events,
       count(DISTINCT e.user_id)  AS unique_users,
       min(e.created_at)          AS first_seen,
       max(e.created_at)          AS last_seen
FROM public.events e
GROUP BY 1
ORDER BY 2 DESC;

COMMENT ON SCHEMA analytics IS 'Pre-aggregated rollups for the pgscope dev fixture.';
COMMENT ON TABLE  public.events IS 'Raw product analytics event stream.';
