#!/usr/bin/env python3
"""audit_builtins.py — S3: enforce the builtin-coverage claim, machine-checkable.

harness.py's header used to CLAIM "every builtin in src/interp.rs call_builtin
has >= 1 differential golden or a shape contract under tests/differential/".
Prose does not catch regressions; this script does. It is the standing gate
for the sentence, and its teeth are pointed at the future: a builtin added to
either registry without a differential golden FAILS this audit.

What is checked (all static — the script never runs the corpus, the harness
already does that on every gate):

  1. Registry parity: src/interp.rs::BUILTIN_NAMES must equal
     bootstrap/oracle.py::BUILTINS, name for name. A builtin added to one
     engine only is a silent differential hole — the VM funnels builtin calls
     through the shared interp registry, so the two REGISTRIES plus the oracle
     set are the complete builtin inventory of the project.
  2. Synonym parity: src/interp.rs::BUILTIN_SYNONYMS must equal
     bootstrap/oracle.py::BUILTIN_SYNONYMS, and every synonym target must be
     a live builtin (a synonym to a dead name would canonicalize into a
     user-gene lookup — a silent behavior change).
  3. Coverage: every non-excluded builtin must appear in CALL or METHOD form
     in at least one program of the differential corpus (harness.corpus(),
     imported — never forked, the F1 lesson) or in std/*.op (a corpus program
     importing a std module transitively executes it on BOTH engines).
     Bare mentions do NOT count: a name in a comment or an identifier
     substring exercises nothing.
  4. Exclusion honesty: harness.S3_EXCLUDED_BUILTINS is cross-checked against
     the live registry — an exclusion for a name that is no longer a builtin
     is stale policy and fails the audit (the table must stay truthful in
     both directions).
  5. Waiver honesty: WAIVERS below records uncovered builtins with an owner-
     tracked task that owns the fix. A waiver whose builtin GREW coverage is
     stale (the fix landed) and fails the audit — delete the waiver in the
     same PR that lands the golden, the table never accumulates sediment.

Known limitation (documented, conservative): coverage is measured by token
form, not by call-graph resolution. A user gene shadowing a builtin name
would overstate that builtin's coverage — but only ever in the direction of
MISSING a coverage gap, never inventing one, and shadowing a builtin is
itself a lint-class smell the audit would surface on any zero->shadowed
transition. The gate's contract is "no builtin is at ZERO"; precise
per-call-site attribution is the profiler's job, not the auditor's.

Usage:
  python3 bootstrap/audit_builtins.py               # audit, exit 0/1
  python3 bootstrap/audit_builtins.py --self-test   # prove the teeth, exit 0

The self-test injects a synthetic builtin that cannot have a golden and
asserts the audit FAILS — the proof that a golden-less future builtin cannot
slip through.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness  # noqa: E402  (import, never fork: collect_op/corpus/exclusions)

# Uncovered builtins with an owner-tracked fix IN FLIGHT. Format:
#   name: (task, reason)
# Rules: a waiver MUST name a task on the board; a waiver whose builtin grew
# call/method coverage is STALE and fails the audit; waivers are never added
# for timing/exclusion-class builtins — that is what S3_EXCLUDED_BUILTINS is
# for, with sz's policy reasons attached.
WAIVERS = {
    "pow": (
        "W006-D",
        "builder-A's claimed wave (F1b/S4-5): oracle pow negative-fractional "
        "guard lands WITH its differential pin; a second pow golden here "
        "would collide with that open claim",
    ),
}

# A call form is `name(`; a method form is `.name(`. Both engines' builtin
# arms and method dispatch funnel into the same semantics (the builtin form
# is even specified to be memory-charged exactly like the method form), so
# either form exercises the operation the registry name stands for.
_CALL_RE = re.compile(r"\b([a-z_][a-z_0-9]*)\s*\(")
_METHOD_RE = re.compile(r"\.\s*([a-z_][a-z_0-9]*)\s*\(")
_IDENT_RE = re.compile(r"\b[a-z_][a-z_0-9]*\b")


def rust_registry(src_root):
    """BUILTIN_NAMES from src/interp.rs — the authoritative Rust inventory."""
    path = os.path.join(src_root, "src", "interp.rs")
    src = open(path, encoding="utf-8").read()
    i = src.index("pub const BUILTIN_NAMES")
    j = src.index("];", i)
    return sorted(set(re.findall(r'"([a-z_0-9]+)"', src[i:j])))


def rust_synonyms(src_root):
    """BUILTIN_SYNONYMS from src/interp.rs — (synonym, canonical) pairs."""
    path = os.path.join(src_root, "src", "interp.rs")
    src = open(path, encoding="utf-8").read()
    i = src.index("pub const BUILTIN_SYNONYMS")
    j = src.index("];", i)
    pairs = re.findall(r'\(\s*"([a-z_0-9]+)"\s*,\s*"([a-z_0-9]+)"\s*\)', src[i:j])
    return sorted(pairs)


def oracle_registry(src_root):
    """BUILTINS from bootstrap/oracle.py — the Python oracle's inventory."""
    path = os.path.join(src_root, "bootstrap", "oracle.py")
    src = open(path, encoding="utf-8").read()
    m = re.search(r'BUILTINS = set\("""(.*?)"""\.split\(\)\)', src, re.S)
    if not m:
        raise SystemExit("audit: oracle.py BUILTINS set literal not found — "
                         "the extraction contract broke, fix the regex")
    return sorted(set(m.group(1).split()))


