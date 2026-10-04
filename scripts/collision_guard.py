#!/usr/bin/env python3
"""R0.10 collision guard — GitHub-native ownership enforcement (roadmap §24.4).

Turns the ownership rule into an executable check. On every pull request it:

  1. reads the ACTIVE path locks from .github/collab/LOCKS.md (the CI mirror of
     the canonical collab/LOCKS.md in the project-vault, kept by builder-F);
  2. reads the PR's claim from the body (Task-ID / Paths contract, roadmap §24.3);
  3. lists the PR's changed files via the GitHub API;
  4. REJECTS a changed file covered by an ACTIVE lock whose task differs from
     the PR's Task-ID (first canonical GitHub claim wins);
  5. REJECTS any changed core path when the PR body carries no Task-ID;
  6. FAILS LOUD on an ill-formed ledger (two ACTIVE rows covering one path) so
     the keeper fixes the ledger, not the PR.

Grandfather clause: PRs created strictly before GUARD_BORN get WARN instead of
FAIL, so in-flight PRs opened before the contract existed are reported but not
blocked.

Modes:
  python3 scripts/collision_guard.py              CI mode (env: GH_TOKEN, PR_NUMBER, GITHUB_REPOSITORY)
  python3 scripts/collision_guard.py --selftest   offline fixture tests, no network
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.request

LOCK_FILE = ".github/collab/LOCKS.md"

# PRs created strictly before this date are grandfathered (WARN-only).
GUARD_BORN = os.environ.get("COLLISION_GUARD_BORN", "2026-10-05")

# Core paths: touching these without a Task-ID claim is a finding (roadmap
# §24.4 "rejects unknown/unclaimed core paths"). Everything an engine, docs,
# packaging or CI change could live under is listed explicitly.
CORE_PATHS = (
    "src/", "std/", "tests/", "examples/", "apps/", "bootstrap/", "scripts/",
    "docs/", "packaging/", "web/", "labs/", ".github/",
    "README.md", "SPEC.md", "CONTRIBUTING.md", "CHANGELOG.md", "BUILD.md",
    "DETERMINISM.md", "BENCH.md", "PROFILING.md", "MODELING-NOTES.md",
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.sh",
)

ROW_RE = re.compile(
    r"^\|\s*(L-\d+|\S+)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|"
    r"\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|"
)
TASK_RE = re.compile(r"^\s*task[- ]id\s*:\s*(\S+)\s*$", re.IGNORECASE | re.MULTILINE)
PATHS_RE = re.compile(r"^\s*paths\s*:\s*(.+?)\s*$", re.IGNORECASE | re.MULTILINE)


class Lock:
    def __init__(self, lock_id, agent, task, paths, status, claimed):
        self.lock_id, self.agent, self.task = lock_id, agent, task
        self.paths = paths
        self.status, self.claimed = status, claimed

    def active(self):
        return self.status.upper().startswith("ACTIVE")

    def covers(self, path):
        return any(path_covers(p, path) for p in self.paths)


def path_covers(cover, path):
    """True if `cover` is `path` itself, a directory prefix, or a `glob:` pattern."""
    cover = cover.strip()
    if cover.startswith("glob:"):
        import fnmatch
        return fnmatch.fnmatch(path, cover[5:].strip())
    if cover == path:
        return True
    return cover.endswith("/") and path.startswith(cover)


def parse_locks(text):
    """Parse the ACTIVE-locks markdown table; returns (locks, errors)."""
    locks, errors, in_table = [], [], False
    for line in text.splitlines():
        if line.startswith("## "):
            in_table = "active locks" in line.lower()
            continue
        if not in_table or not line.startswith("|"):
            continue
        m = ROW_RE.match(line)
        if not m or set(m.group(1)) <= {"-", " "}:
            continue
        lock_id, agent, task, paths_raw, status, claimed, _rel = m.groups()
        if lock_id.lower() == "lock":  # table header row
            continue
        paths = [p.strip() for p in paths_raw.split(",") if p.strip()]
        if not paths or task.lower() in ("", "task", "—", "-"):
            errors.append(f"ill-formed lock row: {line.strip()}")
            continue
        locks.append(Lock(lock_id, agent, task, paths, status, claimed))
    # Ledger self-check: one path claimed ACTIVE by two different tasks.
    seen = {}
    for lk in locks:
        if not lk.active():
            continue
        for p in lk.paths:
            if p in seen and seen[p].task.lower() != lk.task.lower():
                errors.append(
                    f"ledger defect: path '{p}' is ACTIVE on both {seen[p].lock_id} "
                    f"({seen[p].agent}/{seen[p].task}) and {lk.lock_id} ({lk.agent}/{lk.task})"
                )
            else:
                seen[p] = lk
    return locks, errors


def parse_body(body):
    return {
        "task_id": (TASK_RE.search(body or "") or [None, None])[1],
        "paths": [p.strip() for p in (PATHS_RE.search(body or "") or [None, ""])[1].split(",") if p.strip()],
    }


def audit(locks_text, body, changed_files, pr_created):
    """Returns (findings, warnings, summary). A finding is (tag, message)."""
    findings, warnings = [], []
    locks, ledger_errors = parse_locks(locks_text)
    for e in ledger_errors:
        findings.append(("ledger", e))
    claim = parse_body(body)
    task_id = claim["task_id"]
    active = [lk for lk in locks if lk.active()]
    grandfathered = bool(pr_created) and pr_created[:10] < GUARD_BORN

    # 1) changed-file overlap against ACTIVE locks of a DIFFERENT task
    for f in changed_files:
        for lk in active:
            if lk.covers(f) and (task_id or "").lower() != lk.task.lower():
                findings.append((
                    "conflict",
                    f"'{f}' is covered by ACTIVE lock {lk.lock_id} ({lk.agent}/{lk.task}); "
                    f"this PR claims {task_id or 'NO TASK'}. First canonical claim wins "
                    f"(roadmap §24.3) — stop and pick another READY task or negotiate in the chatroom.",
                ))
    # 2) core paths need a claim
    if not task_id:
        unclaimed = [f for f in changed_files if is_core(f)]
        if unclaimed:
            sample = ", ".join(unclaimed[:5]) + (" …" if len(unclaimed) > 5 else "")
            findings.append((
                "unclaimed",
                f"core path(s) changed with no Task-ID in the PR body: {sample}. Add to the body:\n"
                f"      Task-ID: <id from collab/TASKS.md or roadmap §34>\n"
                f"      Paths: <exclusive paths this task touches>",
            ))
    # 3) declared Paths overlapping someone else's lock — draft-time warning
    for p in claim["paths"]:
        for lk in active:
            if path_covers(p, p) and lk.covers(p) and (task_id or "").lower() != lk.task.lower():
                warnings.append((
                    "declared-overlap",
                    f"declared path '{p}' overlaps ACTIVE lock {lk.lock_id} ({lk.agent}/{lk.task})",
                ))

    summary = (
        f"ledger: {len(active)} ACTIVE lock(s) of {len(locks)} rows · "
        f"changed files: {len(changed_files)} · claim: Task-ID={task_id or 'MISSING'} · "
        f"grandfathered={'yes (WARN-only)' if grandfathered else 'no'}"
    )
    if grandfathered and findings:
        warnings.extend(findings)
        findings = []
    return findings, warnings, summary


def is_core(path):
    return any(path == c or path.startswith(c) for c in CORE_PATHS)


def gh_api(url, token):
    req = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "operon-collision-guard",
    })
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read().decode())


def ci_mode():
    token = os.environ.get("GH_TOKEN") or ""
    repo = os.environ.get("GITHUB_REPOSITORY") or ""
    pr_number = os.environ.get("PR_NUMBER") or ""
    if not (token and repo and pr_number):
        print("collision-guard: FAIL — CI mode needs GH_TOKEN, GITHUB_REPOSITORY, PR_NUMBER", file=sys.stderr)
        return 2
    base = f"https://api.github.com/repos/{repo}"
    pr = gh_api(f"{base}/pulls/{pr_number}", token)
    files, page = [], 1
    while True:
        chunk = gh_api(f"{base}/pulls/{pr_number}/files?per_page=100&page={page}", token)
        files.extend(f["filename"] for f in chunk)
        if len(chunk) < 100:
            break
        page += 1
    locks_text = open(LOCK_FILE, encoding="utf-8").read()
    findings, warnings, summary = audit(locks_text, pr.get("body") or "", files, pr.get("created_at") or "")
    print(f"collision-guard: {summary}")
    for tag, msg in warnings:
        print(f"  [WARN {tag}] {msg}")
    for tag, msg in findings:
        print(f"  [FAIL {tag}] {msg}")
    if findings:
        print("collision-guard: REJECTED — resolve the findings above (roadmap §24.3/§24.4).")
        return 1
    print("collision-guard: PASS — no active-ownership overlap, claim present.")
    return 0


SELFTEST_LOCKS = """
## Active locks
| lock | agent | task | paths | status | claimed | released |
|---|---|---|---|---|---|---|
| L-001 | builder-A | P1-batch | src/value.rs, src/vm.rs | ACTIVE | 2026-10-04 | — |
| L-002 | builder-B | Q1 | docs/KEYWORDS.md | ACTIVE | 2026-10-04 | — |
| L-003 | builder-A | fmt-fix | src/tools.rs | RELEASED | 2026-10-04 | 2026-10-04 |
| L-004 | builder-F | pkg | packaging/ | ACTIVE | 2026-10-04 | — |
"""


class Case:
    def __init__(self, name, body, files, created, want_reject, want_tags):
        self.name, self.body, self.files = name, body, files
        self.created, self.want_reject, self.want_tags = created, want_reject, want_tags


SELFTEST_CASES = [
    Case("self-task-overlap-ok",
         "Task-ID: P1-batch\nPaths: src/value.rs",
         ["src/value.rs"], "2026-10-06", False, []),
    Case("other-task-overlap-fail",
         "Task-ID: W999\nPaths: src/value.rs",
         ["src/value.rs"], "2026-10-06", True, ["conflict", "declared-overlap"]),
    Case("released-lock-not-a-conflict",
         "Task-ID: W999\nPaths: src/tools.rs",
         ["src/tools.rs"], "2026-10-06", False, []),
    Case("dir-prefix-lock-covers-file",
         "Task-ID: W999\nPaths: packaging/x",
         ["packaging/registry/app.py"], "2026-10-06", True, ["conflict", "declared-overlap"]),
    Case("unclaimed-core-path-fail",
         "fix typo in the vm\n",
         ["src/vm.rs"], "2026-10-06", True, ["conflict", "unclaimed"]),
    Case("noncore-only-no-claim-ok",
         "notes\n", ["MISC.txt"], "2026-10-06", False, []),
    Case("grandfathered-overlap-warn-only",
         "Task-ID: OLD\nPaths: src/value.rs",
         ["src/value.rs"], "2026-10-04", False, ["conflict", "declared-overlap"]),
    Case("grandfathered-unclaimed-warn-only",
         "old pr\n", ["src/vm.rs"], "2026-10-04", False, ["conflict", "unclaimed"]),
    Case("declared-overlap-is-warning",
         "Task-ID: W777\nPaths: docs/KEYWORDS.md",
         ["MISC.txt"], "2026-10-06", False, ["declared-overlap"]),
]


def selftest():
    failures = 0
    # audit cases
    for c in SELFTEST_CASES:
        findings, warnings, summary = audit(SELFTEST_LOCKS, c.body, c.files, c.created)
        tags = {t for t, _ in findings} | {t for t, _ in warnings}
        rejected = bool(findings)
        ok = rejected == c.want_reject and c.want_tags == sorted(tags)
        print(f"  {'ok  ' if ok else 'FAIL'} {c.name} -> findings={sorted(tags)} reject={rejected}")
        if not ok:
            failures += 1
            print(f"       wanted tags={c.want_tags} reject={c.want_reject}")
    # ledger defect detection
    bad = SELFTEST_LOCKS.replace(
        "| L-002 | builder-B | Q1 | docs/KEYWORDS.md | ACTIVE | 2026-10-04 | — |",
        "| L-002 | builder-B | OTHER | src/value.rs | ACTIVE | 2026-10-04 | — |")
    findings, _, _ = audit(bad, "Task-ID: W1\n", ["MISC.txt"], "2026-10-06")
    led = [m for t, m in findings if t == "ledger"]
    ok = len(led) == 1 and "ledger defect" in led[0]
    print(f"  {'ok  ' if ok else 'FAIL'} ledger-duplicate-path detected")
    if not ok:
        failures += 1
    # contract patterns actually match the PR bodies this lane files
    sample = "Task-ID: W061-L\nPaths: README.md\n\nbody text"
    ok = parse_body(sample)["task_id"] == "W061-L" and parse_body(sample)["paths"] == ["README.md"]
    print(f"  {'ok  ' if ok else 'FAIL'} body contract parse (Task-ID + Paths)")
    if not ok:
        failures += 1
    print(f"collision-guard selftest: {'ALL GREEN' if failures == 0 else f'{failures} FAILURES'}")
    return 1 if failures else 0


def main():
    if "--selftest" in sys.argv:
        return selftest()
    return ci_mode()


if __name__ == "__main__":
    sys.exit(main())
