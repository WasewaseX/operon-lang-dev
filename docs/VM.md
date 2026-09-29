# W09/A1 — The Operon Bytecode VM: Design Document

Status: A1 landed this revision; A2 (arithmetic/values) and A3 (control flow +
genes) implemented behind `--vm`. A4 (sequences/threads in VM frames), A5
(perf pass), A6 (default flip) remain staged. Owner: dev1 (builder-A).

## 1. Why a VM, and why this shape

The tree-walking interpreter re-dispatches on AST nodes for every evaluation
and resolves every name through an `Env` chain walk. BENCH.md shows the cost
concentrates in gene bodies: fib, loops, collection building. The VM replaces
the AST walk with a flat instruction loop while keeping every piece of
semantics that parity depends on in the shared funnels.

The controlling constraint is the differential oracle (byte-identical output
on both engines) and the four invariants. A VM that re-implements operator
semantics would fork the language. Therefore:

**Design rule W09-1 (single source of truth).** The VM owns only *control
flow* and *operand plumbing*. Every operation with observable semantics
(binary operators, builtins, gene/method calls, index, member, sequences,
stress creation, notes, fuel, RNG) delegates to the same functions the
tree-walk uses: `apply_binop`, `call_named`, `call_value`, `call_method`,
`member_value`, `as_index`, `map_insert`, `seq_pull`, `stress_map`, `tick`,
`charge_clone`, `ann_matches`. There is no second implementation of any
operator.

**Design rule W09-2 (stamp-trajectory parity).** Diagnostics locate
themselves through `interp.cur_line`, which the tree-walk stamps at specific
AST-arm entries (Binary, Call, Index) *before* children evaluate, so the
effective stamp at any error is the nearest enclosing stamping arm. The VM
reproduces the trajectory exactly with an `Insn::Line(l)` pseudo-instruction
emitted at the same points. Instruction handlers that correspond to
non-stamping arms (Unary, Method, Ternary) do not touch `cur_line`.

**Design rule W09-3 (evaluation-order parity).** Short-circuit forms
(`and`/`or`/`??`), multi-assign ordering, `op=` read-then-write, RISC
silencing (decision *before* argument evaluation; degraded calls skip
argument evaluation entirely) are all compiled to instruction shapes that
reproduce the tree-walk's order exactly (see §4).

## 2. Artifacts

- `compile.rs` — the compiler: `Program` → per-gene `FuncCode`s. Gene bodies
  compile; the top level stays tree-walk (its per-statement containment loop
  is load-bearing and cold).
- `vm.rs` — `Interp::vm_exec(code, env)`: the dispatch loop.
- Hook: `call_gene_inner` executes a gene's bytecode when
  `interp.vm_funcs` maps the gene's `Arc<GeneDef>` identity to a `FuncCode`,
  else falls back to `exec_block`. All callers (eval calls, builtins,
  methods, entry resolution) inherit the acceleration through this one hook.
- `operon disasm file.op [--json]` — W10 disassembler over the same
  `FuncCode`s.

## 3. Instruction set

Value stack machine; frames are `Env` scopes (no slot arrays yet — A5 may
introduce slots for hot locals; the `Env`-based frame keeps delegation
sound, because delegated leaves read names through the same chain).

| group | instructions |
|---|---|
| operands | `Const(k)`, `Pop`, `Dup`, `Load(n)`, `LoadPlain(n)`, `Define(n)`, `DefineAnn(n, ann)`, `Store(n)`, `StoreOp(n, op)` |
| operators | `Line(l)`, `Bin(op)`, `AndJmp(t)`, `OrJmp(t)`, `NullishJmp(t)`, `Un(op)`, `Index`, `Member(k)`, `MemberSafe(k)`, `Method(k, argc)`, `MethodSafe(k, argc)`, `MakeList(n)`, `MakeMap(n)` |
| control | `Jmp(t)`, `JmpIfFalse(t)`, `Tick`, `PushScope`, `PopScope`, `Ret`, `RetNull` |
| calls | `CallStart(name, line)`, `IfDegraded(argc, t)`, `CallFinish(argc, name)`, `CallValue(argc)` |
| loops | `IterMake`, `IterNext(t)`, `IterBindName(n)`, `IterEnd`, `LoopPop` |
| stress | `CatchEnter(kind, end, handler)`, `CatchLeave`, `CatchTrim(depth)`, `RescueTicks`, `RescueBind(n?)`, `RaiseStmt(kind?, line)` |
| delegation | `Stmt(Rc<Stmt>)`, `Expr(Rc<Expr>)` |