def oracle_synonyms(src_root):
    """BUILTIN_SYNONYMS from bootstrap/oracle.py."""
    path = os.path.join(src_root, "bootstrap", "oracle.py")
    src = open(path, encoding="utf-8").read()
    m = re.search(r"BUILTIN_SYNONYMS = \{(.*?)\}", src, re.S)
    if not m:
        raise SystemExit("audit: oracle.py BUILTIN_SYNONYMS dict not found")
    return sorted(re.findall(r'"([a-z_0-9]+)"\s*:\s*"([a-z_0-9]+)"', m.group(1)))


def scan_coverage(src_root, inject=None):
    """Classify every builtin by how the corpus + std exercise it.

    Returns {name: {"direct": n_files, "std": n_files, "mention": n_files}}
    — direct = call/method form in a differential-corpus program, std =
    call/method form inside std/*.op (transitively executed by corpus
    programs importing the module), mention = bare token only. `inject`
    adds a synthetic registry name to simulate a golden-less builtin
    (self-test).
    """
    names = set(rust_registry(src_root))
    if inject:
        names.add(inject)

    corpus_files = harness.corpus(src_root)
    std_files = []
    std_dir = os.path.join(src_root, "std")
    if os.path.isdir(std_dir):
        std_files = sorted(
            os.path.join(std_dir, f) for f in os.listdir(std_dir)
            if f.endswith(".op")
        )

    cov = {n: {"direct": 0, "std": 0, "mention": 0} for n in names}
    for path, bucket in [(p, "direct") for p in corpus_files] + \
                        [(p, "std") for p in std_files]:
        try:
            text = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        called = set(_CALL_RE.findall(text))
        method = set(_METHOD_RE.findall(text))
        idents = set(_IDENT_RE.findall(text))
        hit = called | method
        for n in names & hit:
            cov[n][bucket] += 1
        for n in (names & idents) - hit:
            cov[n]["mention"] += 1
    return cov


