#!/usr/bin/env python3
"""fuzz_exec.py — S7 stage 3: exec-surface fuzzing.

fuzz_diff.py (stage 2) holds the differential promise over programs nobody
wrote YET, but gen_corpus buckets are deliberately deterministic-safe: no
clock, no threads, no effects. The EXEC surface — `operon run` under a
default-deny profile, the capability funnel, the fuel counter, and the
parse-only tooling commands — needs its own adversary. This lane is it.

Contracts under fuzz:

  E1  default-deny containment: `operon run f.op` with ZERO grants on any
      generated program ends contained-or-clean (rc in {0,1,2,3}, no panic
      text, per-input wall clock). Effects attempted without a grant raise
      interference BEFORE executing (Caps::check precedes every effect).
  E1b breach sentinel: escape-shaped programs print "EXECBREACH" ONLY on
      the effect-success path. The token anywhere in stdout = the sandbox
      let an ungranted effect execute = crash-class finding, independent
      of the exit code.
  E2  fuel containment: loop-shaped escape payloads under `--fuel F` end
      contained (fuel exhaustion is the language's diagnostic), never hang.
  E3  tooling robustness: rna --check, doc [--json], graph [--json],
      crispr --knockout G [--json], disasm [--json] on generated programs
      and generated patches: rc in {0,1,2,3}, no panic, no hang; these
      commands never execute program effects (doc/graph/disasm are
      parse-only; crispr runs the proof suite; rna --check writes NOTHING:
      input bytes hashed before/after must match).

Generators:
  * run-path at scale: gen_corpus bucket programs verbatim (same
    (seed, i) derivation as fuzz_diff.py) under `operon run` default-deny
    with a fuel cap.
  * redteam-directed escape generation: programs seeded from the denial
    vocabulary (read_file/write_file/append_file/read_file_bytes/
    write_file_bytes/exists/file_size/read_dir/fs_delete/fs_rename/
    fs_mkdir/http_get/run/py/env/exit) with adversarial argument shapes
    (traversal, absolute escape paths, symlink names, hosts, commands,
    modules), wrapped stress/rescue or bare, spliced into gen_corpus
    buckets and loop-wrapped for E2.
  * tooling inputs: the same programs; .rna patches generated from a
    mini-grammar (v1 `edit T { replace "a" -> "b"; }` + v2
    `syntax: v2 / rename|delete|body`) with targets drawn from real
    identifiers when possible, plus byte mutations.

Every exec program is deterministic per (seed, i); a finding reproduces
from the manifest line alone. Findings are saved to
fuzz_corpus/exec_<seed>_<i>_<kind>.op (+ patch files where relevant) with
one JSON line each in fuzz_corpus/MANIFEST.jsonl. Exit 1 = findings exist
(BUGS to fix — land the fix with a regression .op; see TRIAGE.md).

Usage:
  python3 scripts/fuzz/fuzz_exec.py [--n 900] [--seed 20261001]
      [--time-budget 600] [--per-input-timeout 10] [--bin bin/operon]
      [--corpus-dir fuzz_corpus] [--max-findings 20]
      [--tooling-every 3] [--skip-run] [--skip-escape] [--skip-tooling]
Exit: 0 no findings, 1 findings, 2 setup error.
"""
import argparse
import hashlib
import json
import os
import random
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import gen_corpus  # noqa: E402  (grammar-aware bucket generators, reused verbatim)

TMPDIR = os.path.join(ROOT, "target", "fuzz_exec")
PANIC_TEXTS = ("panicked at", "stack overflow", "fatal runtime error", "[fatal]")
CONTAINED_RCS = {0, 1, 2, 3}
BREACH_TOKEN = "EXECBREACH"

# The denial vocabulary (src/interp.rs effect builtins) x adversarial
# argument shapes. Each entry: (call text, must be inside stress? safe-bare)
# Calls are emitted with a trailing `;` at the statement level.
ESCAPE_PATHS = [
    "/etc/passwd", "/etc/hostname", "../../../etc/passwd",
    "../../payload.op", "/tmp/fuzz_exec_escape", "/tmp/fuzz_exec_escape/x",
    "fuzz_exec_link", "/proc/self/environ", "/dev/null", "~/escape",
    "/root/.ssh/id_rsa", "./../../out.op",
]
ESCAPE_HOSTS = [
    ("127.0.0.1", "8080"), ("localhost", "80"), ("0.0.0.0", "22"),
    ("169.254.169.254", "80"), ("10.0.0.1", "443"), ("::1", "9000"),
]
ESCAPE_CMDS = [
    ("sh", ["-c", "echo pwned"]), ("curl", ["http://127.0.0.1:9"]),
    ("python3", ["-c", "print(1)"]), ("env", []), ("/bin/ls", ["/"]),
]
ESCAPE_MODULES = [
    ("math", "sqrt", "[4]"), ("os", "system", "['echo pwned']"),
    ("subprocess", "run", "['sh']"), ("builtins", "eval", "['1+1']"),
]
ESCAPE_VARS = ["PATH", "HOME", "OPERON_CAPS", "AWS_SECRET_ACCESS_KEY", ""]
ESCAPE_EXITS = [0, 1, 2, 3, 42, -1, 255]

