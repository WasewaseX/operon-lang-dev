# OPERON THREAT MODEL

W100 of the M100 program · v1.0.0 · 2026-09-26 · owner: sz (dev-3)
Baseline: main @ dd76caa (loop-10 R10-c) · redteam corpus: 97 payloads / 0 breaches

---

## 1. Purpose

Operon executes untrusted or semi-trusted programs by design: a scripting language whose
Total Grammar *accepts anything* is a standing invitation to hostile input. Dozens of
individual defenses exist across the interpreter, CLI, and CI. This document binds them
into one architecture: **what we protect, from whom, with which mechanism, proven by which
evidence, and what remains at risk.**

Rule: every future security mechanism lands with (a) a row added or updated here and
(b) a permanent redteam payload. Security work without a threat-model row is incomplete.

## 2. Assets (what we protect)

| # | Asset | Why it matters |
|---|-------|----------------|
| A1 | Host filesystem integrity | a script must never read/write outside its granted roots |
| A2 | Host compute resources | CPU (fuel), memory (2 GiB charge model), file descriptors, wall-clock (timeouts) are never unboundedly consumed |
| A3 | Process/host boundary | no escape from the interpreter process; no fork bombs; no signal abuse |
| A4 | Data confidentiality | granted reads return file contents; nothing else leaks (paths, env, keys) beyond policy |
| A5 | Network reputation | the HTTP client must not become an SSRF/CRLF-injection vector |
| A6 | Interpreter integrity | malformed/hostile input may produce errors, never memory-unsafety, hangs, or crashes |
| A7 | Supply-chain integrity | what builds and ships Operon (CI, toolchain, release assets) is pinned and auditable |
| A8 | User trust in output | diagnostics show what the interpreter actually did (repairs, spans), no silent divergence |

## 3. Trust boundaries and untrusted inputs

| Input | Trust level | Boundary crossed |
|-------|-------------|------------------|
| `.op` source (file, stdin, REPL, playground) | **untrusted** | parser + evaluator + every builtin |
| `.cell` configuration | **untrusted** | resource limits, capability grants (a hostile .cell must NOT be able to grant itself powers beyond CLI flags) |
| `.rna` edit scripts | **untrusted** | source rewriting (span-targeted edits) |
| modules via `use` (local dirs, `std/`) | **semi-trusted** | loader; same evaluator, caps of the importing run |
| Python packages reached via `py()` | **untrusted*** | OS subprocess boundary; *trusted only as far as the installed Python env is |
| HTTP peers via http builtins | **untrusted** | response parsing, headers, redirects |
| Local filesystem state (symlinks, FIFOs, /dev nodes) | **hostile-possible** | fd open paths, stat-then-use windows |
| CI/release inputs (actions, toolchains, artifacts) | **semi-trusted** | supply chain |

## 4. Defense inventory (threat → mechanism → evidence → residual risk)

### 4.1 Filesystem (A1, A4)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| path traversal (`../../etc/passwd`) | canonicalization + root containment before any I/O | rt_p3c_traversal, rt_p3d_abs_dotdot | none known |
| symlink escape (pre-positioned link swaps target after check) | **open-then-verify** fd discipline (sec-r5 F-8): verify on the opened fd, not the path | rt_p3b_symlink, rt_p3l_write_symlink, live TOCTOU flipper (canary pristine, 1972 denied / 28 granted) | none known |
| FIFO/device hang + /dev/zero OOM | `read_file` accepts **regular files only** + stat-size charge before read | sec-r5 F-11, rt suite | unusual file types fail closed |
| write escape via rename/hardlink games | capability-gated fs ops (`fs_delete/fs_rename/fs_mkdir`), anti-TOCTOU design | dx-r6 wave | rename-target races covered by open-then-verify |
| env/key leakage | `--allow-env` gate; env reads denied by default | rt_x_exit_denied family | documented flag = deliberate exposure |

### 4.2 Resource exhaustion (A2, A3)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| unbounded CPU | parse fuel + exec fuel + `--fuel` override, charged per construct (incl. unique fuel charge per site, sec-r4 F-series) | rt_p2d_fuel_gene, rt_p2f_uorf_fuel, rt_p2g_match_fuel | fuel math reviewed per sec wave |
| unbounded memory | `mem_charge` across interp allocations (sec-r4), 2 GiB contract, file-byte charges, large-clone charges, non-scalar map-key bounded scan (512) + step charge | rt_p2h_mem_list, rt_p2k_json_big | Rc cycle retention, tracked as W13 (walk-safety is proven; lifetime reclamation is the open piece) |
| quadratic/explosive walks (stringify/repr/deep_eq on DAGs) | DAG memoization in `stringify`/`repr`/`deep_eq`; `deep_eq` depth 100k→16k | sec-r5 F-10, rt_p5c_cycle | none known |
| thread bombs | spawn capability gate + worker-cell ceilings; CI uses `timeout -k 5 15` with rc137=HANG detection | rt_p4a/p4b/p4c/p4d/p4e | S9: runner-side TERM-resistance investigation open (builder-A) |
| output flooding | run output cap 64 MiB | sec-r4 F-series | none known |
| HTTP wall-time abuse | wall-time fuel on http builtins | sec-r4 | server-mode (future) will need its own budget |
| interpreter stack depth | interp-nesting depth cap | sec-r4 F-series, rt_p2a/p2c recursion payloads | none known |
| parser pathology (quote storms, 1M parens, big tokens) | parse fuel + token-size bounds; saturating big-integer literals (documented) | rt_p1b/p1b2/p1c/p1c3/p1d/p1d2/p1e | none known |
| parse-note flooding | parse-note caps | sec-r4 | none known |

