#!/usr/bin/env python3
"""Operon Registry API — the hosted tier (ai/ecosystem-r2, W19 items 5/7/10).

The deliberately simple service the project sketched:

    Operon CLI  ->  Registry API (this file)  ->  PostgreSQL
                                              ->  package metadata
                                              ->  Git/source artifacts

It speaks the SAME wire contract the CLI already uses (docs/specs/
REGISTRY.md): the index is NDJSON — one JSON object per line, LAST line
per name wins — so the stock client works against this service unchanged:

    OPERON_REGISTRY=https://your-app.onrender.com/index.jsonl operon add http

Endpoints
    GET  /healthz                 liveness
    GET  /index.jsonl             the full index, NDJSON (name,version order)
    GET  /api/search?q=<query>    JSON array of latest-per-name matches
    POST /api/publish             append one index line (Bearer token)

Publish rules (same class as the file registry):
  * the body is ONE index line: flat JSON strings — name, version, git,
    rev, sha256, description (dir is a LOCAL-registry feature and is
    rejected here, mirroring the client's deny-by-default rule)
  * (name, version) pairs are immutable: a republish is 409, fix forward
  * publishing requires a Bearer token listed in OPERON_TOKENS (comma-
    separated); tokens unset = publish disabled (403) — a registry you
    cannot accidentally leave wide open

Storage: PostgreSQL via DATABASE_URL (Render default) or SQLite when
unset (local dev + CI). Lines are stored whole and served byte-exact,
which is what the sha256 pinning in operon.lock needs.

Run locally:   python3 app.py                       (SQLite, port 8080)
Run on Render: DATABASE_URL + OPERON_TOKENS env; render.yaml sits beside
               this file; gunicorn binds the WSGI `app` on 0.0.0.0:$PORT.
Self-host https: OPERON_TLS_CERT + OPERON_TLS_KEY env (Render terminates
TLS at the edge, so this is only for fronting the service yourself).
"""

import json
import os
import re
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("PORT", "8080"))
NAME_RE = re.compile(r"^[a-z_][a-z0-9_-]{0,63}$")
MAX_BODY = 64 * 1024  # one index line is a few hundred bytes; 64 KiB is generous
LINE_FIELDS = ("name", "version", "git", "rev", "sha256", "description")


def _db_url():
    url = os.environ.get("DATABASE_URL", "").strip()
    if url.startswith("postgres://"):
        url = url.replace("postgres://", "postgresql://", 1)
    return url


def _tokens():
    raw = os.environ.get("OPERON_TOKENS", "")
    return {t.strip() for t in raw.split(",") if t.strip()}


def _validate_line(body):
    """Returns (line_dict, err). Same rules docs/specs/REGISTRY.md §2 states."""
    try:
        obj = json.loads(body.decode("utf-8"))
    except Exception as e:  # noqa: BLE001 — surface a readable error
        return None, f"body is not valid JSON: {e}"
    if not isinstance(obj, dict):
        return None, "index line must be a flat JSON object"
    for k in obj:
        if k not in LINE_FIELDS + ("dir",):
            return None, f"unknown key '{k}' (allowed: {', '.join(LINE_FIELDS + ('dir',))})"
        if not isinstance(obj[k], str):
            return None, f"key '{k}' must be a string (index lines are flat strings)"
    name = obj.get("name", "")
    if not NAME_RE.match(name):
        return None, f"invalid package name '{name}'"
    if not obj.get("version", "").strip():
        return None, "index line needs a version"
    if obj.get("dir"):
        # Mirror the client's deny-by-default: a remote registry must not
        # direct machines to copy local directories (docs/specs §6).
        return None, "remote registries publish git URLs only ('dir' is a local-registry feature)"
    if not obj.get("git", "").strip():
        return None, "index line needs a git URL"
    if not obj.get("rev", "").strip():
        return None, "index line needs a rev (git rev-parse HEAD at publish time)"
    sha = obj.get("sha256", "")
    if sha and not re.match(r"^[0-9a-f]{64}$", sha):
        return None, "sha256 must be 64 lowercase hex chars (checkout_checksum)"
    return obj, None


