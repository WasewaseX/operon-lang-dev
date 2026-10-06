# BENCH — the Operon benchmark suite

Owner: builder-A (this regeneration). Baseline: **v2.9.0** (main @ `189186e`),
re-measured and fully regenerated **2026-10-06** on the P6-wave head. This
file REPLACES the accumulated pre-wave report (that history lives in git:
`git show 189186e:BENCH.md`, plus the evidence archive in `docs/bench/`).

It is the measured answer to "how fast is Operon, where does the time go,
what was weak, what was fixed, and what is still open". Audience note
(D-008): programmer-first — `gene` ≈ a function, `regulate` ≈ a runtime
feature flag. No biology required.

---

## How to run

```sh
bash scripts/build.sh                     # produce bin/operon (rule 1: fresh binary)
bash scripts/bench.sh                     # named workloads: fib/loops/strings/collections/
                                          #   recursion/grn/json/regex/seq/large_map/file_io/modules
bash scripts/bench.sh --micro             # per-construct micro fixtures (scripts/bench/micro/)
bash scripts/bench.sh --json out.json     # machine-readable results
bash scripts/bench.sh --quick             # reduced iterations
```

Chunked passes (reaper-safe sandboxes / bounded foreground calls):

```sh
python3 scripts/bench/bench_chunk.py --named --names fib25,loops --json p1.json
python3 scripts/bench/bench_chunk.py --micro --names m_call,m_intadd --json p2.json
python3 scripts/bench/bench_merge.py full.json p1.json p2.json
```

Fixtures live in `scripts/bench/*.op`; correctness twins for every workload
run in the proof suite on every CI push (`tests/bench/bench_correctness.op`)
— a benchmark that measures the wrong answer fails CI before it can mislead
anyone.

## Methodology (and honesty notes)

- **Three runners on identical algorithms**: `operon` (the Rust core, VM lane
  by default), `oracle` (`bootstrap/oracle.py`, the CPython tree-walk
  semantic mirror — an architecture reference, not a speed target), and
  `native-py` (the same algorithm hand-written in pure CPython — the real
  "CPython-level speed" bar v3.0 targets).
- **Timing**: end-to-end process wall time, **min over N runs** (1 warmup;
  N=5 default, 3 for the oracle). `m_empty` measures the process floor
  (~1.1 ms operon, ~95 ms oracle startup).
- **Interleaved A/B for suspicion**: this box shows ±8–17% cross-session
  drift and ±1–3% run-to-run noise. Rows quoted as wins/regressions were
  confirmed with interleaved base/wave runs on the saved binaries; anything
  inside the noise band is labeled as such. `file_io` is a documented NOISY
  workload (W082) — its single-run rows are indicative only.
- **Names the lane.** The VM is the default engine; `--no-vm` is the
  tree-walk lane. A number without its lane is not a number.

## Environment (this baseline)

x86-64 Linux sandbox, Python 3.12.14, Rust 1.99.0, release profile
(`lto` codegen default per `Cargo.toml`), operon 2.9.0-vm (rust-core,
cpp-kernel). All BEFORE/AFTER rows in this file: same box, same day,
binaries saved and interleaved where it mattered.

## Results — named workloads (min-of-5, ms, end-to-end incl. startup)

| workload | main @ 189186e | P6-wave | Δ | op/py (wave) | class |
|---|---:|---:|---:|---:|---|
| fib25 | 102.8 | 102.0 | −0.8% (noise) | 8.0x | call-funnel bound (P4 floor) |
| loops | 63.0 | 54.5 | **−13.5%** | 4.9x | loop-env bound → fixed |
| strings | 24.0 | 28.3 | noise (interleaved −3.5%) | — (native 0.7 ms) | ratio-indicative |
| collections | 32.6 | 31.5 | −3.7% (noise) | 5.8x | mixed |
| recursion | 202.8 | 203.3 | +0.2% (noise) | 8.6x | call-funnel bound (P4 floor) |
| grn | 35.4 | 34.9 | −1.5% (noise) | 4.1x | regulation machinery |
| json | 72.5 | 79.5 | noise (interleaved +0.2%) | ~2.0x | codec-bound (native work) |
| regex | 76.1 | 77.5 | +1.7% (noise) | 3.1x | regex engine bound |
| seq | 32.6 | 32.0 | −1.8% (noise) | 11.4x | generator pull + string ops |
| large_map | 7.7 | 7.3 | −4.5% (indicative) | 3.7x | MapStore (P1 fixed) |
| file_io | 68.0 | 49.3 | NOISY class — indicative | 1.1x | I/O bound |
| modules | 64.0 | 62.2 | −2.9% (noise) | — | import/startup bound |

## Results — micro (per construct; ns/op = ms × 1e6 / documented ops)

