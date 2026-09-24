# Operon

**The gene-expression language.** A Total Grammar language where nothing you write is ever rejected — it wobbles, repairs, and runs — implemented on a native Rust core with a C runtime kernel and a C++ algorithm kernel.

```operon
gene greet(name) {
    return "hello, {name}!"
}
promote(greet("operon"))        # hello, operon!
```

---

## The stack, rated honestly

The mandate: *"Rust, then C, then Python, then Operon, then C++, then HTML, then TS/JS, then others — rate it honestly, improve it, build it that way."*

**Verdict on that stack order before building: 9/10.** Evidence:

- A new language implemented in a systems language with a C runtime substrate is the mainstream pattern: CPython (C core), Lua (C), Ruby's YJIT (Rust), Rust v1 (hosted in OCaml before self-hosting). Rust-first gives parser/VM performance from day one.
- C second is correct: intern tables, arenas, hashing, clocks — the runtime kernel — are classic C territory (CPython does exactly this).
- Python as bootstrap (not implementation) is the right reduction from v1's 100%-Python mistake.
- One point withheld: **Operon below Python is a snapshot, not a destiny.** Mainstream languages converge on self-hosting (Rust in Rust, Go in Go, TypeScript in TypeScript). Operon's share must grow release over release — the stdlib is already pure `.op`.

### Measured composition (v2.0.0, `bash scripts/stack_report.sh`)

| rank | language | lines | share | role |
|---|---|---|---|---|
| 1 | **Rust** | 8,191 | ~47% | lexer, Total Grammar parser, evaluator, capability sandbox, HTTP/JSON, toolchain CLI (`src/`) |
| 2 | **Python** | 3,317 | ~19% | bootstrap: reference oracle + differential harness + packaging (`bootstrap/`) |
| 3 | **JavaScript** | 1,522 | ~9% | browser playground subset interpreter (`web/playground/app.js`) |
| 4 | **HTML** | 1,214 | ~7% | documentation site (`docs/`) |
| 5 | **Operon** | 1,188 | ~7% | **self-hosted stdlib (6 modules), tests, GenomeLab** (`std/ tests/ examples/ apps/`) |
| 6 | **CSS** | 311 | ~2% | docs + playground styling |
| 7 | **C** | 180 | ~1% | runtime kernel: intern table, arena, FNV-1a, clock (`runtime/operon_rt.c`) |
| 8 | **C++** | 148 | ~1% | algorithm kernel: bit-parallel edit distance, codon-usage scoring (`runtime/codon_kernel.cpp`) |
| 9 | **Shell** | 70 | <1% | build/test/bench/stack scripts (`scripts/`) |
| 10 | **TypeScript** | 23 | <1% | playground type surface (`app.d.ts`) |

**Honest deviations from the requested order, and why:**

1. **Python (2) > C (7).** The oracle is not "too much Python" — it is the differential engine that proves the Rust core correct (16/16 program-level output matches). Deleting it would save lines and lose verification. The C kernel is small because interning, hashing and clocks are small; it is load-bearing, not decorative — every identifier of every parsed file flows through it, `distance()`/`codon()` are C++ kernels, and `memory()`/`clock()` read C state directly. v2.1 grows C honestly (arena-backed GC, C map kernel for environments).
2. **Operon (6) is small — on purpose, for now.** `std/` runs on the Rust core today; every release self-hosts more. That is exactly how Rust/Go/TS historically converged.

### Measured performance (Rust core vs Python oracle, same programs)

| bench | rust | oracle | speedup |
|---|---|---|---|
| fib(25) — 242k recursive calls | 125 ms | 3,076 ms | **24.5×** |
| 200k-iteration loop with arithmetic | 65 ms | 861 ms | **13.2×** |
| 16k string interpolations | 26 ms | 114 ms | **4.3×** |

(Reproduce: `bash scripts/bench.sh`.) Startup overhead included in both — the interpreter-only deltas are larger.

**Honest CPython comparison:** on the call-heavy fib(25) micro-benchmark the Rust tree-walking interpreter runs ~5× slower than CPython's own bytecode VM (125 ms vs ~26 ms incl. startup; CPython's frame machinery is C). Tree-walkers lose to bytecode VMs; a bytecode compiler + VM in the Rust core is the v3 performance path and is why the differential harness exists — it will prove the VM against the tree-walker before it replaces it.

---

## Total Grammar — the 4-rung ladder

No `.op` file is ever rejected. Parse problems become **notes**:

1. **Canonical** — exact match.
2. **Synonym** — `fn/func/def → gene`, `print/echo → promote`, `var/val → let`, `yes/on → true`, `&& → and`, …
3. **Wobble** — a word at a keyword-required position within edit distance ≤ 2 of exactly one keyword is repaired (`les x = 2` runs as `let`). Powered by the C++ bit-parallel edit-distance kernel.
4. **Semantic fallback** — unknown identifiers read as `null`, phantom calls return `null`, unclosed braces auto-close, single quotes repair to double — all noted, all noted on stderr.

```operon
les x = 21              # wobble: repaired to 'let' — still runs
promote("x = {x * 2}")  # x = 42
```

