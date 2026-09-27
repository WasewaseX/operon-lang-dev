# VM, A1 design note (W09, stage 1 of the A1–A6 arc)

**Status:** design (A1). No bytecode exists yet; the tree-walk interpreter in
`src/interp.rs` is the only engine. This document is the contract the VM
implementation must satisfy before `--vm` can flip to default (A6).
**Owner:** dev1/builder-A. **Audience:** D-008 (CS engineers; zero biology
assumed, gene = function, operon = unit, regulate = feature-gate network).

---

## 1. Why (measured, not vibes)

- BENCH.md: the tree-walk core sits at ~8.6× CPython wall-clock on
  collection-heavy programs (the dx-r3 map-memo + builtin-resolution caches
  carry most of it); `fib`/`loops` remain per-node dispatch.
- Every `Expr` node costs a `match` on an enum of ~31 variants plus recursive
  `eval` calls; statement dispatch is the same shape. The interpreter IS the
  bottleneck and it cannot be fixed by micro-optimizing the tree walk.
- The differential harness (158/158 at this writing) is the gate: **the VM is
  a second engine over the same AST, held to byte-identical stdout**, the
  same discipline as the Python oracle, plus fuel/step parity.

## 2. Non-negotiable invariants (from the standing M100 iron rules)

1. **Total Grammar**, the VM consumes the same repaired AST the tree-walk
   consumes. Nothing is "rejected at compile time" that runs today.
2. **Determinism**, same program + same `.cell` + same seed ⇒ byte-identical
   output, same step count charged, same note sequence. Randomness stays
   seeded (`0x9E3779B97F4A7C15`); draw-count invariance holds under any opt
   level (W89).
3. **Fuel parity**, `tick()` is currently called at statement execution and
   loop back-edges. The VM must charge **the same events** (one step per
   statement dispatch, one per loop back-edge, one per builtin call), so a
   program's `burned` behavior at a given budget is engine-independent.
4. **Capability checks at identical points**, `fs`/`run`/`net`/`env` grants
   are checked where the tree-walk checks them (inside the builtin call, not
   at compile time). The VM does not pre-resolve grants.
5. **Gate-funnel parity**, `regulate` networks, `operon` units (polarity,
   Rho termination, ribosome queue shield), `enhance`/`silence` toggles, and
   quorum signals evaluate in the SAME funnel the tree-walk uses today
   (`call_gene`, `src/interp.rs:3236`). A1 keeps the funnel as shared code;
   the VM calls it; it does not re-implement biology.

## 3. Bytecode format (A1 decision record)

- **Instruction set**: stack-based, ~45 opcodes to start. Stack-based (not
  register) because the tree-walk's value semantics (everything is a
  `Value`, containers are `Rc<RefCell>`-shaped references) maps 1:1 onto
  push/pop without a register allocator, A5 can revisit.
- **Encoding**: `Vec<Ins>`, `struct Ins { op: Op, a: u32, b: u32 }` (8 bytes).
  Constant-pool indices in `a`; jump offsets in `b` (i32 via cast). Wide
  operands are rare, if needed, `Op::Wide` prefix.
- **Units of compilation**: one `CompiledGene` per gene/lambda/sequence body
  (frames map to `call_gene` today), plus one for each `frame`/`operon` body.
  Top-level statements compile to a main unit.
- **Value repr**: unchanged, `Value` is shared with the tree-walk. Numbers
  stay tagged enum variants (no NaN-boxing this cycle; A5 measures first).
- **Names**: locals are slot-indexed per frame (slot 0 = `self` in phenotype
  methods); globals/captures resolve through the existing `Env` chain until
  A5 (slot-izing closures is the riskiest change and buys the least).

### Opcode sketch (opening set)