### 4.3 Input robustness / interpreter integrity (A6, A8)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| NUL bytes, BOM/CRLF, mangled UTF-8 | lexer containment, errors/repairs, never crashes | rt_p7b/p7c/p7d/p7e/p7f/p7g(cell)/p7h/p7i/p7j | none known |
| NaN/Inf through JSON | json inf/nan → null (documented) | sec-r5 F-13, rt_p5e_nan_json | none known |
| JSON bombs (depth/size) | json depth + size charges | rt_p2j_json_depth | none known |
| string interpolation escape games | bounded interpolation (rt_p1f, rt_p6d_brescape, rt_p6e_qinterp) | rt suite | none known |
| `.cell` garbage | loader containment; W66 adds formal schema + unknown-key diagnostics | rt_p7g_cell_garbage, rt_p3h_cell_auto | schema work open (W66, builder-B) |
| regex pathology | Thompson NFA (linear), nesting budget | rt_p9a/p9b | none known |
| module loader abuse (`use` of /proc, escapes) | resolution confined to declared roots; CRLF/proc payloads | rt_p7a_selfuse, rt_p3j_use_escape, rt_p3k_use_proc | none known |

### 4.4 Python bridge `py()` (A1/A2/A6)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| silent bridge exposure | **default-off**: requires `--allow-py` AND Caps.py grant entries | substrate-r1 (d78f541), rt_p12a/b/c, tests/granted/ operator-cell suite | the subprocess inherits the user's Python env, an untrusted Python env = untrusted bridge (documented contract, W79) |
| bridge as cap bypass | py() calls are themselves capability-charged; granted suite asserts denial without grant | rt_p12* | none known |

### 4.5 Network (A5)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| CRLF request smuggling | header/URL validation | rt_p9e_http_crlf | none known |
| net as exfil channel | `--allow-net` default-deny | caps_policy.op | flag = deliberate exposure |

### 4.6 Supply chain (A7)

| Threat | Mechanism | Evidence | Residual |
|--------|-----------|----------|----------|
| CI action tampering | **all GitHub Actions SHA-pinned** (D-6, sec-r5) | ci.yml/release.yml | W99 adds advisory scanning (cargo-audit/deny), open |
| toolchain drift | pinned toolchain in CI | workflows | same |
| release artifact tampering | release workflow builds from tagged commit; checksums published | release.yml | W60 per-target smoke, open |

## 5. What we do NOT protect against (honest residuals)

1. **A malicious granted configuration is out of scope**: if the operator passes
   `--allow-read --allow-write --allow-net`, the program is supposed to touch those
   resources. The model protects the *default*; explicit grants are operator consent.
2. **An untrusted Python environment behind `py()`**: the bridge is a door, not a filter.
   Whatever the installed interpreter can do, a granted `py()` call can do.
3. **Hostile OS-level attacker** (rootkits, memory inspection of the interpreter process):
   out of scope; Operon is not a security sandbox against the machine's own kernel.
4. **Side channels** (timing across granted/ungranted branches): not a design goal today.
5. **Rc cycle lifetime reclamation** (W13): walk-safety is proven; a pathological cycle
   builder can retain memory up to the 2 GiB charge ceiling, bounded, not zero.
6. **Formal verification**: invariants are test-proven (redteam + differential), not
   proof-carrying. Fuzzing (W51) is the next evidence tier.

## 6. Operating rules

1. **Every security fix ships with a redteam payload that fails on the old binary.**
   (Practice since sec-r2; kept.)
2. **Default-deny is non-negotiable**: new capabilities (fs/net/py/run/env/exit) land
   denied unless a capability gate and a grant path exist.
3. **Charge everything**: any new allocator, loop, or I/O primitive gets fuel/mem charges
   in the same PR.
4. **Disclosure**: security issues go to the owner privately first; fixes land as sec-r*
   waves with payloads; this document updates in the same merge.
5. **Review cadence**: every sec-r* wave re-reads §4/§5 of this file and reconciles rows;
   sz re-verifies the full redteam suite on every release tag.

## 7. Traceability

| Era | Wave | Rows touched |
|-----|------|--------------|
| v2.0–2.1 | original sandbox (caps, fuel, mem) | 4.1, 4.2 |
| sec-r2 (A15) | C kernel deletion, Rust symbol table | 4.3 |
| sec-r4 | nesting cap, interp mem_charge, note caps, unique fuel, http wall-time, output cap | 4.2, 4.3 |
| sec-r5 | fd open-then-verify, regular-file reads, DAG memo, clone/key charges, json null, SHA pinning | 4.1, 4.2, 4.3, 4.6 |
| dx-r6 | fs mutate ops + anti-TOCTOU | 4.1 |
| substrate-r1 | py() grants + default-off | 4.4 |
| loop-9/10 | redteam 95→97, queue shield, pin preferences | 4.2 |
| **this doc (W100)** | first consolidated threat model | all |