`Load` mirrors the `Ident` arm including the clone charge and the
unbound-reads-as-null note; `LoadPlain` exists because the `op=` arm reads
the current value *without* the charge or the note — two distinct loads,
both exact.

## 4. Parity-critical shapes

- **Short-circuit**: `AndJmp/OrJmp/NullishJmp` peek (do not pop); on the
  short-circuit outcome the left value *is* the result — the jump skips the
  right-side code with the value in place; otherwise the right-side value
  overwrites it. Byte-shape of `a and b` = eval a, AndJmp end, eval b, end.
- **Calls, named**: `Line(l)`, `CallStart(name, l)` (RISC gate: filter,
  immunity, survival product, xorshift draw, notes — decision and notes
  happen here, before arguments), `IfDegraded` (degraded ⇒ args skipped,
  result `null`, jump past finish), argument code, `CallFinish` (redirect ⇒
  `call_value` on the pre-resolved replacement; normal ⇒ `call_named`).
- **Calls, value callee**: `Line(l)`, callee code, args, `CallValue` —
  callee first, then args left-to-right, matching the arm.
- **`op=` on names**: RHS first (`StoreOp` pops it), then current read via
  `LoadPlain` semantics inside the instruction, then write — never a `Load`
  (which would emit the unbound note the tree-walk does not).
- **Loops**: `for` over a sequence ticks *before* each pull; over
  materialized items ticks *after* fetch — `IterNext` carries both paths.
  Materialization (list clone, string chars, map keys, non-iterable note +
  skip) happens once at `IterMake`, exactly like the arm.
- **Scope discipline**: branch/iteration bodies run in child scopes
  (`PushScope`/`PopScope`); `break`/`continue` emit `CatchTrim` +
  `PopScope×(static depth delta)` + `Jmp`; breaks arriving from *delegated*
  statements are restored at runtime from the loop entry's saved base env /
  base catch depth (no static knowledge needed).
- **stress/rescue**: `CatchEnter` pushes a runtime record (protected range,
  handler, kind filter, saved stack/iter/mark/catch depths and base env). On
  a stress in range: propagation (`?!`) is pre-armed *before* kind matching
  and converts to a return (D-014); kind match truncates all state to the
  record's bases and jumps the handler; kind mismatch keeps searching
  outward (the tree-walk's `return Err`). The handler runs `RescueTicks`
  (64 ticks, exact order), a child scope, the optional `stress_map`
  binding, the rescue body, and rejoins after the guarded region.
- **return/break from rescue bodies** pass through as flows — `vm_exec`
  returns `Result<Flow, Stress>` exactly like `exec_block`, so the funnel's
  boundary semantics (propagation conversion, return-annotation checks,
  profiler close) apply unchanged.

## 5. What is delegated (and why that is safe)

Anything without hot-loop value compiles to `Stmt`/`Expr` delegation:
`use`, `match` (A4 will compile pattern dispatch), destructuring lets/for,
multi-assign, index/member assignment, phenotype/splice/fate/regulation
declarations, frames, edits, anchors, tads, interpolations, comprehensions,
lambdas (registered for VM execution but constructed by `eval`),
`New`, `FateNew`, `?!` (A4), ternary is native. Delegation is *total*: a
construct the compiler does not know still runs, on the tree-walk, inside a
bytecode frame — the Total Grammar property transfers to the compiler.
Coverage is reported by `operon disasm --json` (per-gene instruction counts
+ delegation list), never by runtime notes (output must stay byte-identical).

## 6. Sequences and threads (A4 boundary)

Sequence bodies run on worker threads with fresh `Interp`s whose
`vm_funcs` is empty — they execute tree-walk by construction. Registering
sequence/worker code objects is A4, together with kernel/regex hot paths.

## 7. Fuel and budgets

The VM ticks once per instruction (the tree-walk ticks per `eval`/`exec_stmt`
node). Per-program tick counts therefore differ between engines by a small
deterministic factor; burn ceilings trip at the same scale. Memory charges,
depth limits, and the shared run-wide fuel pool are untouched (the same
helpers run). Differential output is unaffected on the whole corpus —
verified by `scripts/vm_parity.sh`, which runs every proof + differential
corpus file under `--vm` and diffs byte-for-byte against the tree-walk run.

## 8. Gate contract (A2/A3 done-when)

- differential harness stays green (unchanged — the oracle is untouched);
- `scripts/vm_parity.sh`: VM vs tree-walk byte-identical on the full corpus;
- redteam suite green under both modes;
- disasm round-trips: every compiled gene lists, JSON mode parses;
- BENCH delta recorded for fib/loops/collections in BENCH.md.
