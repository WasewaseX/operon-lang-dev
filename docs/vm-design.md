# Operon VM design (W09 stage A1)

Status: **A1-A6: A1 adopted; A2 bridge architecture landed (2026-09-27); A3/A4
full-corpus parity campaign green (219/219, redteam 106/0 both engines); W11
stage-1 superinstructions AND stage-2 reachability DCE landed; A5 perf pass
reached cross-engine parity (0.88x -> 0.99x on fib25, see BENCH.md) with the
>=3x stretch target still open under W11; A6 DEFAULT FLIP landed (v2.6.0,
`--interp` escape hatch)**
· Owner: dev-1 (builder-A) · Track: A1-A6 (W09)
Target reader: a CS engineer implementing or reviewing the A-track (D-008: zero biology assumed).
This document is the contract the A2–A6 phases implement against. It decides the value
representation, the bytecode format, the calling/entropy/error/fuel contracts, and the phase
gates. It defers JIT (W12), the optimizer pipeline (W11), and the disassembler UX (W10, only
its hook is fixed here).

---

## 1. The measured problem

BENCH.md (three-runner method, floor-clean rows): the tree-walking interpreter pays a per-node
dispatch + `Rc<Env>` chain walk on every variable read and every call.

| workload | operon (ms) | native-py (ms) | op/py |
|---|---|---|---|
| fib25 | 127.7 | 12.8 | **10.0x slower** |
| loops | (see BENCH) |, | 4–8x slower |
| grn | 33.2 | 8.6 | **3.9x slower** |
| collections |, |, | ~1x (map memo already lands 8.6x vs CPython on the loop-3 fixture) |

The hot cost centers are (a) call funnels (fib25 = 242,785 calls), (b) variable resolution
(Env chain walk per read), (c) per-node recursion in `eval`. Collections already do fine,
the VM must not regress them. Target: **VM ≥ 3x tree-walk on the BENCH set** (W09 done-when),
which puts function-call-heavy code at parity with CPython or better.

## 2. Non-negotiable invariants (every phase re-proves them)

1. **Differential byte-parity**, the oracle (Python, tree-walk) stays the reference engine.
   The VM is Rust-side only. `bootstrap/harness.py` grows a `--vm` lane in A2: every corpus
   target must be byte-identical across {tree-walk Rust, VM Rust, oracle Python}. Today 154/154;
   any phase that cannot hold 154/154 on both lanes does not merge.
2. **Entropy discipline**, the mirrored xorshift64* stream is program order. Draws happen at
   the same AST nodes in the same order (telegraph promoter per call attempt, regulation
   captures, ring noise, quorum decay, Rho scan). The VM may not reorder, short-circuit, or
   batch any evaluation that can draw. Pinned today by telegraph/quorum/noise/rho corpus proofs.
3. **Total Grammar**, nothing is rejected at compile time that the tree-walk parses. Notes
   (wobble) surface at the same program points with the same text. The compiler emits IR for
   everything; recoverable conditions stay runtime notes, not compile errors.
4. **Sandbox default-deny + containment**, capability checks, `mem_charge`, note caps, and
   output caps are engine-level services, not bytecode. Fuel: every VM opcode has a charge
   **≥** its tree-walk construct charge (conservative table in §8) so every redteam payload
   contained under tree-walk (100/0) is contained under the VM. `scripts/redteam.sh` runs both
   engines in A4+.
5. **Gate-funnel order**, RISC check → toggle → GRN pass (operon unit gate inside) →
   methylation → promoter telegraph, then `rbs`-weighted `trans_integrate`. This funnel is
   implemented ONCE and shared: the VM's `Call` routes through the same funnel code the
   tree-walk uses (§6). A reimplementation that drifts a gate order is a bug, not an
   optimization.
6. **Version honesty (D-009)**, SATISFIED at the A6 flip: the default flipped at the v2.6.0
   milestone and the banner gained the `-vm` suffix (`Operon 2.6.0-vm (rust-core, cpp-kernel)`);
   `--interp` is the escape hatch named by §9 (pre-flip the flag was `--vm`, still accepted).

## 2b. W11 stage 1+2 delivered: the optimization pipeline (2026-09-27, main; stage 2 same series, commit 340a29f)

