# Operon VM design (W09 stage A1)

Status: **A1 — design note, adopted** · Owner: dev-1 (builder-A) · Track: A1–A6 (W09)
Target reader: a CS engineer implementing or reviewing the A-track (D-008: zero biology assumed).
This document is the contract the A2–A6 phases implement against. It decides the value
representation, the bytecode format, the calling/entropy/error/fuel contracts, and the phase
gates. It defers JIT (W12), the optimizer pipeline (W11), and the disassembler UX (W10 — only
its hook is fixed here).

---

## 1. The measured problem

BENCH.md (three-runner method, floor-clean rows): the tree-walking interpreter pays a per-node
dispatch + `Rc<Env>` chain walk on every variable read and every call.

| workload | operon (ms) | native-py (ms) | op/py |
|---|---|---|---|
| fib25 | 127.7 | 12.8 | **10.0x slower** |
| loops | (see BENCH) | — | 4–8x slower |
| grn | 33.2 | 8.6 | **3.9x slower** |
| collections | — | — | ~1x (map memo already lands 8.6x vs CPython on the loop-3 fixture) |

The hot cost centers are (a) call funnels (fib25 = 242,785 calls), (b) variable resolution
(Env chain walk per read), (c) per-node recursion in `eval`. Collections already do fine —
the VM must not regress them. Target: **VM ≥ 3x tree-walk on the BENCH set** (W09 done-when),
which puts function-call-heavy code at parity with CPython or better.

## 2. Non-negotiable invariants (every phase re-proves them)

1. **Differential byte-parity** — the oracle (Python, tree-walk) stays the reference engine.
   The VM is Rust-side only. `bootstrap/harness.py` grows a `--vm` lane in A2: every corpus
   target must be byte-identical across {tree-walk Rust, VM Rust, oracle Python}. Today 154/154;
   any phase that cannot hold 154/154 on both lanes does not merge.
2. **Entropy discipline** — the mirrored xorshift64* stream is program order. Draws happen at
   the same AST nodes in the same order (telegraph promoter per call attempt, regulation
   captures, ring noise, quorum decay, Rho scan). The VM may not reorder, short-circuit, or
   batch any evaluation that can draw. Pinned today by telegraph/quorum/noise/rho corpus proofs.
3. **Total Grammar** — nothing is rejected at compile time that the tree-walk parses. Notes
   (wobble) surface at the same program points with the same text. The compiler emits IR for
   everything; recoverable conditions stay runtime notes, not compile errors.
4. **Sandbox default-deny + containment** — capability checks, `mem_charge`, note caps, and
   output caps are engine-level services, not bytecode. Fuel: every VM opcode has a charge
   **≥** its tree-walk construct charge (conservative table in §8) so every redteam payload
   contained under tree-walk (100/0) is contained under the VM. `scripts/redteam.sh` runs both
   engines in A4+.
5. **Gate-funnel order** — RISC check → toggle → GRN pass (operon unit gate inside) →
   methylation → promoter telegraph, then `rbs`-weighted `trans_integrate`. This funnel is
   implemented ONCE and shared: the VM's `Call` routes through the same funnel code the
   tree-walk uses (§6). A reimplementation that drifts a gate order is a bug, not an
   optimization.
6. **Version honesty (D-009)** — the VM ships behind `--vm` until A6 flips the default at a
   milestone; `operon version` gains a `-vm` banner suffix only when the default flips.

## 3. Value representation: unchanged (v1 decision)

The VM reuses `src/value.rs` `Value` verbatim: `Null/Bool/Int(i64)/Float(f64)/Str/List(ListRef)/
Map(MapRef)/Gene/Seq/Obj/Variant`. Consequences and rationale:

- Every builtin, the `py()` bridge, kernel calls (C++ codon kernel), JSON/fmt, and the
  `SendValue` thread boundary keep working with zero adaptation.
- Oracle parity is structural: the oracle models the same shapes; no new repr = no new
  parity surface.
- `ListRef`/`MapRef` are `Rc<RefCell<...>>` sharing semantics (SPEC §19) — identity, aliasing,
  and the D-013 no-cycle-reclamation decision carry over untouched.
