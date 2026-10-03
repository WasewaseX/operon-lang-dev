#!/usr/bin/env python3
"""gen_doc_stats.py, W53/W55: ONE generated source of truth for doc numbers.

Emits docs/stats.json + docs/STATS.md. Humans never hand-type these numbers:
  - version (Cargo.toml)          - SPEC version lines
  - std modules + function counts - keyword inventory (src/parser.rs KEYWORDS)
  - red-team file count           - proof file count
  - CLI subcommand inventory      - LSP method inventory
Run:  python3 scripts/gen_doc_stats.py            (from repo root)
Exit: 0 on success. Deterministic output (sorted keys, no timestamps beyond
the source commit the repo provides via OPERON_STATS_COMMIT or 'unknown').

GEN_DOC_STATS_FAST=1 skips the two subprocess-heavy recounts (the full
differential harness, ~10 min, and the proof-suite run) — the W006 wave-2
session added this so a regeneration fits inside one working step. The
seven drift-checked fields (version, keyword/std/redteam/proof/test_op
counts) are pure file walks and are IDENTICAL in both modes; the skipped
fields are informational only (check_docs_sync does not drift-check them,
the harness itself is run separately by scripts/test.sh on every gate).
"""
import json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# D-008: every keyword gets a one-line programmer analogy. Missing entries are
# rendered as "analogy pending" so the table can never silently lie.
KEYWORD_ANALOGY = {
    "gene": "named function (def)", "let": "variable binding",
    "if": "conditional", "elif": "else-if branch", "else": "fallback branch",
    "while": "condition loop", "loop": "infinite loop (break to exit)",
    "scope": "structured-concurrency block (children joined at exit)",
    "for": "iteration", "in": "membership / loop binder", "return": "exit with value",
    "break": "leave loop", "continue": "next iteration",
    "match": "pattern switch", "case": "match arm", "use": "import module",
    "tad": "module boundary (TAD-insulated exports)", "anchor": "explicit export anchor",
    "export": "export declaration", "import": "synonym of use",
    "enhance": "feature-flag boost (lowers a gene's gate threshold)",
    "silence": "disable a gene (soft-off)", "stress": "throwable error value (exception)",
    "rescue": "catch handler", "raise": "throw a stress",
    "fate": "state-machine tag on a value", "state": "declare state cell",
    "regulate": "feature-flag network gating calls", "activates": "positive regulation edge",
    "inhibits": "negative regulation edge", "strength": "regulation edge weight",
    "toggle": "boolean gate switch", "repressilator": "3-node oscillator (negative-feedback ring)",
    "period": "oscillator timing", "frame": "proof/measurement window",
    "proof": "named assertion block (test)", "guard": "early-exit clause",
    "splice": "runtime variant swap for a gene", "variant": "named alternative implementation",
    "edit": "in-place value edit", "replace": "swap an implementation",
    "ires": "secondary entry point", "as": "alias binding",
    "collect": "comprehension body", "enter": "scope-entry hook",
    "phenotype": "class (fields + methods)", "sequence": "generator function",
    "yield": "generator yield", "new": "constructor call", "threshold": "gate cutoff",
    "from": "inheritance (phenotype C from P)", "self": "method receiver",
    "decoy": "decoy binding site (absorbs interference)", "ligand": "named signal value (global pool)",
    "autoinducer": "quorum-sensing counter (population medium)",
    "bind": "attach a binding site", "inducer": "activating ligand",
    "cofactor": "ligand modifier", "operon": "polycistronic unit (batch of genes)",
    "trait": "interface (required + default methods)",
}

def read(p):
    with open(os.path.join(ROOT, p), encoding="utf-8", errors="replace") as f:
        return f.read()

def cargo_version():
    m = re.search(r'^version\s*=\s*"([^"]+)"', read("Cargo.toml"), re.M)
    return m.group(1) if m else "unknown"

def spec_version_lines():
    spec = read("SPEC.md")
    h1 = spec.splitlines()[0] if spec else ""
    m = re.search(r'^\*\*Status:\*\*\s*([vV]?\d+\.\d+\.\d+(?:-dev)?)', spec, re.M)
    return {"h1": h1.strip(), "status": m.group(1) if m else "unknown"}

def keywords():
    src = read("src/parser.rs")
    m = re.search(r'const KEYWORDS: &\[&str\] = &\[(.*?)\];', src, re.S)
    items = re.findall(r'"([^"]+)"', m.group(1)) if m else []
    return sorted(set(items))