# (statement text with {path}-style slots filled by the caller, is_effect)
ESCAPE_TEMPLATES = [
    ('read_file("{p}")', True),
    ('write_file("{p}", "fuzz")', True),
    ('append_file("{p}", "fuzz")', True),
    ('read_file_bytes("{p}")', True),
    ('write_file_bytes("{p}", [70, 85])', True),
    ('exists("{p}")', True),
    ('file_size("{p}")', True),
    ('read_dir("{p}")', True),
    ('fs_delete("{p}")', True),
    ('fs_rename("{p}", "{p}.new")', True),
    ('fs_mkdir("{p}")', True),
    ('http_get("{h}", {port}, "/")', True),
    ('run("{c}", {cargs})', True),
    ('py("{m}", "{f}", {margs})', True),
    ('env("{v}")', True),
    ('exit({x})', True),
    ('let _e = read_file("{p}")', True),
    ('print("pre-" + exists("{p}"))', True),
]


def find_binary(explicit):
    cands = [
        explicit,
        os.environ.get("OPERON_BIN"),
        os.path.join(ROOT, "bin", "operon"),
        os.path.join(ROOT, "target", "release", "operon"),
        os.path.join(ROOT, "target", "debug", "operon"),
    ]
    for c in cands:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


def probe(argv, timeout):
    try:
        p = subprocess.run(argv, cwd=ROOT, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        return "TIMEOUT", e.stdout or b"", e.stderr or b""


def robustness(rc, err, out=b""):
    """R1/E1 core: contained-or-clean. Returns a finding string or None."""
    if rc == "TIMEOUT":
        return "hang (per-input timeout)"
    if not isinstance(rc, int):
        return f"non-integer rc {rc!r}"
    if rc not in CONTAINED_RCS:
        return f"exit {rc} outside the contained set"
    blob = (err or b"").decode("utf-8", errors="replace")
    for t in PANIC_TEXTS:
        if t in blob:
            return f"panic text {t!r} on stderr"
    if BREACH_TOKEN.encode() in (out or b""):
        return f"{BREACH_TOKEN} sentinel in stdout (ungranted effect executed)"
    return None


def gen_program(seed, i):
    """The exact gen_corpus program for (seed, i) — same derivation as
    fuzz_diff.py, so the two lanes share grammar coverage."""
    bucket = gen_corpus.BUCKETS[i % len(gen_corpus.BUCKETS)]
    idx = i // len(gen_corpus.BUCKETS)
    tag = f"{bucket[0]}{idx:03d}"
    rng = random.Random(seed * 1000003 + i * 7919)
    frags = gen_corpus.GEN[bucket](rng, tag)
    header = [
        "# exec-surface fuzz corpus — generated by fuzz_exec.py, do not edit",
        f"# bucket={bucket} seed={seed} idx={idx} tag={tag}",
        "",
    ]
    text = "\n".join(header) + "\n" + "\n\n".join(frags) + "\n"
    return bucket, idx, text


def _escape_stmt(rng):
    tpl, _ = rng.choice(ESCAPE_TEMPLATES)
    p = rng.choice(ESCAPE_PATHS)
    h, port = rng.choice(ESCAPE_HOSTS)
    c, cargs = rng.choice(ESCAPE_CMDS)
    m, f, margs = rng.choice(ESCAPE_MODULES)
    v = rng.choice(ESCAPE_VARS)
    x = rng.choice(ESCAPE_EXITS)
    return tpl.format(p=p, h=h, port=port, c=c, cargs=json.dumps(cargs),
                      m=m, f=f, margs=margs, v=v, x=x)


def gen_escape_program(seed, i):
    """Redteam-directed: capability-escape attempts seeded from the denial
    vocabulary. Shape A: bare call (uncaught stress = contained fatal).
    Shape B: stress/rescue with the breach sentinel on the success path.
    Shape C: loop-wrapped call (E2, run with --fuel). Shape D: an escape
    call spliced into a real gen_corpus bucket program."""
    rng = random.Random(seed * 9176 + i * 15485863)
    bucket = gen_corpus.BUCKETS[i % len(gen_corpus.BUCKETS)]
    idx = i // len(gen_corpus.BUCKETS)
    shape = rng.choice(["bare", "stress", "loop", "splice"])
    stmt = _escape_stmt(rng)
    tag = f"esc{idx:03d}"
    lines = [
        "# exec-surface escape fuzz — generated by fuzz_exec.py, do not edit",
        f"# seed={seed} idx={i} shape={shape} bucket={bucket}",
        "",
    ]
    if shape == "bare":
        lines += ["gene main() {", f"    {stmt}", "}"]
    elif shape == "stress":
        lines += [
            "gene main() {",
            "    stress {",
            f"        {stmt}",
            f'        print("{BREACH_TOKEN} {stmt[:24]}")',
            "    } rescue (e) {",
            '        print("contained")',
            "    }",
            "}",
        ]
    elif shape == "loop":
        # E2: must exhaust fuel, never hang (fuzzer always passes --fuel).
        lines += [
            "gene main() {",
            "    let n = 0",
            "    while (true) {",
            "        stress {",
            f"            {stmt}",
            f'            print("{BREACH_TOKEN} looped")',
            "        } rescue (e) {",
            "            let n = n + 1",
            "        }",
            "    }",
            "}",
        ]
    else:  # splice: escape call dropped into a real bucket program
        _, _, text = gen_program(seed, i)
        insert_at = text.find("\n\n")
        cut = insert_at + 1 if insert_at >= 0 else len(text)
        lines = [
            text[:cut],
            "stress {",
            f"    {stmt}",
            f'    print("{BREACH_TOKEN} spliced")',
            "} rescue (e) {",
            '    print("contained")',
            "}",
            text[cut:],
        ]
        return bucket, idx, "\n".join(lines) + "\n", shape
    return bucket, idx, "\n".join(lines) + "\n", shape


# ---------------- tooling surfaces ----------------

TOOL_TARGET_WORDS = ["main", "process", "gene", "splice", "variant",
                     "phenotype", "method", "fate", "regulate", "replicate"]


def _idents_of(text):
    return sorted(set(re.findall(r"\b[a-z][a-zA-Z0-9_]{2,15}\b", text)))


def gen_patch(seed, i, program_text):
    """A .rna patch from the v1/v2 mini-grammar + occasional byte mutation.
    Targets prefer identifiers actually present in the program so v2 node
    addressing is exercised on real shapes, not only on honest refusals."""
    rng = random.Random(seed * 31337 + i * 104729)
    idents = _idents_of(program_text) or TOOL_TARGET_WORDS
    tgt = rng.choice(idents)
    new = rng.choice(TOOL_TARGET_WORDS) + str(rng.randrange(100))
    src = rng.choice(idents)
    if rng.random() < 0.5:
        # v2 node-addressed
        verbs = [
            f"rename gene {tgt} -> {new}",
            f"delete gene {tgt}",
            f"rename splice {tgt} -> {new}",
            f"delete variant {tgt}.{rng.choice(idents)}",
            f"rename phenotype {tgt} -> {new}",
            f"rename method {tgt}.{rng.choice(idents)} -> {new}",
            f"delete fate {tgt}",
            f"delete regulate #{rng.randrange(1, 5)}",
            f"body {tgt} {{\n    gene patchbody() {{\n        print(\"pb\")\n    }}\n}}",
        ]
        text = "syntax: v2\n\n" + rng.choice(verbs) + "\n"
    else:
        # v1 span/text semantics
        text = (f'edit {tgt} {{\n'
                f'    replace "{src}" -> "{new}";\n'
                f'}}\n')
    if rng.random() < 0.25:
        # byte-level mutation of a structurally valid patch
        b = bytearray(text.encode())
        for _ in range(rng.randrange(1, 4)):
            if not b:
                break
            j = rng.randrange(len(b))
            b[j] = rng.randrange(32, 127)
        text = bytes(b).decode("utf-8", errors="replace")
    return text


def gen_knockout(seed, i, program_text):
    rng = random.Random(seed * 65537 + i * 1548)
    idents = _idents_of(program_text) or ["main"]
    return rng.choice(idents[:12])


def mutation_of(text, rng):
    """1-4 byte mutations of a real program (truncation, splice, flip)."""
    b = bytearray(text.encode())
    if not b:
        return text
    for _ in range(rng.randrange(1, 5)):
        op = rng.randrange(3)
        if op == 0:  # truncate
            b = b[:rng.randrange(len(b))]
        elif op == 1 and len(b) > 4:  # duplicate a chunk
            j = rng.randrange(len(b) - 4)
            k = min(len(b), j + rng.randrange(4, 64))
            b = b[:j] + b[j:k] + b[j:]
        else:  # flip bytes
            j = rng.randrange(len(b))
            b[j] = rng.randrange(32, 127)
    return bytes(b).decode("utf-8", errors="replace")


def main():
    ap = argparse.ArgumentParser(description="S7 stage 3 exec-surface fuzzer")
    ap.add_argument("--n", type=int, default=900,
                    help="programs (default 900; round multiple of 12)")
    ap.add_argument("--seed", type=int, default=20261001)
    ap.add_argument("--time-budget", type=float, default=600.0)
    ap.add_argument("--per-input-timeout", type=float, default=10.0)
    ap.add_argument("--fuel", type=int, default=20000,
                    help="fuel cap for run-path legs (E2)")
    ap.add_argument("--bin", default=None)
    ap.add_argument("--corpus-dir", default=os.path.join(ROOT, "fuzz_corpus"))
    ap.add_argument("--max-findings", type=int, default=20)
    ap.add_argument("--tooling-every", type=int, default=3,
                    help="tooling sweep on every Kth program (default 3)")
    ap.add_argument("--skip-run", action="store_true")
    ap.add_argument("--skip-escape", action="store_true")
    ap.add_argument("--skip-tooling", action="store_true")
    args = ap.parse_args()

    binpath = find_binary(args.bin)
    if binpath is None:
        print("no operon binary found — cargo build first", file=sys.stderr)
        return 2
    os.makedirs(TMPDIR, exist_ok=True)
    os.makedirs(args.corpus_dir, exist_ok=True)
    manifest = os.path.join(args.corpus_dir, "MANIFEST.jsonl")
    path = os.path.join(TMPDIR, "p.op")
    patch_path = os.path.join(TMPDIR, "p.rna")

    print("fuzz_exec — S7 stage 3: exec-surface")
    print(f"  binary: {binpath}")
    print(f"  seed {args.seed}, {args.n} program(s), budget {args.time_budget:.0f}s, "
          f"per-input {args.per_input_timeout:.0f}s, fuel {args.fuel}")
    print("  contracts: E1 default-deny contained-or-clean, E1b breach "
          "sentinel, E2 fuel containment, E3 tooling robustness "
          "(rna --check / doc / graph / crispr / disasm), R1 no panic/hang")

    deadline = time.time() + args.time_budget
    findings = 0
    seen = set()
    legs = {"run": 0, "escape": 0, "tooling": 0}

    def finding(kind, detail, text, idx, extra=None):
        nonlocal findings
        h = hashlib.sha256(text.encode()).hexdigest()
        if h in seen:
            return
        seen.add(h)
        findings += 1
        name = f"exec_{args.seed}_{idx}_{kind}.op"
        fpath = os.path.join(args.corpus_dir, name)
        with open(fpath, "w", encoding="utf-8") as fh:
            fh.write(text)
        entry = {
            "kind": kind, "tool": "fuzz_exec",
            "input": os.path.relpath(fpath, ROOT),
            "seed": args.seed, "idx": idx, "detail": detail,
            "repro": f"python3 scripts/fuzz/fuzz_exec.py --seed {args.seed} --n {idx + 1}",
        }
        if extra:
            entry.update(extra)
        with open(manifest, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry) + "\n")
        print(f"  FINDING #{findings} [{kind}] {detail} | idx={idx} | saved {name}")

    i = 0
    while i < args.n and time.time() < deadline and findings < args.max_findings:
        # --- leg 1: run-path at scale (default-deny + fuel cap) ---
        if not args.skip_run:
            bucket, idx, text = gen_program(args.seed, i)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)
            legs["run"] += 1
            rc, out, err = probe(
                [binpath, "run", path, "--fuel", str(args.fuel)],
                args.per_input_timeout)
            bad = robustness(rc, err, out)
            if bad:
                finding("crash" if not bad.startswith("hang") else "hang",
                        f"run leg: {bad}", text, i, {"leg": "run"})
            i += 1

        # --- leg 2: redteam-directed escape generation ---
        if not args.skip_escape and i < args.n:
            bucket, idx, etext, shape = gen_escape_program(args.seed, i)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(etext)
            legs["escape"] += 1
            argv = [binpath, "run", path]
            if shape == "loop":
                argv += ["--fuel", str(args.fuel)]
            rc, out, err = probe(argv, args.per_input_timeout)
            bad = robustness(rc, err, out)
            if bad:
                finding("crash" if not bad.startswith("hang") else "hang",
                        f"escape leg ({shape}): {bad}", etext, i,
                        {"leg": "escape", "shape": shape})
            # E3 spill-over: escape-shaped programs through `check` too —
            # the checker sees the same vocabulary and must stay robust.
            if not args.skip_tooling:
                rc2, out2, err2 = probe([binpath, "check", path],
                                        args.per_input_timeout)
                bad2 = robustness(rc2, err2, out2)
                if bad2:
                    finding("crash" if not bad2.startswith("hang") else "hang",
                            f"escape->check: {bad2}", etext, i,
                            {"leg": "escape-check"})
            i += 1

        # --- leg 3: tooling surfaces every Kth program ---
        if not args.skip_tooling and i % max(1, args.tooling_every) == 0 \
                and i < args.n:
            bucket, idx, text = gen_program(args.seed, i)
            rng = random.Random(args.seed * 7 + i)
            if rng.random() < 0.2:
                text = mutation_of(text, rng)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)
            before = hashlib.sha256(open(path, "rb").read()).hexdigest()
            legs["tooling"] += 1

            surfaces = [
                ("doc", [binpath, "doc", path]),
                ("doc-json", [binpath, "doc", path, "--json"]),
                ("graph", [binpath, "graph", path]),
                ("graph-json", [binpath, "graph", path, "--json"]),
                ("disasm", [binpath, "disasm", path]),
                ("disasm-json", [binpath, "disasm", path, "--json"]),
                ("crispr", [binpath, "crispr", path, "--knockout",
                            gen_knockout(args.seed, i, text)]),
                ("crispr-json", [binpath, "crispr", path, "--knockout",
                                 gen_knockout(args.seed, i, text), "--json"]),
            ]
            for name, argv in surfaces:
                rc, out, err = probe(argv, args.per_input_timeout)
                bad = robustness(rc, err, out)
                if bad:
                    finding("crash" if not bad.startswith("hang") else "hang",
                            f"{name}: {bad}", text, i,
                            {"leg": "tooling", "surface": name})

            # rna --check on a generated patch; must be contained AND
            # never write (E3: input hash unchanged).
            ptext = gen_patch(args.seed, i, text)
            with open(patch_path, "w", encoding="utf-8") as fh:
                fh.write(ptext)
            for name, argv in (
                    ("rna-check", [binpath, "rna", path, patch_path, "--check"]),
                    ("rna-check-json", [binpath, "rna", path, patch_path,
                                        "--check", "--json"])):
                rc, out, err = probe(argv, args.per_input_timeout)
                bad = robustness(rc, err, out)
                if bad:
                    finding("crash" if not bad.startswith("hang") else "hang",
                            f"{name}: {bad}", text, i,
                            {"leg": "tooling", "surface": name,
                             "patch": ptext[:120]})
            after = hashlib.sha256(open(path, "rb").read()).hexdigest()
            if before != after:
                finding("sandbox-write",
                        "rna --check modified the input file (wrote while "
                        "checking)", text, i, {"leg": "tooling",
                                               "surface": "rna-check"})
            i += 1

    if os.path.isfile(path) and findings == 0:
        os.remove(path)
    if os.path.isfile(patch_path) and findings == 0:
        os.remove(patch_path)
    try:
        os.rmdir(TMPDIR)
    except OSError:
        pass

    elapsed = args.time_budget - (deadline - time.time())
    covered = ", ".join(f"{k}:{v}" for k, v in legs.items())
    print(f"\nfuzz_exec done: seed {args.seed} in {elapsed:.0f}s "
          f"({covered})")
    if findings:
        print(f"  unique findings saved: {findings} "
              f"({os.path.relpath(args.corpus_dir, ROOT)}/, manifest: "
              f"MANIFEST.jsonl)")
        print("  findings are BUGS to triage (see scripts/fuzz/TRIAGE.md), "
              "not a score.")
        return 1
    print("  no findings — default-deny run path, fuel counter and tooling "
          "surfaces hold contained-or-clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