| micro | ops | before ns/op | after ns/op | Δ | before op/py | after op/py |
|---|---:|---:|---:|---:|---:|---:|
| m_empty (startup floor) | — | 1.1 ms | 1.1 ms | −2.9% | — | — |
| m_call | 300k | 194 | 181 | **−6.5%** | 7.0x | 6.4x |
| m_forrange | 600k | 91 | 77 | **−15.2%** | 6.1x | 4.9x |
| m_while | 600k | 119 | 85 | **−28.7%** | 5.9x | 4.0x |
| m_varread | 600k | 102 | 88 | **−13.6%** | 6.8x | 5.4x |
| m_intadd | 900k | 98 | 87 | **−11.4%** | 6.6x | 5.8x |
| m_listpush | 150k | 113 | 115 | +1.8% (noise) | 7.4x | 7.5x |
| m_listidx | 300k | 124 | 111 | **−10.2%** | 4.9x | 4.3x |
| m_mapset | 80k | 208 | 213 | +2.4% (noise) | 2.9x | 3.0x |
| m_mapget | 154k | 177 | 169 | −4.8% | 3.5x | 3.3x |
| m_strcat | 24k | 167 | 171 | +2.5% (noise) | 5.9x | 5.9x |

The wins land exactly where the wave aimed: every loop-heavy construct
improved 10–29%, the call row moved via the Env::set/define paths it rides,
and the untouched rows (map store, strcat's in-place append, push's bridge)
held flat inside the noise band. No regression survived the interleaved A/B.

## The weakpoint audit (2026-10-06) — what was slow and exactly why

Instrumentation: W009A counters (`OPERON_W009A_COUNTS=1`), the P4 ablation
archive (`docs/bench/2026-10-02-w009a-ablation.txt`), and the three-engine
micro table above. Findings, ranked by measured damage:

### W1 — every loop iteration allocated a fresh Env (the big one)

`OPERON_W009A_COUNTS=1` on the micros: `env_new` = iterations + 2 for EVERY
loop shape — `for i in range(n)` (tree-walk and bridged), `while`, `loop`,
seq-pull and materialized iterables alike. 300k iterations minted 300,002
`Rc<Env>` objects: one allocation + one drop + a parent `Rc` clone/drop +
`RefCell` init per iteration, plus a first-insert HashMap capacity churn
per iteration. The VM's compiled `while` pays it through `EnterScope`
("fresh child env, tree-walk shape" — one per iteration); the `for` family
pays it in the tree-walk loop arms. Native CPython reuses one frame dict
across the whole loop; Operon allocated n of them.

**Why it was hard to fix naively**: closures. A gene defined in iteration i
captures that iteration's env — a shared/reused env would change what the
closure observes (per-iteration isolation → last-iteration-wins). The W011
A_POOL ablation proved map-recycling alone is a measured dead end (pool-ON
measured SLOWER on m_intadd/m_forrange/m_while — 90.7→95.0, 54.6→57.2,
69.5→73.1 ms — the TLS queue + RefCell take/put cost more than the fresh
allocs it replaces; the P4 ladder's "banked win" reading was a labeling
error, corrected here).

