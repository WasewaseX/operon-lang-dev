# REGISTRY.md — the W21 static git-index registry (cheap first version)

Status: **adopted** · Owner: dev-1 (builder-A) · Supersedes: nothing
Audience: package authors and operators (D-008: zero biology assumed).

## 1. What this is

A registry is a FILE, not a service. One JSON object per line (JSON lines), flat
string fields only, append-only by convention. The file is meant to live in a git
repo, so publishing is a commit and mirroring is a clone. No server, no crates, no
network protocol: everything the toolchain needs is `git` plus this file.

The expensive version (hosted index, moderation, per-name reservation) stays open
on the W21 board entry until the owner makes infrastructure decisions. This format
is deliberately the cheapest thing that makes `add by name` honest and verifiable.

## 2. Line format

```
{"name": "beta", "version": "0.1.0", "git": "https://host/user/beta", "rev": "a1b2c3…", "sha256": "d4e5…", "description": "a leaf library"}
```

| field | required | meaning |
|---|---|---|
| `name` | yes | the dependency name used by `operon mod add NAME` |
| `version` | no | the package's manifest version at publish time |
| `git` | yes | the clone URL handed to `git clone` |
| `rev` | yes | the pinned full git rev (never a moving ref) |
| `sha256` | no | content checksum of the published tree (same routine the lockfile uses) |
| `description` | no | one line, human-readable |

Rules:

- One object per line; `#`-prefixed lines and blank lines are comments.
- Malformed lines are HARD ERRORS at resolve time with the line number. A registry
  that lies would resolve installs against the wrong code, so nothing is skipped.
- The LAST matching line for a name wins. Republishing appends; nothing is mutated.
- Every entry pins a rev. A registry line that points at a moving ref is invalid
  by definition (the parser cannot check intent, the convention is the contract).

## 3. Commands

```
operon mod publish --registry FILE [--url GIT] [--desc TEXT]   # append THIS package
operon mod add NAME --registry FILE [--rev OVERRIDE] [--as N]  # resolve + install
```

- `publish` reads the local `operon.toml` (name, version), takes `git rev-parse HEAD`
  as the rev, and checksums the working tree with the same routine the lockfile uses
  (`path:sha256` rows, sorted). The git URL comes from `--url` or the `origin` remote.
  Publishing the same (name, version, rev) twice is idempotent (no duplicate line).
- `add NAME --registry FILE` resolves NAME to `{git, rev}` and then runs the ordinary
  install path: full transitive closure, `operon.lock` pinning, checksum verification,
  `--locked` drift detection. An explicit `--rev` overrides the registry's pin.
- `$OPERON_REGISTRY` is NOT consulted by design: an implicit registry would make
  resolution depend on the environment. The flag is always explicit.

## 4. Threat notes

- The `sha256` on a registry line is advisory (the lockfile checksum is the enforced
  one). After `add`, compare `operon mod verify` output against the registry line.
- Publishing checksums the WORKING TREE, not the committed tree. Dirty trees publish
  checksums that a fresh clone will not reproduce. Check `git status` before publish
  (the e2e does; tooling enforcement is future work).
- Moderation, name reservation, and revocation are exactly the things a static file
  cannot do. Treat registry contents like any other third-party input: review diffs.

## 5. Evidence

`scripts/registry_e2e.sh` proves the loop end to end: publish two fixture packages
into one index, resolve a consumer's dependency by NAME through the index, install
offline from the lockfile, verify checksums, and re-publish idempotently.
