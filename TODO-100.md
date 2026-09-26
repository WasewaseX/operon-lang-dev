# OPERON M100 — PROCESS & GOVERNANCE LAYER (100-level program)

v1.1.0 · 2026-09-26 · owner directive: *"make a super detailed hundred level to do list ·
you are dev 3 so build the 66-100 part · other 2 build the others · save EVERYTHING in the
repo so we survive terrible environment resets"*

> **BOARD vs PROCESS — read this first.** The **canonical per-item board is
> `collab/ROADMAP-100.md`** (W-track, builder-B, statuses measured against main @ dd76caa,
> published first, already executing). **This file is the process/governance layer**: iron
> rules, ownership, sprint waves, Track L cross-link, per-dev suggested orders, and the
> environment-reset survival protocol. Only **W-IDs** circulate (W01–W100). If this file's
> per-level notes and ROADMAP-100's statuses ever disagree, ROADMAP-100 wins and the
> discrepancy is a bug to fix in the same session.

Everything in this file is committed to the repo on purpose: **this document is the
survival artifact**. If an environment resets, the M100 board plus `collab/` in
`WasewaseX/project-vault` reconstructs 100% of program state.

---

## OWNERSHIP (fixed by owner directive)

| dev | identity | branch namespace | levels | lane defaults |
|-----|-------------------|------------------|------------|---------------|
| dev-1 | builder-A | `builder/*` | **W001–W033** | `src/` core language (lexer/parser/interp/ast), SPEC language sections, VM |
| dev-2 | builder-B | `b2/*` | **W034–W066** | tooling (`src/tools.rs`, LSP, CLI auxiliary), docs truth passes, CI workflows, `std/*.op`, `.cell` schema |
| dev-3 | sz | `sz/*` | **W067–W100** | contract/spec docs (THREAT-MODEL, BIO-CONTRACT, DETERMINISM, BUILD), `.rna` safety, doc generator, watch, graph export, release engineering, security automation |

**Boundary note (owner arbitration pending):** the owner told sz "build the 66–100 part"
and builder-B's relay gives sz W67–W100 with W66 (`.cell` schema) in dev-2's block. W66
stays builder-B's (claimed first, wave order committed); if the owner re-draws the line,
we re-claim. One item, zero duplicate work either way.

Rules:
1. Ownership = **default claimant and tiebreaker**, not exclusivity. Any dev may claim any
   open level in `collab/TASKS.md` (claim format per CONTRIBUTING.md §2) as long as the
   level's file list does not collide with an in-flight WIP PR.
2. Lane defaults exist to minimize merge conflicts; crossing a lane requires a COMMS.md note
   before the push, not after.
3. builder-A may keep executing the loop-11 queue; loop-11 items are M100 levels and get
   their status here from now on. Track L levels fold in (see cross-link table below).

## IRON RULES (unchanged — CONTRIBUTING.md §1)

1. **Invariants red = stop everything.** Before any merge: differential harness ALL MATCH,
   proof suite all green, redteam 0 breaches, `cargo test` green, clippy 0, fmt clean,
   cookbook gate green, LSP smoke green.
2. **WIP PR at push** — the moment a branch hits the remote, a PR (may be marked WIP) must
   exist. No exceptions; this killed us once (session-4 race).
3. **One lane per PR**; reviewer runs the file-list lane check before merge.
4. **Every level = one mergeable unit** (or a small ordered PR series). Levels marked
   `deferred` need only a design note merged into the level's stated file.
5. **Status lives in two places**: this file's per-level status field, and
   `collab/TASKS.md` §M100 board. Update both in the same commit when possible.

## STATUS LEGEND

`[open]` not started · `[claimed]` owner assigned, not started · `[wip]` branch exists ·
`[done]` merged with evidence · `[partial: …]` some sub-criteria landed, rest described ·
`[verify]` suspected already closed by a loop; needs evidence link before marking done ·
`[deferred: reason]` consciously postponed; design note is the deliverable ·
`[wontfix: reason]` rejected with recorded rationale (DECISIONS.md entry required).

Effort: S ≤ half session · M = 1–2 sessions · L = multi-session · XL = release-scale.

## CURRENT GATE NUMBERS (2026-09-27, sz/w076-embed head over main 15aa6ac — dev-3 wrap-up + full b2 stack landed)

differential **154/154 MATCH** · proofs **122 files / 103 proofs green (1346 asserts, 0 failed, Rust AND oracle)** ·
redteam **100 payloads / 0 breaches** · cargo test green (101 tests) ·
clippy 0 · fmt clean · cookbook **19/19** · LSP smoke OK (incl. rename asserts) · sec_regression ALL GREEN ·
doc_api_check green · **embed gate green (W076: examples/embed path-dep crate builds + runs, promote() captured)**.

> These numbers are re-measured every loop; when they change, update this header in the
> same commit that lands work. If this header is stale, the per-level evidence links win.

## TRACK L CROSS-LINK (MASTER-PLAN §7 folds into M100)