# ---------------------------------------------------------------------------
# storage adapter: Postgres (psycopg2) when DATABASE_URL is set, else SQLite
# ---------------------------------------------------------------------------

class Store:
    """Two backends, one interface. Index lines round-trip byte-exactly."""

    def __init__(self):
        self.url = _db_url()
        if self.url:
            import psycopg2  # hosted dependency (requirements.txt)

            self.conn = psycopg2.connect(self.url)
            self.param = "%s"
            self._init_pg()
        else:
            path = os.environ.get("OPERON_REGISTRY_DB", "registry.db")
            self.conn = sqlite3.connect(path, check_same_thread=False)
            self.param = "?"
            self._init_sqlite()

    def _init_sqlite(self):
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS index_lines ("
            " name TEXT NOT NULL, version TEXT NOT NULL,"
            " line TEXT NOT NULL, seq INTEGER PRIMARY KEY AUTOINCREMENT)"
        )
        self.conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS nv ON index_lines (name, version)"
        )
        self.conn.commit()

    def _init_pg(self):
        cur = self.conn.cursor()
        cur.execute(
            "CREATE TABLE IF NOT EXISTS index_lines ("
            " name TEXT NOT NULL, version TEXT NOT NULL,"
            " line TEXT NOT NULL, seq BIGSERIAL PRIMARY KEY)"
        )
        cur.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS nv ON index_lines (name, version)"
        )
        self.conn.commit()

    def publish(self, line_obj, line_text):
        """Append one line. Returns (status, payload)."""
        name, version = line_obj["name"], line_obj["version"]
        cur = self.conn.cursor()
        try:
            cur.execute(
                "INSERT INTO index_lines (name, version, line) VALUES (%s, %s, %s)"
                % (self.param, self.param, self.param),
                (name, version, line_text),
            )
            self.conn.commit()
            return 201, {"published": f"{name} {version}"}
        except Exception:
            self.conn.rollback()
            return 409, {
                "error": f"{name} {version} already exists — (name, version) is "
                "immutable; bump the version (docs/specs/REGISTRY.md §2)"
            }

    def index_lines(self):
        cur = self.conn.cursor()
        cur.execute("SELECT line FROM index_lines ORDER BY name, version, seq")
        return [r[0] for r in cur.fetchall()]

    def search(self, query):
        cur = self.conn.cursor()
        like = f"%{query.lower()}%"
        cur.execute(
            "SELECT line FROM index_lines WHERE LOWER(line) LIKE "
            + self.param
            + " ORDER BY name, version, seq",
            (like,),
        )
        seen, out = set(), []
        for (line,) in cur.fetchall():
            obj = json.loads(line)
            if obj["name"] in seen:
                continue
            seen.add(obj["name"])
            out.append(obj)
        return out


_STORE = None


def store():
    global _STORE
    if _STORE is None:
        _STORE = Store()
    return _STORE


# ---------------------------------------------------------------------------
# HTTP surface — the same rules for stdlib serving and gunicorn/WSGI
# ---------------------------------------------------------------------------