**The fix — capture-proof reuse**: each loop site owns a cache slot
(`loop_scope_env` in the tree-walk; a per-call local / per-frame
`scope_cache` in the VM's `EnterScope`/`ExitScope`). On scope exit, the
child env is stashed ONLY IF `Rc::strong_count == 1` — nothing captured it,
so the machine holds the sole reference — with both maps cleared
(capacity-preserving). On the next entry the cached env is reused instead
of minted. If anything captured the env, the count check fails and it
drops normally — the captured env keeps exactly the values its iteration
had, byte-identical to the fresh-Env lifetime. Per iteration cost after:
two map clears + a define, zero allocator traffic.

### W2 — a String malloc per StoreName/AssignName in both VM machines

`Instr::StoreName`/`Instr::AssignName` cloned the instruction's name
(`code.names[idx].clone()`) on every execution although `Env::define/set`
take `&str`. Every assignment in every VM program paid a malloc+free. The
name is now borrowed. `StoreName` additionally called `cur.get(&name)
.is_some()` for the rebinding note — `Env::get` CLONES the bound value just
to test presence; it now uses `Env::contains` (the clone-free chain walk
P5 added), so a `let` re-binding no longer clones the old value either.

### W3 — Env::set did two hash lookups and a throwaway key malloc

`set()` ran `contains_key(name)` then `insert(name.to_string(), val)`. With
the key already present — the loop-assignment case — the insert still
constructed the owned key String, hashed it, then dropped it (HashMap keeps
the original key). Now: one `get_mut` per chain level, rebind in place; the
miss path keeps the single owned insert (auto-decl semantics unchanged).

### W4 — `push(xs, i)` rides the full call funnel per push (measured, NOT fixed here)

`m_listpush` calls the bridged builtin every iteration: named-call funnel
gates + arg binding + the +49.8 ns bridge delta (P4 evidence) before the
actual `Vec::push`. The P6 wave left it untouched — the P4 profile already
sandboxed the fix direction ("builtin-call batching", an
`AppendName`-style compile-time shape match like P5's string append), and
it deserves its own wave with its own pins. Still the weakest remaining
micro at 7.5x native.

### Bonus find — the pins caught a real oracle bug (fixed)

Writing the wave's differential pin for closure-in-loop capture exposed a
pre-existing Rust↔oracle divergence the 3,5k-program corpus never
generated: a gene DEFINED INSIDE a loop body must capture ITS OWN
iteration's env (Rust: the env rides the value, `Value::Gene(def, env)` →
`0,1,2`). The oracle's gene arm mutated `g.closure` on the SHARED AST node,
so every instance of the same syntactic gene saw the LAST definition env
(`2,2,2`). The oracle now copies the gene per instance (shallow copy,
per-instance closure), byte-matching the core. Fixed in this wave, pinned
in `tests/differential/p6_loop_scope_pins.op` (c1/c1b), differential lanes
re-run green.

## The P6 wave — what landed

| fix | sites | mechanism |
|---|---|---|
| W1 loop-env reuse | tree-walk: `Stmt::For` (lazy-range, seq-pull, materialized), `Stmt::While`, `Stmt::Loop`; VM: `EnterScope`/`ExitScope` sync + fiber (`VmFrame::scope_cache`) | `Rc::strong_count == 1` capture-proof, clear-preserving-capacity reuse |
| W2 borrowed names | VM `StoreName`/`AssignName`, both machines | `&str` instead of `String::clone`; `contains()` instead of cloning `get()` |
| W3 single-lookup set | `Env::set` | `get_mut` rebind, no throwaway key |
| oracle capture fix | `bootstrap/oracle.py` gene/lambda arms | per-instance closure copy |

Pins: `tests/differential/p6_loop_scope_pins.op` — closure capture
(fallback path), per-iteration reset, no leak-out, const re-definition,
break/continue, nested loops, loop-var scoping. Byte-identical on VM lane,
tree-walk lane, and oracle.

### Gates (all green on the wave head)

- differential: main lane 3,508 match / 0 diverge; tree-walk lane 3,500 / 0
- proofs: 143 passed / 0 failed (2,285 asserts) + apps 3/3 (99 asserts)
- cargo tests: 129 passed / 0 failed (+ all sub-suites green)
- redteam: 109 contained / 0 breached
- vm parity: 3,569 identical, 7 divergent, 4 load-sensitive — the 7 are
  PRE-EXISTING documented mode-class divergences: mode signatures
  (`--no-vm`/default/`--opt 1`/`--opt 2`) verified byte-identical between
  the base and wave binaries per file
- doc-sync: stats pair carried mechanically (`test_op_files` → 3,576 = the
  actual tests/ walk; repairs a pre-existing −1 drift)

## Top remaining targets (input to the next waves)

1. **W4 builtin-call batching** — `push`/`pop`/`get`-in-loop compile to a
   dedicated opcode with the exact replaced-sequence fallback (the P5
   `AppendName` pattern). Projected: m_listpush 7.5x → ~4x; `collections`
   and `seq` ride along. Evidence direction already sanctioned by the P4
   profile ("the +49.8 ns bridge delta for builtin-call batching").
2. **P4 call floor (388 ns/call, owner-gated)** — fib25 8.0x and recursion
   8.6x are call-machinery bound: 1.5 envs/call, frame-slot alloc, per-arg
   binding 157–166 ns (P4 ablation archive). The lazy-fenv-class redesign
   stays owner-gated per §34 (second signature required); this wave's
   W1–W3 shaved the env paths the floor is made of (m_call −6.5%) without
   touching the call contract.
3. **m_strcat residual** — in-place append landed in the P5 wave (454 →
   ~170 ns/op); the remaining gap is dispatch + the per-op owned name in
   `AppendName`'s walk (borrowable — same W2 treatment, needs the fallback
   arms audited for ownership first).
4. **Oracle speed is NOT a target** — it is the semantic mirror; its 45 s
   on `collections`/`large_map` is the price of architecture parity, not
   an optimization surface.

## Startup floor

`scripts/bench_startup.sh` (n=30, this box, 2026-10-06):

| startup (exec-only / run-hello) | min | median | p90 |
|---|---:|---:|---:|
| exec-only | 1.42 ms | 1.54 ms | 1.62 ms |
| run-hello | 1.54 ms | 1.72 ms | 1.82 ms |

Unchanged by the P6 wave (expected: no startup-path code touched).

## Baseline tracking

| head | date | m_while ns/op | m_forrange | m_intadd | m_call | note |
|---|---|---:|---:|---:|---:|---|
| 189186e (v2.9.0) | 2026-10-06 | 119 | 91 | 98 | 194 | pre-P6-wave re-measure |
| P6-wave | 2026-10-06 | 85 | 77 | 87 | 181 | W1+W2+W3, gates green |

Cross-session absolute drift on this sandbox is real (+8–17%): never read
cross-session deltas as regressions — re-run interleaved A/B with saved
binaries, as this regeneration did.

## Reproducing

```sh
bash scripts/build.sh && bash scripts/bench.sh --json /tmp/now.json
bash scripts/bench.sh --micro --json /tmp/now_micro.json
OPERON_W009A_COUNTS=1 ./bin/operon run scripts/bench/micro/m_intadd.op   # env_new = iters+2 pre-wave
python3 scripts/perf_gate.py base.json head.json                          # regression gate
```