| Track L | M100 level(s) | status |
|---------|---------------|--------|
| L1a null-safety/destructuring/builtins | done (PR #15) | closed |
| L1b stdlib breadth | W027 | wip (builder-A, loop-10 wave S) |
| L1c regex subset | done | closed (re_split verify-or-drop: see W027 note) |
| L1d wall-clock time | W033 | done (unix_time/date_parts/date_fmt) |
| L2a channels | W015 | queued (builder-A) |
| L2b match patterns v2 | W002 | queued (builder-A) |
| L2c type annotations | W001 | queued (builder-A) |
| L2d operator overloading | W004 (partial) | open |
| L2e pipeline `\|>` | — | blocked on sz grammar verdict |
| L3a fmt config | W047 | open |
| L3b doc | W073/W074 | **dev-3** |
| L3c manifest | W019/W022/W023 | open |
| L3d check --json | W041 | open |
| L3e REPL v2 | W008 (partial) | open |

---
---

# DEV-1 BLOCK — W001–W033 (builder-A): language core

> Suggested execution order within the block (P0/P1 first): **W007 → W006 → W013 → W014 →
> W001 → W002 → W003 → W004 → W005 → W009 → W031 → W015**, then the P2s. Rationale: the
> audit's own "fix first" list puts the error model and memory contract ahead of new syntax.
> The VM (W009) starts only after W007/W006 land so the bytecode design targets the final
> error model, not a moving one.

### W001 — Optional static type annotations [P1] [dev-1] [XL] [Track L2c] [open]
- Goal: `gene add(a: int, b: int) -> int` parses and soft-checks; annotations are optional
  everywhere (gradual typing); unknown/unexpected types become semantic **warnings**, never
  rejections (Total Grammar preserved).
- Done when: grammar accepts annotations on genes, `let`, phenotype fields; `check` emits
  `type-mismatch` diagnostics with spans; SPEC §5 documents the annotation surface; oracle
  mirrors the parser; ≥4 differential corpus programs exercise annotation paths.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/differential/`.
- Depends: none. Blocks W003/W026 typing story.

### W002 — Algebraic data types + match v2 [P1] [dev-1] [XL] [Track L2b] [open]
- Goal: match gains or-patterns, struct/map patterns, nested patterns, guards in every arm,
  and variant-style payloads; Option/Result-shaped matching becomes idiomatic once W006 lands.
- Done when: `match x { 1 | 2 => .., [a, rest] => .., {k: v} if v > 0 => .. }` parses and
  evaluates; unreachable-arm detection added to `check` (feeds W042); SPEC §5 patterns
  section; oracle parity; differential programs.
- Files: `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/`.
- Depends: none (builder-A already claimed via L2b).

### W003 — Generics [P1] [dev-1] [XL] [open]
- Goal: generic genes `gene map<T, U>(xs: List<T>, f: gene(T) -> U)` with monomorphized
  execution (no runtime cost), typed containers `List<T>`, `Map<K,V>` as annotation sugar
  over existing dynamic containers (soft-checked per W001).
- Done when: parse+soft-check of generic signatures; monomorphic call specialization in the
  interpreter; ≥3 std functions gain typed wrappers; SPEC section; oracle mirror.
- Files: `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `std/collections.op`,
  `bootstrap/oracle.py`, `SPEC.md`.
- Depends: W001. Phased: (a) generic genes, (b) container annotations, (c) trait bounds (W004).

### W004 — Traits / interfaces [P1] [dev-1] [XL] [open]
- Goal: capability-oriented abstraction alongside phenotypes: `trait Show { gene show() }`,
  `phenotype User implements Show`. First four std traits: `Show`, `Eq`, `Serialize`
  (feeds W034), `Iterate`.
- Done when: trait declaration + implementation + dynamic dispatch work; conflict =
  semantic warning with spans; phenotype method resolution checks traits first, inheritance
  second; SPEC §phenotypes extended; oracle mirror.
- Files: `src/ast.rs`, `src/parser.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `std/*.op`.
- Depends: W001 recommended. L2d (operator overloading) is a special case of this.

### W005 — Immutability split: let / const / mut [P2] [dev-1] [M] [open]
- Goal: `let` = single-assignment binding (today's `let`), `const` = compile-time constant
  with literal-fold guarantee, `mut` re-binding for mutable containers; today's
  `const → let` synonym gets a deprecation note (feeds W064).
- Done when: grammar accepts the triad; `check` warns on mutation of non-`mut` containers;
  SPEC §6 rewritten; compatibility note in README; oracle mirror.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `README.md`.
- Depends: none. Coordinate with W064 (deprecation machinery, dev-2).

### W006 — First-class Option / Result [P0] [dev-1] [L] [partial: D-014 stage 1 on main c6ad132 — Variant values (some/none/ok/err), ?! propagation (unforgeable Stress.prop marker; crosses rescue, no chain frames), 10 builtins, unwrap kind, none-synonym retirement, SPEC §9 four-tier hierarchy, oracle byte-identical, differential + proofs + rt_p16a; REMAIN: match-integration rides W002, std null→Result migration = stage 2 (compat note in SPEC §9)]
- Goal: `Option<T>` / `Result<T, E>` as built-in variant values with `unwrap/unwrap_or/
  is_ok/is_err/?`-style propagation operator; Stress becomes purely the runtime containment
  mechanism (its current dual role as everyday error value ends).
- Done when: constructors + matchability via W002 patterns; `?`-propagation inside genes;
  std functions that today return `null` on failure gain documented Result returns behind a
  compatibility note; SPEC §9 (error model) rewritten around the hierarchy
  null → Result → Stress → termination; oracle mirror; differential programs.
- Files: `src/value.rs`, `src/interp.rs`, `src/parser.rs`, `src/ast.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/`.
- Depends: W002 (pattern payloads) strongly recommended; may start in parallel.

### W007 — Real error tracebacks [P0] [dev-1] [M] [done: main 563a331 — call-chain capture + oracle line-parity + rt_p15a-c + SPEC §9a]
- Goal: `Stress overflow / at calculate() line 12 col 3 / at process() line 40 / at main()`
  — full call chain with file:line:col, gene name, and stress kind, for every uncaught
  Stress and hard error.
- Done when: interpreter maintains a frame stack with spans (dx-r4 spans exist — extend);
  uncaught Stress prints the chain; `--json` diagnostics include the chain; LSP surfaces it;
  ≥3 redteam payloads assert containment still holds (no info leak through paths);
  SPEC §9 updated.
- Files: `src/interp.rs` (frame push/pop), `src/tools.rs`, `src/ls.rs`, `SPEC.md`,
  `tests/redteam/`.
- Partial today: `stress.line` render (dx-r5) is the seed. Keep behavior backward-compatible.

### W008 — Debugger (REPL v2 stepping + DAP later) [P2] [dev-1] [XL] [open]
- Goal: breakpoints, step over/into/out, locals, watch, call-stack inspection in the REPL
  first (`:break`, `:step`, `:frame`, `:watch`); DAP adapter as a follow-up so VSCode gets
  the same via `operon-ls`.
- Done when: REPL subcommands work on a loaded file with `--entry main`; stepping respects
  Total Grammar (all rungs debuggable); fuel/caps state visible in `:frame`; docs +
  tutorial section.
- Files: `src/bin/` REPL (see `src/main.rs`), `src/interp.rs` (instrumentation hooks),
  `tests/repl.rs`, `TUTORIAL.md`.
- Depends: W007 (stack infrastructure).

### W009 — Bytecode VM [P1] [dev-1] [XL] [open] — **v3.0 flagship**
- Goal: source → lexer → parser → AST → **bytecode** → stack VM; tree-walker stays as the
  differential reference forever (the oracle discipline, applied internally).
- Done when: `operon run --vm` executes the full differential corpus (128/128) with
  byte-identical output to the tree-walker; fib25 speedup ≥ 2x measured by
  `scripts/bench.sh`; fuel/mem/caps contracts identical; fall-back flag if divergent.
- Files: new `src/bytecode.rs`, `src/vm.rs`, `src/main.rs` flag, `scripts/bench.sh`,
  `BENCH.md`, `SPEC.md` §VM, `tests/differential/`.
- Depends: W007/W006 (error model freeze). Design note first (one page in SPEC §VM),
  then compiler, then VM loop, then parity campaign. **Do not start JIT (W012) before this
  ships and is profiled.**

### W010 — Bytecode disassembler [P2] [dev-1] [M] [open]
- Goal: `operon compile app.op -o app.ob` + `operon disasm app.ob` printing annotated
  bytecode (op, operand, source span).
- Done when: round-trip dump is stable across runs (feeds W088); every opcode documented in
  SPEC §VM; tests assert dump stability.
- Files: `src/bytecode.rs`, `src/main.rs`, `SPEC.md`.
- Depends: W009.

### W011 — Optimization pipeline [P2] [dev-1] [L] [open]
- Goal: constant folding, dead-code elimination, constant propagation, trivial-gene inlining,
  monomorphic call specialization, builtin/global resolution caching, list-op fast paths.
- Done when: each optimization has a micro-benchmark delta (BENCH.md row) and a differential
  parity requirement; optimizations are individually toggleable (`--opt=none/fast/all`).
- Files: `src/bytecode.rs` (pass infra), `src/vm.rs`, `BENCH.md`.
- Depends: W009.

### W012 — JIT [P3] [dev-1] [XL] [deferred: audit orders VM → profiling → opt → JIT]
- Deliverable until un-deferred: one design paragraph in SPEC §VM (Cranelift vs hand-rolled
  option table) + the measurement plan that would justify it. Owner sign-off required to start.

### W013 — Memory-cycle strategy [P0] [dev-1] [L] [partial: D-013 decision recorded (document model + opt-in escape hatch, tracing GC rejected); SPEC §19 + memory_model.op on main 563a331; memory() cycle-count + weak refs remain]
- Already done (evidence): sec-r5 DAG-memoized `stringify`/`repr`/`deep_eq` kills quadratic
  walks; cycle-safe JSON; equality/repr safety proven.
- Remaining: pick reclamation strategy — (a) weak references API, (b) cycle collector at
  scope exit, (c) documented ownership ban + `break_cycle()` builtin. Recommend (a)+(c).
- Done when: decision recorded in DECISIONS.md; `memory()` reports live-cycle count;
  redteam payload rt for self-referencing containers leaks measurably less; SPEC §14
  documents the chosen model.
- Files: `src/value.rs`, `src/interp.rs`, `src/ffi.rs` (memory() tables), `SPEC.md`,
  `tests/redteam/`.

### W014 — Standardized memory model spec [P0] [dev-1] [M] [done: main 563a331 — SPEC §19 + differential memory_model.op]
- Goal: one SPEC section answering, for every value type: copy vs reference semantics of
  `let b = a`, container sharing, closure capture, thread transfer, cycle behavior,
  ownership/lifetime rules.
- Done when: SPEC §14 (new) states semantics per type with examples that are also
  differential corpus programs; README links it.
- Files: `SPEC.md`, `tests/differential/`.
- Depends: W013 decision (states the *current* truth even if reclamation defers).

### W015 — Channels + select [P1] [dev-1] [L] [Track L2a] [queued: builder-A]
- Goal: `channel()` primitive (buffered), `send/recv/close`, `select` over multiple channels,
  language-level, capability-gated like spawn.
- Done when: producer/consumer differential programs are deterministic under seeding;
  blocked-recv fuel accounting proven; redteam: channel leaks + close-while-recv contained;
  SPEC §16 (concurrency) rewritten.
- Files: `src/interp.rs`, `src/value.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### W016 — Async model [P2] [dev-1] [XL] [open]
- Goal: `async gene fetch() { await .. }` green-thread executor for HTTP/file/sleep; OS
  threads remain for CPU work.
- Done when: async HTTP + timers run N=1000 concurrent waits under thread counts ≈ cores;
  capability model unchanged; SPEC §16b; oracle note (oracle may serialize async — document).
- Files: `src/interp.rs`, `src/value.rs`, `SPEC.md`.
- Depends: W015. Do not start before channels land.

### W017 — Structured concurrency [P2] [dev-1] [M] [open]
- Goal: `scope { spawn t1; spawn t2 }` — tasks auto-join (or auto-cancel on Stress) at scope
  exit; un-joined spawn inside a scope becomes a `check` warning.
- Done when: scope semantics + cancellation propagation defined and tested; redteam: leaked
  task cannot outlive scope; SPEC §16.
- Files: `src/interp.rs`, `src/ast.rs`, `src/parser.rs`, `SPEC.md`.
- Depends: W018 (cancel semantics) recommended.

### W018 — General task cancellation [P2] [dev-1] [M] [partial]
- Already: repressilator machinery has cancellation. Remaining: `cancel(id)`, `is_done(id)`,
  `task_state(id)` for plain spawn tasks + defined propagation (cancel → child tasks).
- Done when: the three builtins exist, fuel-charged, capability-gated; differential programs;
  SPEC §16; oracle mirror.
- Files: `src/interp.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### W019 — Module system → package system [P1] [dev-1] [L] [Track L3c] [open]
- Goal: deterministic resolution for package projects: `operon.toml` declares deps (path +
  git + registry-stub), resolver builds the graph, `use pkg::mod` resolves through it.
- Done when: resolution algorithm pinned in SPEC §13 (feeds W069); path deps work end-to-end;
  git deps behind capability gate; diagnostics per W070; tests incl. cycle detection.
- Files: `src/main.rs` (loader), `src/interp.rs`, `operon.toml` support, `SPEC.md`, `tests/`.
- Depends: W022 (manifest format), W069 (resolution pin).

### W020 — Package manager CLI [P1] [dev-1] [L] [open]
- Goal: `operon init/add/remove/update/install/search/tree` operating on `operon.toml` +
  `operon.lock`.
- Done when: init+add+install+tree work for path deps offline (registry stub = local dir
  documented as OPERON_REGISTRY); every subcommand capability-gated and fuel-charged;
  README tutorial; tests.
- Files: `src/main.rs`, new `src/pkg.rs`, `SPEC.md`, `README.md`, `tests/`.
- Depends: W019, W022, W023.

### W021 — Central package registry [P3] [dev-1] [XL] [deferred: needs infra + owner decision]
- Deliverable until un-deferred: registry API sketch (SPEC §ecosystem) + local-dir stub
  contract consumed by W020. No hosted service.

### W022 — `operon.toml` manifest standard [P1] [dev-1] [M] [Track L3c] [open]
- Goal: package/project metadata manifest, **separate from `.cell`** (runtime config stays
  in `.cell` — audit item 22).
- Done when: schema (name, version, operon-version, deps, caps-profile, entry) defined in
  SPEC §13a; parser reuses W066 `.cell` schema validation machinery; unknown-key diagnostics.
- Files: `SPEC.md`, `src/pkg.rs`/`src/main.rs`, `tests/`.
- Coordinate: format co-designed with dev-3's W066 schema code to avoid two validators.

### W023 — Lockfile `operon.lock` [P2] [dev-1] [M] [open]
- Goal: resolved dependency versions + checksums; reproducible installs.
- Done when: lock written/updated by W020 commands; `operon install --frozen` fails on
  drift; format documented + versioned.
- Files: `src/pkg.rs`, `SPEC.md`, `tests/`.
- Depends: W020.

### W024 — Formal visibility model [P2] [dev-1] [M] [partial: anchors/exports exist]
- Goal: `pub/priv` per gene in modules; modules declare their public surface; `use` only
  binds public genes (privacy violation = semantic warning, hard under `--strict`).
- Done when: grammar + enforcement + SPEC §13b + oracle + tests; LSP respects visibility
  in completion (feeds W045).
- Files: `src/parser.rs`, `src/interp.rs`, `src/ls.rs`, `SPEC.md`, `tests/`.

### W025 — Dotted namespaces [P2] [dev-1] [M] [partial: use-as exists]
- Goal: `math.vector.add`-style nesting: modules may declare sub-modules; `use bio::seq::*`.
- Done when: nested module syntax + qualified calls resolve per W069 algorithm; std modules
  keep flat compat via generated re-export blocks; SPEC §13.
- Files: `src/parser.rs`, `src/interp.rs`, `std/*.op`, `SPEC.md`.

### W026 — Typed collections library [P2] [dev-1] [L] [open]
- Goal: `Set<T>/Deque<T>/Queue<T>/Stack<T>/Heap<T>/Graph<T>` as `std/collections.op`
  constructs (dynamic today) that gain W003 annotation sugar; no new Rust builtins
  (oracle lane stays closed for std work).
- Done when: each container has proofs in `tests/stdlib_selfhost.op` or `tests/std_*.op`,
  STDLIB.md rows, complexity notes.
- Files: `std/collections.op`, `tests/`, `STDLIB.md`.
- Depends: W003 for annotations; containers themselves can land untyped first.

### W027 — Stdlib breadth to mainstream [P1] [dev-1 + any dev] [L] [partial: 15 modules live]
- Already done (evidence): 15 modules `args bio collections csv fmt fs iter json math
  motifs random seq set strings testing` (loop-10 wave S landed set/testing/random).
- Remaining per audit: `path`, `process`, `env`, `logging`, `terminal`, `compression`,
  `hashing`, `url`, `http`-high-level, `walk`, `binary`, `unicode`, `encoding`, `db` (stub
  via py() bridge acceptable), plus the L1c leftover: **re_split verify-or-drop** (sz note
  in TASKS.md).
- Done when: each new module ships with proofs + STDLIB.md row + differential program;
  `.op`-only lane respected (no Rust builtins; py() bridge for the deep end per D-010-A).
- Files: `std/*.op`, `tests/std_*.op`, `STDLIB.md`.
- Note: `hashing`/`binary` unblock W029; `path` unblocks W019 ergonomics.

### W028 — Unicode depth for strings [P2] [dev-1] [L] [partial: unicode redteam payloads exist]
- Goal: normalization (NFC/NFD), grapheme segmentation, case folding, category queries;
  documented char-index semantics (byte vs char vs grapheme) for every string builtin.
- Done when: `std/unicode.op` (pure .op where feasible) or builtins with SPEC §10b;
  differential corpus incl. CJK/emoji combining cases; redteam: malformed UTF-8 containment
  already proven (rt_p7j) — extend with normalization storms.
- Files: `std/unicode.op` or `src/interp.rs`, `SPEC.md`, `tests/`.

### W029 — First-class bytes type [P2] [dev-1] [L] [open]
- Goal: `bytes` value (immutable buffer + builder), literals `b"..."`, indexing/slicing,
  conversions to/from str/list/numbers, std `hashing` consumes it (W027).
- Done when: value variant + builtins + SPEC §7; capability model: file read-bytes gated by
  read caps; redteam: 2GiB mem-charge contract holds for buffers (feeds off sec-r5 charges);
  oracle mirror.
- Files: `src/value.rs`, `src/interp.rs`, `src/lexer.rs`, `bootstrap/oracle.py`, `SPEC.md`.

### W030 — Raw / multiline / byte strings [P2] [dev-1] [S] [done: main 563a331 — r"..." + """...""" oracle-mirrored; b"..." → W029]
- Verify first: multiline strings + escape behavior are partially proven (rt_p6c/brescape);
  confirm what exists, then land `r"..."` raw + `b"..."` (with W029) + heredoc `'''...'''`
  if missing.
- Done when: lexer tests + SPEC §6 literals table + oracle mirror.
- Files: `src/lexer.rs`, `src/parser.rs`, `SPEC.md`, `tests/`.

### W031 — Numeric literal forms [P1] [dev-1] [S] [done: main 563a331 — 0x/0b/0o + _ separators, oracle-mirrored, SPEC §3]
- Goal: `0xFF`, `0b101010`, `0o755`, `1_000_000` (underscores in decimal+hex).
- Done when: lexer accepts forms; overflow behavior matches the existing saturate-to-0
  contract (f0fe2ec) with notes; oracle mirror; differential program.
- Files: `src/lexer.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### W032 — BigInt / Decimal [P3] [dev-1] [L] [open]
- Goal: arbitrary-precision ints behind an explicit value variant or std module; Decimal
  for money/science.
- Done when: decision recorded (builtin vs std via py()); if builtin: fuel/charge model,
  SPEC §7, differential programs (parity vs Python ints via oracle is a gift here — use it).
- Files: `src/value.rs`, `src/interp.rs` or `std/bigint.op`, `SPEC.md`.

### W033 — Date/time value types [P2] [dev-1] [M] [partial: L1d landed]
- Already: `unix_time/date_parts/date_fmt` (builder-A, interp.rs), monotonic clock/now
  pinned (SPEC §22).
- Remaining: `Duration` arithmetic, timezone handling contract (UTC-only v1, documented),
  parse ISO-8601.
- Done when: `std/time.op` extensions + proofs; SPEC §22 extended; differential time-shape
  corpus extended (never pin wall-clock absolute values — shape only).
- Files: `std/` (new time module or extensions), `tests/`, `SPEC.md`.

---
---

# DEV-2 BLOCK — W034–W065 (builder-B): tooling, truth, ecosystem

> builder-B board was complete and idle since 2026-09-24 — this block is your new mandate.
> Suggested order: **W054 → W056 → W057 → W058 → W055 → W053** (the truth sweep, one PR),
> then **W039 → W043 → W049** (quick tooling wins), then **W037 → W041 → W042**, then
> W051/W052, then W045/W047/W048, then the P3s.

### W034 — Serialization trait/interface [P2] [dev-2] [M] [partial: json/csv exist]
- Goal: one `Serialize` concept (trait per W004, or duck-typed convention until then) so
  JSON/CSV/custom formats share a contract.
- Done when: `to_json`/`from_json` honor the convention for phenotypes; docs table of
  default encodings per value type; tests incl. round-trips.
- Files: `std/json.op`/`std/csv.op`, `src/interp.rs` (builtin hooks), `SPEC.md`, `tests/`.
- Depends: W004 ideally; convention-only version can land first.

### W035 — Macros / metaprogramming [P3] [dev-2] [XL] [deferred: design only]
- Deliverable: design note (SPEC §meta): why macro-lite (`gene` templates or declarative
  `@rule` blocks) over full syntax macros; review against the bio-keyword pressure documented
  in W036. Owner sign-off required to implement.

### W036 — Hard core/bio boundary [P0] [dev-2 (spec) + dev-1 (grammar freeze)] [M] [open]
- Goal: formal split — **core language** (`gene/let/if/for/match/return/stress/modules/
  types/traits/concurrency`) vs **biology layer** (`regulate/splice/methylate/m6a/operon/
  repressilator/ligand/riboswitch/quorum/fate/...`). New biological mechanisms land as
  libraries or declarative `.cell`-style APIs by default, not parser keywords.
- Done when: SPEC §1b draws the boundary and lists every current keyword on its side; a
  new-contributor rule in CONTRIBUTING.md; the audit's design-risk note answered with a
  governance paragraph (what would ever justify a new keyword).
- Files: `SPEC.md`, `CONTRIBUTING.md`.
- Coordinate: dev-3's W091/W092 split the *semantics* docs; this level splits the *syntax
  surface*. Land together for one coherent story.

### W037 — Total Grammar semantic contract [P0] [dev-2] [L] [partial]
- Goal: precise rung hierarchy — **canonical → repairable syntax → recoverable syntax →
  semantic warning → hard semantic error** — with a documented rule for which layer each
  behavior belongs to; the "unknown identifier becomes string" soft-miss gets an explicit
  contract (warning + `--strict` hard error) instead of silent acceptance.
- Done when: SPEC §2 rewritten with the 5-rung table; every current repair classified;
  `--strict` audit: which warnings escalate; differential programs pinning each rung;
  README honest-summary updated.
- Files: `SPEC.md`, `src/tools.rs`, `src/main.rs`, `tests/`.
- Note: W001/W002/W006 build on this contract; land before them if possible.

### W038 — Repair explanation mode [P1] [dev-2] [M] [open]
- Goal: `operon explain file.op` shows original token → repaired token → reason → rung →
  resulting AST fragment; `operon fmt --show-repairs` for the diff view.
- Done when: both commands exist; every repair note the parser emits is machine-listable;
  tests; README recipe.
- Files: `src/main.rs`, `src/tools.rs`, `src/parser.rs` (note plumbing), `README.md`.
- Depends: W037 (rung taxonomy).

### W039 — AST dump [P1] [dev-2] [S] [open — verified missing]
- Goal: `operon ast file.op` prints the parsed AST (S-expression or JSON via `--json`).
- Done when: command exists for canonical AND repaired parses (`--show-repairs` flag shows
  both); helps W037/W038 evidence; tested.
- Files: `src/ast.rs` (Debug impls as needed), `src/main.rs`, `tests/`.

### W040 — IR / bytecode dump [P2] [dev-2] [S] [open]
- Goal: `operon bytecode file.op` dumps bytecode once W009 exists; until then the CLI stub
  returns a clear "VM not landed (M100 W009)" note instead of a silent unknown-command.
- Done when: stub + later real dump; documented in `operon --help`.
- Files: `src/main.rs`, `SPEC.md`.
- Depends: W009 for the real dump.

### W041 — `check` rework: diagnostics, not grades [P1] [dev-2] [M] [Track L3d] [open]
- Goal: separate streams: `error / warning / style / repair / security / performance`;
  keep the 100-point score only behind `--score` (nobody's default view); add `check --json`.
- Done when: output groups by severity with counts; CI consumes `--json`;
  README/docs updated; no invariant gates depend on the score number.
- Files: `src/tools.rs`, `src/main.rs`, `README.md`, `tests/`.

### W042 — Static analysis depth [P1] [dev-2] [L] [partial: phantom/wobble/NMD/anchor checks live]
- Goal: add: unreachable code, unused variables/genes/imports, shadowing, infinite-loop
  detection, constant conditions, dead stores, obvious type mismatches, duplicate match
  cases, unused capabilities, possible-null flow (synergy with W001).
- Done when: each check lands with a differential corpus case + a false-positive budget
  (zero tolerance on the existing corpus: current 18 cookbook + 128 differential must stay
  warning-clean unless the warning is genuinely warranted); `check --json` classification.
- Files: `src/tools.rs`, `tests/`, `SPEC.md`.

### W043 — Wrong-arity static detection [P1] [dev-2] [S] [open]
- Goal: calls to known genes with too-few/too-many args = `check` error before execution.
- Done when: arity table built from parse; direct calls checked; dynamic calls excluded
  with a documented escape hatch; tests.
- Files: `src/tools.rs`, `tests/`.

### W044 — LSP signature help + parameter docs [P2] [dev-2] [S] [partial: hover exists]
- Goal: `textDocument/signatureHelp` wired to gene signatures + doc comments (feeds from
  dev-3's W074); active-parameter highlight.
- Done when: trigger characters `,`/`(` produce signatures in Neovim/VSCode recipe;
  lsp_smoke assertions; README.
- Files: `src/ls.rs`, `tests/lsp_smoke.py`, `README.md`.
- Depends: W074 (doc comments) for parameter docs — can land hover-only first.

### W045 — LSP depth wave [P1] [dev-2] [L] [partial: 5/7 baseline wired (lsp-r1)]
- Goal: references, rename, workspace symbols, semantic tokens, folding ranges, selection
  ranges, code actions (quick-fix for known repairs), inlay hints (types per W001),
  document links for `use`.
- Done when: each feature behind the existing doc-cache architecture with incremental-sync
  guard; smoke assertions per feature; priority order: references → rename → workspace
  symbols → folding → semantic tokens.
- Files: `src/ls.rs`, `tests/lsp_smoke.py`, `README.md`.

### W046 — LSP understands repairs [P2] [dev-2] [M] [partial]
- Goal: editor-visible distinction between canonical code and repaired code: repair
  diagnostics carry the rung (W037), quick-fix code actions offer the canonical form.
- Done when: diagnostics carry rung metadata; a repair quick-fix lands with tests.
- Files: `src/ls.rs`, `src/tools.rs`, `tests/`.
- Depends: W037, synergizes W038.

### W047 — Formatter configuration [P2] [dev-2] [M] [Track L3a] [open]
- Goal: `.operon-fmt.toml` or `.cell`-adjacent config: indent width, line length, brace
  style, quote preference, import ordering, canonical mode, minimal-change mode.
- Done when: config parsed + honored; fmt idempotence tests with 3 config profiles;
  defaults unchanged (back-compat).
- Files: `src/tools.rs`, `tests/`, `SPEC.md`.

### W048 — `operon lint` separate from check [P2] [dev-2] [M] [open]
- Goal: `check` = language correctness (W041 contract); `lint` = code-quality rules,
  configurable, pluggable rule list in `SPEC §lint`.
- Done when: lint command with ≥8 rules migrated/extracted from check's style side;
  per-rule disable comments; docs.
- Files: `src/tools.rs`, `src/main.rs`, `SPEC.md`, `tests/`.
- Depends: W041.

### W049 — Test runner filtering + repeat [P1] [dev-2] [S] [open]
- Goal: `operon test path/to/file.op`, `--filter name`, `--failed` (uses last report),
  `--repeat N` (flakiness probe), `--shuffle-seed` for order independence.
- Done when: flags live, `--json` report extended, scripts/test.sh exposes common
  invocations, tests.
- Files: `src/tools.rs` (run_tests), `src/main.rs`, `scripts/test.sh`.

### W050 — Property-based testing [P2] [dev-2] [M] [partial: differential harness is the base]
- Goal: seeded random property programs: parser repair invariants (AST round-trip), JSON
  round-trip, regex vs oracle, GRN transition invariants (mass-balance/decay bounds),
  Total Grammar never-crash law.
- Done when: `scripts/property.py` (oracle-side generator) with fixed seeds produces N
  cases per property, all feeding harness.py; CI job (non-blocking first, promotion
  criteria stated); first real bug hunt documented if found.
- Files: `scripts/property.py`, `bootstrap/harness.py`, `.github/workflows/ci.yml`.

### W051 — Fuzzing infrastructure (S7) [P1] [dev-2] [L] [partial: redteam 95 payloads]
- Goal: continuous fuzzing of lexer/parser/fmt/JSON/regex/module loader/`.cell`:
  cargo-fuzz targets (libFuzzer) for the Rust core + `scripts/fuzz_op.py` AFL-style
  .op mutator for the semantic layer.
- Done when: 4+ cargo-fuzz targets run 10 min each in a weekly CI job + nightly smoke;
  crash → minimized testcase → redteam payload pipeline documented; every fuzz-discovered
  crash = redteam regression test forever.
- Files: `fuzz/` (cargo-fuzz crate), `.github/workflows/fuzz.yml`, `scripts/fuzz_op.py`,
  `tests/redteam/`.
- Note: this is the queued S7 lane — fold its status here.

### W052 — Coverage reporting [P2] [dev-2] [M] [open]
- Goal: Rust line/branch coverage (llvm-cov) + proof-coverage script (which parser
  productions / builtins / rungs the corpus touches) — the S3 builtin audit proved this
  pattern works (74 arms → 15 uncovered → 3 programs → real oracle bug).
- Done when: `scripts/coverage.sh` produces HTML + a text summary; CI job uploads artifact;
  coverage numbers quoted in BENCH.md-adjacent QUALITY.md (new).
- Files: `scripts/coverage.sh`, `.github/workflows/ci.yml`, `QUALITY.md` (new).

### W053 — Generated documentation numbers [P1] [dev-2] [S] [partial]
- Goal: every count quoted in docs (proof files/assertions, redteam payloads, keyword
  count, std modules, builtin count) comes from ONE generator: `scripts/gen_stats.py`
  writing `docs/STATS.md` + injecting into README/SPEC marked sections.
- Done when: generator exists; all stale-number sites (audit found 67/88/92/95 redteam and
  80-proof doc drift) now generated; CI fails on manual edits to marked sections
  (simple grep check).
- Files: `scripts/gen_stats.py`, `docs/STATS.md`, `README.md`, `SPEC.md`, `.github/workflows/ci.yml`.

### W054 — Versioning consistency [P0] [dev-2] [S] [verify]
- Verify: SPEC header said `v2.3.0-dev` while Cargo says 2.2.0 (loop-10 truth passes may
  have fixed this). Land `scripts/gen_stats.py` version section: ONE source (Cargo.toml)
  → README/SPEC/STDLIB badges.
- Done when: zero version mismatches via grep script in CI; SPEC carries only the
  generated version line.
- Files: `scripts/gen_stats.py`, `SPEC.md`, `README.md`, `.github/workflows/ci.yml`.

### W055 — Keyword count generated [P1] [dev-2] [S] [verify]
- Verify: SPEC said 51 reserved keywords, parser had 58 (audit). reg-r4 claims "SPEC
  inventory complete" — confirm, then fold into W053 generator so it can never drift again.
- Done when: keyword table in SPEC is generated from `src/lexer.rs` truth.
- Files: `scripts/gen_stats.py`, `SPEC.md`.

### W056 — README C-kernel truth [P0] [dev-2] [S] [verify]
- Verify: C runtime kernel was deleted (audit A15: `runtime/operon_rt.c` gone, codon
  kernel C++ remains). Sweep README/docs for any remaining "C runtime" claims.
- Done when: `rg -i "c runtime|c kernel" README.md docs/` shows only accurate statements;
  architecture diagram (if any) shows Rust core + C++ codon kernel + Python oracle.
- Files: `README.md`, `docs/*.html`.

### W057 — README stdlib truth [P1] [dev-2] [S] [verify]
- Verify: audit said README described 6 modules, 15 exist; R10-b did an "STDLIB truth
  pass" — confirm every module row matches reality (function names, counts).
- Done when: STDLIB.md generated (or validated) from `std/*.op` scans per W053; zero drift.
- Files: `STDLIB.md`, `scripts/gen_stats.py`.

### W058 — Build-dependency claim fix [P1] [dev-2] [S] [verify]
- Verify: README claims "no crates, no network, no external dependencies" but Cargo.toml
  uses `cc` as build-dep. Fix the sentence to the honest form: "runtime dependency surface
  = zero crates; build-time = `cc` only; network never required after bootstrap."
- Done when: claim matches reality in README + website copy.
- Files: `README.md`.

### W059 — Windows CI becomes blocking [P2] [dev-2] [S] [open]
- Goal: remove `continue-on-error: true` from the Windows job once it is stable for 14
  consecutive days (post hotfix r4). Until then, a tracking badge/comment in the workflow.
- Done when: flag removed, Windows failures block merges, note in COMMS.
- Files: `.github/workflows/ci.yml`.

### W060 — Per-target release smoke [P2] [dev-2] [M] [open]
- Goal: post-build smoke per release target where practical: linux x64/arm64 run hello.op
  in QEMU/container; macos runner smoke; windows smoke via the existing windows job artifact.
- Done when: release.yml runs smoke per produced asset (or documents why a target cannot be
  smoked); failed smoke = failed release.
- Files: `.github/workflows/release.yml`.

### W061 — Distribution channels [P2] [dev-2] [M] [partial: B5 landed binstall + brew draft + winget/scoop notes]
- Remaining: actual Homebrew tap formula PR (owner publishes), winget manifest submission,
  one Linux channel (deb or AUR) as proof, Nix flake optional.
- Done when: 2+ channels work end-to-end from public artifacts; README install table
  updated with verified commands.
- Files: `packaging/`, `README.md`.

### W062 — LSP version compatibility policy [P3] [dev-2] [S] [open]
- Goal: operon ↔ operon-ls ↔ editor-extension compatibility matrix; operon-ls reports the
  Operon version it was built with in initialize.
- Done when: initialize result carries version + capabilities; policy paragraph in
  CONTRIBUTING.md (LSP ships in-lockstep with the binary).
- Files: `src/ls.rs`, `CONTRIBUTING.md`.

### W063 — Language compatibility policy [P1] [dev-2] [M] [partial]
- Goal: written policy: 2.x runs all 2.x programs; Total Grammar repairs never change
  output within a minor; breaking changes require major; deprecations survive N releases
  (feeds W064); each release ships a compat changelog section.
- Done when: SPEC §compat (new) + CHANGELOG policy header; the v2.2→v2.3 Track L additions
  classified as additive (policy's first worked example).
- Files: `SPEC.md`, `CHANGELOG.md` (exists? verify — else CONTRIBUTING section).

### W064 — Deprecation system [P2] [dev-2] [M] [open]
- Goal: `@deprecated("use X instead", since="2.4")` marks on genes/keywords; emits check
  warnings with migration text; `--strict` fails; removal scheduled by W063 policy.
- Done when: mark infrastructure + first real deprecation (`const → let` synonym per W005);
  docs.
- Files: `src/parser.rs`, `src/tools.rs`, `SPEC.md`, `tests/`.

### W065 — Automated migrator [P3] [dev-2] [L] [deferred: needs W064 + fmt config]
- Deliverable until un-deferred: design note for `operon fix` (AST-based rewrites feeding
  off deprecation marks + fmt canonicalization). First candidate rewrite: synonym
  canonicalization (already half-exists in fmt canonical mode).

---
---

# DEV-3 BLOCK — W067–W100 (sz): contracts, safety, ecosystem surface, release engineering

> Suggested execution order: **W100 → W091 → W092 → W088/W089/W090 → W086 → W068 →
> W070 → W069 → W074 → W073 → W072 → W084 → W082 → W083 → W096 → W099 → W094**, then the
> remaining P2/P3s. Rationale: contract docs first (they unblock dev-1/2 decisions and
> survive even if sessions reset), then code in the order the audit's "fix first" implies
> for this range.

### W066 — `.cell` formal schema [P1] [dev-2 — CLAIMED by builder-B per canonical board; owner-arbitration pending] [M] [claimed]
> sz hand-off note: full implementation spec retained below for builder-B; the L022/W022
> manifest-validator reuse contract still holds — coordinate before landing.
- Goal: `.cell` gets a typed, versioned, documented schema: known keys with types/ranges/
  defaults, unknown-key diagnostics (warning, hard under `--strict`), `--cell-schema` dump,
  validation errors with spans.
- Done when: single `src/cell_schema.rs` (or section in tools) is the ONE source of truth
  for keys — every key the runtime reads today is declared there with type+default+doc;
  loader validates against it; unknown-key warning fires (differential program pins it);
  `operon --cell-schema` prints JSON schema; redteam: rt_p7g garbage-cell payload asserts
  containment unchanged; SPEC §8a documents the schema.
- Files: `src/main.rs` (loader), new `src/cell_schema.rs`, `src/tools.rs`, `SPEC.md`,
  `tests/redteam/`, `tests/differential/`.
- Coordinate: W022 (`operon.toml`) reuses this validator.

### W067 — `.rna` AST-based edit model [P2] [dev-3] [L] [done: stage 2 shipped on sz/m100-docs — node-addressed engine `src/rna2.rs` behind the same CLI, `syntax: v2` header dispatch (header-less = v1 byte-compatible), verbs rename/delete/body over paths gene[#ord]/splice/variant/phenotype/method/fate/regulate#N, all-or-nothing + ambiguity refusal + plain-comment preflight (—allow-comment-drop), reprint via format_program = fmt fixpoint, 14 pinned tests tests/rna_v2.rs, SPEC §rna + docs/design/RNA-V2.md as-built; stage 3 span deprecation remains future work per the migration path]
- Goal: today's `.rna` edits target source spans/text — fragile under reformatting. Future:
  parse → identify AST node → apply AST edit → reprint.
- Delivered: design note (RNA-V2.md) + the implementation: parse current source fresh,
  address declarations by name+ordinal, mutate the AST, reprint via the canonical
  formatter. Safety contract inherits W068 and tightens it (all-or-nothing, ordinal
  disambiguation, comment preflight, parse-first body replacement).

### W068 — `.rna` safety mode [P1] [dev-3] [S] [done: PR #18 — checked engine + CLI, 6 tests]
- Goal: `operon rna --check` (or the existing rna surface with `--check`): dry run that
  reports target found/not, replacement count, old→new text diff, affected gene, and exits
  non-zero if a target is missing (script-friendly).
- Done when: check mode exists and writes nothing; JSON output via `--json`; tests cover
  hit/miss/multi-hit/missing-target; README recipe.
- Files: `src/main.rs`, `src/tools.rs`, `tests/`, `README.md`.

### W069 — Module resolution algorithm pinned [P2] [dev-3] [S] [done: PR #20 — SPEC §8 6-root table + honest parity scope + LSP divergence note; oracle base_dir parity; differential mod_res.op pins roots 1+2 (harness 148/148)]
- Goal: the de-facto chain (doc-relative → CWD → std → OPERON_STD → exe-std →
  manifest-std) is SPEC-official with a decision table, and LSP + runtime + harness use
  the SAME order (lsp-r1 made LSP CWD-independent; runtime already exe-relative per dx-r5).
- Done when: SPEC §13 resolution table; a differential program that resolves from each
  root; divergence between LSP/runtime documented as allowed only where justified.
- Files: `SPEC.md`, `tests/differential/`, cross-check `src/ls.rs`.

### W070 — Import diagnostics detail [P2] [dev-3] [S] [done: PR #18 — attempted-roots detail, C-7 preserved]
- Goal: failed `use` reports: requested path, every attempted root in order, why each
  failed (missing / not-a-file / capability-denied / parse-error-in-target), resolved
  std root version.
- Done when: diagnostics format landed (Rust + oracle note-parity where applicable);
  differential program; redteam: no path leakage beyond policy (sandbox info-disclosure
  review).
- Files: `src/interp.rs` (loader), `bootstrap/oracle.py`, `tests/`.

### W071 — Hot reload [P3] [dev-3] [M] [deferred: design note docs/design/HOT-RELOAD.md landed (state-family analysis: caps/seed re-derivation, module-cache invalidation, GRN full-reset default with replay opt-in, GenomeLab sketch); W072 precondition met; implementation consciously postponed — child-per-run is strictly safer at W084's ~2 ms startup]
- Deliverable: design paragraph (watch-mode dependency, GRN state reset semantics, GenomeLab
  integration sketch) in `docs/design/HOT-RELOAD.md`. No implementation until W072 ships.

### W072 — `operon watch` [P2] [dev-3] [S] [done: PR #18 — mtime poll, fresh child per run]
- Goal: `operon watch app.op [-- args]` re-runs on mtime change (poll ≥200ms, no new
  deps), clears screen or prints separator, shows run duration + exit status; `--quiet`
  for CI-ish loops.
- Done when: command exists with `.cell`-aware file set (watches imports too); SIGINT
  clean exit; test via scripted file touches; README.
- Files: `src/main.rs`, `tests/`, `README.md`.

### W073 — `operon doc` generator [P1] [dev-3] [M] [Track L3b] [done: PR #20 — operon doc f.op|dir [-o dir] [--json]; docs/api/ generated for all 15 std modules; scripts/doc_api_check.sh regen-check (CI-wirable); std/json.op exemplar doc set; md+json tests]
- Goal: `operon doc file.op|dir` emits markdown: per gene — signature, doc comment (W074),
  marks/effects, capabilities required, examples found in doc comments; per module — index.
- Done when: `operon doc std/` renders every std module; output committed under
  `docs/api/` as a generated artifact (CI regen-check like W053); tests.
- Files: `src/main.rs`, `src/tools.rs` (parse reuse), `docs/api/` (generated), `CI`.

### W074 — Doc comments [P1] [dev-3] [M] [done: PR #20 — `##` marker (wobble-safe: side table, zero token-stream drift), hug-rule attachment incl. @marks + module doc, survives repair rungs + fmt byte-exact, LSP hover + REPL :doc, SPEC §3 contract (pure metadata, oracle parity not required); tests/docgen.rs 6 + differential neutrality pin]
- Goal: `///` line doc comments before `gene`/`phenotype`/module headers; parser captures
  them into AST metadata; hover shows them (LSP synergy with W044); `operon doc` consumes
  them (W073).
- Done when: grammar + capture + retention through repair rungs (repaired genes keep
  docs); REPL `:doc gene` prints them; oracle parity not required (docs don't affect
  semantics — note that in SPEC); differential program asserting doc-text survives fmt.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/ls.rs`, `src/main.rs`,
  `SPEC.md`, `tests/`.

### W075 — Example testing as first-class docs gate [P2] [dev-3] [S] [done: PR #20 — CONTRIBUTING §8a names the chain (doc example → executable example → test → rendered doc) with the three enforcement points: cookbook.sh byte-gate, apps proof gate, doc_api_check regen gate; no-copy-paste-rot rule + doc-comment scope rule]
- Goal: formalize the existing chain: doc example → executable example → test → rendered
  doc. Cookbook (18/18) + apps/ gate already implement most of it; name it, document it,
  and make `operon doc` examples runnable-checked.
- Done when: CONTRIBUTING.md documents the docs-gate contract; `scripts/cookbook.sh` + the
  apps gate + W073 examples check referenced as the three enforcement points.
- Files: `CONTRIBUTING.md`, `scripts/cookbook.sh`.

### W076 — Stable embedding API [P2] [dev-3] [M] [done: docs/EMBEDDING.md (both integration levels, version note per W063, honest NOT-exposed list) + examples/embed/ — external-style crate, own workspace/lockfile, path dep on operon, exercises core-level parse/eval + tool-level load_file/run_entry with default-deny caps + captured promote() sink — built+run by scripts/embed_example_check.sh as a blocking CI step (Linux); curated pub module tree in src/lib.rs (ast ffi genes graph interp lexer ls parser pybridge rna2 tools value, all documented); prelude module deliberately sequenced post-W001-type-settle per the guide's own versioning rule]
- Goal: documented public Rust API: parse/compile/run/sandbox-config/register-builtin/
  capture-output/inspect-diagnostics; semver'd releases; embedding example crate.
- Done when: `docs/EMBEDDING.md` + lib.rs pub-use surface curated + `examples/embed/`
  (a tiny dependent crate using operon as a path dep, built in CI); version note per W063.
- Files: `src/lib.rs`, `docs/EMBEDDING.md`, `examples/embed/`, `CI`.

### W077 — C ABI [P3] [dev-3] [XL] [deferred: design note docs/design/C-ABI.md landed — symbol surface sketch (operon_handle/run_source/notes/status/version), 4-line ownership contract, catch_unwind boundary, caps-struct 1:1 with NULL = default-deny, why-not-yet (Rust surface freeze + Native calling convention post-W004 + W009 crash boundary) + implementation order + FFI redteam plan]
- Deliverable: `docs/design/C-ABI.md` — symbol surface (operon_run_source/operon_free/
  diagnostic access), ownership rules, why-not-yet (embedding API first, W076).

### W078 — Python bridge explicitly optional [P2] [dev-3] [S] [done: PR #20 — verified default-off (caps fence fires BEFORE interpreter probe, catchable Stress interference); interpreter-absent = clean Stress missing, never a crash; tests/pybridge_off.op pins bridge-off parity; SPEC §15 optionality contract ('fully functional with zero Python present', oracle = toolchain dep not runtime dep)]
- Verify: substrate-r1 landed `py()` behind `--allow-py` + Caps.py grants (opt-in already).
  Confirm default-off behavior + graceful failure message when Python absent; document the
  optionality contract in SPEC §15 (bridge section).
- Done when: missing-Python produces a clean operational error (not a crash); docs state
  "Operon is fully functional with zero Python present"; test asserting bridge-off parity.
- Files: `src/interp.rs` (py bridge), `SPEC.md`, `tests/`.

### W079 — Python bridge version contracts [P2] [dev-3] [S] [done: PR #20 — bridge child reports version (py key, sys.version_info[:2]); interp warn-once per run below the 3.10 floor; SPEC §15b table (floor 3.10 / newer forward-as-is / no numpy-pandas guarantee beyond marshal rules / per-call spawn cost / Windows probe caveat)]
- Goal: documented contract: supported Python (≥3.10), what happens on newer/older, no
  NumPy/pandas guarantee (forward as-is statement), startup-cost note, Windows caveat.
- Done when: SPEC §15b contract table; runtime warns once when interpreter < documented
  floor; docs.
- Files: `SPEC.md`, `src/interp.rs`, `README.md`.

### W080 — Native C/C++ FFI [P3] [dev-3] [XL] [deferred: design note docs/design/NATIVE-FFI.md landed — foreign "c" grammar sketch (Total-Grammar: declaration is grammar, execution is capability), Caps.ffi per-library exact-match grants (py()-shaped), v1 marshalling table (values only, no pointers/callbacks), threat sequencing (py() subprocess → W077 core C-ABI → this, gated on W009 boundary + wasm callee evaluation), implementation order with fence-first landing]
- Deliverable: `docs/design/NATIVE-FFI.md` — `foreign` grammar sketch, capability gate
  (`--allow-ffi`), struct-layout story, why py() + C-ABI (W077) come first.

### W081 — C++ kernel boundary guard [done] [dev-3] [S]
- Evidence: sec-r5 verdict recorded — kernel "safe as-is, do not strengthen" (no signed
  shifts, fail-closed malloc, no ABI ownership, bounded DP); budget guard DP_CELL_BUDGET.
- Standing rule (new, this level): **no biological mechanism may move into the kernel
  without a benchmark proving ≥2x and a security re-audit** — add this line to
  CONTRIBUTING.md (done as part of W100's doc sweep).

### W082 — Performance regression gating [P1] [dev-3] [S] [done: PR #18 — perf_gate.py + perf.yml]
- Goal: bench CI job gains thresholds: any tracked benchmark regressing >20% vs the
  recorded baseline fails the job (noise-tolerant: median of 3 runs, ±5% band), turning
  the non-blocking job into a blocking gate on main.
- Done when: baseline JSON committed (`bench/baseline.json`, updated by script with
  machine-variance caveats), threshold logic in `scripts/bench_compare.py` (exists —
  extend), CI job blocking on main, one documented escape hatch (re-baseline PR).
- Files: `scripts/bench_compare.py`, `scripts/bench/baseline.json`, `.github/workflows/ci.yml`.

### W083 — Real-program benchmarks [P2] [dev-3] [M] [done: PR #20 — +6 suites: json round-trip, regex corpus, seq motifs (seeded LCG — reproducible), large_map, file_io (the one grant-needing bench; runner passes grants itself), modules (resolution-table exercise; native = import-machinery shape-compare, noted); BENCH.md rows + honesty notes; oracle-tractable sizing]
- Goal: add: JSON parse+serialize, regex corpus, sequence processing (motifs), module-heavy
  program, large-map workload, file-I/O micro (within caps), py()-bridge round-trip
  (non-blocking variant).
- Done when: each new bench has a deterministic workload, BENCH.md row, baseline entry;
  no bench depends on network.
- Files: `scripts/bench/*.op`, `scripts/bench.sh`, `BENCH.md`.

### W084 — Startup time benchmark [P1] [dev-3] [S] [done: PR #18 — exec 1.5ms / run-hello 1.7ms measured]
- Goal: measure binary cold start → hello output, and startup composition (lexer/parser/
  stdlib import) via `--iters` timing harness; tracked per release.
- Done when: `scripts/bench_startup.sh` (hyperfine if available, else manual loop) writes
  BENCH.md table; regression threshold folded into W082; VM (W009) inherits this as a
  non-regression gate.
- Files: `scripts/bench_startup.sh`, `BENCH.md`, `.github/workflows/ci.yml`.

### W085 — Standalone executable compilation [P2] [dev-3] [L] [done-as-specced: the level's 'deliverable now' = roadmap + honesty — docs/spec/BUILD-CONTRACT.md §4 carries the 3-stage roadmap (bytecode bundle post-W009 → Rust-embed runtime → native codegen only if W011 demands), README/`build --help` make no standalone-executable promise ('bake splices, strip proofs'); true standalone binaries are stage 2/3, correctly gated on W009 by design]
- Goal: `operon build app.op -o app` eventually produces a truly standalone binary.
  Roadmap: (1) bytecode bundle + tiny interpreter entry (post-W009), (2) Rust embed of
  bundle into a released `operon` runtime, (3) native codegen only if W011 profiling
  demands it.
- Deliverable now: roadmap section in SPEC §build + W086 contract; no false promises in
  README (audit's complaint).
- Files: `SPEC.md`, `README.md`.

### W086 — `operon build` contract [P1] [dev-3] [S] [done: PR #17 — BUILD-CONTRACT.md]
- Goal: honest documented contract of what build does TODAY (source specialization/baking):
  inputs, outputs, what is NOT guaranteed (no VM bytecode, no native exe yet), stability
  of baked output.
- Done when: SPEC §build + `operon build --help` text state the contract; tests pin
  baked-output shape (feeds W088 determinism).
- Files: `SPEC.md`, `src/main.rs`, `tests/`.

### W087 — Single-file bundle [P3] [dev-3] [M] [done: design note docs/design/BUNDLE.md on sz/m100-docs — .opb text envelope (meta/cell/modules sections, source_hash per module, byte-identical-under-W088 by construction), v1 source payload + std-resolved-at-run + first-party-only closure, loader contract (dev-2 lane: load_file extension + explicit-cell-with-allow-refusal), CLI surface incl. bundle --check; payload:bytecode plugs in at W009, external-dep closure at W023 — the envelope is the fixed target both waves build against]
- Deliverable: `docs/design/BUNDLE.md` — `operon bundle app.op` artifact layout (source +
  bytecode + stdlib deps + metadata), why it waits for W023/W009.

### W088 — Reproducible build contract [P2] [dev-3] [S] [done: PR #17 — DETERMINISM.md §6]
- Goal: define reproducibility scope: same operon version + same source + same seed ⇒ same
  output bytes for `operon build` and `operon bundle` (future); document what is excluded
  (paths, timestamps) and how they are normalized.
- Done when: `docs/spec/DETERMINISM.md` §builds (this work); build normalization test
  (build twice, byte-compare) in CI or scripts.
- Files: `docs/spec/DETERMINISM.md`, `scripts/`, `tests/`.

### W089 — Randomness determinism model [P1] [dev-3] [S] [done: PR #17 — DETERMINISM.md §4]
- Goal: document what determinism means: same source + same seed + same operon version ⇒
  same random stream, single-thread AND multi-thread (worker pinning landed: worker_seed_pin
  test, F-1m pin preference); cross-platform stream identity promise or explicit
  non-promise.
- Done when: DETERMINISM.md §randomness with the exact stream contract + evidence links
  (tests/worker_seed_pin.op, std/random.op wrappers); SPEC §22 cross-link.
- Files: `docs/spec/DETERMINISM.md`, `SPEC.md`.

### W090 — Floating-point determinism [P2] [dev-3] [S] [done: PR #17 — DETERMINISM.md §5]
- Goal: define whether bit-identical f64 results across platforms are promised: compiler
  flags (no fast-math), operation order, transcendental policy (libm variance disclaimer),
  JSON float formatting policy.
- Done when: DETERMINISM.md §floats; differential corpus runs on x86_64 + arm64 in CI
  already (release targets) — add explicit float-parity assertion program.
- Files: `docs/spec/DETERMINISM.md`, `tests/differential/`.

### W091 — Bio semantics separation [P1] [dev-3] [M] [blocked: SPEC lane — after builder-B W54 lands]
- Goal: SPEC currently interleaves language semantics with biological modeling semantics.
  Split: language sections state syntax/evaluation ONLY; bio modeling moves to a dedicated
  volume (docs/spec/BIO-MODEL.md) referenced from SPEC §11.
- Done when: SPEC §11 is a pointer + core evaluation rules only; BIO-MODEL.md holds the
  mechanism math; no information lost (diff-audit by sz); future bio changes cannot be
  breaking language changes by construction.
- Files: `SPEC.md`, `docs/spec/BIO-MODEL.md` (new).
- Coordinate: W036 (syntax boundary, dev-2) + W092 (modeling contract, below) land as one
  coherent doc wave.

### W092 — Biological modeling contract [P1] [dev-3] [M] [done: PR #17 — BIO-CONTRACT.md]
- Goal: every biological mechanism labeled honestly: **real mechanism / mathematical
  approximation / Operon-specific abstraction / fictional simplification**, with what the
  simulation output does and does NOT mean.
- Done when: BIO-CONTRACT.md table covering all live mechanisms (regulation, methylation,
  m6A+readers, RISC, IRES, riboswitch cis, operon polycistronic, quorum, repressilator,
  ligand, attenuation, decoy, enhancer, bursting, CRISPR, Rho, telegraph promoter,
  allostery/titration/decay-clock from reg-bio-2); GenomeLab help references it.
- Files: `docs/spec/BIO-CONTRACT.md` (new), `apps/genomelab/genomelab.op`, `README.md`.

### W093 — Scientific validation layer [P2] [dev-3] [M] [done: docs/spec/VALIDATION.md registry live — V1 EC50/Hill curve (tests/sci_ec50_hill.op, threshold-is-the-EC50 identity + canonical 10-90 points at 1e-12, op-order exact) and V2 repressilator period (tests/sci_repressi_period.op, peak-to-peak = 6 ticks vs the Elowitz-Leibler 2000 discrete parameterization, peak-count-in-window pinning) landed on sz/m100-docs; V3-V8 rows registry-link the existing repressi/trp/riboswitch/silence/occupy/copies tests; std approx_eq helper deliberately left to the dev-2 std lane to avoid the open tooling PR]
- Delivered: the registry (each validated model → source, mapping, tolerance,
  enforcing test), two new literature-anchored reproduction tests, and the
  follow-up list (approx_eq helper, two-tier/quorum/Rho dose curves).
- Files: `docs/spec/VALIDATION.md`, `tests/sci_ec50_hill.op`, `tests/sci_repressi_period.op`.

### W094 — Graph visualization export [P2] [dev-3] [M] [done: PR #18 — src/graph.rs, DOT+JSON, 2 tests]
- Goal: `operon graph file.op --format dot|json|svg-stub` exporting the GRN/regulation
  graph: nodes (genes, levels), edges (activation/inhibition, cooperativity), modifiers
  (methylation/m6A marks).
- Done when: DOT output renders in graphviz (golden-file test); JSON output schema
  documented; capability-neutral (pure static analysis, no run required — or run-mode with
  final levels); README recipe.
- Files: `src/main.rs`, new `src/graph.rs`, `tests/`, `README.md`.

### W095 — Live regulation visualizer [P3] [dev-3] [L] [done: tick-stream shipped on sz/m100-docs — `operon run f.op --trace-grn trace.jsonl` emits one JSONL frame per engine update point (grn_fire pulse / decay-clock tick, both funneled through trans_integrate) {"tick":N,"phase":"fire"|"decay","levels":{byte-sorted map, 6-dp}}; deterministic (W089 discipline), 200k-frame cap, interpreter performs no I/O (CLI drains after run, success or contained failure); 4 pinned tests tests/grn_trace.rs; SPEC CLI block documented; GenomeLab UI wiring remains open]
- Delivered: the time-series half of W094's JSON — a runtime tick-stream (frames) a
  visualizer replays or consumes live. GenomeLab timeline integration is a UI wave on top.

### W096 — Profiler output formats [P2] [dev-3] [S] [partial: --json landed (PR #18); Chrome-trace blocked on per-call spans (dev-1 lane)]
- Goal: `operon profile --json` (self-describing: units, version, run metadata) +
  Chrome-trace format (`.json` events) so about://tracing / perfetto render it; flamegraph
  text format optional.
- Done when: both formats emitted and schema-documented; BENCH.md/README recipes;
  golden-file test.
- Files: `src/tools.rs` (profiler), `src/main.rs`, `SPEC.md`, `tests/`.

### W097 — Memory profiler [P3] [dev-3] [M] [partial: memory() accounting PINNED in SPEC §10 (arena_bytes/interns/allocs = symbol-table gauge, honest limits stated); MEM-PROFILER.md design sketch (counting allocator, attribution windows, per-gene-by-extent); per-gene attribution deferred post-W009 by design]
- Deliverable now: document `memory()`'s exact accounting (interns/bytes/allocs semantics
  from ffi.rs) in SPEC §10; hotspot/peak/per-gene allocation deferred with design sketch.
- Files: `SPEC.md`, `docs/design/MEM-PROFILER.md` (sketch).

### W098 — Thread profiler [P3] [dev-3] [M] [partial: promoter_telemetry (F-3) + spawn/join semantics documented; WORKER-TELEMETRY.md design note (workers() sketch, differential shape-pin plan, timing values honestly unpinnable); workers() builtin deferred — full parity wave in dev-1's lane]
- Deliverable now: expose worker telemetry (spawn/join/attempt counts, lifetimes) via a
  `workers()` introspection builtin or profile extension; document; blocked-time +
  task-fuel reporting deferred.
- Files: `src/interp.rs`, `SPEC.md`, `tests/`.

### W099 — Security audit automation [P1] [dev-3] [M] [done: PR #18 — advisories + sweep landed; floating-ref finding filed]
- Goal: add: `cargo-deny`/`cargo-audit` job (Rust advisories), dependency diff review
  note in CONTRIBUTING, periodic sandbox-regression runner (the live TOCTOU flipper
  pattern → scripted), resource-exhaustion smoke in CI, Windows/Unicode path probes on
  schedule (not per-PR), fuzz smoke tie-in to W051.
- Done when: advisory job blocking (with documented unblock procedure), sandbox regression
  script `scripts/sec_regression.sh` wired weekly, all existing sec test files enumerated
  as the regression manifest.
- Files: `.github/workflows/security.yml` (new), `scripts/sec_regression.sh`,
  `CONTRIBUTING.md`.

### W100 — Formal threat model [P0] [dev-3] [M] [done: PR #17 — THREAT-MODEL.md]
- Goal: ONE document answering: what Operon protects against, for each untrusted input —
  malicious source, `.cell`, `.rna`, module, dependency, Python package (py()), network
  peer (http), local filesystem, resource exhaustion, data exfiltration.
- Done when: `THREAT-MODEL.md` maps: assets → adversaries → attack surface → defense →
  evidence (test/redteam payload) → residual risk; every existing defense
  (caps default-deny, fuel/mem charges, fd open-then-verify, path canon, JSON charges,
  spawn caps, py() grants, output caps) appears exactly once with its evidence link;
  gaps honestly listed with M100 level references; CONTRIBUTING links it as §security.
- Files: `THREAT-MODEL.md` (new), `CONTRIBUTING.md`, `README.md` link.

---
---

# PROGRAM-LEVEL TRACKING

## Sprint plan (rolling, updated per session)

| wave | content | dev | status |
|------|---------|-----|--------|
| W1 | truth sweep W053/W054/W055/W056/W057/W058 (one PR) | dev-2 | queued |
| W1 | contract wave: W100 + W092 + DETERMINISM (W088/89/90) + W086 — 5/6 landed; W091 blocked on SPEC lane | dev-3 | **done (PR #17)** |
| W1 | error model: W007 (tracebacks) | dev-1 | queued (loop-11 compatible) |
| W2 | W068 + W070 + W069 | dev-3 | **done (PRs #18, #20)** |
| W2 | W039 + W043 + W049 | dev-2 | queued |
| W2 | W006 (Option/Result) | dev-1 | queued |
| W3 | W074 + W073 + W072 + W084 | dev-3 | **done (PRs #18, #20 — W074/W073 landed post-W007-merge)** |
| W3 | W041 + W042 + W037 | dev-2 | queued |
| W3 | W001/W002 (Track L2c/L2b) | dev-1 | queued |
| W4 | W082 + W083 + W096 + W099 | dev-3 | W082 + W083 + W099 done (PRs #18, #20); W096 partial (--json landed; Chrome-trace awaits dev-1 per-call spans) |
| W4 | W009 VM design → compiler | dev-1 | gated on W007/W006 |
| W4b | W094 graph export | dev-3 | done (PR #18) |
| W5+ | remaining P2/P3 per suggested orders | all | rolling |

## Rules for evolving this document
1. Status changes go in the SAME commit as the work (or immediately after merge), with the
   evidence link (PR number or commit) inside the level's entry.
2. New audit findings become L101+ appended at the end, never renumber existing levels —
   external references (COMMS, PRs) cite level IDs.
3. No level is ever deleted; rejected work gets `[wontfix]` + a DECISIONS.md entry.
4. The dev-repo copy (`TODO-100.md`) is canonical for code-linked work; the vault copy
   (`collab/TODO-100.md`) is the cross-AI coordination mirror. Sync both in one session.
5. If the environment resets: re-clone both repos, read this file + `collab/COMMS.md`
   tail + `/home/z/my-project/worklog.md` (if present), rebuild toolchain per
   `collab/guide.md`, continue from the sprint table above.
