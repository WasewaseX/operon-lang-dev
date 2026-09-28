#!/usr/bin/env python3
# board_sync.py: bring TODO-100.md dev1 statuses + gate header up to measured
# reality on main. Evidence cites the commit that landed each unit. One-shot
# helper for the board-truth commit; kept in scripts/ so the next stale-board
# session can reuse or extend the map.
import re

PATH = "TODO-100.md"

STATUS = {
    "W001": "partial: stage 1 L2c soft annotations on main 8f59164 (gene param/return + let anns, unions, optionals, unknown-name typo armor, oracle op-for-op, type_anns.op 30 asserts + differential); REMAIN: check-time inference + reporting, List<T>/Map<K,V> sugar, type aliases",
    "W002": "partial: stage 1 match-v2 on main cb25d46 (variant payloads, list/map patterns, or-patterns, guards, capture scopes, oracle mirror, match_v2.op 45 asserts + rt_p17a); REMAIN: unreachable-arm detection in check (feeds W042)",
    "W003": "partial: stage 1 on main 56097b9 (docs/specs/GENERICS.md staged plan; callable-generic std APIs with zero per-type duplication; std_generics.op + differential generics.op); stage 2 type params rides W01; stage 3 explicit-instantiation monomorphism spec'd, deliberately unscheduled",
    "W004": "done: main 8f1b8a2, trait declarations + phenotype implements + default methods, both cores",
    "W005": "done: main ecbff93 const bindings + deep freeze, both cores; hardening b22f484 (registry keep-alive + iterative walk, rt_p18a); hotfix 796514c retired the fixer const-to-let migration since const is live semantics",
    "W006": "partial: stage 1 D-014 on main c6ad132 (variant values, ?! propagation, 10 builtins, SPEC §9 hierarchy, oracle byte-identical, rt_p16a) + null-payload/non-finite-JSON parity fix 14bda8a; REMAIN: stage 2 std null-to-Result migration behind a compat note",
    "W007": "done: main 563a331, call-chain capture + oracle line-parity + rt_p15a-c + SPEC §9a",
    "W008": "partial: phase 1 debug REPL on main 1fb9803; REMAIN: stepping breadth + DAP adapter so VSCode gets the same via operon-ls",
    "W009": "partial: A1 design note adopted 372f30a (docs/vm-design.md); A2 OIR1 bytecode machine on main 09cf7c6, vm lane 184/184 vs the same oracle outputs; REMAIN: full-corpus parity campaign + fib25 ≥2x bench gate confirmation",
    "W010": "partial: stage 1 disasm rides OIR1 09cf7c6; REMAIN: annotated dump stability tests + every opcode documented in SPEC §VM",
    "W011": "partial: stage 1 on main 61249a8 (constant folding + jump threading behind --opt 1); REMAIN: DCE, trivial-gene inlining, monomorphic specialization, per-pass bench rows + toggle matrix",
    "W012": "deferred: audit orders VM → profiling → opt → JIT; deliverable until un-deferred lives in docs/vm-design.md (fiber-field reservation §6, async posture); owner sign-off required to start",
    "W013": "partial: D-013 decision recorded + SPEC §19 + memory_model.op on main 563a331; REMAIN: memory() live-cycle count + weak refs API (recommend (a)+(c) per audit)",
    "W014": "done: main 563a331, SPEC §19 + differential memory_model.op",
    "W015": "partial: task groups wait_all/wait_any on main f0a3c1d; REMAIN: channel()/send/recv/close + select over multiple channels (L2a)",
    "W016": "partial: spec sketch docs/specs/ASYNC.md on main cc35e95 (green threads over the VM loop, suspension at builtin boundaries only, fiber-field reservation carried in vm-design.md §6); REMAIN: implementation",
    "W017": "done: main dc982a9, scope block with auto-join/auto-cancel semantics",
    "W018": "done: main 1b71bd0, cooperative task cancellation",
    "W019": "done: main 8632a69, src/pkg.rs (in-house sha256, minimal-TOML manifest parser, deterministic transitive closure, vendored cache, managed-tree import gating) + scripts/pkg_e2e.sh offline lockfile run proven",
    "W020": "done: main 8632a69, operon mod init/add/remove/update/install/tree/verify, git CLI, zero crates",
    "W021": "partial: static git-index registry on main 1b4941c (the cheap first version); hosted service stays deferred pending owner infrastructure decisions",
    "W022": "done: main 8e2d060 (written rule; .cell = runtime config ONLY, package metadata belongs to operon.toml)",
    "W023": "done: main 8632a69, operon.lock (name/git/rev/sha256 content checksum) + --locked drift rejection incl. transitive justification",
    "W024": "done: main c656223, contextual pub marker (not a keyword) + .cell modules.visibility=strict + SPEC §8 visibility contract + granted-cell differential",
    "W025": "partial: '::' separator sugar on main 6b984f6 (use std::bio, multi-segment a::b::c, mixable with / . - separators, differential namespaces.op); REMAIN: nested sub-module declarations",
    "W026": "partial: std/set (loop-10) + std/deque and std/heap on main 88d9b63 (deterministic, comparator-gene capable); REMAIN: Graph<T>, W003 annotation sugar over the containers",
    "W027": "partial: 22 modules / 226 functions on main f0527e5 era (adds path 09e81ba, deque/heap/time 88d9b63, serialize, unicode b0e12ba, bigint 1152851); REMAIN per audit list: process, env, logging, terminal, compression, hashing, url, http-high-level, walk, binary, db-stub",
    "W028": "partial: stage 1 on main b0e12ba (byte/char/grapheme semantics + case-fold subset + std/unicode); REMAIN: NFC/NFD normalization, full case folding, category queries",
    "W029": "done: main 08386fa, bytes kind + b\"\" literals with full escape set + conversions + read_file_bytes/write_file_bytes + SendValue membrane + oracle native-bytes mirror + rt_p19a",
    "W030": "done: main 563a331, r\"...\" + \"\"\"...\"\"\" oracle-mirrored; b\"...\" landed with W029 (08386fa)",
    "W031": "done: main 563a331, 0x/0b/0o + _ separators, oracle-mirrored, SPEC §3",
    "W032": "done: main 1152851, overflow contract pinned per-op both engines (i64::MIN%-1 oracle corner fixed) + std/bigint digit-list bignum (from_int/from_str/to_str/cmp/add/sub/mul/pow/fact/neg/abs) + SPEC decision note",
    "W033": "partial: L1d builtins + std/time.op duration/instant arithmetic on main 88d9b63 (UTC-only contract per W89); REMAIN: ISO-8601 parsing",
}

