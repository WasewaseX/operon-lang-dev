#!/usr/bin/env python3
"""Operon Registry API — the hosted package registry (ai/ecosystem lane).

A deliberately simple service, exactly the shape the project sketched:

    Operon CLI  ->  Registry API (this file)  ->  PostgreSQL  ->  metadata
                                                              ->  artifacts

Endpoints (same surface the Rust client in src/pkg.rs speaks):
    GET  /healthz                              liveness
    GET  /api/search?q=<query>                 search name/description
    GET  /api/packages/<name>                  index JSON (identical shape to
                                               a file registry's index/<name>.json)
    GET  /api/packages/<n>/<v>/download        package envelope (application/json)
    POST /api/publish                          upload an envelope (Bearer token)

Storage: PostgreSQL via DATABASE_URL (Render default) or SQLite when unset
(local dev + CI). Envelopes are small JSON documents; they are stored whole
and served byte-identical, which is what sha256 pinning in operon.lock needs.

Auth: publish requires a Bearer token listed in the OPERON_TOKENS env var
(comma-separated). Downloads/search/healthz are open. Tokens unset = publish
disabled (403) — a registry you cannot accidentally leave wide open.

Run locally:   python3 app.py                       (SQLite, port 8080)
Run on Render: DATABASE_URL + OPERON_TOKENS env; render.yaml is beside this
               file. gunicorn app:server binds 0.0.0.0:$PORT.
"""

import base64
import hashlib
import json
import os
import re
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("PORT", "8080"))
NAME_RE = re.compile(r"^[a-z][a-z0-9_-]{1,63}$")
VER_RE = re.compile(r"^\d+\.\d+\.\d+$")
MAX_BODY = 8 * 1024 * 1024  # 8 MiB envelope cap


def _db_url():
    url = os.environ.get("DATABASE_URL", "").strip()
    if url.startswith("postgres://"):
        url = url.replace("postgres://", "postgresql://", 1)
    return url


# ---------------------------------------------------------------------------
# storage adapter: Postgres (psycopg2) when DATABASE_URL is set, else SQLite
# ---------------------------------------------------------------------------