```console
$ operon run bad.op
[fallback] unbound 'undefined_thing' read as null
$ operon check bad.op
operon check: bad.op — score 94/100 (grade A)
```

## The gene-expression regulation layer

Real mechanisms, real semantics — the professor-level feature set:

| mechanism | feature |
|---|---|
| wobble base pairing | 4-rung Total Grammar + codon scoring |
| overlapping reading frames | `frame proof { }` — tests encoded in the same file |
| alternative splicing | `splice name { variant a { } variant b { } }` + `--variant` / `.cell` |
| RNA editing | `.rna` hot patches: `edit target { replace "a" -> "b"; }` |
| upstream ORF repression | `gene f(x) guard (x > 0) else { … } { … }` |
| DNA methylation / epigenetics | `.cell` config layer + `methyl("k")` |
| TADs + CTCF anchors | `tad Name { anchor export f; }` — module insulation |
| super-enhancers | `enhance f, g;` — profile flags + grading bonus |
| histone marks | `@acetylate` (priority, silence-immune) · `@methylate` (repressed, soft-noted) |
| nonsense-mediated decay | `operon check --nmd` finds premature stops & dead transcripts; `--nmd=purge` rewrites |
| miRNA → RISC | `silence old_api -> new_api;` — calls redirect, noted |
| m6A modification | `@m6a` — dispatch priority among splice variants |
| IRES cap-independent entry | `ires alt_main;` + `--ires` |
| UPR / ISR stress programs | `stress { RISKY } rescue (e) { CONTAIN }` — kind-filtered, catchable overflow |
| fate landscapes | `fate Cycle { state a -> b; enter a; }` — `.shift()/.state()/.can()`, valley semantics |
| gene regulatory networks | `regulate { a activates b strength 0.9; c inhibits d; }` + `grn_fire/grn_state` |
| toggle-switch bistability | `toggle a, b;` — exactly one on |
| repressilator oscillation | `repressilator a -> b -> c period 3;` — manual ring or timed OS thread |
| single-cell telemetry | `fingerprint()` — calls, burst index, spliced/unspliced/velocity |
| CRISPR knockout screens | `operon crispr app.op --knockout fetch_data` — which proofs survive? |

## Toolchain

```
operon run f.op      [--entry g] [--variant v] [--cell c] [--rna r] [--frame n] [--ires] [--strict]
operon check f.op    [--nmd | --nmd=purge]        # 100-point grade + letter
operon test [dirs]                                # proof-frame runner (10 files / 10 proofs green)
operon fmt f.op     [--write]                     # precedence-correct canonical formatter
operon build f.op   [--variant v] [-o out.op]     # bake splices, strip proofs
operon profile f.op                               # per-gene calls, µs, flags, velocity, enhance candidates
operon crispr f.op --knockout gene [--json]
operon bench f.op   [--iters n]
operon version                                    # Operon 2.0.0 (rust-core, c-runtime, cpp-kernel)
```

## Build from source

```console
$ ./scripts/build.sh          # gcc + g++ the kernels, rustc the core
OK: bin/operon
$ ./scripts/test.sh           # 10/10 proof files, C kernel smoke 14/14
$ python3 bootstrap/harness.py  # differential: 16/16 programs MATCH across implementations
$ bash scripts/bench.sh
```

Requires: rustc (≥1.70), gcc, g++. No crates, no network, no external dependencies.

## Language

- **Values**: null, bool, int(i64, overflow = catchable Stress), float, str (with `"interp {expr}"`), list, map (insertion-ordered), gene (closure), native.
- **Control**: `if/elif/else`, `while`, `loop`, `for…in`, `match/case` (literals, binding, wildcard), `for x in xs if cond collect body` comprehensions, `break/continue`, `return`.
- **Genes**: named, anonymous, lambdas, defaults, closures, recursion, `guard` clauses, marks.
- **Errors**: everything is a catchable `Stress{kind, message}` — `unfolded | missing | overflow | burned` — plus Total-Grammar runtime notes. A program never crashes; worst case it narrates what it repaired.
- **Concurrency**: `spawn(gene, args)` / `join(id)` — real OS threads with value serialization; timed repressilator threads.
- **Modules**: `use std/bio;` — TAD-insulated, anchor-controlled exports, module cache, cycle-tolerant.

## Repository layout

```
src/         Rust core (lexer, parser, interp, genes, tools, cli)
runtime/     C runtime kernel + C++ algorithm kernel
bootstrap/   Python oracle + differential harness (the verification layer)
std/         self-hosted Operon standard library (.op)
tools/       (reserved for .op tooling as self-hosting grows)
std/seq.op std/math.op std/iter.op  (v2.1: self-hosted sequence/math/iter modules)
tests/       proof-frame test suite (.op) + C kernel smoke test
apps/        GenomeLab demo (pure .op)
examples/    tour programs (.op)
docs/        documentation site (HTML/CSS)
web/         browser playground (JS + TS declarations)
scripts/     build.sh · test.sh · bench.sh · stack_report.sh
packaging/   PyInstaller spec for the bootstrap path
```

## License

MIT — see [LICENSE](LICENSE).
