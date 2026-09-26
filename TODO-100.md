# OPERON M100 — MASTER 100-LEVEL TODO

v1.0.0 · 2026-09-26 · owner directive: *"make a super detailed hundred level to do list ·
you are dev 3 so build the 66-100 part · other 2 build the others · save EVERYTHING in the
repo so we survive terrible environment resets"*

Source of the 100 levels: owner's external audit of the v2.2.0 ZIP (2026-09-26, 100 numbered
findings), cross-checked against repo ground truth at `dd76caa` (loop-10 R10-c). Several
findings were already closed by loop-1..10 work; each level below records the evidence-based
status so nobody re-does closed work.

Everything in this file is committed to the repo on purpose: **this document is the
survival artifact**. If an environment resets, the M100 board plus `collab/` in
`WasewaseX/project-vault` reconstructs 100% of program state.

---

## OWNERSHIP (fixed by owner directive)

| dev | identity | branch namespace | levels | lane defaults |
|-----|-------------------|------------------|------------|---------------|
| dev-1 | builder-A | `builder/*` | **L001–L033** | `src/` core language (lexer/parser/interp/ast), SPEC language sections, VM |
| dev-2 | builder-B | `b2/*` | **L034–L065** | tooling (`src/tools.rs`, LSP, CLI auxiliary), docs truth passes, CI workflows, `std/*.op` |
| dev-3 | sz | `sz/*` | **L066–L100** | contract/spec docs (THREAT-MODEL, BIO-CONTRACT, DETERMINISM, BUILD), `.cell` schema, `.rna` safety, doc generator, watch, graph export, release engineering, security automation |

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

## CURRENT GATE NUMBERS (2026-09-26, main @ dd76caa)

differential **128/128 MATCH** · proofs **99 files / 92 proofs / 1171 assertions** ·
redteam **95 payloads / 0 breaches** · cargo test green (incl. 5 REPL contracts) ·
clippy 0 · fmt clean · cookbook **18/18** · LSP smoke OK · CI success.

> These numbers are re-measured every loop; when they change, update this header in the
> same commit that lands work. If this header is stale, the per-level evidence links win.

## TRACK L CROSS-LINK (MASTER-PLAN §7 folds into M100)