def std_modules():
    std = sorted(f for f in os.listdir(os.path.join(ROOT, "std")) if f.endswith(".op"))
    out = []
    for f in std:
        body = read(os.path.join("std", f))
        funcs = len(re.findall(r'^\s*(?:pub\s+)?gene\s+(\w+)\s*\(', body, re.M))
        out.append({"module": f[:-3], "functions": funcs})
    return out

def redteam_files():
    d = os.path.join(ROOT, "tests", "redteam")
    if not os.path.isdir(d):
        return 0
    return len([f for f in os.listdir(d) if f.endswith(".op")])

def _walk_op_files(rel_dir):
    """Recursive .op listing, matches `operon test` (tests/ scanned recursively).
    Without this, the 'Proof files' line (top-level only) contradicts the
    binary's 'Proof run' line (recursive) inside the same generated STATS.md."""
    tdir = os.path.join(ROOT, rel_dir)
    out = []
    for dirpath, _dirnames, filenames in os.walk(tdir):
        for f in sorted(filenames):
            if f.endswith(".op"):
                out.append(os.path.join(dirpath, f))
    return sorted(out)

def proof_files():
    n = 0
    for p in _walk_op_files("tests"):
        if "frame proof" in read(os.path.relpath(p, ROOT)):
            n += 1
    return n

def test_op_files():
    return len(_walk_op_files("tests"))

def cli_subcommands():
    src = read("src/main.rs")
    seen = sorted(set(re.findall(r'^\s{8}"([a-z-]+)"\s*=>', src, re.M)))
    return seen

def lsp_methods():
    try:
        src = read("src/bin/operon-ls.rs")
    except FileNotFoundError:
        return []
    return sorted(set(re.findall(r'"(textDocument/[A-Za-z]+|initialize|shutdown)"', src)))

def proof_totals_from_binary():
    """Ask the freshly built binary (bin/operon or target/release/operon) for
    authoritative proof counts; graceful 'unavailable' when not built."""
    for cand in ("bin/operon", "target/release/operon"):
        p = os.path.join(ROOT, cand)
        if os.path.exists(p):
            try:
                import subprocess
                r = subprocess.run([p, "test", "tests"], cwd=ROOT,
                                   capture_output=True, text=True, timeout=600)
                # human summary carries the assertion count; --json omits it
                m = re.search(r"(\d+) file\(s\), (\d+) proof\(s\): (\d+) passed, "
                              r"(\d+) failed \((\d+) assertion", r.stdout + r.stderr)
                if not m:
                    continue
                return {"files": int(m.group(1)), "proofs": int(m.group(2)),
                        "asserts": int(m.group(5)), "source": cand}
            except Exception:
                continue
    return {"files": None, "proofs": None, "asserts": None, "source": "binary-not-built"}

def harness_counts():
    """Run the differential harness; parse 'result: N match, M diverge, K skipped'.
    The granted lane (explicit operator cells) is reported alongside."""
    import subprocess
    try:
        r = subprocess.run(["python3", "bootstrap/harness.py"], cwd=ROOT,
                           capture_output=True, text=True, timeout=1800)
        m = re.search(r"result: (\d+) match, (\d+) diverge, (\d+) skipped",
                      r.stdout + r.stderr)
        if not m:
            return None
        granted = len([p for p in os.listdir(os.path.join(ROOT, "tests", "granted"))
                       if p.endswith(".op")]) if os.path.isdir(os.path.join(ROOT, "tests", "granted")) else 0
        return {"match": int(m.group(1)), "diverge": int(m.group(2)),
                "skipped": int(m.group(3)), "granted_cells": granted}
    except Exception:
        return None

def compute():
    kw = keywords()
    mods = std_modules()
    spec = spec_version_lines()
    # GEN_DOC_STATS_FAST=1 (W006 wave 2): the harness recount re-runs the
    # full differential battery (~10 min) and the proof recount the whole
    # proof suite — both are run separately by scripts/test.sh on every
    # gate, so fast mode reports them as skipped instead. The seven
    # drift-checked fields never take this path.
    fast = os.environ.get("GEN_DOC_STATS_FAST") == "1"
    return {
        "version": cargo_version(),
        "spec": spec,
        "keywords": kw,
        "keyword_count": len(kw),
        "keyword_analogy_pending": [k for k in kw if k not in KEYWORD_ANALOGY],
        "std_modules": mods,
        "std_module_count": len(mods),
        "std_function_count": sum(m["functions"] for m in mods),
        "redteam_files": redteam_files(),
        "proof_files": proof_files(),
        "test_op_files": test_op_files(),
        "proof_totals": ({"files": None, "proofs": None, "asserts": None,
                          "source": "fast-mode skip"} if fast
                         else proof_totals_from_binary()),
        "harness": (None if fast else harness_counts()),
        "cli_subcommands": cli_subcommands(),
        "lsp_methods": lsp_methods(),
    }