class Handler(BaseHTTPRequestHandler):
    server_version = "operon-registry/2"

    def log_message(self, fmt, *args):  # quieter logs on Render
        pass

    def _send(self, status, data, ctype="application/json; charset=utf-8"):
        body = data if isinstance(data, bytes) else json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0].rstrip("/") or "/"
        if path == "/healthz":
            return self._send(200, {"ok": True, "service": "operon-registry"})
        if path == "/index.jsonl":
            lines = store().index_lines()
            payload = ("\n".join(lines) + ("\n" if lines else "")).encode()
            return self._send(200, payload, "text/plain; charset=utf-8")
        if path == "/api/search":
            q = ""
            if "?" in self.path:
                from urllib.parse import parse_qs, unquote

                qs = parse_qs(self.path.split("?", 1)[1])
                q = unquote(qs.get("q", [""])[0])
            return self._send(200, store().search(q))
        return self._send(404, {"error": "not found"})

    def do_POST(self):
        path = self.path.split("?")[0].rstrip("/")
        if path != "/api/publish":
            return self._send(404, {"error": "not found"})
        auth = self.headers.get("Authorization", "")
        token = auth[7:] if auth.startswith("Bearer ") else ""
        toks = _tokens()
        if not toks:
            return self._send(
                403, {"error": "publish is disabled (no OPERON_TOKENS configured)"}
            )
        if token not in toks:
            return self._send(403, {"error": "publish requires a valid OPERON_TOKENS token"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._send(400, {"error": "bad Content-Length"})
        if length <= 0 or length > MAX_BODY:
            return self._send(
                413, {"error": f"body must be 1..{MAX_BODY} bytes (one index line)"}
            )
        body = self.rfile.read(length)
        obj, err = _validate_line(body)
        if err:
            return self._send(400, {"error": err})
        line_text = body.decode("utf-8").strip()
        status, payload = store().publish(obj, line_text)
        return self._send(status, payload)


def main():
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
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
    from urllib.parse import parse_qs, unquote

    path = (environ.get("PATH_INFO", "/")).rstrip("/") or "/"
    if environ.get("REQUEST_METHOD", "GET") == "GET":
        if path == "/healthz":
            body = json.dumps({"ok": True, "service": "operon-registry"}).encode()
            start_response("200 OK", [("Content-Type", "application/json")])
            return [body]
        if path == "/index.jsonl":
            lines = store().index_lines()
            body = ("\n".join(lines) + ("\n" if lines else "")).encode()
            start_response("200 OK", [("Content-Type", "text/plain; charset=utf-8")])
            return [body]
        if path == "/api/search":
            qs = parse_qs(environ.get("QUERY_STRING", ""))
            q = unquote(qs.get("q", [""])[0])
            body = json.dumps(store().search(q)).encode()
            start_response("200 OK", [("Content-Type", "application/json")])
            return [body]
    if environ.get("REQUEST_METHOD", "GET") == "POST" and path == "/api/publish":
        auth = environ.get("HTTP_AUTHORIZATION", "")
        token = auth[7:] if auth.startswith("Bearer ") else ""
        toks = _tokens()
        if not toks:
            start_response("403 Forbidden", [("Content-Type", "application/json")])
            return [json.dumps({"error": "publish is disabled (no OPERON_TOKENS)"}).encode()]
        if token not in toks:
            start_response("403 Forbidden", [("Content-Type", "application/json")])
            return [json.dumps({"error": "publish requires a valid OPERON_TOKENS token"}).encode()]
        try:
            length = int(environ.get("CONTENT_LENGTH", "0"))
        except ValueError:
            length = 0
        if length <= 0 or length > MAX_BODY:
            start_response("413 Payload Too Large", [("Content-Type", "application/json")])
            return [json.dumps({"error": "body must be one index line"}).encode()]
        body_bytes = environ["wsgi.input"].read(length)
        obj, err = _validate_line(body_bytes)
        if err:
            start_response("400 Bad Request", [("Content-Type", "application/json")])
            return [json.dumps({"error": err}).encode()]
        status, payload = store().publish(obj, body_bytes.decode("utf-8").strip())
        start_response(
            f"{status} {'Created' if status == 201 else 'Conflict'}",
            [("Content-Type", "application/json")],
        )
        return [json.dumps(payload).encode()]
    start_response("404 Not Found", [("Content-Type", "application/json")])
    return [json.dumps({"error": "not found"}).encode()]


if __name__ == "__main__":
    main()