| Track L | M100 level(s) | status |
|---------|---------------|--------|
| L1a null-safety/destructuring/builtins | done (PR #15) | closed |
| L1b stdlib breadth | L027 | wip (builder-A, loop-10 wave S) |
| L1c regex subset | done | closed (re_split verify-or-drop: see L027 note) |
| L1d wall-clock time | L033 | done (unix_time/date_parts/date_fmt) |
| L2a channels | L015 | queued (builder-A) |
| L2b match patterns v2 | L002 | queued (builder-A) |
| L2c type annotations | L001 | queued (builder-A) |
| L2d operator overloading | L004 (partial) | open |
| L2e pipeline `\|>` | — | blocked on sz grammar verdict |
| L3a fmt config | L047 | open |
| L3b doc | L073/L074 | **dev-3** |
| L3c manifest | L019/L022/L023 | open |
| L3d check --json | L041 | open |
| L3e REPL v2 | L008 (partial) | open |

---
---

# DEV-1 BLOCK — L001–L033 (builder-A): language core

> Suggested execution order within the block (P0/P1 first): **L007 → L006 → L013 → L014 →
> L001 → L002 → L003 → L004 → L005 → L009 → L031 → L015**, then the P2s. Rationale: the
> audit's own "fix first" list puts the error model and memory contract ahead of new syntax.
> The VM (L009) starts only after L007/L006 land so the bytecode design targets the final
> error model, not a moving one.

### L001 — Optional static type annotations [P1] [dev-1] [XL] [Track L2c] [open]
- Goal: `gene add(a: int, b: int) -> int` parses and soft-checks; annotations are optional
  everywhere (gradual typing); unknown/unexpected types become semantic **warnings**, never
  rejections (Total Grammar preserved).
- Done when: grammar accepts annotations on genes, `let`, phenotype fields; `check` emits
  `type-mismatch` diagnostics with spans; SPEC §5 documents the annotation surface; oracle
  mirrors the parser; ≥4 differential corpus programs exercise annotation paths.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/differential/`.
- Depends: none. Blocks L003/L026 typing story.

### L002 — Algebraic data types + match v2 [P1] [dev-1] [XL] [Track L2b] [open]
- Goal: match gains or-patterns, struct/map patterns, nested patterns, guards in every arm,
  and variant-style payloads; Option/Result-shaped matching becomes idiomatic once L006 lands.
- Done when: `match x { 1 | 2 => .., [a, rest] => .., {k: v} if v > 0 => .. }` parses and
  evaluates; unreachable-arm detection added to `check` (feeds L042); SPEC §5 patterns
  section; oracle parity; differential programs.
- Files: `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/`.
- Depends: none (builder-A already claimed via L2b).

### L003 — Generics [P1] [dev-1] [XL] [open]
- Goal: generic genes `gene map<T, U>(xs: List<T>, f: gene(T) -> U)` with monomorphized
  execution (no runtime cost), typed containers `List<T>`, `Map<K,V>` as annotation sugar
  over existing dynamic containers (soft-checked per L001).
- Done when: parse+soft-check of generic signatures; monomorphic call specialization in the
  interpreter; ≥3 std functions gain typed wrappers; SPEC section; oracle mirror.
- Files: `src/parser.rs`, `src/ast.rs`, `src/interp.rs`, `std/collections.op`,
  `bootstrap/oracle.py`, `SPEC.md`.
- Depends: L001. Phased: (a) generic genes, (b) container annotations, (c) trait bounds (L004).

### L004 — Traits / interfaces [P1] [dev-1] [XL] [open]
- Goal: capability-oriented abstraction alongside phenotypes: `trait Show { gene show() }`,
  `phenotype User implements Show`. First four std traits: `Show`, `Eq`, `Serialize`
  (feeds L034), `Iterate`.
- Done when: trait declaration + implementation + dynamic dispatch work; conflict =
  semantic warning with spans; phenotype method resolution checks traits first, inheritance
  second; SPEC §phenotypes extended; oracle mirror.
- Files: `src/ast.rs`, `src/parser.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `std/*.op`.
- Depends: L001 recommended. L2d (operator overloading) is a special case of this.

### L005 — Immutability split: let / const / mut [P2] [dev-1] [M] [open]
- Goal: `let` = single-assignment binding (today's `let`), `const` = compile-time constant
  with literal-fold guarantee, `mut` re-binding for mutable containers; today's
  `const → let` synonym gets a deprecation note (feeds L064).
- Done when: grammar accepts the triad; `check` warns on mutation of non-`mut` containers;
  SPEC §6 rewritten; compatibility note in README; oracle mirror.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/interp.rs`, `src/tools.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `README.md`.
- Depends: none. Coordinate with L064 (deprecation machinery, dev-2).

### L006 — First-class Option / Result [P0] [dev-1] [L] [open]
- Goal: `Option<T>` / `Result<T, E>` as built-in variant values with `unwrap/unwrap_or/
  is_ok/is_err/?`-style propagation operator; Stress becomes purely the runtime containment
  mechanism (its current dual role as everyday error value ends).
- Done when: constructors + matchability via L002 patterns; `?`-propagation inside genes;
  std functions that today return `null` on failure gain documented Result returns behind a
  compatibility note; SPEC §9 (error model) rewritten around the hierarchy
  null → Result → Stress → termination; oracle mirror; differential programs.
- Files: `src/value.rs`, `src/interp.rs`, `src/parser.rs`, `src/ast.rs`,
  `bootstrap/oracle.py`, `SPEC.md`, `tests/`.
- Depends: L002 (pattern payloads) strongly recommended; may start in parallel.

### L007 — Real error tracebacks [P0] [dev-1] [M] [open]
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

### L008 — Debugger (REPL v2 stepping + DAP later) [P2] [dev-1] [XL] [open]
- Goal: breakpoints, step over/into/out, locals, watch, call-stack inspection in the REPL
  first (`:break`, `:step`, `:frame`, `:watch`); DAP adapter as a follow-up so VSCode gets
  the same via `operon-ls`.
- Done when: REPL subcommands work on a loaded file with `--entry main`; stepping respects
  Total Grammar (all rungs debuggable); fuel/caps state visible in `:frame`; docs +
  tutorial section.
- Files: `src/bin/` REPL (see `src/main.rs`), `src/interp.rs` (instrumentation hooks),
  `tests/repl.rs`, `TUTORIAL.md`.
- Depends: L007 (stack infrastructure).

### L009 — Bytecode VM [P1] [dev-1] [XL] [open] — **v3.0 flagship**
- Goal: source → lexer → parser → AST → **bytecode** → stack VM; tree-walker stays as the
  differential reference forever (the oracle discipline, applied internally).
- Done when: `operon run --vm` executes the full differential corpus (128/128) with
  byte-identical output to the tree-walker; fib25 speedup ≥ 2x measured by
  `scripts/bench.sh`; fuel/mem/caps contracts identical; fall-back flag if divergent.
- Files: new `src/bytecode.rs`, `src/vm.rs`, `src/main.rs` flag, `scripts/bench.sh`,
  `BENCH.md`, `SPEC.md` §VM, `tests/differential/`.
- Depends: L007/L006 (error model freeze). Design note first (one page in SPEC §VM),
  then compiler, then VM loop, then parity campaign. **Do not start JIT (L012) before this
  ships and is profiled.**

### L010 — Bytecode disassembler [P2] [dev-1] [M] [open]
- Goal: `operon compile app.op -o app.ob` + `operon disasm app.ob` printing annotated
  bytecode (op, operand, source span).
- Done when: round-trip dump is stable across runs (feeds L088); every opcode documented in
  SPEC §VM; tests assert dump stability.
- Files: `src/bytecode.rs`, `src/main.rs`, `SPEC.md`.
- Depends: L009.

### L011 — Optimization pipeline [P2] [dev-1] [L] [open]
- Goal: constant folding, dead-code elimination, constant propagation, trivial-gene inlining,
  monomorphic call specialization, builtin/global resolution caching, list-op fast paths.
- Done when: each optimization has a micro-benchmark delta (BENCH.md row) and a differential
  parity requirement; optimizations are individually toggleable (`--opt=none/fast/all`).
- Files: `src/bytecode.rs` (pass infra), `src/vm.rs`, `BENCH.md`.
- Depends: L009.

### L012 — JIT [P3] [dev-1] [XL] [deferred: audit orders VM → profiling → opt → JIT]
- Deliverable until un-deferred: one design paragraph in SPEC §VM (Cranelift vs hand-rolled
  option table) + the measurement plan that would justify it. Owner sign-off required to start.

### L013 — Memory-cycle strategy [P0] [dev-1] [L] [partial: walk-guards landed, reclamation open]
- Already done (evidence): sec-r5 DAG-memoized `stringify`/`repr`/`deep_eq` kills quadratic
  walks; cycle-safe JSON; equality/repr safety proven.
- Remaining: pick reclamation strategy — (a) weak references API, (b) cycle collector at
  scope exit, (c) documented ownership ban + `break_cycle()` builtin. Recommend (a)+(c).
- Done when: decision recorded in DECISIONS.md; `memory()` reports live-cycle count;
  redteam payload rt for self-referencing containers leaks measurably less; SPEC §14
  documents the chosen model.
- Files: `src/value.rs`, `src/interp.rs`, `src/ffi.rs` (memory() tables), `SPEC.md`,
  `tests/redteam/`.

### L014 — Standardized memory model spec [P0] [dev-1] [M] [partial]
- Goal: one SPEC section answering, for every value type: copy vs reference semantics of
  `let b = a`, container sharing, closure capture, thread transfer, cycle behavior,
  ownership/lifetime rules.
- Done when: SPEC §14 (new) states semantics per type with examples that are also
  differential corpus programs; README links it.
- Files: `SPEC.md`, `tests/differential/`.
- Depends: L013 decision (states the *current* truth even if reclamation defers).

### L015 — Channels + select [P1] [dev-1] [L] [Track L2a] [queued: builder-A]
- Goal: `channel()` primitive (buffered), `send/recv/close`, `select` over multiple channels,
  language-level, capability-gated like spawn.
- Done when: producer/consumer differential programs are deterministic under seeding;
  blocked-recv fuel accounting proven; redteam: channel leaks + close-while-recv contained;
  SPEC §16 (concurrency) rewritten.
- Files: `src/interp.rs`, `src/value.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### L016 — Async model [P2] [dev-1] [XL] [open]
- Goal: `async gene fetch() { await .. }` green-thread executor for HTTP/file/sleep; OS
  threads remain for CPU work.
- Done when: async HTTP + timers run N=1000 concurrent waits under thread counts ≈ cores;
  capability model unchanged; SPEC §16b; oracle note (oracle may serialize async — document).
- Files: `src/interp.rs`, `src/value.rs`, `SPEC.md`.
- Depends: L015. Do not start before channels land.

### L017 — Structured concurrency [P2] [dev-1] [M] [open]
- Goal: `scope { spawn t1; spawn t2 }` — tasks auto-join (or auto-cancel on Stress) at scope
  exit; un-joined spawn inside a scope becomes a `check` warning.
- Done when: scope semantics + cancellation propagation defined and tested; redteam: leaked
  task cannot outlive scope; SPEC §16.
- Files: `src/interp.rs`, `src/ast.rs`, `src/parser.rs`, `SPEC.md`.
- Depends: L018 (cancel semantics) recommended.

### L018 — General task cancellation [P2] [dev-1] [M] [partial]
- Already: repressilator machinery has cancellation. Remaining: `cancel(id)`, `is_done(id)`,
  `task_state(id)` for plain spawn tasks + defined propagation (cancel → child tasks).
- Done when: the three builtins exist, fuel-charged, capability-gated; differential programs;
  SPEC §16; oracle mirror.
- Files: `src/interp.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### L019 — Module system → package system [P1] [dev-1] [L] [Track L3c] [open]
- Goal: deterministic resolution for package projects: `operon.toml` declares deps (path +
  git + registry-stub), resolver builds the graph, `use pkg::mod` resolves through it.
- Done when: resolution algorithm pinned in SPEC §13 (feeds L069); path deps work end-to-end;
  git deps behind capability gate; diagnostics per L070; tests incl. cycle detection.
- Files: `src/main.rs` (loader), `src/interp.rs`, `operon.toml` support, `SPEC.md`, `tests/`.
- Depends: L022 (manifest format), L069 (resolution pin).

### L020 — Package manager CLI [P1] [dev-1] [L] [open]
- Goal: `operon init/add/remove/update/install/search/tree` operating on `operon.toml` +
  `operon.lock`.
- Done when: init+add+install+tree work for path deps offline (registry stub = local dir
  documented as OPERON_REGISTRY); every subcommand capability-gated and fuel-charged;
  README tutorial; tests.
- Files: `src/main.rs`, new `src/pkg.rs`, `SPEC.md`, `README.md`, `tests/`.
- Depends: L019, L022, L023.

### L021 — Central package registry [P3] [dev-1] [XL] [deferred: needs infra + owner decision]
- Deliverable until un-deferred: registry API sketch (SPEC §ecosystem) + local-dir stub
  contract consumed by L020. No hosted service.

### L022 — `operon.toml` manifest standard [P1] [dev-1] [M] [Track L3c] [open]
- Goal: package/project metadata manifest, **separate from `.cell`** (runtime config stays
  in `.cell` — audit item 22).
- Done when: schema (name, version, operon-version, deps, caps-profile, entry) defined in
  SPEC §13a; parser reuses L066 `.cell` schema validation machinery; unknown-key diagnostics.
- Files: `SPEC.md`, `src/pkg.rs`/`src/main.rs`, `tests/`.
- Coordinate: format co-designed with dev-3's L066 schema code to avoid two validators.

### L023 — Lockfile `operon.lock` [P2] [dev-1] [M] [open]
- Goal: resolved dependency versions + checksums; reproducible installs.
- Done when: lock written/updated by L020 commands; `operon install --frozen` fails on
  drift; format documented + versioned.
- Files: `src/pkg.rs`, `SPEC.md`, `tests/`.
- Depends: L020.

### L024 — Formal visibility model [P2] [dev-1] [M] [partial: anchors/exports exist]
- Goal: `pub/priv` per gene in modules; modules declare their public surface; `use` only
  binds public genes (privacy violation = semantic warning, hard under `--strict`).
- Done when: grammar + enforcement + SPEC §13b + oracle + tests; LSP respects visibility
  in completion (feeds L045).
- Files: `src/parser.rs`, `src/interp.rs`, `src/ls.rs`, `SPEC.md`, `tests/`.

### L025 — Dotted namespaces [P2] [dev-1] [M] [partial: use-as exists]
- Goal: `math.vector.add`-style nesting: modules may declare sub-modules; `use bio::seq::*`.
- Done when: nested module syntax + qualified calls resolve per L069 algorithm; std modules
  keep flat compat via generated re-export blocks; SPEC §13.
- Files: `src/parser.rs`, `src/interp.rs`, `std/*.op`, `SPEC.md`.

### L026 — Typed collections library [P2] [dev-1] [L] [open]
- Goal: `Set<T>/Deque<T>/Queue<T>/Stack<T>/Heap<T>/Graph<T>` as `std/collections.op`
  constructs (dynamic today) that gain L003 annotation sugar; no new Rust builtins
  (oracle lane stays closed for std work).
- Done when: each container has proofs in `tests/stdlib_selfhost.op` or `tests/std_*.op`,
  STDLIB.md rows, complexity notes.
- Files: `std/collections.op`, `tests/`, `STDLIB.md`.
- Depends: L003 for annotations; containers themselves can land untyped first.

### L027 — Stdlib breadth to mainstream [P1] [dev-1 + any dev] [L] [partial: 15 modules live]
- Already done (evidence): 15 modules `args bio collections csv fmt fs iter json math
  motifs random seq set strings testing` (loop-10 wave S landed set/testing/random).
- Remaining per audit: `path`, `process`, `env`, `logging`, `terminal`, `compression`,
  `hashing`, `url`, `http`-high-level, `walk`, `binary`, `unicode`, `encoding`, `db` (stub
  via py() bridge acceptable), plus the L1c leftover: **re_split verify-or-drop** (sz note
  in TASKS.md).
- Done when: each new module ships with proofs + STDLIB.md row + differential program;
  `.op`-only lane respected (no Rust builtins; py() bridge for the deep end per D-010-A).
- Files: `std/*.op`, `tests/std_*.op`, `STDLIB.md`.
- Note: `hashing`/`binary` unblock L029; `path` unblocks L019 ergonomics.

### L028 — Unicode depth for strings [P2] [dev-1] [L] [partial: unicode redteam payloads exist]
- Goal: normalization (NFC/NFD), grapheme segmentation, case folding, category queries;
  documented char-index semantics (byte vs char vs grapheme) for every string builtin.
- Done when: `std/unicode.op` (pure .op where feasible) or builtins with SPEC §10b;
  differential corpus incl. CJK/emoji combining cases; redteam: malformed UTF-8 containment
  already proven (rt_p7j) — extend with normalization storms.
- Files: `std/unicode.op` or `src/interp.rs`, `SPEC.md`, `tests/`.

### L029 — First-class bytes type [P2] [dev-1] [L] [open]
- Goal: `bytes` value (immutable buffer + builder), literals `b"..."`, indexing/slicing,
  conversions to/from str/list/numbers, std `hashing` consumes it (L027).
- Done when: value variant + builtins + SPEC §7; capability model: file read-bytes gated by
  read caps; redteam: 2GiB mem-charge contract holds for buffers (feeds off sec-r5 charges);
  oracle mirror.
- Files: `src/value.rs`, `src/interp.rs`, `src/lexer.rs`, `bootstrap/oracle.py`, `SPEC.md`.

### L030 — Raw / multiline / byte strings [P2] [dev-1] [S] [verify]
- Verify first: multiline strings + escape behavior are partially proven (rt_p6c/brescape);
  confirm what exists, then land `r"..."` raw + `b"..."` (with L029) + heredoc `'''...'''`
  if missing.
- Done when: lexer tests + SPEC §6 literals table + oracle mirror.
- Files: `src/lexer.rs`, `src/parser.rs`, `SPEC.md`, `tests/`.

### L031 — Numeric literal forms [P1] [dev-1] [S] [open]
- Goal: `0xFF`, `0b101010`, `0o755`, `1_000_000` (underscores in decimal+hex).
- Done when: lexer accepts forms; overflow behavior matches the existing saturate-to-0
  contract (f0fe2ec) with notes; oracle mirror; differential program.
- Files: `src/lexer.rs`, `bootstrap/oracle.py`, `SPEC.md`, `tests/`.

### L032 — BigInt / Decimal [P3] [dev-1] [L] [open]
- Goal: arbitrary-precision ints behind an explicit value variant or std module; Decimal
  for money/science.
- Done when: decision recorded (builtin vs std via py()); if builtin: fuel/charge model,
  SPEC §7, differential programs (parity vs Python ints via oracle is a gift here — use it).
- Files: `src/value.rs`, `src/interp.rs` or `std/bigint.op`, `SPEC.md`.

### L033 — Date/time value types [P2] [dev-1] [M] [partial: L1d landed]
- Already: `unix_time/date_parts/date_fmt` (builder-A, interp.rs), monotonic clock/now
  pinned (SPEC §22).
- Remaining: `Duration` arithmetic, timezone handling contract (UTC-only v1, documented),
  parse ISO-8601.
- Done when: `std/time.op` extensions + proofs; SPEC §22 extended; differential time-shape
  corpus extended (never pin wall-clock absolute values — shape only).
- Files: `std/` (new time module or extensions), `tests/`, `SPEC.md`.

---
---

# DEV-2 BLOCK — L034–L065 (builder-B): tooling, truth, ecosystem

> builder-B board was complete and idle since 2026-09-24 — this block is your new mandate.
> Suggested order: **L054 → L056 → L057 → L058 → L055 → L053** (the truth sweep, one PR),
> then **L039 → L043 → L049** (quick tooling wins), then **L037 → L041 → L042**, then
> L051/L052, then L045/L047/L048, then the P3s.

### L034 — Serialization trait/interface [P2] [dev-2] [M] [partial: json/csv exist]
- Goal: one `Serialize` concept (trait per L004, or duck-typed convention until then) so
  JSON/CSV/custom formats share a contract.
- Done when: `to_json`/`from_json` honor the convention for phenotypes; docs table of
  default encodings per value type; tests incl. round-trips.
- Files: `std/json.op`/`std/csv.op`, `src/interp.rs` (builtin hooks), `SPEC.md`, `tests/`.
- Depends: L004 ideally; convention-only version can land first.

### L035 — Macros / metaprogramming [P3] [dev-2] [XL] [deferred: design only]
- Deliverable: design note (SPEC §meta): why macro-lite (`gene` templates or declarative
  `@rule` blocks) over full syntax macros; review against the bio-keyword pressure documented
  in L036. Owner sign-off required to implement.

### L036 — Hard core/bio boundary [P0] [dev-2 (spec) + dev-1 (grammar freeze)] [M] [open]
- Goal: formal split — **core language** (`gene/let/if/for/match/return/stress/modules/
  types/traits/concurrency`) vs **biology layer** (`regulate/splice/methylate/m6a/operon/
  repressilator/ligand/riboswitch/quorum/fate/...`). New biological mechanisms land as
  libraries or declarative `.cell`-style APIs by default, not parser keywords.
- Done when: SPEC §1b draws the boundary and lists every current keyword on its side; a
  new-contributor rule in CONTRIBUTING.md; the audit's design-risk note answered with a
  governance paragraph (what would ever justify a new keyword).
- Files: `SPEC.md`, `CONTRIBUTING.md`.
- Coordinate: dev-3's L091/L092 split the *semantics* docs; this level splits the *syntax
  surface*. Land together for one coherent story.

### L037 — Total Grammar semantic contract [P0] [dev-2] [L] [partial]
- Goal: precise rung hierarchy — **canonical → repairable syntax → recoverable syntax →
  semantic warning → hard semantic error** — with a documented rule for which layer each
  behavior belongs to; the "unknown identifier becomes string" soft-miss gets an explicit
  contract (warning + `--strict` hard error) instead of silent acceptance.
- Done when: SPEC §2 rewritten with the 5-rung table; every current repair classified;
  `--strict` audit: which warnings escalate; differential programs pinning each rung;
  README honest-summary updated.
- Files: `SPEC.md`, `src/tools.rs`, `src/main.rs`, `tests/`.
- Note: L001/L002/L006 build on this contract; land before them if possible.

### L038 — Repair explanation mode [P1] [dev-2] [M] [open]
- Goal: `operon explain file.op` shows original token → repaired token → reason → rung →
  resulting AST fragment; `operon fmt --show-repairs` for the diff view.
- Done when: both commands exist; every repair note the parser emits is machine-listable;
  tests; README recipe.
- Files: `src/main.rs`, `src/tools.rs`, `src/parser.rs` (note plumbing), `README.md`.
- Depends: L037 (rung taxonomy).

### L039 — AST dump [P1] [dev-2] [S] [open — verified missing]
- Goal: `operon ast file.op` prints the parsed AST (S-expression or JSON via `--json`).
- Done when: command exists for canonical AND repaired parses (`--show-repairs` flag shows
  both); helps L037/L038 evidence; tested.
- Files: `src/ast.rs` (Debug impls as needed), `src/main.rs`, `tests/`.

### L040 — IR / bytecode dump [P2] [dev-2] [S] [open]
- Goal: `operon bytecode file.op` dumps bytecode once L009 exists; until then the CLI stub
  returns a clear "VM not landed (M100 L009)" note instead of a silent unknown-command.
- Done when: stub + later real dump; documented in `operon --help`.
- Files: `src/main.rs`, `SPEC.md`.
- Depends: L009 for the real dump.

### L041 — `check` rework: diagnostics, not grades [P1] [dev-2] [M] [Track L3d] [open]
- Goal: separate streams: `error / warning / style / repair / security / performance`;
  keep the 100-point score only behind `--score` (nobody's default view); add `check --json`.
- Done when: output groups by severity with counts; CI consumes `--json`;
  README/docs updated; no invariant gates depend on the score number.
- Files: `src/tools.rs`, `src/main.rs`, `README.md`, `tests/`.

### L042 — Static analysis depth [P1] [dev-2] [L] [partial: phantom/wobble/NMD/anchor checks live]
- Goal: add: unreachable code, unused variables/genes/imports, shadowing, infinite-loop
  detection, constant conditions, dead stores, obvious type mismatches, duplicate match
  cases, unused capabilities, possible-null flow (synergy with L001).
- Done when: each check lands with a differential corpus case + a false-positive budget
  (zero tolerance on the existing corpus: current 18 cookbook + 128 differential must stay
  warning-clean unless the warning is genuinely warranted); `check --json` classification.
- Files: `src/tools.rs`, `tests/`, `SPEC.md`.

### L043 — Wrong-arity static detection [P1] [dev-2] [S] [open]
- Goal: calls to known genes with too-few/too-many args = `check` error before execution.
- Done when: arity table built from parse; direct calls checked; dynamic calls excluded
  with a documented escape hatch; tests.
- Files: `src/tools.rs`, `tests/`.

### L044 — LSP signature help + parameter docs [P2] [dev-2] [S] [partial: hover exists]
- Goal: `textDocument/signatureHelp` wired to gene signatures + doc comments (feeds from
  dev-3's L074); active-parameter highlight.
- Done when: trigger characters `,`/`(` produce signatures in Neovim/VSCode recipe;
  lsp_smoke assertions; README.
- Files: `src/ls.rs`, `tests/lsp_smoke.py`, `README.md`.
- Depends: L074 (doc comments) for parameter docs — can land hover-only first.

### L045 — LSP depth wave [P1] [dev-2] [L] [partial: 5/7 baseline wired (lsp-r1)]
- Goal: references, rename, workspace symbols, semantic tokens, folding ranges, selection
  ranges, code actions (quick-fix for known repairs), inlay hints (types per L001),
  document links for `use`.
- Done when: each feature behind the existing doc-cache architecture with incremental-sync
  guard; smoke assertions per feature; priority order: references → rename → workspace
  symbols → folding → semantic tokens.
- Files: `src/ls.rs`, `tests/lsp_smoke.py`, `README.md`.

### L046 — LSP understands repairs [P2] [dev-2] [M] [partial]
- Goal: editor-visible distinction between canonical code and repaired code: repair
  diagnostics carry the rung (L037), quick-fix code actions offer the canonical form.
- Done when: diagnostics carry rung metadata; a repair quick-fix lands with tests.
- Files: `src/ls.rs`, `src/tools.rs`, `tests/`.
- Depends: L037, synergizes L038.

### L047 — Formatter configuration [P2] [dev-2] [M] [Track L3a] [open]
- Goal: `.operon-fmt.toml` or `.cell`-adjacent config: indent width, line length, brace
  style, quote preference, import ordering, canonical mode, minimal-change mode.
- Done when: config parsed + honored; fmt idempotence tests with 3 config profiles;
  defaults unchanged (back-compat).
- Files: `src/tools.rs`, `tests/`, `SPEC.md`.

### L048 — `operon lint` separate from check [P2] [dev-2] [M] [open]
- Goal: `check` = language correctness (L041 contract); `lint` = code-quality rules,
  configurable, pluggable rule list in `SPEC §lint`.
- Done when: lint command with ≥8 rules migrated/extracted from check's style side;
  per-rule disable comments; docs.
- Files: `src/tools.rs`, `src/main.rs`, `SPEC.md`, `tests/`.
- Depends: L041.

### L049 — Test runner filtering + repeat [P1] [dev-2] [S] [open]
- Goal: `operon test path/to/file.op`, `--filter name`, `--failed` (uses last report),
  `--repeat N` (flakiness probe), `--shuffle-seed` for order independence.
- Done when: flags live, `--json` report extended, scripts/test.sh exposes common
  invocations, tests.
- Files: `src/tools.rs` (run_tests), `src/main.rs`, `scripts/test.sh`.

### L050 — Property-based testing [P2] [dev-2] [M] [partial: differential harness is the base]
- Goal: seeded random property programs: parser repair invariants (AST round-trip), JSON
  round-trip, regex vs oracle, GRN transition invariants (mass-balance/decay bounds),
  Total Grammar never-crash law.
- Done when: `scripts/property.py` (oracle-side generator) with fixed seeds produces N
  cases per property, all feeding harness.py; CI job (non-blocking first, promotion
  criteria stated); first real bug hunt documented if found.
- Files: `scripts/property.py`, `bootstrap/harness.py`, `.github/workflows/ci.yml`.

### L051 — Fuzzing infrastructure (S7) [P1] [dev-2] [L] [partial: redteam 95 payloads]
- Goal: continuous fuzzing of lexer/parser/fmt/JSON/regex/module loader/`.cell`:
  cargo-fuzz targets (libFuzzer) for the Rust core + `scripts/fuzz_op.py` AFL-style
  .op mutator for the semantic layer.
- Done when: 4+ cargo-fuzz targets run 10 min each in a weekly CI job + nightly smoke;
  crash → minimized testcase → redteam payload pipeline documented; every fuzz-discovered
  crash = redteam regression test forever.
- Files: `fuzz/` (cargo-fuzz crate), `.github/workflows/fuzz.yml`, `scripts/fuzz_op.py`,
  `tests/redteam/`.
- Note: this is the queued S7 lane — fold its status here.

### L052 — Coverage reporting [P2] [dev-2] [M] [open]
- Goal: Rust line/branch coverage (llvm-cov) + proof-coverage script (which parser
  productions / builtins / rungs the corpus touches) — the S3 builtin audit proved this
  pattern works (74 arms → 15 uncovered → 3 programs → real oracle bug).
- Done when: `scripts/coverage.sh` produces HTML + a text summary; CI job uploads artifact;
  coverage numbers quoted in BENCH.md-adjacent QUALITY.md (new).
- Files: `scripts/coverage.sh`, `.github/workflows/ci.yml`, `QUALITY.md` (new).

### L053 — Generated documentation numbers [P1] [dev-2] [S] [partial]
- Goal: every count quoted in docs (proof files/assertions, redteam payloads, keyword
  count, std modules, builtin count) comes from ONE generator: `scripts/gen_stats.py`
  writing `docs/STATS.md` + injecting into README/SPEC marked sections.
- Done when: generator exists; all stale-number sites (audit found 67/88/92/95 redteam and
  80-proof doc drift) now generated; CI fails on manual edits to marked sections
  (simple grep check).
- Files: `scripts/gen_stats.py`, `docs/STATS.md`, `README.md`, `SPEC.md`, `.github/workflows/ci.yml`.

### L054 — Versioning consistency [P0] [dev-2] [S] [verify]
- Verify: SPEC header said `v2.3.0-dev` while Cargo says 2.2.0 (loop-10 truth passes may
  have fixed this). Land `scripts/gen_stats.py` version section: ONE source (Cargo.toml)
  → README/SPEC/STDLIB badges.
- Done when: zero version mismatches via grep script in CI; SPEC carries only the
  generated version line.
- Files: `scripts/gen_stats.py`, `SPEC.md`, `README.md`, `.github/workflows/ci.yml`.

### L055 — Keyword count generated [P1] [dev-2] [S] [verify]
- Verify: SPEC said 51 reserved keywords, parser had 58 (audit). reg-r4 claims "SPEC
  inventory complete" — confirm, then fold into L053 generator so it can never drift again.
- Done when: keyword table in SPEC is generated from `src/lexer.rs` truth.
- Files: `scripts/gen_stats.py`, `SPEC.md`.

### L056 — README C-kernel truth [P0] [dev-2] [S] [verify]
- Verify: C runtime kernel was deleted (audit A15: `runtime/operon_rt.c` gone, codon
  kernel C++ remains). Sweep README/docs for any remaining "C runtime" claims.
- Done when: `rg -i "c runtime|c kernel" README.md docs/` shows only accurate statements;
  architecture diagram (if any) shows Rust core + C++ codon kernel + Python oracle.
- Files: `README.md`, `docs/*.html`.

### L057 — README stdlib truth [P1] [dev-2] [S] [verify]
- Verify: audit said README described 6 modules, 15 exist; R10-b did an "STDLIB truth
  pass" — confirm every module row matches reality (function names, counts).
- Done when: STDLIB.md generated (or validated) from `std/*.op` scans per L053; zero drift.
- Files: `STDLIB.md`, `scripts/gen_stats.py`.

### L058 — Build-dependency claim fix [P1] [dev-2] [S] [verify]
- Verify: README claims "no crates, no network, no external dependencies" but Cargo.toml
  uses `cc` as build-dep. Fix the sentence to the honest form: "runtime dependency surface
  = zero crates; build-time = `cc` only; network never required after bootstrap."
- Done when: claim matches reality in README + website copy.
- Files: `README.md`.

### L059 — Windows CI becomes blocking [P2] [dev-2] [S] [open]
- Goal: remove `continue-on-error: true` from the Windows job once it is stable for 14
  consecutive days (post hotfix r4). Until then, a tracking badge/comment in the workflow.
- Done when: flag removed, Windows failures block merges, note in COMMS.
- Files: `.github/workflows/ci.yml`.

### L060 — Per-target release smoke [P2] [dev-2] [M] [open]
- Goal: post-build smoke per release target where practical: linux x64/arm64 run hello.op
  in QEMU/container; macos runner smoke; windows smoke via the existing windows job artifact.
- Done when: release.yml runs smoke per produced asset (or documents why a target cannot be
  smoked); failed smoke = failed release.
- Files: `.github/workflows/release.yml`.

### L061 — Distribution channels [P2] [dev-2] [M] [partial: B5 landed binstall + brew draft + winget/scoop notes]
- Remaining: actual Homebrew tap formula PR (owner publishes), winget manifest submission,
  one Linux channel (deb or AUR) as proof, Nix flake optional.
- Done when: 2+ channels work end-to-end from public artifacts; README install table
  updated with verified commands.
- Files: `packaging/`, `README.md`.

### L062 — LSP version compatibility policy [P3] [dev-2] [S] [open]
- Goal: operon ↔ operon-ls ↔ editor-extension compatibility matrix; operon-ls reports the
  Operon version it was built with in initialize.
- Done when: initialize result carries version + capabilities; policy paragraph in
  CONTRIBUTING.md (LSP ships in-lockstep with the binary).
- Files: `src/ls.rs`, `CONTRIBUTING.md`.

### L063 — Language compatibility policy [P1] [dev-2] [M] [partial]
- Goal: written policy: 2.x runs all 2.x programs; Total Grammar repairs never change
  output within a minor; breaking changes require major; deprecations survive N releases
  (feeds L064); each release ships a compat changelog section.
- Done when: SPEC §compat (new) + CHANGELOG policy header; the v2.2→v2.3 Track L additions
  classified as additive (policy's first worked example).
- Files: `SPEC.md`, `CHANGELOG.md` (exists? verify — else CONTRIBUTING section).

### L064 — Deprecation system [P2] [dev-2] [M] [open]
- Goal: `@deprecated("use X instead", since="2.4")` marks on genes/keywords; emits check
  warnings with migration text; `--strict` fails; removal scheduled by L063 policy.
- Done when: mark infrastructure + first real deprecation (`const → let` synonym per L005);
  docs.
- Files: `src/parser.rs`, `src/tools.rs`, `SPEC.md`, `tests/`.

### L065 — Automated migrator [P3] [dev-2] [L] [deferred: needs L064 + fmt config]
- Deliverable until un-deferred: design note for `operon fix` (AST-based rewrites feeding
  off deprecation marks + fmt canonicalization). First candidate rewrite: synonym
  canonicalization (already half-exists in fmt canonical mode).

---
---

# DEV-3 BLOCK — L066–L100 (sz): contracts, safety, ecosystem surface, release engineering

> Suggested execution order: **L100 → L091 → L092 → L088/L089/L090 → L086 → L066 → L068 →
> L070 → L069 → L074 → L073 → L072 → L084 → L082 → L083 → L096 → L099 → L094**, then the
> remaining P2/P3s. Rationale: contract docs first (they unblock dev-1/2 decisions and
> survive even if sessions reset), then code in the order the audit's "fix first" implies
> for this range.

### L066 — `.cell` formal schema [P1] [dev-3] [M] [open]
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
- Coordinate: L022 (`operon.toml`) reuses this validator.

### L067 — `.rna` AST-based edit model [P2] [dev-3] [L] [partial: design]
- Goal: today's `.rna` edits target source spans/text — fragile under reformatting. Future:
  parse → identify AST node → apply AST edit → reprint.
- Deliverable now: design note in SPEC §rna (or docs/design/RNA-V2.md) defining node
  addressing (gene name + ordinal, not byte spans), reprint strategy (reuse fmt), and the
  migration path. Implementation lands only after fmt canonical mode is stable (L047).
- Files: `SPEC.md` or `docs/design/RNA-V2.md`, `src/tools.rs` (evidence comments).

### L068 — `.rna` safety mode [P1] [dev-3] [S] [open]
- Goal: `operon rna --check` (or the existing rna surface with `--check`): dry run that
  reports target found/not, replacement count, old→new text diff, affected gene, and exits
  non-zero if a target is missing (script-friendly).
- Done when: check mode exists and writes nothing; JSON output via `--json`; tests cover
  hit/miss/multi-hit/missing-target; README recipe.
- Files: `src/main.rs`, `src/tools.rs`, `tests/`, `README.md`.

### L069 — Module resolution algorithm pinned [P2] [dev-3] [S] [partial]
- Goal: the de-facto chain (doc-relative → CWD → std → OPERON_STD → exe-std →
  manifest-std) is SPEC-official with a decision table, and LSP + runtime + harness use
  the SAME order (lsp-r1 made LSP CWD-independent; runtime already exe-relative per dx-r5).
- Done when: SPEC §13 resolution table; a differential program that resolves from each
  root; divergence between LSP/runtime documented as allowed only where justified.
- Files: `SPEC.md`, `tests/differential/`, cross-check `src/ls.rs`.

### L070 — Import diagnostics detail [P2] [dev-3] [S] [partial]
- Goal: failed `use` reports: requested path, every attempted root in order, why each
  failed (missing / not-a-file / capability-denied / parse-error-in-target), resolved
  std root version.
- Done when: diagnostics format landed (Rust + oracle note-parity where applicable);
  differential program; redteam: no path leakage beyond policy (sandbox info-disclosure
  review).
- Files: `src/interp.rs` (loader), `bootstrap/oracle.py`, `tests/`.

### L071 — Hot reload [P3] [dev-3] [M] [deferred: design note]
- Deliverable: design paragraph (watch-mode dependency, GRN state reset semantics, GenomeLab
  integration sketch) in `docs/design/HOT-RELOAD.md`. No implementation until L072 ships.

### L072 — `operon watch` [P2] [dev-3] [S] [open]
- Goal: `operon watch app.op [-- args]` re-runs on mtime change (poll ≥200ms, no new
  deps), clears screen or prints separator, shows run duration + exit status; `--quiet`
  for CI-ish loops.
- Done when: command exists with `.cell`-aware file set (watches imports too); SIGINT
  clean exit; test via scripted file touches; README.
- Files: `src/main.rs`, `tests/`, `README.md`.

### L073 — `operon doc` generator [P1] [dev-3] [M] [Track L3b] [open]
- Goal: `operon doc file.op|dir` emits markdown: per gene — signature, doc comment (L074),
  marks/effects, capabilities required, examples found in doc comments; per module — index.
- Done when: `operon doc std/` renders every std module; output committed under
  `docs/api/` as a generated artifact (CI regen-check like L053); tests.
- Files: `src/main.rs`, `src/tools.rs` (parse reuse), `docs/api/` (generated), `CI`.

### L074 — Doc comments [P1] [dev-3] [M] [open]
- Goal: `///` line doc comments before `gene`/`phenotype`/module headers; parser captures
  them into AST metadata; hover shows them (LSP synergy with L044); `operon doc` consumes
  them (L073).
- Done when: grammar + capture + retention through repair rungs (repaired genes keep
  docs); REPL `:doc gene` prints them; oracle parity not required (docs don't affect
  semantics — note that in SPEC); differential program asserting doc-text survives fmt.
- Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/ls.rs`, `src/main.rs`,
  `SPEC.md`, `tests/`.

### L075 — Example testing as first-class docs gate [P2] [dev-3] [S] [partial]
- Goal: formalize the existing chain: doc example → executable example → test → rendered
  doc. Cookbook (18/18) + apps/ gate already implement most of it; name it, document it,
  and make `operon doc` examples runnable-checked.
- Done when: CONTRIBUTING.md documents the docs-gate contract; `scripts/cookbook.sh` + the
  apps gate + L073 examples check referenced as the three enforcement points.
- Files: `CONTRIBUTING.md`, `scripts/cookbook.sh`.

### L076 — Stable embedding API [P2] [dev-3] [M] [partial: lib.rs exists]
- Goal: documented public Rust API: parse/compile/run/sandbox-config/register-builtin/
  capture-output/inspect-diagnostics; semver'd releases; embedding example crate.
- Done when: `docs/EMBEDDING.md` + lib.rs pub-use surface curated + `examples/embed/`
  (a tiny dependent crate using operon as a path dep, built in CI); version note per L063.
- Files: `src/lib.rs`, `docs/EMBEDDING.md`, `examples/embed/`, `CI`.

### L077 — C ABI [P3] [dev-3] [XL] [deferred: design note]
- Deliverable: `docs/design/C-ABI.md` — symbol surface (operon_run_source/operon_free/
  diagnostic access), ownership rules, why-not-yet (embedding API first, L076).

### L078 — Python bridge explicitly optional [P2] [dev-3] [S] [verify]
- Verify: substrate-r1 landed `py()` behind `--allow-py` + Caps.py grants (opt-in already).
  Confirm default-off behavior + graceful failure message when Python absent; document the
  optionality contract in SPEC §15 (bridge section).
- Done when: missing-Python produces a clean operational error (not a crash); docs state
  "Operon is fully functional with zero Python present"; test asserting bridge-off parity.
- Files: `src/interp.rs` (py bridge), `SPEC.md`, `tests/`.

### L079 — Python bridge version contracts [P2] [dev-3] [S] [partial]
- Goal: documented contract: supported Python (≥3.10), what happens on newer/older, no
  NumPy/pandas guarantee (forward as-is statement), startup-cost note, Windows caveat.
- Done when: SPEC §15b contract table; runtime warns once when interpreter < documented
  floor; docs.
- Files: `SPEC.md`, `src/interp.rs`, `README.md`.

### L080 — Native C/C++ FFI [P3] [dev-3] [XL] [deferred: design note]
- Deliverable: `docs/design/NATIVE-FFI.md` — `foreign` grammar sketch, capability gate
  (`--allow-ffi`), struct-layout story, why py() + C-ABI (L077) come first.

### L081 — C++ kernel boundary guard [done] [dev-3] [S]
- Evidence: sec-r5 verdict recorded — kernel "safe as-is, do not strengthen" (no signed
  shifts, fail-closed malloc, no ABI ownership, bounded DP); budget guard DP_CELL_BUDGET.
- Standing rule (new, this level): **no biological mechanism may move into the kernel
  without a benchmark proving ≥2x and a security re-audit** — add this line to
  CONTRIBUTING.md (done as part of L100's doc sweep).

### L082 — Performance regression gating [P1] [dev-3] [S] [open]
- Goal: bench CI job gains thresholds: any tracked benchmark regressing >20% vs the
  recorded baseline fails the job (noise-tolerant: median of 3 runs, ±5% band), turning
  the non-blocking job into a blocking gate on main.
- Done when: baseline JSON committed (`bench/baseline.json`, updated by script with
  machine-variance caveats), threshold logic in `scripts/bench_compare.py` (exists —
  extend), CI job blocking on main, one documented escape hatch (re-baseline PR).
- Files: `scripts/bench_compare.py`, `scripts/bench/baseline.json`, `.github/workflows/ci.yml`.

### L083 — Real-program benchmarks [P2] [dev-3] [M] [partial: fib/loops/strings/collections/grn/recursion/micro live]
- Goal: add: JSON parse+serialize, regex corpus, sequence processing (motifs), module-heavy
  program, large-map workload, file-I/O micro (within caps), py()-bridge round-trip
  (non-blocking variant).
- Done when: each new bench has a deterministic workload, BENCH.md row, baseline entry;
  no bench depends on network.
- Files: `scripts/bench/*.op`, `scripts/bench.sh`, `BENCH.md`.

### L084 — Startup time benchmark [P1] [dev-3] [S] [open]
- Goal: measure binary cold start → hello output, and startup composition (lexer/parser/
  stdlib import) via `--iters` timing harness; tracked per release.
- Done when: `scripts/bench_startup.sh` (hyperfine if available, else manual loop) writes
  BENCH.md table; regression threshold folded into L082; VM (L009) inherits this as a
  non-regression gate.
- Files: `scripts/bench_startup.sh`, `BENCH.md`, `.github/workflows/ci.yml`.

### L085 — Standalone executable compilation [P2] [dev-3] [L] [partial: operon build = source bake]
- Goal: `operon build app.op -o app` eventually produces a truly standalone binary.
  Roadmap: (1) bytecode bundle + tiny interpreter entry (post-L009), (2) Rust embed of
  bundle into a released `operon` runtime, (3) native codegen only if L011 profiling
  demands it.
- Deliverable now: roadmap section in SPEC §build + L086 contract; no false promises in
  README (audit's complaint).
- Files: `SPEC.md`, `README.md`.

### L086 — `operon build` contract [P1] [dev-3] [S] [open]
- Goal: honest documented contract of what build does TODAY (source specialization/baking):
  inputs, outputs, what is NOT guaranteed (no VM bytecode, no native exe yet), stability
  of baked output.
- Done when: SPEC §build + `operon build --help` text state the contract; tests pin
  baked-output shape (feeds L088 determinism).
- Files: `SPEC.md`, `src/main.rs`, `tests/`.

### L087 — Single-file bundle [P3] [dev-3] [M] [deferred: design note]
- Deliverable: `docs/design/BUNDLE.md` — `operon bundle app.op` artifact layout (source +
  bytecode + stdlib deps + metadata), why it waits for L023/L009.

### L088 — Reproducible build contract [P2] [dev-3] [S] [open]
- Goal: define reproducibility scope: same operon version + same source + same seed ⇒ same
  output bytes for `operon build` and `operon bundle` (future); document what is excluded
  (paths, timestamps) and how they are normalized.
- Done when: `docs/spec/DETERMINISM.md` §builds (this work); build normalization test
  (build twice, byte-compare) in CI or scripts.
- Files: `docs/spec/DETERMINISM.md`, `scripts/`, `tests/`.

### L089 — Randomness determinism model [P1] [dev-3] [S] [partial: seeded RNG + worker pinning exist]
- Goal: document what determinism means: same source + same seed + same operon version ⇒
  same random stream, single-thread AND multi-thread (worker pinning landed: worker_seed_pin
  test, F-1m pin preference); cross-platform stream identity promise or explicit
  non-promise.
- Done when: DETERMINISM.md §randomness with the exact stream contract + evidence links
  (tests/worker_seed_pin.op, std/random.op wrappers); SPEC §22 cross-link.
- Files: `docs/spec/DETERMINISM.md`, `SPEC.md`.

### L090 — Floating-point determinism [P2] [dev-3] [S] [open]
- Goal: define whether bit-identical f64 results across platforms are promised: compiler
  flags (no fast-math), operation order, transcendental policy (libm variance disclaimer),
  JSON float formatting policy.
- Done when: DETERMINISM.md §floats; differential corpus runs on x86_64 + arm64 in CI
  already (release targets) — add explicit float-parity assertion program.
- Files: `docs/spec/DETERMINISM.md`, `tests/differential/`.

### L091 — Bio semantics separation [P1] [dev-3] [M] [open]
- Goal: SPEC currently interleaves language semantics with biological modeling semantics.
  Split: language sections state syntax/evaluation ONLY; bio modeling moves to a dedicated
  volume (docs/spec/BIO-MODEL.md) referenced from SPEC §11.
- Done when: SPEC §11 is a pointer + core evaluation rules only; BIO-MODEL.md holds the
  mechanism math; no information lost (diff-audit by sz); future bio changes cannot be
  breaking language changes by construction.
- Files: `SPEC.md`, `docs/spec/BIO-MODEL.md` (new).
- Coordinate: L036 (syntax boundary, dev-2) + L092 (modeling contract, below) land as one
  coherent doc wave.

### L092 — Biological modeling contract [P1] [dev-3] [M] [open]
- Goal: every biological mechanism labeled honestly: **real mechanism / mathematical
  approximation / Operon-specific abstraction / fictional simplification**, with what the
  simulation output does and does NOT mean.
- Done when: BIO-CONTRACT.md table covering all live mechanisms (regulation, methylation,
  m6A+readers, RISC, IRES, riboswitch cis, operon polycistronic, quorum, repressilator,
  ligand, attenuation, decoy, enhancer, bursting, CRISPR, Rho, telegraph promoter,
  allostery/titration/decay-clock from reg-bio-2); GenomeLab help references it.
- Files: `docs/spec/BIO-CONTRACT.md` (new), `apps/genomelab/genomelab.op`, `README.md`.

### L093 — Scientific validation layer [P2] [dev-3] [M] [partial: repressi_alpha/params, trp_attenuator, riboswitch_cis, silence_dose tests live]
- Goal: formalize: reference datasets, known-model tests with numerical tolerances,
  published-model reproduction notes.
- Done when: `docs/spec/VALIDATION.md` lists each validated model, its source, tolerance,
  and the test that enforces it; tolerance framework (approx_eq with documented eps) in
  std/testing.op; one new reproduction test (e.g., classic repressilator period vs
  published parameterization note).
- Files: `docs/spec/VALIDATION.md`, `std/testing.op`, `tests/`.

### L094 — Graph visualization export [P2] [dev-3] [M] [open]
- Goal: `operon graph file.op --format dot|json|svg-stub` exporting the GRN/regulation
  graph: nodes (genes, levels), edges (activation/inhibition, cooperativity), modifiers
  (methylation/m6A marks).
- Done when: DOT output renders in graphviz (golden-file test); JSON output schema
  documented; capability-neutral (pure static analysis, no run required — or run-mode with
  final levels); README recipe.
- Files: `src/main.rs`, new `src/graph.rs`, `tests/`, `README.md`.

### L095 — Live regulation visualizer [P3] [dev-3] [L] [deferred: depends L094]
- Deliverable: design note extending L094 JSON with time-series frames; GenomeLab timeline
  integration sketch. Implementation after L094 + GenomeLab next wave.

### L096 — Profiler output formats [P2] [dev-3] [S] [open]
- Goal: `operon profile --json` (self-describing: units, version, run metadata) +
  Chrome-trace format (`.json` events) so about://tracing / perfetto render it; flamegraph
  text format optional.
- Done when: both formats emitted and schema-documented; BENCH.md/README recipes;
  golden-file test.
- Files: `src/tools.rs` (profiler), `src/main.rs`, `SPEC.md`, `tests/`.

### L097 — Memory profiler [P3] [dev-3] [M] [partial: memory() builtin + intern tables live]
- Deliverable now: document `memory()`'s exact accounting (interns/bytes/allocs semantics
  from ffi.rs) in SPEC §10; hotspot/peak/per-gene allocation deferred with design sketch.
- Files: `SPEC.md`, `docs/design/MEM-PROFILER.md` (sketch).

### L098 — Thread profiler [P3] [dev-3] [M] [partial: attempt telemetry (F-3) landed]
- Deliverable now: expose worker telemetry (spawn/join/attempt counts, lifetimes) via a
  `workers()` introspection builtin or profile extension; document; blocked-time +
  task-fuel reporting deferred.
- Files: `src/interp.rs`, `SPEC.md`, `tests/`.

### L099 — Security audit automation [P1] [dev-3] [M] [partial: SHA-pinned supply chain + redteam 95 in CI]
- Goal: add: `cargo-deny`/`cargo-audit` job (Rust advisories), dependency diff review
  note in CONTRIBUTING, periodic sandbox-regression runner (the live TOCTOU flipper
  pattern → scripted), resource-exhaustion smoke in CI, Windows/Unicode path probes on
  schedule (not per-PR), fuzz smoke tie-in to L051.
- Done when: advisory job blocking (with documented unblock procedure), sandbox regression
  script `scripts/sec_regression.sh` wired weekly, all existing sec test files enumerated
  as the regression manifest.
- Files: `.github/workflows/security.yml` (new), `scripts/sec_regression.sh`,
  `CONTRIBUTING.md`.

### L100 — Formal threat model [P0] [dev-3] [M] [open]
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
| W1 | truth sweep L053/L054/L055/L056/L057/L058 (one PR) | dev-2 | queued |
| W1 | contract wave: L100 + L091 + L092 + DETERMINISM + L086 | dev-3 | **wip (this session)** |
| W1 | error model: L007 (tracebacks) | dev-1 | queued (loop-11 compatible) |
| W2 | L066 + L068 + L070 + L069 | dev-3 | queued |
| W2 | L039 + L043 + L049 | dev-2 | queued |
| W2 | L006 (Option/Result) | dev-1 | queued |
| W3 | L074 + L073 + L072 + L084 | dev-3 | queued |
| W3 | L041 + L042 + L037 | dev-2 | queued |
| W3 | L001/L002 (Track L2c/L2b) | dev-1 | queued |
| W4 | L082 + L083 + L096 + L099 | dev-3 | queued |
| W4 | L009 VM design → compiler | dev-1 | gated on L007/L006 |
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