class Store:
    """Two backends, one interface. Envelope bytes round-trip exactly."""

    def __init__(self):
        self.url = _db_url()
        if self.url:
            import psycopg2
            import psycopg2.extras
            self.pg = psycopg2
            self.extras = psycopg2.extras
            self.conn = psycopg2.connect(self.url)
            self.conn.autocommit = True
            with self.conn.cursor() as c:
                c.execute(
                    """CREATE TABLE IF NOT EXISTS packages (
                        name TEXT PRIMARY KEY,
                        description TEXT NOT NULL DEFAULT '',
                        created_at TIMESTAMPTZ NOT NULL DEFAULT now())"""
                )
                c.execute(
                    """CREATE TABLE IF NOT EXISTS versions (
                        package TEXT NOT NULL REFERENCES packages(name),
                        version TEXT NOT NULL,
                        yanked BOOLEAN NOT NULL DEFAULT FALSE,
                        sha256 TEXT NOT NULL,
                        deps JSONB NOT NULL DEFAULT '{}',
                        description TEXT NOT NULL DEFAULT '',
                        artifact BYTEA NOT NULL,
                        size INTEGER NOT NULL,
                        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                        PRIMARY KEY (package, version))"""
                )
        else:
            self.sqlite = sqlite3.connect(
                os.environ.get("OPERON_REGISTRY_DB", "registry.db"),
                check_same_thread=False,
            )
            self.sqlite.execute(
                """CREATE TABLE IF NOT EXISTS packages (
                    name TEXT PRIMARY KEY,
                    description TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL DEFAULT (datetime('now')))"""
            )
            self.sqlite.execute(
                """CREATE TABLE IF NOT EXISTS versions (
                    package TEXT NOT NULL REFERENCES packages(name),
                    version TEXT NOT NULL,
                    yanked INTEGER NOT NULL DEFAULT 0,
                    sha256 TEXT NOT NULL,
                    deps TEXT NOT NULL DEFAULT '{}',
                    description TEXT NOT NULL DEFAULT '',
                    artifact BLOB NOT NULL,
                    size INTEGER NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (package, version))"""
            )
            self.sqlite.commit()

    # -- writes ------------------------------------------------------------
    def publish(self, name, version, description, deps, artifact):
        sha = hashlib.sha256(artifact).hexdigest()
        size = len(artifact)
        if self.url:
            with self.conn.cursor() as c:
                c.execute(
                    "INSERT INTO packages (name, description) VALUES (%s, %s) "
                    "ON CONFLICT (name) DO UPDATE SET description = CASE "
                    "WHEN packages.description = '' THEN EXCLUDED.description "
                    "ELSE packages.description END",
                    (name, description),
                )
                c.execute(
                    "INSERT INTO versions (package, version, sha256, deps, description, artifact, size) "
                    "VALUES (%s, %s, %s, %s, %s, %s, %s)",
                    (name, version, sha, json.dumps(deps), description,
                     psycopg2.Binary(artifact), size),
                )
        else:
            cur = self.sqlite.cursor()
            cur.execute(
                "INSERT OR IGNORE INTO packages (name, description) VALUES (?, ?)",
                (name, description),
            )
            cur.execute("SELECT description FROM packages WHERE name = ?", (name,))
            if cur.fetchone()[0] == "" and description:
                cur.execute(
                    "UPDATE packages SET description = ? WHERE name = ?",
                    (description, name),
                )
            cur.execute(
                "INSERT OR REPLACE INTO versions (package, version, sha256, deps, description, artifact, size) "
                "VALUES (?, ?, ?, ?, ?, ?, ?)",
                (name, version, sha, json.dumps(deps), description,
                 sqlite3.Binary(artifact), size),
            )
            self.sqlite.commit()
        return sha, size

    # -- reads -------------------------------------------------------------
    def package_index(self, name):
        if self.url:
            with self.conn.cursor() as c:
                c.execute("SELECT description FROM packages WHERE name = %s", (name,))
                row = c.fetchone()
                if not row:
                    return None
                desc = row[0]
                c.execute(
                    "SELECT version, yanked, sha256, deps, description FROM versions "
                    "WHERE package = %s ORDER BY version",
                    (name,),
                )
                rows = c.fetchall()
        else:
            cur = self.sqlite.cursor()
            cur.execute("SELECT description FROM packages WHERE name = ?", (name,))
            row = cur.fetchone()
            if not row:
                return None
            desc = row[0]
            cur.execute(
                "SELECT version, yanked, sha256, deps, description FROM versions "
                "WHERE package = ? ORDER BY version",
                (name,),
            )
            rows = cur.fetchall()
        versions = []
        for version, yanked, sha, deps, vdesc in rows:
            if isinstance(deps, str):
                deps = json.loads(deps or "{}")
            versions.append(
                {
                    "version": version,
                    "yanked": bool(yanked),
                    "sha256": sha,
                    "deps": deps,
                    "description": vdesc or "",
                }
            )
        return {"name": name, "description": desc or "", "versions": versions}

    def artifact(self, name, version):
        if self.url:
            with self.conn.cursor() as c:
                c.execute(
                    "SELECT artifact FROM versions WHERE package = %s AND version = %s",
                    (name, version),
                )
                row = c.fetchone()
                return bytes(row[0]) if row else None
        cur = self.sqlite.cursor()
        cur.execute(
            "SELECT artifact FROM versions WHERE package = ? AND version = ?",
            (name, version),
        )
        row = cur.fetchone()
        return bytes(row[0]) if row else None

    def search(self, query):
        like = f"%{query.lower()}%"
        if self.url:
            with self.conn.cursor() as c:
                c.execute(
                    """SELECT p.name, p.description FROM packages p
                       WHERE LOWER(p.name) LIKE %s OR LOWER(p.description) LIKE %s
                       ORDER BY p.name""",
                    (like, like),
                )
                names = c.fetchall()
        else:
            cur = self.sqlite.cursor()
            cur.execute(
                """SELECT p.name, p.description FROM packages p
                   WHERE LOWER(p.name) LIKE ? OR LOWER(p.description) LIKE ?
                   ORDER BY p.name""",
                (like, like),
            )
            names = cur.fetchall()
        rows = []
        for name, desc in names:
            idx = self.package_index(name)
            if not idx:
                continue
            live = [v["version"] for v in idx["versions"] if not v["yanked"]]
            rows.append({"name": name, "latest": max_ver(live), "description": desc})
        return rows