`--opt 1` runs THREE draw-free passes over each compiled body before execution
(`optimize` in src/vm.rs): constant folding (Push/Push/Bin triples and the
Push/BinImm fused pair over pure Int/Float/Bool arithmetic, i64 CHECKED so
anything that would stress at runtime stays runtime), jump threading (Jmp
chains resolved, a Jmp to the next instruction removed), and reachability
DCE (instructions unreachable from ip 0 — dead code after a Ret, abandoned
jump islands — are dropped, with every Jmp/JmpIfF/Brk/Cont target remapped
through the old->new table; unreachable code can never execute, so no tick,
note, draw or output can change). The passes never touch Bridge/EvalExpr
instructions: folding cannot reorder or remove a draw (invariant 2). Cache
note: an optimized body caches under a shifted key so --opt 0 and --opt 1 do
not share entries. Evidence: unit tests pin the folded shape of
`return 6 * 7` and the exact DCE drops (`dce_drops_only_unreachable_code`);
the corpus parity gate `scripts/vm_parity.sh` runs every tests/**.op and
apps/**.op byte-identical across --no-vm, default VM, and --opt 1.
REMAIN on the W11 board entry: trivial-gene inlining, monomorphic call
specialization, builtin/global resolution caching, list-op fast paths,
per-pass bench rows. LANDED since: the toggle matrix (--opt-passes
fold,thread,dce,prop | none | all, --opt 2 = full) and pass 4 constant
propagation (1:1 LoadName -> Push rewrites inside straight-line runs;
every jump target, scope edge, bridge and call resets the facts — full
corpus byte-identical on the --opt 2 parity axis).

## 2c. W11 stage 3 design: trivial-gene inlining + monomorphic specialization (designed, gated - 2026-10-01)

Both remaining W011 passes share one structural prerequisite the current
compiler does not have: a compile-time symbol table. CallNamed compiles
from a bare identifier; the callee resolves at RUNTIME through the
dynamic chain (env shadow check first - a user gene may shadow any
builtin, and a binding may change between calls). Inlining a body at
compile time would risk executing a stale definition; caching a call-site
resolution (the classic monomorphic-site cache) would skip that same
dynamic re-resolution.

The measured answer (BENCH.md W011 matrix) reframes the value honestly:
dispatch is semantics-bound, not lookup-bound - the gate funnel, the
mem_charge/cycle-note security charges, and the env-chain shadow check
ARE the contract. A trivial-gene fast path can only skip machinery the
gene provably does not use.

The safe design, when picked up:

1. **Trivial-gene fast dispatch (runtime, not compile-time) — DELIVERED
   (2026-10-01, this batch).** As built, in two halves. CODE half:
   `shapes_only()` on the CACHED GeneCode (computed in gene_code_cached
   AFTER optimize_with — folding can only shrink a shapes-only body into
   more whitelist shapes, never out of them), stored as the `trivial` bit
   on GeneCode. Whitelist = the designed Push/LoadName/Bin/BinImm/Ret/
   RetName PLUS the fused pure-read shapes LoadNameQuiet/LoadBinImm, Pop
   and Nop — each addition provably read-only or no-op; StoreName,
   AssignName, scopes, bridges, calls and ALL jump shapes disqualify.
   DEF half (checked per call in call_gene_inner, cheap field reads):
   zero args, zero params, no guard, no param/return annotations, no
   acetylate/methylate/m6a/riboswitch/burst, copies == 1, not seq. The
   fast path executes the cached body against the parent env (closure or
   global) directly — the frame env it skips is provably EMPTY — and
   duplicates the slow tail verbatim (timing push/close_timing,
   propagation-as-return, check_ret_ann). Everything with semantics runs
   BEFORE the divergence point: RISC, toggle, GRN, methylation,
   riboswitch, promoter, RHO, operon transcripts/queue,
   bump_call_bookkeeping, the methylate announcement; the extra-args note
   cannot fire (zero params AND zero args). The fiber hook falls back to
   the prepared-frame path (a spawn needs a real frame to park). Reads
   stay LIVE per execution — only the code is cached, never a value — so
   rebound globals, unbound-read notes and shadowing ride the same bytes.
   Evidence: tests/w011_trivial_dispatch.op (the anti-stale-evidence
   program) in the vm_parity corpus all four axes, unit test
   trivial_bit_shape_matrix, full corpus + redteam runs recorded in the
   batch commit.
2. **Monomorphic specialization — DELIVERED (2026-10-01, this batch).**
   As built: the DEF_GEN generation counter (process-global AtomicU64,
   Relaxed) bumped INSIDE Env's three mutators (define/set/define_const —
   the audited-only mutators of any vars map, so let/assign/const/
   auto-declare/pattern-capture are covered completely); param binding is
   deliberately exempt (Env::define_param) with the shadowing-safety
   argument: a cached site only stores GLOBAL-level resolutions, and a
   frame's own params cannot shadow a name that resolved past them (if a
   param shared the name, the walk would have hit the param and the site
   would have stored nothing — origin-checked via Env::get_from). The
   cache lives on the Interp keyed by (code-object address, site ip);
   both VM machines supply the key (sync machine: code pointer + ip-1,
   fiber machine: frame Rc + cur_ip), the tree-walk passes None. Entries
   store the PRE-call generation, so any write the callee performs
   invalidates the site; cached values are global-hits (Value::Gene or a
   global's callee value) and full env-misses (the builtin branches then
   decide). Workers start with an empty cache (fresh interps never copy
   the map). Evidence bar RUN GREEN: redteam 109 contained / 0 breached
   (the shadowing corpus), vm_parity 3537 identical / 0 divergent all
   four axes, fuzz_diff 600x5 0 findings, BENCH.md fib25 row measured
   143.9ms -> 145.9ms (1.01x, within run noise — dispatch is
   semantics-bound, recorded honestly).

Both items' evidence bars have run green; W011's enumerated list is
fully delivered.

## 2a. A2 delivered: the bridge architecture (2026-09-27, main)

Stage A2 is ON MAIN behind `--vm`, and it ships with an architecture decision
this document adopts as the A3 baseline:

- The compiler (`src/vm.rs`) compiles every gene body to OIR1. Compilation is
  INFALLIBLE (Total Grammar): constructs outside the native set are bridged,
  not rejected.
- Native instructions: literals, name loads/stores (Env-based, identical
  chain semantics, charge_clone + unbound notes preserved), the non-short-
  circuit binops via the SHARED `apply_binop`, jumps, block scopes, return.
- Bridged instructions re-enter the tree-walk for the sub-AST (calls,
  methods, builtins, interpolation, patterns, and/or/nullish short-circuit,
  every statement outside the native set). Bridged code IS the tree-walk, so
  gates, entropy draws, note text and stress kinds are byte-identical by
  construction. A bridged statement's flow is honored: Ret leaves the gene;
  a Brk/Cont from a statement bridged inside a COMPILED loop is patched at
  compile time to jump to that loop's end/top (BridgeStmtInLoop).
- Param binding, the gate funnel and guards stay in `call_gene_inner` (§6's
  "A2 does not bypass the funnel" posture); the hook swaps ONLY the body
  execution. Fuel: every native opcode ticks once (never cheaper than the
  tree-walk); bridged code costs what the tree-walk costs.
- `operon ir` is the real OIR1 listing (W10 stage 1); the encoding of one
  compiled function is pinned by a unit test.
- Evidence: the differential harness grew a --vm lane; 184/184 targets are
  byte-identical against the oracle on the same run that checks tree-walk.
  Slot locals, escape analysis and the 3x perf pass remain A3/A5 work; the
  call path enters the VM only through the shared funnel by design.

## 3. Value representation: unchanged (v1 decision)

The VM reuses `src/value.rs` `Value` verbatim: `Null/Bool/Int(i64)/Float(f64)/Str/List(ListRef)/
Map(MapRef)/Gene/Seq/Obj/Variant`. Consequences and rationale:

- Every builtin, the `py()` bridge, kernel calls (C++ codon kernel), JSON/fmt, and the
  `SendValue` thread boundary keep working with zero adaptation.
- Oracle parity is structural: the oracle models the same shapes; no new repr = no new
  parity surface.
- `ListRef`/`MapRef` are `Rc<RefCell<...>>` sharing semantics (SPEC §19), identity, aliasing,
  and the D-013 no-cycle-reclamation decision carry over untouched.
- Cost: values stay boxed/enumed (8–16 bytes + payload). The v1 win comes from **slot locals**
  (§6) and **no AST walk**, not from unboxing. If A5 profiling shows Int/Float boxing is the
  remaining wall, A5 may add a tagged-array representation *inside* list storage only,
  flagged now as the one sanctioned repr experiment; anything else requires a new A-doc.

Variable resolution redesign (the actual parity-safe win): locals live in **frame slots**
(indexed), not in the `Env` chain. Closure capture (genes returned from genes, lambdas) uses
**captured cells**: a slot that is captured by an escaping inner function is allocated as a
`Rc<RefCell<Value>>` cell instead of a raw slot, decided at compile time (the compiler knows
escapes), so runtime never checks. `Env`-chain semantics (block scoping, shadowing, the
arm-child capture scopes of match-v2, proof-frame locals) are modeled by the compiler's scope
numbering; behavior is pinned by the existing corpus, not by re-derivation.

## 4. Bytecode format: `OIR1`

A compiled module is a versioned, hashable blob:

```
OIR1 {
  version: u32            // ir format version; rejects older silently-regenerated formats
  source_hash: u128       // hash of the .op source; diagnostics map back to it
  consts:  [Const]        // pool: Int/Float/Str/Bytes-of-sorted-map-shape/Null
  genes:   [GeneMeta]     // name, arity, slot count, cell list, flags (entry/ires/variant table)
  code:    [u8]           // per-gene instruction sections, u8-opcode streams
  maps:    [SrcMap]       // ip -> (line, col), Stress.line, notes, and tracebacks (W007) read this
}
```

- Instruction encoding: `u8 opcode` + operands. Small operands are inline (`u8`/`u16`);
  large ones (const index, jump offset) are `u32` little-endian postfix. No variable-width
  prefixes beyond opcode+fixed operands, decoding is a match on one byte.
- Jump offsets are `u32` byte deltas. A gene's code section is capped at 1 MiB of IR
  (parse-level size caps already bound source; this is the IR-side backstop, charged).
- Const pool is interned by structural hash. Strings never re-allocate on repeat loads.
- `GeneMeta.cell_list` precomputes the .cell knobs a gene's body consulted at compile time
  (documentation + snapshot diagnostics); behavior still reads live cells at runtime.

## 5. Instruction set

v1 (A2: arithmetic/values), full set by A3/A4. Names are fixed now so the disassembler (W10)
and the oracle-side IR mirror (optional, diagnostics only) are stable.

```
Stack ops      Push cidx | Pop | Dup
Locals         LoadLocal slot | StoreLocal slot | LoadCell c | StoreCell c
Globals        LoadGlobal nameidx | DefGlobal nameidx
Arithmetic     Add Sub Mul Div Mod Neg          // Rust i64/f64 semantics EXACTLY:
                                                 // i64 overflow -> "overflow" stress (same
                                                 // kind+message), f64 IEEE, floored % (SPEC §19)
Compare        Eq Lt Le Gt Ge Not
Jump           Jmp off | JmpIfF off             // pop-and-test uses the SAME truthy() order
Calls          Call geneidx argc | CallV expr   // CallV = value-position callee (lambdas/genes
                                                 // in maps), resolves through the same funnel
Return         Ret
Data           MakeList n | MakeMap n | IndexGet | IndexSet | Slice
Namespaces     ImportMod nameidx | ModGet m k
```

Everything else (regex, threads, py(), kernel calls, all builtins) is `CallBuiltin idx argc`,
builtins are NOT bytecode; they are the same Rust functions the tree-walk calls. This is what
keeps A4 from being a rewrite: the entire builtin surface (including capability gating) is
shared code.

Explicitly deferred opcodes (never in v1): vector/SIMD, tailcall, try/catch handlers (stress
unwinding is table-based, §7, but no user-visible handler opcodes beyond rescue's existing
runtime construct), any inline-cache encoding (A5 may add IC metadata as a side table, not as
opcodes).

## 6. Frames, calling, and the funnel

```
CallFrame {
  gene:   Arc<GeneDef>,
  ip:     u32,          // resume point
  base:   u32,          // operand-stack base in the frame's value stack
  slots:  Vec<Value>,   // locals; escaped slots are Rc<RefCell<Value>> cells
  ret:    Flow-slot,    // normal return vs ?!-propagated return vs stress
  // --- W16 (docs/specs/ASYNC.md): LANDED 2026-10-01. The fiber machine
  // (src/vm.rs) owns Vec<VmFrame> — the heap frame stack this section
  // reserved — with fiber_state / wake_deadline / cancel_flag as real
  // fields; native gene calls push frames through the one hook in
  // call_gene_inner so the gate funnel stays shared.
  fiber:  Running | SuspendedOn(WakeReason),  // park at builtin calls only
  wake_deadline: Option<u64>,                 // timer-wheel slot (virtual ms)
  cancel_flag: bool                           // W18 cooperative cancellation
}
```

- `Call` does: arity check (same note on mismatch) → **the shared gate funnel** (RISC → toggle
  → GRN unit/edges → methylation → promoter telegraph; entropy fast paths p/q∈{0,1} draw
  nothing, same as today) → compile-time-known slot layout → execute body → `Ret`.
- The funnel code is factored OUT of the tree-walk path into a shared module in A3 so both
  engines call one implementation. Until then A2/A3 only run funnels via the existing
  `call_named` path, i.e., A2 does not bypass the funnel, it charges it.
- Worker cells (`spawn`): the worker compiles the same module (shared `Arc<OIR1>`), inherits
  regulation snapshots exactly as today (seed decorrelation included), and drains the SAME
  run-wide `fuel_pool` (Arc<AtomicI64>), the loop-5 rule "one pool per run" is structural.
- Deep recursion: frames move from the host stack to a heap-allocated frame stack with the
  existing depth cap (F-1 class). The cap value and the stress text stay identical.

## 7. Errors, notes, ?! propagation

- `Stress` (kinds, messages, `.line`, chain capture, unforgeable `prop` marker) is unchanged.
  The compiler emits a **stress table** per gene: `{ip_range -> rescue target}` for `rescue`
  constructs. Unwinding pops frames to the matching target, attaches the chain frame
  (gene name + `SrcMap` line), and continues. Propagation (`?!`) rides `Stress.prop` exactly
  as today, the VM's pre-arm points are the same construct boundaries (gene return, rescue
  crossing, proof runner, seq end, REPL).
- Notes: the VM emits notes through the same sink; `SrcMap` gives identical file:line
  rendering. Note caps unchanged (parser/lexer caps stay; runtime note flood caps charge per
  note as today).
- Proof frames: `operon test` compiles the whole file; proof frames run on the VM too,
  the vacuous-proof and exited-early rules are runner-level, unchanged.

## 8. Fuel + containment charge table (conservative)

Rule: **no opcode may be cheaper than the tree-walk construct it replaces** (the tree-walk's
charge IS the current containment invariant; the VM must not buy speed by selling containment).
A2 publishes the full table as code (`vm_charges.rs`) with a unit test asserting monotonicity
against the tree-walk charges extracted from interp.rs; the redteam suite runs on the VM from
A4 and must stay 100/0. `step_budget` semantics: one opcode = one step (tree-walk: one eval
node = one step), charged from the same shared pool. `mem_charge` calls are identical (same
allocation sites in Value construction helpers, which are shared).

## 9. Phases and exit gates

| Phase | Scope | Exit gate (all must be green) |
|---|---|---|
| A1 (this doc) | design adopted | this file + DECISIONS entry; `ir` stub points here (already does) |
| A2 | compiler + VM for arithmetic/values/locals/Jmp behind `--vm`; `operon ir` flips from stub to real disassembly (W10 stage 1); harness `--vm` lane on the corpus subset that avoids calls | 154/154 tree-walk lane unchanged; `--vm` lane green on its subset; redteam unchanged; clippy 0/fmt clean |
| A3 | full control flow + gene calls + the shared funnel + rescue/stress table + match-v2 + Seq/Obj + proof frames | full corpus 154/154 on BOTH lanes; telegraph/quorum/noise/rho proofs byte-identical; workers green |
| A4 | builtins-heavy surfaces under the VM (regex, py(), kernels, threads), redteam on VM | redteam 100/0 both engines; caps/charges monotonicity test green |
| A5 | perf pass (ICs, const folding only where provably draw-free, slot unboxing inside lists if needed) | ≥3x tree-walk mean on BENCH set, no containment regression, parity holds |
| A6 | flip default, keep `--interp` escape hatch, D-009 version move | release matrix green; SPEC §15/§22 truth pass; the flip is the milestone |

Rollback rule: any phase whose parity or containment gate breaks and cannot be fixed within
the session reverts the flag default (never the corpus).

## 10. What this design deliberately does NOT do

- No new parser keywords, no syntax changes (compile step is invisible to source).
- No optimizer pipeline in v1 (W11 comes after A5's profiling data; const-folding that could
  skip draws is FORBIDDEN until it proves stream-identity, a folded expression must draw or
  not draw exactly as the unfolded one did).
- No JIT (W12 stays parked; bytecode-first per the audit).
- No repr churn beyond §3's sanctioned list-storage experiment.
- The Python oracle never grows a VM, it is the reference, not a peer.

---

## 11. A6 delivered: the default flip (2026-09-30, v2.6.0)

`operon run` now executes gene bodies on the OIR1 machine BY DEFAULT.
Evidence and scope:

- **Differential**: the harness's default lane (now the VM) 225/225 vs the
  oracle; the renamed tree-walk lane (`--no-vm`) 219/219 vs the same oracle.
  Both engines stay differentially pinned on every step; the labels
  inverted at the flip, the coverage contract did not.
- **Red-team**: 106 payloads contained, 0 breached on BOTH engines
  (`OPERON_EXTRA_ARGS="--no-vm" bash scripts/redteam.sh` re-runs the suite
  against the tree-walk).
- **Escape hatch**: `--interp` (§9's name) or the `--no-vm` alias; `--vm`
  remains accepted. Workers/sequences were always tree-walk (fresh Interps,
  no vm_program) and stay so — A4's registration work remains the open
  A-track item alongside the A5 ≥3x stretch.
- **Banner**: `Operon 2.6.0-vm (rust-core, cpp-kernel)` (D-009 suffix).
- **Perf at the flip** (median of 5, end-to-end): fib25 0.99x, loops 1.04x,
  collections 1.00x, recursion 0.98x vs the tree-walk. The A5 campaign
  cleared the 0.88x regression (def-name re-clone per call removed, SipHash
  pointer-key cache -> identity hash, silences empty-gate malloc skipped,
  call-counter entry clones -> get_mut fast path). The ≥3x stretch stays
  OPEN under W11/A5 (slot locals, VM-native call lane); parity is the
  shipped floor, not the ceiling.

## 12. W009-A delivered: the call-funnel measurement (2026-10-02, builder-A)

The A5/§2c campaign left three named suspects for the fib25 call-heavy gap
(fresh Env per call, per-param clone, per-instruction fuel tick — §1, §11).
W009-A replaced suspicion with measurement: a runtime-gated ablation harness
(`src/w009a.rs`, default-inert, evidence-only) neutralizes each mechanism
independently and the fib25 gate re-times it. Full method + raw numbers:
BENCH.md "W009-A" section; evidence `docs/bench/2026-10-02-w009a-{ablation.txt,baseline.json}`.

What the measurement established on the call-heavy workload:

1. **The bottleneck is the shared call funnel, not the bytecode machine.**
   VM 144.2 ms vs tree-walk 145.3 ms on fib25; `--opt 2` / `--opt-passes
   all` change nothing (143.2). Codegen quality and dispatch-loop speed are
   NOT the differentiator — both lanes converge on `call_gene_inner`, and
   that shared path is where the ~595 ns/call (11x CPython) lives.
2. **Measured, fixable-shape costs ≈ 22% of fib25**: regulatory gate checks
   + operon transcript scan + call bookkeeping (16.8% — three to four
   SipHash'd map operations per call across call_counts/gene_buckets/cell
   lookups), the traceback frame String clone (5.2%, built on every
   happy-path call despite the comment's "no allocation" intent), and the
   promoter_veto name clone that runs before its `expr_stochastic`
   early-return (4.0%). The SAFE bundle (all three, semantics-preserving
   shapes) reaches 111.4 ms = −22.4%.
3. **Refuted by measurement**: the per-instruction fuel tick (~0.1% — the
   65,536-batch fuel-pool design is already cheap enough), the m6a/grn decay
   tickers (±0 on gate-less programs), and NAIVE Env pooling (+6% — a TLS
   free-list round-trip costs more than glibc's small-alloc fast path; if
   frame allocation is ever worth attacking it must be by not allocating:
   slot locals, not recycling).
4. **The dominant residual (~78%) is structural**: after the SAFE floor the
   per-call cost is the ~8-deep Rust call chain (named_call_tail_vm →
   named_call_tail → call_named → call_value → call_gene → call_gene_inner
   → exec_gene_code), string-keyed env-chain resolution inside the body
   (~6+ SipHashes per call: two `fib` lookups each walking frame→global,
   plus `n` reads), scope-env creation (~1.5 `Env::new` per call), and args
   Vec plumbing. This is the §3 "slot variables" defer — now with a number
   attached to it.

Ranked W009-B candidates (owner's call; none scheduled by this task):
(a) per-gene cached clean-regulation bit → skip the veto funnel and cheapen
bookkeeping when a gene carries no regulation and no .cell knobs (~17%
proven shape); (b) eliminate the happy-path String clones (~9%); (c) slot-
indexed locals for plain frames (targets the structural residual; largest
expected value, largest design lift); (d) a collapsed mono-cache-hit call
lane that skips the redundant gates between call_named and call_gene_inner.
Explicitly NOT worth it per measurement: fuel-tick tuning, decay-ticker
short-circuits (already ~0), map recycling. No performance claim in this
document is unreproducible — every row re-derives from the committed
harness + matrix script on the recorded sandbox.

## 13. W009-B delivered: slot-indexed call bookkeeping (2026-10-03, builder-A)

Candidate (b) of §12's ranked list was absorbed by W011-r2 before this
session started (lazy traceback frame, promoter early-out); the fresh
ablation matrix on 19cb1a8 re-attributed the remaining gates block to
bookkeeping (13.0%) + veto checks (~1.5%). W009-B implemented the
bookkeeping half of candidate (a): the per-name counter/burst-bin HashMaps
are now interned slot arrays with the slot hint cached on the GeneDef.

Invariants this document records for the next person touching the funnel:

1. **The hint is a hint.** GeneDefs are value-carried Arcs that outlive
   interps (workers, repl, proof re-runs). `bk_slot_for` name-validates
   the cached index on EVERY call and re-interns on mismatch — the cost
   of never trusting the cache is one short-string compare; the cost of
   trusting it blindly would be silent cross-interp miscounts.
2. **Bump order is load-bearing.** count → clock → decay-tick, in that
   order, because the translation integration reads the counter delta at
   tick time (ffl_coherent_delay's first y() call integrates yp = 0.3;
   a post-tick bump integrates 0 and three grn-timing proofs fail). The
   old HashMap code had this order by accident of layout; the slot code
   has it by comment.
3. **Zero-count slots never exist.** `bk_count_for` is a pure read —
   materializing slots for never-called names would drift the
   mature/nascent telemetry that the oracle mirrors.
4. **Bins are ascending pair-vectors**, exact under the monotone call
   clock; the burst reader keeps the D9 float-sum order (b-ascending,
   gene names sorted).

Measured: fib25 1.12–1.13x, fib27 1.14x, recursion 1.09x, grn 1.05x,
loops 0.99x (no-call control, unchanged) — interleaved A/B with
byte-identical-output enforcement; full row set in BENCH.md §W009-B.
Candidate (a)'s veto-block clean-bit (the remaining ~1.5%) is re-scoped
to W009-C: the epoch-invalidation surface now spans every regulation
mutation site for an EV below the risk bar. The structural residual
(funnel depth, env-chain resolution, args plumbing) is unchanged —
locals-in-frame remains the big lever.