- Cost: values stay boxed/enumed (8–16 bytes + payload). The v1 win comes from **slot locals**
  (§6) and **no AST walk**, not from unboxing. If A5 profiling shows Int/Float boxing is the
  remaining wall, A5 may add a tagged-array representation *inside* list storage only —
  flagged now as the one sanctioned repr experiment; anything else requires a new A-doc.

Variable resolution redesign (the actual parity-safe win): locals live in **frame slots**
(indexed), not in the `Env` chain. Closure capture (genes returned from genes, lambdas) uses
**captured cells**: a slot that is captured by an escaping inner function is allocated as a
`Rc<RefCell<Value>>` cell instead of a raw slot — decided at compile time (the compiler knows
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
  maps:    [SrcMap]       // ip -> (line, col) — Stress.line, notes, and tracebacks (W007) read this
}
```

- Instruction encoding: `u8 opcode` + operands. Small operands are inline (`u8`/`u16`);
  large ones (const index, jump offset) are `u32` little-endian postfix. No variable-width
  prefixes beyond opcode+fixed operands — decoding is a match on one byte.
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
                                                 // in maps) — resolves through the same funnel
Return         Ret
Data           MakeList n | MakeMap n | IndexGet | IndexSet | Slice
Namespaces     ImportMod nameidx | ModGet m k
```

Everything else (regex, threads, py(), kernel calls, all builtins) is `CallBuiltin idx argc` —
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
  // --- W16/A2 reservation (docs/specs/ASYNC.md): carried from A2 onward,
  // unused until async lands; cost = one enum tag + one u32 + one bool.
  fiber:  Running | SuspendedOn(WakeReason),  // park at builtin calls only
  wake_deadline: Option<u32>,                 // timer-wheel slot
  cancel_flag: bool                           // W18 cooperative cancellation
}
```

- `Call` does: arity check (same note on mismatch) → **the shared gate funnel** (RISC → toggle
  → GRN unit/edges → methylation → promoter telegraph; entropy fast paths p/q∈{0,1} draw
  nothing — same as today) → compile-time-known slot layout → execute body → `Ret`.
- The funnel code is factored OUT of the tree-walk path into a shared module in A3 so both
  engines call one implementation. Until then A2/A3 only run funnels via the existing
  `call_named` path — i.e., A2 does not bypass the funnel, it charges it.
- Worker cells (`spawn`): the worker compiles the same module (shared `Arc<OIR1>`), inherits
  regulation snapshots exactly as today (seed decorrelation included), and drains the SAME
  run-wide `fuel_pool` (Arc<AtomicI64>) — the loop-5 rule "one pool per run" is structural.
- Deep recursion: frames move from the host stack to a heap-allocated frame stack with the
  existing depth cap (F-1 class). The cap value and the stress text stay identical.

## 7. Errors, notes, ?! propagation

- `Stress` (kinds, messages, `.line`, chain capture, unforgeable `prop` marker) is unchanged.
  The compiler emits a **stress table** per gene: `{ip_range -> rescue target}` for `rescue`
  constructs. Unwinding pops frames to the matching target, attaches the chain frame
  (gene name + `SrcMap` line), and continues. Propagation (`?!`) rides `Stress.prop` exactly
  as today — the VM's pre-arm points are the same construct boundaries (gene return, rescue
  crossing, proof runner, seq end, REPL).
- Notes: the VM emits notes through the same sink; `SrcMap` gives identical file:line
  rendering. Note caps unchanged (parser/lexer caps stay; runtime note flood caps charge per
  note as today).
- Proof frames: `operon test` compiles the whole file; proof frames run on the VM too —
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
  skip draws is FORBIDDEN until it proves stream-identity — a folded expression must draw or
  not draw exactly as the unfolded one did).
- No JIT (W12 stays parked; bytecode-first per the audit).
- No repr churn beyond §3's sanctioned list-storage experiment.
- The Python oracle never grows a VM — it is the reference, not a peer.
