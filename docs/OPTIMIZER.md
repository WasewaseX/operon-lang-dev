# The Operon Optimizer (W011)

Status: **implemented, parity-proven** — every pass preserves semantics
byte-for-byte on the full corpus (see §Gates). This document describes the
pass inventory, the exactness argument for each pass, the CLI switches, and
the benchmark methodology. The governing rule (owner directive) is:

> Every optimization must preserve semantics.

"Semantics" here means the FULL observable envelope: stdout bytes, stderr
bytes, notes (the sandbox event log), stress kinds/messages/lines/traceback
chains, the xorshift RNG stream position, tick fuel, and sandbox charges.
Anything less than byte-identity across that envelope is a bug, not an
optimization.

## Levels and switches

| CLI flag | Meaning |
|---|---|
| `--opt=0` | no compile-time passes (pure W09 bytecode) |
| `--opt=1` | fold + thread + dce (**default** for `--vm`) |
| `--opt=2` | O1 + block-local constant propagation |
| `--opt-passes=a,b,c` | pin an explicit pass set (`prop`, `fold`, `thread`, `dce`) |
| `--no-fast` | disable the engine fast paths (§Engine) |
| `--dump-optimized` | per-function insn counts + active pass list (stderr) |

Both discrete (`--opt 2`) and attached (`--opt=2`) flag forms are accepted.
The engine fast paths are behavior-identical and on by default; `--no-fast`
exists for benchmarking isolation.

Implementation: `src/opt.rs` (passes), `src/interp.rs` (`pure_binop`,
trivial-body detector, builtin dispatch), wiring in `src/tools.rs` +
`src/main.rs`. The pipeline order per function is **prop → fold (to
fixpoint) → thread → dce**; all passes are index-ordered and deterministic
(same input FuncCode + config → same output FuncCode).

## Compile-time passes

### prop — block-local constant propagation (O2)

A `Define(k)` whose value is the immediately-preceding `Const(c)` lets
later `Load(k)`s in the **same basic block** become `Const(c)`.

Exactness argument:

- **Dominance** — propagation state resets at every jump target (join
  point), so only the straight-line path from the Define can reach the
  Load; the Define provably executed.
- **Kills** — `Store/StoreOp/Define/DefineAnn/IterBindName` of k remove k;
  any call-shaped insn (`CallFinish/CallValue/Method/MethodSafe`) and any
  delegated `Stmt/Expr` clear ALL tracked names: a closure that captured
  k's cell could `Store(k)` from another FuncCode, and delegated
  statements can define arbitrary names in the current scope.
- **Scopes** — `PushScope/PopScope` clear all (same-depth re-entry is a
  different frame; the Load would resolve a different binding).
- **Unbound reads** (Total Grammar) cannot fire: the dominating Define
  executed.
- **Charges** — `charge_clone` (sec-r5 F-9) only charges `Str > 64 KiB`;
  large-string constants are refused, so the Load's charge is preserved
  exactly. Scalar loads charge nothing.
- The replacement pushes the same value the Load would have cloned.

### fold — constant folding

Patterns (each rewrite preserves the exact stack shape of every path):

- `Const; Const; Bin` → `Const` via `pure_binop` — the SAME function the
  runtime evaluates through (agreement by construction). Overflow,
  division by zero, type errors → **refused** (the runtime raises
  identically, with its own cur_line stamp).
- `Const; Un` → `Const` via the same Un arm mirror.
- `Const; AndJmp/OrJmp/NullishJmp/JmpIfFalse` → `Jmp(t)` or `Pop`,
  decided by `Value::truthy()` — the same predicate the VM branches on.
- `Line`/`Tick` stamps inside a folded window are preserved (the back-scan
  skips only stack-neutral insns), so the cur_line trajectory — and every
  later traceback line — is unchanged.

`pure_binop` returns `None` for anything that charges (`mem_charge`):
string concat, list concat, string repetition. The folder refuses those —
the sandbox allocation ceilings stay exactly where they were.

**Float exactness** — folded floats are bit-identical to the runtime
result, and the const pool dedupes floats on **bit pattern**, not `==`:
IEEE says `0.0 == -0.0`, and reusing a `+0.0` slot for a folded `-0.0`
would flip printed output (caught by `rt_p5a_arith` / `rt_p5e_nan_json`;
the same fix was applied to the pre-existing `FuncCompiler::konst`, which
had the latent flaw for signed-zero literals).

### thread — jump threading

Every single-target jump (`Jmp`, `JmpIfFalse`, `AndJmp`, `OrJmp`,
`NullishJmp`, `IfDegraded(_, t)`, `IterNext(t)`) is retargeted through
chains of unconditional `Jmp`s. An unconditional Jmp has no side effects,
so landing after the chain has the identical stack, env, marks, and
cur_line. Cycles are left untouched (visited set) — a jump into a cycle
loops forever either way. Targets of `insns.len()` are the implicit
function end (the exec loop returns `Flow::Norm` there) and are bounds-
checked, never indexed.

### dce — reachability dead-code elimination

A conservative superset walk from pc 0:

- `Jmp` is jump-only; `Ret/RetNull/RaiseStmt` are terminal (a RaiseStmt's
  handler reachability comes from the matching `CatchEnter` edge).
