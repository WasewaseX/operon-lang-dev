"""operon_pg — the Python side of the Operon postgres package.

Called only through Operon's py() substrate bridge, which marshals every
argument and every result through JSON. Each function opens a short-lived
connection from the given DSN (simple, safe under the bridge's wall-clock
timeout); connection pooling is a documented non-goal for v0.

Grants required by the caller (deny-by-default holds):
    --allow-py operon_pg          the bridge call itself
DATABASE_URL is passed as a value, not read here, so no env grant leaks.
"""

import json

import psycopg2
import psycopg2.extras


def _conn(dsn):
    return psycopg2.connect(dsn)


def query(dsn, sql, params):
    """SELECT -> list of row maps (JSON-able by construction)."""
    with _conn(dsn) as conn, conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
        cur.execute(sql, list(params or []))
        rows = cur.fetchall()
    return json.loads(json.dumps(rows, default=str))


def query_one(dsn, sql, params):
    """SELECT one row -> row map, or None when absent (Operon none())."""
    with _conn(dsn) as conn, conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
        cur.execute(sql, list(params or []))
        row = cur.fetchone()
    if row is None:
        return None
    return json.loads(json.dumps(row, default=str))


def execute(dsn, sql, params):
    """INSERT/UPDATE/DELETE -> affected row count."""
    with _conn(dsn) as conn, conn.cursor() as cur:
        cur.execute(sql, list(params or []))
        count = cur.rowcount
        conn.commit()
    return count
