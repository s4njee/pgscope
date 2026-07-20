# dev fixture

A disposable PostgreSQL 16 instance seeded to look like the design in `design/` —
`analytics_prod` with a `public` schema (7 tables, 3 views), a sibling `analytics`
schema, the `events` table with exactly the columns and 4 indexes the details panel
shows, and the 5 FK edges the relationships canvas draws.

Roughly 1.2M synthetic rows (500k `events`), enough to exercise paging and make
last-page (`⇥`) performance a real measurement.

## Up

```sh
docker compose -f dev/docker-compose.yml up -d
docker compose -f dev/docker-compose.yml logs -f db   # watch the seed run
```

First boot runs `seed.sql` then `generate.sql` from `docker-entrypoint-initdb.d`
(~30–60s). The healthcheck only reports healthy once `public.events` is queryable,
so `--wait` is a reliable "seed finished" signal:

```sh
docker compose -f dev/docker-compose.yml up -d --wait
```

## Connect

```
postgres://pgscope:pgscope@localhost:54330/analytics_prod
```

Point the app at it:

```sh
export PGSCOPE_DEV_URL=postgres://pgscope:pgscope@localhost:54330/analytics_prod
pnpm tauri dev
```

A psql shell inside the container:

```sh
docker exec -it pgscope-dev-db psql -U pgscope -d analytics_prod
```

## Down

```sh
docker compose -f dev/docker-compose.yml down          # stop, keep data
docker compose -f dev/docker-compose.yml down -v       # stop and delete the volume
```

## Reset

The init scripts only run against an empty data directory, so re-seeding means
dropping the volume:

```sh
docker compose -f dev/docker-compose.yml down -v
docker compose -f dev/docker-compose.yml up -d --wait
```

Data generation is deterministic (`setseed(0.42)` + `generate_series`), so a reset
reproduces the same rows — only the `now()`-relative timestamps shift.

## Notes

- Port **54330** and the names `pgscope-dev-db` / `pgscope-dev-data` are deliberately
  distinct so this can't collide with another local postgres or compose project.
  `plan.md` §7 says 54329; that port is currently held by the unrelated `postexplore`
  stack on this machine. Override with `PGSCOPE_DEV_PORT=54329 docker compose … up -d`
  once it's free.
- Row counts in the sidebar come from `pg_class.reltuples`, which is `-1` until the
  first `ANALYZE`. `generate.sql` ends with one; if you load more data by hand, run
  `ANALYZE;` again or the tree shows `—`.
