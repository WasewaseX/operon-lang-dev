#!/usr/bin/env bash
# ============================================================
# Packages , cookbook #20
# you already know: cargo new / npm init and the add-install-run loop
# teaches: the first-class package workflow end to end on REAL command
#          runs: operon new scaffolds a project (manifest + src + a
#          green smoke test), operon add pulls packages from the
#          bundled seed registry (web pulls http transitively),
#          operon.lock pins exact revs + sha256 checksums, and
#          tree / run / test / verify close the loop
# run: bash examples/cookbook/packages.sh        (prints the session)
# expected output: examples/cookbook/expected/packages.out
# verified by scripts/cookbook.sh on the Rust core (the Python oracle
# is a language interpreter, not a package manager, so this chapter is
# Rust-only; every .op chapter verifies on both cores)
#
# How this stays honest: every command below executes for real against
# the repo binary, with HOME / OPERON_DEPS / OPERON_REGISTRY_HOME
# pointed into a throwaway mktemp sandbox (the scripts/pkg_e2e.sh
# pattern), so the chapter never writes outside /tmp and never touches
# your machine. Sandbox paths are printed as $SANDBOX so the frozen
# expected output is byte-stable across runs and machines. If a real
# output changes (toolchain bump, seed package edit), the diff fails
# the gate: re-freeze with `bash scripts/cookbook.sh --update` AFTER
# auditing that the new output is correct.
# ============================================================
set -uo pipefail
cd "$(dirname "$0")/../.."

# same binary priority scripts/cookbook.sh uses
if [ -x target/release/operon ]; then
    OPERON="$PWD/target/release/operon"
elif [ -x bin/operon ]; then
    OPERON="$PWD/bin/operon"
else
    echo "packages: no operon binary found (build one: bash scripts/build.sh)" >&2
    exit 1
fi

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
export OPERON_REGISTRY_HOME="$SB/registry-home"
export OPERON_DEPS="$SB/deps"
export HOME="$SB/home"
mkdir -p "$HOME" "$SB/proj"
cd "$SB/proj"

# norm replaces the throwaway sandbox path with the literal $SANDBOX,
# so the transcript is identical no matter which temp dir mktemp picked
norm() { sed -e "s|$SB|\$SANDBOX|g"; }

# tx prints the command like a prompt, runs it for real, normalizes and
# prints its stdout (stderr is noise here; failures fail the chapter)
tx() {
    printf '$ %s\n\n' "$1"
    shift
    if ! "$@" 2>/dev/null | norm; then
        echo "packages: command failed: $*" >&2
        exit 1
    fi
    printf '\n'
}

# ---- scaffold: manifest, src/main.op, a green smoke test
tx 'operon new myapp'          "$OPERON" new myapp

# ---- the session moves into the project (like you just did)
printf '$ cd myapp\n\n'
cd myapp || exit 1

# ---- the generated manifest is plain TOML: name, version, empty [deps]
tx 'cat operon.toml'           cat operon.toml

# ---- add a package by name, zero flags: the bundled seed registry
#      (http, json, postgres, web) materializes on first use, offline
tx 'operon add http'           "$OPERON" add http

# ---- add web: it declares http as a dependency of its own, the
#      closure handles it (on a bare project this one command locks BOTH)
tx 'operon add web'            "$OPERON" add web

# ---- the resolved tree: web nests http; (…) = already printed above
tx 'operon tree'               "$OPERON" tree

# ---- the lockfile: exact rev + sha256 per package, byte-stable
#      across machines; checked in, --locked fails on any drift
tx 'cat operon.lock'           cat operon.lock

# ---- a small main that uses the deps: pkg http parses a raw request
#      head into a map, pkg web routes it (handlers are plain genes)
cat > src/app.op <<'EOF'
# src/app.op: the two deps in action.
use web
use http

## A route handler: web hands it the match result (params + splat).
gene hello(hit) {
    return "hello, " + hit.params["name"]
}

gene main() {
    # pkg http: parse a raw request head into a plain map
    let req = http.http_request_parse("GET /hello/alice?q=1 HTTP/1.1\nHost: localhost\n\n")

    # pkg web: a router whose route table is plain data
    let r = web.web_router_new()
    web.web_route_add(r, "GET", "/hello/:name", hello)
    let hit = web.web_match(r, req.method, req.path)

    promote(hit.handler(hit))
    promote("q=" + req.query_map["q"])
}
EOF
tx 'cat src/app.op'            cat src/app.op

# ---- vendored module resolution: `use http` / `use web` find the
#      checksum-verified trees in the dep cache, no PATH or PYTHONPATH
tx 'operon run src/app.op'     "$OPERON" run src/app.op

# ---- the scaffold's smoke proof is green out of the box
tx 'operon test'               "$OPERON" test

# ---- every vendored tree re-hashes to its locked checksum
tx 'operon verify'             "$OPERON" verify