- **Every stored target of every insn is an edge** — including catch
  handlers (reachable only through the catch machinery) and loop records
  (`VLoop.end` is consumed by delegated breaks landing via `pc = l.end`).
  Missing either would delete live code.
- Delegated `Stmt/Expr` are modeled as plain fall-through (they may also
  `Ret/Brk/Cont` — over-approximating reachability is always safe).
- Renumbering drops only Nops and no-op jumps (`Jmp(next)`, trailing
  `Jmp(end)`), never at target pcs; `JmpIfFalse(next)` canonicalizes to
  `Pop` (the arm pops the condition on the fall-through path).
- **All** stored targets are remapped: the jump family, `IfDegraded`,
  `IterNext`, `LoopEnter(top, end)`, `CatchEnter(kind, leave, handler)`.
  The catch range check (`pc ∈ [start, end]`) stays correct because the
  remap is order-preserving on reachable insns.

`Insn::Nop` is a new ISA scratch slot used by fold; the VM arm is a no-op;
dce removes them. The compiler never emits Nop.

## Engine fast paths (`--no-fast` disables)

1. **Trivial-body fast return** (`exec_gene_body`) — the sound form of
   "trivial-gene inlining". A body that is exactly `return <literal>;`
   (or `return;`) returns the same `Flow::Ret` that `vm_exec`/`exec_block`
   would produce, without frame construction or dispatch. **Why not
   compile-time inlining:** a named call must run the RISC silencing gate
   (notes + xorshift RNG stream + Redirect/Degraded decisions from runtime
   config), the recursion-depth counter, guard bodies, and annotation
   checks — hoisting any of these to compile time is observable
   (redteam pins all of them). The fast path lives INSIDE the funnel:
   every gate, counter, arity check, param binding/annotation/default,
   guard, and profiler span runs unchanged above the hook; return
   annotations run unchanged below it. The trigger is a pure AST test, so
   both engines fast-path the same defs — parity by construction.
2. **Lazy traceback frames** (`call_gene`) — the W007 frame `(name, line)`
   was cloned (a String allocation) on EVERY happy-path call. The line is
   captured before the call (a usize copy) and the String is built only on
   the error path. Byte-identical: the frame was only ever read there.
3. **Builtin dispatch tables** (`call_named` + the `call` builtin) — the
   linear scans over `BUILTIN_SYNONYMS` (4 entries) and `BUILTIN_NAMES`
   (~120 entries) per call became match-based dispatch. The slices stay
   the source of truth; unit tests pin agreement.
4. **List operation fast paths** — list methods dispatch receiver-type-
   first through the shared `call_method` funnel (no per-call re-matching
   of the receiver); `push`/`pop`/`len` stay charged exactly as before.
   (Benchmark finding, §Benchmarks: the list path is NOT hot in the
   corpus workloads — the per-variable Env HashMap traffic dominates.
   locals-in-frame is the stage-2 win.)

## Gates (all green at time of writing)

| Gate | Result |
|---|---|
| `cargo test` | 51/51 (14 new optimizer unit tests included) |
| proof suite | 113 files / 99 proofs / 1302 asserts, ALL GREEN |
| differential harness (Rust vs Python oracle) | 144/144 |
| `scripts/vm_parity.sh` (VM O1 default vs tree-walk) | 218/218 |
| `scripts/opt_parity.sh` (6 configs × 218 corpus files) | 1308/1308 |
| `scripts/redteam.sh` | 100 contained / 0 breached |
| pkg e2e | 38/38 |
| clippy / fmt | 0 / clean |

`opt_parity.sh` runs the matrix `O0+fast0, O0, O1+fast0, O1, O2,
opt-passes=all+fast0` against the tree-walk over the whole corpus. One
payload (`rt_p4b_threadbomb_join`) is containment-checked rather than
byte-checked: its output message depends on which sandbox cap trips first
under OS scheduler load, which flips run-to-run on the SAME binary
(verified) — see the allowlist comment in `vm_parity.sh`.

## Bugs the gates caught during bring-up

1. **Signed-zero const dedupe** — `0.0 == -0.0` in IEEE; the pool reuse
   flipped `-0.0` output to `0.0` (rt_p5a/rt_p5e). Fix: bit-pattern dedupe
   for floats, in both the optimizer and the pre-existing compiler konst.
2. **Implicit-end targets** — jump targets of `insns.len()` are legal (the
   exec loop treats pc ≥ len as function end); unguarded indexing panicked
   on real corpus programs (security_caps, option_result, 27 more). Fix:
   bounds-checked threading + end-target remap.
3. **`Jmp(end)` mid-function is load-bearing** — only a trailing
   `Jmp(end)` may be dropped; elsewhere fallthrough would execute the
   insn the jump was skipping.

## Benchmarks

`scripts/bench_opt.sh` (`--quick` for N=3) times the corpus in
`scripts/bench/opt/` at every configuration with a median-of-N runner and
an output-identity gate inside the benchmark itself. Current numbers
(N=7 medians, this host): all configurations within ±1% — the passes are
parity-safe and code-size-reducing (e.g. `plan` 24 → 20 insns), but the
micro-corpus is dominated by per-variable Env HashMap traffic, not
dispatch. The stage-2 lever with real headroom is **locals-in-frame**
(allocate hot locals in a Vec frame instead of the scoped HashMap chain),
which feeds the same call-path work as monomorphic call specialization.
Per-pass insn-reduction is visible via `--dump-optimized`.