HEADER = """## CURRENT GATE NUMBERS (2026-09-28, main @ f0527e5, re-measured on a fresh build)

differential **190/190 MATCH** (10 granted cells) + vm lane **184/184** · proofs
**154 files / 117 proofs / 1,675 assertions, 117 passed 0 failed** · redteam
**104 contained / 0 breached** · cargo test green · clippy 0 · fmt clean · docs
sync green · CodeQL **0 findings** (f0527e5) · CI success.

> These numbers are re-measured every loop; when they change, update this header in the
> same commit that lands work. If this header is stale, the per-level evidence links win."""

txt = open(PATH, encoding="utf-8").read()

# replace the gate header block (from the heading line to the blank line before the next ---)
txt = re.sub(r"## CURRENT GATE NUMBERS.*?(?=\n\n---)", HEADER.replace("\\", "\\\\"), txt, flags=re.S)

changed = 0
for wid, status in STATUS.items():
    pat = re.compile(r"(^### " + wid + r"\b.*?)\s*\[(?:open|claimed|wip|done|partial|verify|deferred|wontfix|queued)[^\]]*\]+(.*)$", re.M)
    def repl(m):
        return m.group(1) + " [" + status + "]" + m.group(2)
    txt2, n = pat.subn(repl, txt, count=1)
    if n:
        txt, changed = txt2, changed + n
    else:
        print(f"WARN: no heading matched for {wid}")

open(PATH, "w", encoding="utf-8").write(txt)
print(f"updated {changed} status lines + gate header")