Push/Load/Store (const, local slot, global, capture), Pop/Dup, arithmetic +
comparison (Int/Float fast paths, generic slow path calling the same
`binop` logic), List/Map literal builders, Index/Member read + assign,
Jump/JumpIfFalse/JumpIfTrue (truthiness = the language's), IterNext (for-in,
incl. sequence pull), Call (argc in `a`; goes through the shared gate
funnel), Return, Spawn/Join, RaiseStress/ArmRescue (rescue is a handler
table per frame, not a stack unwind, matches `rescue` scoping), Match
(pattern-match on stack top; the match-v2 matcher is shared code),
NoteEmit (Total Grammar notes are real output, they must fire in the same
order with the same text), builtins stay `Call` (no opcode per builtin).

## 4. Frames

```text
Frame {
  unit: Rc<CompiledGene>,  ip: usize,
  base: usize,             // stack base for locals
  rescue_table: Vec<(catch_span, handler_ip, binder_slot)>,
  gate_ctx: GateContext,   // handle to the shared regulate/operon funnel state
}
```

- `rescue` compiles to a handler table entry + a Jump over the guarded block;
  `raise` = search current frame's table, else pop frame (the traceback chain
  capture from W07 hooks frame-pop, chain text must be identical).
- **Worker threads** (`spawn`): the worker compiles nothing new, it runs the
  same units; `SendValue` serialization happens at the funnel exactly as
  today (W14 membrane).
- **Sequences**: `SeqState` (Rc<RefCell>) is carried as-is; `IterNext` calls
  the pull path, worker-cell pull included (lazy worker semantics from the
  sequence arc are observable output, parity-gated).

## 5. Gate-funnel parity (the biology boundary)

`regulate`/`operon`/quorum/Rho are `.cell`-configured behavior around calls.
A1 keeps every gate decision inside the shared funnel (`call_gene`) so the
VM cannot drift: the VM's `Call` op does `push(args); FUNNEL.call(...)` and
the funnel decides (threshold, polarity, Rho catch-distance, queue shield)
exactly as now. This preserves the loop-10 entropy-stream parity work (the
128→158 program corpus pins those streams byte-for-byte) and review item 36's
core/bio boundary: **no gate logic compiles into bytecode**; gates are
runtime decisions that read `.cell` state.

## 6. Staging plan (each stage = separate PR + full gates)

| Stage | Scope | Gate |
|---|---|---|
| A1 | this doc | board ratification |
| A2 | compiler + VM for arithmetic/values/locals behind `--vm`; harness runs corpus twice | 158/158 twice, fuel-identical burn on the fuel-pin corpus |
| A3 | control flow + calls + match + rescue + notes | harness green; traceback text identical; note order identical |
| A4 | builtins full surface, sequences, workers, kernel calls | harness green incl. granted cells; redteam 100/0 |
| A5 | perf pass: slot locals, builtin-resolution cache, op fusion where measured | BENCH ≥3× tree-walk on fib/loops, no parity loss |
| A6 | `--vm` becomes default (old engine kept one release behind a flag) | board sign-off + docs truth pass |

Blocked-adjacent items: W10 disassembler (ships at A2, `operon disasm` prints
the encoding; JSON mode for tooling), W08 debugger phase 1 (breakpoints become
`unit:ip` traps, REPL-on-break reuses the loop), W11 optimization pipeline
(stages between compile and run: const-fold → DCE → inline-at-A5), W16 async
spec reserves `Frame.suspend_at` + scheduler-owned stacks (ASYNC.md sketch).

## 7. Risks (honest)

- **Note-order drift**: notes are output. The VM must emit through the same
  note channel with the same dedup/cap logic, this is the most likely place
  byte-parity breaks (it is also where the oracle historically broke).
- **Fuel accounting drift**: silent behavioral difference (a program that
  survives at budget N on one engine, burns on the other). A2 adds a
  fuel-pin differential corpus: N budgets × M programs, same verdict.
- **Closure capture via Env chains** is the largest shared-state surface;
  slot-izing too early risks semantic drift (mutation visibility through
  closures is pinned by memory_model differential, it stays the arbiter).
- The C++ kernel boundary (`runtime/`, `src/ffi.rs`) is untouched by the VM;
  budget-guarded kernel calls happen through the same funnel.
