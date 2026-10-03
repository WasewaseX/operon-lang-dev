#!/usr/bin/env python3
"""pkg_meta_check.py — W061-A: the distribution/ecosystem standing gate.

Three hermetic contract pins, stdlib-only, no network, <2s:

  1. cargo-binstall template contract (Cargo.toml [package.metadata.binstall])
     vs release.yml's produced asset naming: pkg-url expansion, the inner
     bin-dir layout (operon-{v}-{target}/<bin>) and the windows .zip override.
     The binstall table once died silently this way — its B5-stack merge
     never carried the table to main, and nothing noticed (docs/PACKAGING.md
     "The B5 stack note"). This pin makes template drift impossible.

  2. the hosted registry's WSGI surface (packaging/registry/app.py `app`) —
     the exact entry gunicorn binds on Render. pkg_hosted_e2e.sh exercises
     the stdlib ThreadingHTTPServer path; the WSGI path had NO test. Covers:
     healthz, byte-exact NDJSON index, publish auth (no tokens / bad token),
     the full 400 validation classes, (name,version) immutability (409),
     413 oversize, 404s — and the search latest-per-name law (REGISTRY.md §2:
     the LAST line per name wins; the first implementation returned the
     OLDEST version, a hosted-vs-file-tier divergence).

  3. render.yaml + requirements.txt wiring: the blueprint references the
     real module/app, health path, build command and token env.

Run:  python3 scripts/pkg_meta_check.py   (exit 1 on any failure)
"""
import io
import json
import os
import re
import sys
import tempfile
import urllib.parse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

PASS = 0
FAILS = []


def ok(what):
    global PASS
    PASS += 1
    print(f"ok    {what}")


def bad(what, detail=""):
    FAILS.append(what)
    print(f"FAIL  {what}" + (f"\n      {detail}" if detail else ""))


def check(cond, what, detail=""):
    if cond:
        ok(what)
    else:
        bad(what, detail)


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


# ---------------------------------------------------------------------------
# 1. cargo-binstall template contract
# ---------------------------------------------------------------------------

def expand(template, mapping):
    """cargo-binstall template expansion: { key } with optional spacing."""
    def sub(m):
        return mapping[m.group(1).strip()]
    return re.sub(r"\{\s*([a-z-]+)\s*\}", sub, template)


def check_binstall():
    cargo = read("Cargo.toml")
    mver = re.search(r'^version = "([^"]+)"', cargo, re.M)
    mrepo = re.search(r'^repository = "([^"]+)"', cargo, re.M)
    check(mver and mrepo, "Cargo.toml [package] version + repository present")
    if not (mver and mrepo):
        return
    ver, repo_url = mver.group(1), mrepo.group(1)
    repo_slug = repo_url.replace("https://github.com/", "")

    mb = re.search(
        r"\[package\.metadata\.binstall\]\s*"
        r'pkg-url\s*=\s*"([^"]+)"\s*'
        r'pkg-fmt\s*=\s*"([^"]+)"\s*'
        r'bin-dir\s*=\s*"([^"]+)"', cargo)
    check(mb is not None, "binstall table present (pkg-url/pkg-fmt/bin-dir)")
    if not mb:
        return
    pkg_url, pkg_fmt, bin_dir = mb.group(1), mb.group(2), mb.group(3)

    # release.yml is the naming authority: ST="operon-${V}-${target}";
    # the matrix is inline-flow style ({ os: ..., target: <triple>, pkg: tar|zip })
    release_yml = read(".github", "workflows", "release.yml")
    targets = sorted(set(re.findall(r"target:\s*([a-z0-9_-]+(?:-[a-z0-9_]+)*)", release_yml)))
    targets = [t for t in targets if t not in ("macos", "latest")]
    check(len(targets) >= 5, f"release.yml matrix parsed, {len(targets)} targets",
          f"targets={targets}")
    windows = [t for t in targets if "windows" in t]
    unix = [t for t in targets if "windows" not in t]

    base = {
        "repo": repo_url,
        "version": ver,
        "bin": "operon",
        "binary-ext": "",
    }
    for t in unix:
        m = {
            **base, "target": t, "pkg-name": "operon",
            "archive-suffix": ".tar.gz" if pkg_fmt == "tgz" else pkg_fmt,
        }
        url = expand(pkg_url, m)
        want = f"https://github.com/{repo_slug}/releases/download/v{ver}/operon-{ver}-{t}.tar.gz"
        check(url == want, f"binstall pkg-url matches release.yml asset ({t})", f"{url}")
        bindir = expand(bin_dir, {**m, "bin": "operon-ls"})
        check(bindir == f"operon-{ver}-{t}/operon-ls",
              f"bin-dir layout covers operon-ls ({t})", bindir)

    mwin = re.search(
        r"\[package\.metadata\.binstall\.overrides\.x86_64-pc-windows-msvc\]\s*"
        r'pkg-fmt\s*=\s*"([^"]+)"', cargo)
    check(mwin is not None, "windows override present (release ships .zip)")
    if mwin:
        m = {
            **base, "target": "x86_64-pc-windows-msvc", "pkg-name": "operon",
            "binary-ext": ".exe",
            "archive-suffix": ".zip" if mwin.group(1) == "zip" else mwin.group(1),
        }
        url = expand(pkg_url, m)
        want = f"https://github.com/{repo_slug}/releases/download/v{ver}/operon-{ver}-x86_64-pc-windows-msvc.zip"
        check(url == want, "windows pkg-url expands to the .zip asset", url)
        bindir = expand(bin_dir, {**m, "bin": "operon-ls"})
        check(bindir == "operon-{v}-x86_64-pc-windows-msvc/operon-ls.exe".replace("{v}", ver),
              "windows bin-dir carries binary-ext", bindir)


