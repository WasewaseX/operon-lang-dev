# docs/ecosystem — the Operon package ecosystem (ai/ecosystem lane)

This directory anchors the ecosystem lane: the package CLI surface, the
registry service, the official seed packages, and the contracts between
them. The full contract is `docs/PACKAGING.md`; this file is the map.

## Layout

```
packaging/
  registry/            the hosted registry service (deploy target: Render)
    app.py             single-file API — Postgres via DATABASE_URL, SQLite fallback
    render.yaml        blueprint: web service + free Postgres, DATABASE_URL wired
    requirements.txt   gunicorn + psycopg2-binary (hosted only; local needs nothing)
  packages/            the official seed packages (each is itself a project)
    http/              client helpers over the core http_get builtin
    json/              json/extra: path lookup, merge, stable pretty
    postgres/          py()-bridge PostgreSQL access (+ py/operon_pg.py)
    web/               HTML escaping, responses, exact-match routing
tests/package/
  pkg_e2e.sh           the 38-check end-to-end gate (runs in scripts/test.sh)
scripts/
  pkg_seed_registry.py builds the deterministic envelopes into a dir registry
```

## The dependency flow (who calls whom)

```
Operon CLI (src/main.rs: new/add/remove/update/publish/search)
   |
src/pkg.rs  — manifest, semver, resolver, lockfile, envelopes, sha256,
   |          HTTP/1.1 + directory transports (zero external crates)
   v
Registry API (packaging/registry/app.py)
   |
PostgreSQL (Render) or SQLite (local)   — metadata + immutable artifacts
   |
Package envelopes (.opkg) -> installed to operon_modules/<pkg>/
   |
`use <pkg>` resolves through genes.rs package candidates
   (mirrored in bootstrap/oracle.py — differential-pinned)
```

## Non-negotiable rules of the lane

1. Every artifact is sha256-pinned in `operon.lock`; a mismatch refuses to
   install. Reproducibility beats freshness: locks are frozen until an
   explicit `operon update`.
2. Published versions are immutable. Fix-forward, never overwrite.
3. Packages never bypass the sandbox. `--allow-net`/`--allow-py`/
   `--allow-env` gates apply to package code exactly like user code.
4. Resolution is deterministic: same requirements + same registry, byte-
   identical `operon.lock`. This is unit-tested, not aspirational.
5. Both cores import packages identically — the differential corpus pins
   the installed-layout import path byte-for-byte.