def max_ver(vs):
    def key(v):
        try:
            return tuple(int(x) for x in v.split("."))
        except Exception:
            return (0, 0, 0)
    return max(vs, key=key) if vs else ""


STORE = None


def store():
    global STORE
    if STORE is None:
        STORE = Store()
    return STORE


def tokens():
    raw = os.environ.get("OPERON_TOKENS", "").strip()
    return {t.strip() for t in raw.split(",") if t.strip()} if raw else set()


def envelope_validate(body_bytes):
    """Validate the envelope the same way the client will on download."""
    try:
        env = json.loads(body_bytes.decode("utf-8"))
    except Exception as e:
        return None, f"invalid JSON: {e}"
    if env.get("envelope") != 1:
        return None, "unsupported envelope version"
    name = env.get("name", "")
    version = env.get("version", "")
    if not NAME_RE.match(name):
        return None, f"invalid package name '{name}'"
    if not VER_RE.match(version):
        return None, f"invalid version '{version}'"
    files = env.get("files", [])
    if not isinstance(files, list) or not files:
        return None, "envelope has no files"
    for f in files:
        p = f.get("path", "")
        if p.startswith("/") or "\\" in p or ".." in p.split("/"):
            return None, f"unsafe file path '{p}'"
        if not (p.endswith(".op") or p.endswith(".toml") or p.endswith(".md")):
            return None, f"file '{p}' is not .op/.toml/.md"
        try:
            base64.b64decode(f.get("b64", ""), validate=True)
        except Exception:
            return None, f"file '{p}' is not valid base64"
    deps = env.get("deps", {})
    if not isinstance(deps, dict):
        return None, "deps must be an object"
    return env, None