# ---------------------------------------------------------------------------
# 2. the hosted registry WSGI surface
# ---------------------------------------------------------------------------

def load_app():
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("OPERON_TOKENS", None)
    os.environ.clear()
    os.environ.update(env)
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "operon_registry_app", os.path.join(ROOT, "packaging", "registry", "app.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class WSGIClient:
    def __init__(self, mod):
        self.mod = mod

    def call(self, method, path, body=None, query="", token=None):
        body = body if body is not None else b""
        environ = {
            "REQUEST_METHOD": method,
            "PATH_INFO": path,
            "QUERY_STRING": query,
            "SERVER_NAME": "localhost",
            "SERVER_PORT": "80",
            "SERVER_PROTOCOL": "HTTP/1.1",
            "CONTENT_LENGTH": str(len(body)),
            "wsgi.version": (1, 0),
            "wsgi.url_scheme": "http",
            "wsgi.input": io.BytesIO(body),
            "wsgi.errors": io.StringIO(),
            "wsgi.multithread": False,
            "wsgi.multiprocess": False,
            "wsgi.run_once": False,
        }
        if token is not None:
            environ["HTTP_AUTHORIZATION"] = f"Bearer {token}"
        captured = {}

        def start_response(status, headers, exc_info=None):
            captured["status"] = status
            captured["headers"] = headers

        chunks = self.mod.app(environ, start_response)
        return captured["status"], b"".join(chunks)


def line_for(name, version, desc="a test package"):
    return json.dumps({
        "name": name, "version": version,
        "git": f"https://example.com/{name}.git", "rev": "a" * 7,
        "sha256": "b" * 64, "description": desc,
    }).encode()


def check_wsgi(mod, tmp):
    c = WSGIClient(mod)
    st, body = c.call("GET", "/healthz")
    check(st == "200 OK" and json.loads(body) == {"ok": True, "service": "operon-registry"},
          "WSGI /healthz identifies the service", f"{st} {body!r}")

    st, body = c.call("GET", "/index.jsonl")
    check(st == "200 OK" and body == b"", "WSGI /index.jsonl empty index is byte-clean")

    # auth: tokens unset = publish disabled; then bad token = 403
    st, body = c.call("POST", "/api/publish", body=line_for("alpha", "1.0.0"), token="tok")
    check(st == "403 Forbidden" and b"publish is disabled" in body,
          "WSGI publish with no OPERON_TOKENS is 403 (never wide open)", f"{st}")
    os.environ["OPERON_TOKENS"] = "goodtoken"
    st, body = c.call("POST", "/api/publish", body=line_for("alpha", "1.0.0"), token="badtoken")
    check(st == "403 Forbidden" and b"publish requires a valid" in body,
          "WSGI publish with a bad token is 403", f"{st}")

    good = line_for("alpha", "1.0.0", "the hello package")
    # the 400 validation classes (docs/specs/REGISTRY.md §2 rules)
    for name, payload, needle in [
        ("not JSON", b"{nope", b"not valid JSON"),
        ("non-object", b'["alpha"]', b"flat JSON object"),
        ("unknown key", json.dumps({"name": "alpha", "version": "1", "git": "g",
                                    "rev": "r", "wat": "x"}).encode(), b"unknown key"),
        ("non-string", json.dumps({"name": "alpha", "version": 3, "git": "g",
                                   "rev": "r"}).encode(), b"must be a string"),
        ("bad name", line_for("Bad Name", "1"), b"invalid package name"),
        ("no version", json.dumps({"name": "alpha", "git": "g", "rev": "r"}).encode(),
         b"needs a version"),
        ("dir rejected", json.dumps({"name": "alpha", "version": "1", "git": "g",
                                     "rev": "r", "dir": "/tmp/x"}).encode(),
         b"publish git URLs only"),
        ("no git", json.dumps({"name": "alpha", "version": "1", "rev": "r"}).encode(),
         b"needs a git URL"),
        ("no rev", json.dumps({"name": "alpha", "version": "1", "git": "g"}).encode(),
         b"needs a rev"),
        ("bad sha256", json.dumps({"name": "alpha", "version": "1", "git": "g",
                                   "rev": "r", "sha256": "XYZ"}).encode(),
         b"64 lowercase hex"),
    ]:
        st, body = c.call("POST", "/api/publish", body=payload, token="goodtoken")
        check(st == "400 Bad Request" and needle in body,
              f"WSGI 400 class: {name}", f"{st} {body!r}")

    # publish + byte-exact index
    st, body = c.call("POST", "/api/publish", body=good, token="goodtoken")
    check(st == "201 Created" and json.loads(body) == {"published": "alpha 1.0.0"},
          "WSGI publish accepts a valid line (201)", f"{st} {body!r}")
    st, body = c.call("GET", "/index.jsonl")
    check(body == good + b"\n", "WSGI index serves the line byte-exactly (+ trailing \\n)",
          f"{body!r}")

    # (name, version) immutability
    st, body = c.call("POST", "/api/publish", body=good, token="goodtoken")
    check(st == "409 Conflict" and b"already exists" in body,
          "WSGI republish of (name,version) is 409 (immutable, fix forward)", f"{st}")

    # 413 oversize + 404s
    st, body = c.call("POST", "/api/publish", body=b"x" * (64 * 1024 + 1), token="goodtoken")
    check(st == "413 Payload Too Large", "WSGI oversized body is 413", st)
    st, body = c.call("GET", "/nope")
    check(st == "404 Not Found", "WSGI unknown GET is 404", st)
    st, body = c.call("POST", "/api/other", body=b"{}", token="goodtoken")
    check(st == "404 Not Found", "WSGI unknown POST is 404", st)

    # THE LAW: search returns the LATEST line per name (NDJSON last-line-wins).
    # The pre-W061-A implementation ordered by (name, version, seq) and kept
    # the FIRST row per name — the oldest version — diverging from the file
    # registry's resolver. Publish 1.1.0 after 1.0.0 and pin 1.1.0.
    st, _ = c.call("POST", "/api/publish",
                   body=line_for("alpha", "1.1.0", "the hello package"), token="goodtoken")
    check(st == "201 Created", "second version of alpha publishes (distinct (name,version))")
    st, body = c.call("POST", "/api/publish",
                      body=line_for("beta-pkg", "0.1.0", "unrelated"), token="goodtoken")
    check(st == "201 Created", "beta-pkg publishes")
    st, body = c.call("GET", "/api/search", query="q=" + urllib.parse.quote("hello"))
    got = json.loads(body)
    alpha = [p for p in got if p["name"] == "alpha"]
    check(st == "200 OK" and len(alpha) == 1 and alpha[0]["version"] == "1.1.0",
          "search returns the LATEST line per name (1.1.0, not 1.0.0)", f"{got}")
    check(all(p["name"] != "beta-pkg" for p in got),
          "search query does not match unrelated packages", f"{got}")


def check_render_yaml():
    ry = read("packaging", "registry", "render.yaml")
    req = read("packaging", "registry", "requirements.txt")
    check("rootDir: packaging/registry" in ry, "render.yaml rootDir points at the service dir")
    check("gunicorn" in ry and "app:app" in ry,
          "render.yaml startCommand binds the WSGI app this gate just exercised")
    check("healthCheckPath: /healthz" in ry, "render.yaml health check is /healthz")
    check("pip install -r requirements.txt" in ry, "render.yaml build installs requirements.txt")
    check("OPERON_TOKENS" in ry and "sync: false" in ry,
          "render.yaml requires OPERON_TOKENS to be set in the dashboard (never committed)")
    check("DATABASE_URL" in ry, "render.yaml wires DATABASE_URL from the blueprint database")
    check(re.search(r"^gunicorn==", req, re.M) and re.search(r"^psycopg2-binary==", req, re.M),
          "requirements.txt pins gunicorn + psycopg2-binary")


def check_release_manifest():
    """S5: the whole-release SHA256SUMS contract.

    install.sh --verify (B1-U3) is fail-closed against the manifest — but the
    release workflow never published one, so strict mode could only ever
    refuse. The sha256sums job (added with this gate) closes that; this pin
    keeps the job and the build matrix in lockstep: a matrix target added
    without a matching manifest entry must fail here, not on a live release.
    Also pins the installer's fail-closed surface and the release-notes
    template wiring (verification preamble on every release).
    """
    text = read(".github", "workflows", "release.yml")
    check("sha256sums:" in text, "release.yml has a sha256sums job")
    job = text.split("sha256sums:", 1)[1] if "sha256sums:" in text else ""
    check("needs: build" in job, "sha256sums job waits for the build job")
    targets = re.findall(r"target: ([a-z0-9_\-]+)", text)
    check(len(targets) >= 5, f"build matrix parsed ({len(targets)} targets)", "regex found none of the matrix targets")
    for t in targets:
        ext = ".zip" if "windows" in t else ".tar.gz"
        check(f"operon-${{V}}-{t}{ext}" in job,
              f"manifest covers matrix target {t}{ext}",
              f"the sha256sums EXPECTED list must name operon-${{V}}-{t}{ext}")
    check("sha256sum -c" in job, "sha256sums job re-verifies each sidecar before hashing")
    check("must not get a manifest" in job, "sha256sums job refuses a partial release")
    check("gh release upload" in job and "SHA256SUMS" in job,
          "sha256sums job uploads the whole-release SHA256SUMS")
    # the job must hash the published bytes back, never runner-local copies
    check("gh release download" in job, "sha256sums job hashes the exact published bytes")
    # installer side of the contract
    inst = read("scripts", "install.sh")
    check("SUMS_URL=" in inst, "install.sh consults the SHA256SUMS manifest")
    check("no SHA256SUMS manifest published" in inst,
          "install.sh --verify fails closed without the manifest (B1-U3)")
    check("not listed in the SHA256SUMS manifest" in inst,
          "install.sh --verify fails closed on an unlisted asset")
    check("OPERON_INSTALL_ASSET_DIR" in inst,
          "install.sh exposes the hermetic offline mode install_e2e.sh exercises")
    # release notes template wiring
    check("body_path: .github/release-notes-template.md" in text,
          "release attach step wires the release-notes template")
    check(os.path.isfile(os.path.join(ROOT, ".github", "release-notes-template.md")),
          "release-notes template exists")
    tpl = read(".github", "release-notes-template.md")
    check("SHA256SUMS" in tpl and "install.sh" in tpl,
          "release-notes template documents verification (SHA256SUMS + install.sh)")
    # issue #51: dry-run rehearsal plumbing — the smoke wiring's first live
    # runs should be dispatchable WITHOUT cutting a release or touching the
    # releases page
    check("dry_run:" in text, "workflow_dispatch dry_run input declared (issue #51)")
    check(re.search(r"if: \$\{\{ inputs\.dry_run != true \}\}\s*\n\s*uses: softprops/action-gh-release", text) is not None,
          "attach step is skipped on dry-run (a rehearsal attaches nothing)")
    job2 = text.split("sha256sums:", 1)[1]
    check("inputs.dry_run != true" in job2.split("steps:", 1)[0],
          "sha256sums job is skipped on dry-run (nothing is uploaded to manifest)")
    check(text.count("| tr '/' '-')") >= 2,
          "dispatch-ref slash sanitization present (unix pkg + manifest job)")
    # live-rehearsal catch (run 36992837397): the smoke expected the branch
    # ref as the binary version — no dispatch-built binary can ever carry a
    # branch name. The expectation must be tag-derived on tags, Cargo.toml-
    # derived everywhere else.
    smoke = text.split("Release smoke (W060)", 1)[1].split("- name:", 1)[0]
    check('refs/tags/*' in smoke and 'GITHUB_REF_NAME#v' in smoke,
          "smoke expectation is tag-derived on a tag push (release-version contract)")
    check("Cargo.toml" in smoke and "refs/tags/*" in smoke,
          "smoke expectation falls back to Cargo.toml on non-tag refs (dry-run rehearsal)")
    check("[ -n \"$V\" ]" in smoke,
          "smoke refuses to run without a derivable expected version")
    # second live-rehearsal catch (run 37045410736, windows leg): the
    # sidecar producer wrote CRLF (Out-File), and sha256sum -c reads the
    # trailing CR into the filename -> "No such file or directory". The
    # producer must write LF; every consumer CR-normalizes before -c.
    check("[System.IO.File]::WriteAllText" in text and 'WriteAllText("$ST.zip.sha256"' in text,
          "windows sidecar producer writes LF (no Out-File on the .sha256)")
    check(text.count("tr -d '\\r'") >= 2,
          "smoke + manifest job CR-normalize sidecars before sha256sum -c")


def main():
    with tempfile.TemporaryDirectory(prefix="operon-pkgmeta-") as tmp:
        os.environ["OPERON_REGISTRY_DB"] = os.path.join(tmp, "reg.db")
        mod = load_app()
        check_binstall()
        check_wsgi(mod, tmp)
        check_render_yaml()
        check_release_manifest()
    if FAILS:
        print(f"pkg_meta_check: {len(FAILS)} FAILED / {PASS} ok")
        return 1
    print(f"pkg_meta_check: ALL GREEN ({PASS} checks)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