def render(s):
    L = []
    L.append("# Operon repo statistics, GENERATED, do not hand-edit\n")
    L.append("Source of truth: `scripts/gen_doc_stats.py` (run from repo root).\n"
             "Validated by `scripts/check_docs_sync.py`. Hand-typed numbers in\n"
             "README/SPEC/BENCH are forbidden, link here instead.\n")
    L.append(f"- **Version**: {s['version']}  · SPEC Status: {s['spec']['status']}")
    L.append(f"- **Keywords (parser reserved set)**: {s['keyword_count']}, table in "
             f"[KEYWORDS.md](KEYWORDS.md)")
    if s["keyword_analogy_pending"]:
        L.append(f"  - analogy pending: {', '.join(s['keyword_analogy_pending'])}")
    L.append(f"- **Std modules**: {s['std_module_count']} "
             f"({', '.join(m['module'] for m in s['std_modules'])})")
    L.append(f"- **Std functions (.op-level `gene` defs)**: {s['std_function_count']}")
    L.append(f"- **Red-team payload files**: {s['redteam_files']}")
    L.append(f"- **Proof files**: {s['proof_files']} (of {s['test_op_files']} test .op files)")
    pt = s["proof_totals"]
    if pt["proofs"] is not None:
        L.append(f"- **Proof run** ({pt['source']}): {pt['files']} files, "
                 f"{pt['proofs']} proofs, {pt['asserts']} asserts")
    elif pt.get("source") == "fast-mode skip":
        L.append("- **Proof run / harness recount**: skipped (GEN_DOC_STATS_FAST=1), "
                 "run by scripts/test.sh on every gate")
    else:
        L.append("- **Proof run**: binary not built, run scripts/build.sh then regenerate")
    h = s.get("harness")
    if h:
        L.append(f"- **Differential harness**: {h['match']} match / {h['diverge']} diverge "
                 f"({h['skipped']} skipped) · granted lane: {h['granted_cells']} cells")
    L.append(f"- **CLI subcommands**: {', '.join(s['cli_subcommands'])}")
    L.append(f"- **LSP methods**: {', '.join(s['lsp_methods'])}")
    L.append("\n## Std module inventory\n")
    L.append("| module | .op-level functions |")
    L.append("|---|---|")
    for m in s["std_modules"]:
        L.append(f"| std/{m['module']} | {m['functions']} |")
    return "\n".join(L) + "\n"

def main():
    s = compute()
    os.makedirs(os.path.join(ROOT, "docs"), exist_ok=True)
    with open(os.path.join(ROOT, "docs", "stats.json"), "w", encoding="utf-8") as f:
        json.dump(s, f, indent=1, sort_keys=True)
        f.write("\n")
    with open(os.path.join(ROOT, "docs", "STATS.md"), "w", encoding="utf-8") as f:
        f.write(render(s))
    # docs/KEYWORDS.md, W55: generated keyword inventory (D-008 analogies)
    pending = s["keyword_analogy_pending"]
    K = ["# Operon keywords, GENERATED from src/parser.rs, do not hand-edit\n",
         "The parser's reserved set (source of truth: `KEYWORDS` in `src/parser.rs`).\n",
         f"Count: **{s['keyword_count']}**. Regenerate: `python3 scripts/gen_doc_stats.py`.\n",
         "Analogy voice per D-008 (zero biology assumed; one-line programmer meaning).\n",
         "| keyword | programmer analogy |", "|---|---|"]
    for k in s["keywords"]:
        a = KEYWORD_ANALOGY.get(k, "*(analogy pending, file a docs finding)*")
        K.append(f"| `{k}` | {a} |")
    K.append("\nLiteral words `true false null` and logical `and or not` are recognized in\n"
             "expression positions but are not part of the reserved table (SPEC §3).\n")
    if pending:
        K.append(f"NOTE: {len(pending)} keyword(s) lack a D-008 analogy: {', '.join(pending)}.\n")
    with open(os.path.join(ROOT, "docs", "KEYWORDS.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(K))
    print(f"stats: v{s['version']} kw={s['keyword_count']} std={s['std_module_count']} modules/"
          f"{s['std_function_count']} funcs redteam={s['redteam_files']} proofs={s['proof_files']}f "
          f"bin={s['proof_totals']['source']}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