class Handler(BaseHTTPRequestHandler):
    server_version = "operon-registry/1.0"

    def log_message(self, fmt, *args):  # quieter logs on Render
        if os.environ.get("OPERON_REGISTRY_LOG"):
            super().log_message(fmt, *args)

    def _send(self, code, body, ctype="application/json"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def _json(self, code, obj):
        self._send(code, json.dumps(obj).encode("utf-8") + b"\n")

    # -- GET ----------------------------------------------------------------
    def do_GET(self):
        from urllib.parse import urlsplit, parse_qs, unquote
        split = urlsplit(self.path)
        path = split.path.rstrip("/")
        if path == "/healthz":
            return self._json(200, {"ok": True, "service": "operon-registry"})
        if path == "/api/search":
            q = parse_qs(split.query).get("q", [""])[0]
            q = unquote(q)
            return self._json(200, {"results": store().search(q)})
        parts = [p for p in path.split("/") if p]
        # /api/packages/<name>
        if len(parts) == 3 and parts[0] == "api" and parts[1] == "packages":
            name = parts[2]
            if not NAME_RE.match(name):
                return self._json(404, {"error": "not found"})
            idx = store().package_index(name)
            if idx is None:
                return self._json(404, {"error": f"package '{name}' not found"})
            return self._json(200, idx)
        # /api/packages/<name>/<version>/download
        if (
            len(parts) == 5
            and parts[0] == "api"
            and parts[1] == "packages"
            and parts[4] == "download"
        ):
            data = store().artifact(parts[2], parts[3])
            if data is None:
                return self._json(404, {"error": "artifact not found"})
            return self._send(200, data)
        return self._json(404, {"error": "not found"})

    # -- POST /api/publish ----------------------------------------------------
    def do_POST(self):
        path = self.path.split("?")[0].rstrip("/")
        if path != "/api/publish":
            return self._json(404, {"error": "not found"})
        auth = self.headers.get("Authorization", "")
        token = auth[7:] if auth.startswith("Bearer ") else ""
        if token not in tokens():
            return self._json(403, {"error": "publish requires a valid OPERON_TOKENS token"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._json(400, {"error": "bad Content-Length"})
        if length <= 0 or length > MAX_BODY:
            return self._json(413, {"error": f"body must be 1..{MAX_BODY} bytes"})
        body = self.rfile.read(length)
        env, err = envelope_validate(body)
        if err:
            return self._json(400, {"error": err})
        name, version = env["name"], env["version"]
        if store().artifact(name, version) is not None:
            return self._json(
                409,
                {"error": f"{name} {version} already exists — versions are immutable; bump the version"},
            )
        sha, size = store().publish(
            name, version, env.get("description", ""), env.get("deps", {}), body
        )
        return self._json(
            201, {"published": f"{name} {version}", "sha256": sha, "size": size}
        )


def main():
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    # Self-hosted https in one env pair (Render terminates TLS at the edge,
    # so this is only needed when you front the service yourself):
    #   OPERON_TLS_CERT=cert.pem OPERON_TLS_KEY=key.pem python3 app.py
    cert = os.environ.get("OPERON_TLS_CERT", "").strip()
    key = os.environ.get("OPERON_TLS_KEY", "").strip()
    scheme = "http"
    if cert and key:
        import ssl

        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        server.socket = ctx.wrap_socket(server.socket, server_side=True)
        scheme = "https"
    print(
        f"operon-registry listening on :{PORT} ({scheme}, "
        f"{'postgres' if _db_url() else 'sqlite'})",
        flush=True,
    )
    server.serve_forever()


def app(environ, start_response):  # WSGI entry for gunicorn on Render
    Handler_class = Handler

    class Mini:
        pass

    # Bridge WSGI -> the same handler logic without a socket: implement the
    # five routes directly against the store.
    from urllib.parse import urlsplit, parse_qs, unquote
    path = urlsplit(environ.get("PATH_INFO", "/")).path.rstrip("/")
    method = environ.get("REQUEST_METHOD", "GET")
    body = b""
    if method == "POST":
        try:
            length = int(environ.get("CONTENT_LENGTH", "0"))
        except ValueError:
            length = 0
        body = environ["wsgi.input"].read(length) if 0 < length <= MAX_BODY else b""

    def respond(code, obj, ctype="application/json"):
        payload = obj if isinstance(obj, bytes) else json.dumps(obj).encode("utf-8") + b"\n"
        start_response(f"{code} OK", [("Content-Type", ctype), ("Content-Length", str(len(payload)))])
        return [payload]

    if path == "/healthz" and method == "GET":
        return respond(200, {"ok": True, "service": "operon-registry"})
    if path == "/api/search" and method == "GET":
        q = parse_qs(urlsplit(environ.get("PATH_INFO", "") + "?" + environ.get("QUERY_STRING", "")).query).get("q", [""])[0]
        return respond(200, {"results": store().search(unquote(q))})
    parts = [p for p in path.split("/") if p]
    if method == "GET" and len(parts) == 3 and parts[0] == "api" and parts[1] == "packages":
        if not NAME_RE.match(parts[2]):
            return respond(404, {"error": "not found"})
        idx = store().package_index(parts[2])
        return respond(200, idx) if idx else respond(404, {"error": "package not found"})
    if (
        method == "GET"
        and len(parts) == 5
        and parts[4] == "download"
    ):
        data = store().artifact(parts[2], parts[3])
        return respond(200, data) if data else respond(404, {"error": "artifact not found"})
    if path == "/api/publish" and method == "POST":
        auth = environ.get("HTTP_AUTHORIZATION", "")
        token = auth[7:] if auth.startswith("Bearer ") else ""
        if token not in tokens():
            return respond(403, {"error": "publish requires a valid OPERON_TOKENS token"})
        env, err = envelope_validate(body)
        if err:
            return respond(400, {"error": err})
        if store().artifact(env["name"], env["version"]) is not None:
            return respond(409, {"error": "version exists — versions are immutable"})
        sha, size = store().publish(
            env["name"], env["version"], env.get("description", ""),
            env.get("deps", {}), body,
        )
        return respond(201, {"published": f"{env['name']} {env['version']}", "sha256": sha, "size": size})
    return respond(404, {"error": "not found"})


if __name__ == "__main__":
    main()