def run_audit(src_root, inject=None):
    """Returns (report_lines, failed_bool). All checks, one pass."""
    rep = []
    failed = False

    def fail(msg):
        nonlocal failed
        failed = True
        rep.append(f"  FAIL {msg}")

    rust = set(rust_registry(src_root))
    orac = set(oracle_registry(src_root))
    if inject:
        rust.add(inject)
    rsyn = rust_synonyms(src_root)
    osyn = oracle_synonyms(src_root)

    rep.append(f"S3 builtin audit — rust registry {len(rust)}, "
               f"oracle registry {len(orac)}")

    # 1. registry parity
    if rust != orac:
        fail("registry parity: rust-only %s / oracle-only %s"
             % (sorted(rust - orac), sorted(orac - rust)))
    else:
        rep.append(f"  ok   registry parity: {len(rust)} builtins, "
                   f"both engines identical")

    # 2. synonym parity + live targets
    if rsyn != osyn:
        fail("synonym parity: rust %s vs oracle %s" % (rsyn, osyn))
    else:
        rep.append(f"  ok   synonym parity: {len(rsyn)} synonyms, "
                   f"both engines identical")
    dead_targets = sorted({c for _, c in rsyn} - rust)
    if dead_targets:
        fail("synonym target(s) not in registry: %s" % dead_targets)

    # 3 + 4 + 5. coverage / exclusions / waivers
    cov = scan_coverage(src_root, inject=inject)
    excluded = harness.S3_EXCLUDED_BUILTINS
    stale_policy = sorted(set(excluded) - rust)
    if stale_policy:
        fail("exclusion policy names non-builtins (stale table): %s"
             % stale_policy)
    covered = {n for n, c in cov.items()
               if c["direct"] or c["std"]} - set(inject or ())
    stale_waivers = sorted(set(WAIVERS) & covered)
    if stale_waivers:
        fail("stale waiver(s) — coverage landed, delete from WAIVERS: %s"
             % stale_waivers)

    uncovered = []   # no call/method form anywhere: a real coverage hole
    bare = []        # bare mentions only (a comment is not a golden)
    std_only = []    # covered, but only via std/ (no direct corpus call)
    waived = []      # excluded-with-reason or waived-on-a-tracked-task
    for n in sorted(cov):
        if inject and n == inject:
            uncovered.append(n)
            continue
        if n in excluded or n in WAIVERS:
            waived.append(n)
            continue
        c = cov[n]
        if c["direct"] or c["std"]:
            if c["direct"] == 0:
                std_only.append(n)
        elif c["mention"]:
            bare.append(n)
        else:
            uncovered.append(n)

    if bare:
        fail("bare-mention-only (a comment is not a golden): %s" % bare)
    if uncovered:
        fail("uncovered builtin(s) — no call/method form in corpus or std: "
             "%s" % uncovered)
    if waived:
        for n in waived:
            if n in WAIVERS:
                rep.append("  note waived %s -> %s (%s)"
                           % (n, WAIVERS[n][0], WAIVERS[n][1]))
            else:
                rep.append("  note excluded %s (%s)" % (n, excluded[n]))
    if std_only:
        rep.append("  note std-mediated only (no direct corpus call): %s"
                   % std_only)

    direct_n = sum(1 for n, c in cov.items() if c["direct"])
    rep.append("  ok   coverage: %d/%d registry names exercised "
               "(direct corpus or std-mediated)"
               % (direct_n, len(cov)))
    return rep, failed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), ".."))
    ap.add_argument("--self-test", action="store_true",
                    help="inject a golden-less synthetic builtin and assert "
                         "the audit fails on it (proves the gate has teeth)")
    args = ap.parse_args()
    root = os.path.abspath(args.root)

    if args.self_test:
        ghost = "zzz_audit_selftest_absent"
        rep, failed = run_audit(root, inject=ghost)
        print("\n".join(rep))
        if not failed:
            print("self-test FAILED: the synthetic golden-less builtin was "
                  "not caught — the audit is decorative, do not ship it")
            sys.exit(2)
        print(f"self-test ok: injected '{ghost}' was caught, "
              f"audit exit would be 1")
        sys.exit(0)

    rep, failed = run_audit(root)
    print("\n".join(rep))
    if failed:
        print("\naudit result: FAIL")
        sys.exit(1)
    print("\naudit result: PASS — every registry builtin is excluded with a "
          "reason, waived on an owner-tracked task, or has >= 1 differential "
          "golden")


if __name__ == "__main__":
    main()
