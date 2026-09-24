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

### Measured composition (v2.2.0, `bash scripts/stack_report.sh`)

| rank | language | lines | share | role |
|---|---|---|---|---|
| 1 | **Rust** | 12,452 | ~46% | lexer, Total Grammar parser, evaluator, capability sandbox, HTTP/JSON, toolchain CLI, REPL, `operon-ls` LSP seed (`src/`) |
| 2 | **Operon** | 7,147 | ~27% | **self-hosted stdlib (6 modules), proof tests, red-team suite, differential corpus, GenomeLab** (`std/ tests/ examples/ apps/`) |
| 3 | **Python** | 3,617 | ~13% | bootstrap: reference oracle + differential harness (`bootstrap/`) — test infrastructure only, nothing shipped depends on it |
| 4 | **JavaScript** | 1,522 | ~6% | browser playground subset interpreter (`web/playground/app.js`) |
| 5 | **HTML** | 1,318 | ~5% | documentation site (`docs/`) |
| 6 | **CSS** | 311 | ~1% | docs + playground styling |
| 7 | **Shell** | 202 | <1% | build/test/bench/stack/install scripts (`scripts/`) |
| 8 | **C** | 187 | <1% | runtime kernel: intern table, arena, FNV-1a, clock (`runtime/operon_rt.c`) |
| 9 | **C++** | 148 | <1% | algorithm kernel: bit-parallel edit distance, codon-usage scoring (`runtime/codon_kernel.cpp`) |
| 10 | **TypeScript** | 23 | <1% | playground type surface (`app.d.ts`) |

**Honest deviations from the requested order, and why:**

1. **Python (2) > C (7).** The oracle is not "too much Python" — it is the differential engine that proves the Rust core correct (28/28 program-level output matches). Deleting it would save lines and lose verification. The C kernel is small because interning, hashing and clocks are small; it is load-bearing, not decorative — every identifier of every parsed file flows through it, `distance()`/`codon()` are C++ kernels, and `memory()`/`clock()` read C state directly.
2. **Operon (2) has overtaken everything except Rust.** Between v2.1.0 and v2.2.0 the `.op` share grew from ~7% to ~27% (proof suite, 59-payload red-team containment, 6-program differential corpus, stdlib). `std/` runs on the Rust core today; every release self-hosts more. That is exactly how Rust/Go/TS historically converged.

### Measured performance (Rust core vs Python oracle, same programs)

| bench | rust | oracle | speedup |
|---|---|---|---|
| fib(25) — 242k recursive calls | 127 ms | 3,119 ms | **24.5×** |
| 200k-iteration loop with arithmetic | 59 ms | 863 ms | **14.5×** |
| 16k string interpolations | 24 ms | 112 ms | **4.6×** |

(Reproduce: `bash scripts/bench.sh`.) Startup overhead included in both — the interpreter-only deltas are larger.

**Honest CPython comparison:** on the call-heavy fib(25) micro-benchmark the Rust tree-walking interpreter runs **~12× slower than CPython 3.12** (≈125 ms vs ≈10.5 ms compute-only, startup excluded on both sides; CPython's frame machinery is C). Tree-walkers lose to bytecode VMs; a bytecode compiler + VM in the Rust core is the v3 performance path and is why the differential harness exists — it will prove the VM against the tree-walker before it replaces it.

---

## New here? Learn it in an hour

Operon is built for programmers, not biologists — the gene vocabulary is flavor, not a
prerequisite. **[TUTORIAL.md](TUTORIAL.md)** takes you from `hello.op` to proof frames,
splice variants, regulation-as-feature-flags and the REPL, with every example verified
against the toolchain. Then try `operon repl` — `:help` is your friend.

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
| phenotypic differentiation | `phenotype Cell { let f = 0; gene init(v) { self.f = v } }` — `new Cell(1)`, inheritance `from`, `self` |
| polypeptide elongation | `sequence gen() { yield v }` — lazy `.next()` / `.collect()` on worker cells |
| gene regulatory networks | `regulate { a activates b strength 0.9; c inhibits d; }` — stateful, two-phase fire, optional `threshold` dose–response |
| toggle-switch bistability | `toggle a, b;` — exactly one on; calls to the repressed allele return null with a note |
| repressilator oscillation | `repressilator a -> b -> c period 3;` — manual ring or timed OS thread (`repressi_start(ms)`) |
| single-cell telemetry | `fingerprint()` — calls, burst index, mature / nascent / maturation |
| RNA interference (antiviral silencing) | capability sandbox — default-deny file/net/env/run; violations raise catchable Stress `interference` |
| CRISPR knockout screens | `operon crispr app.op --knockout fetch_data` — which proofs survive? (`--matrix` knocks out every gene) |

## Security model — the culture flask

An Operon program is an organism in a flask: the runtime is **default-deny** about the world outside. `read_file`, `write_file`, `append_file`, `exists`, `read_dir`, `run`, `http_get`, `serve`, and `env` raise a catchable Stress `interference` (the cell's RNA-interference machinery silencing an untrusted operation) unless the operator grants access:

```console
$ operon run app.op --allow-read /data --allow-write /tmp/out \
      --allow-run gzip --allow-net 127.0.0.1:8080 --allow-env API_KEY
$ operon run app.op --allow-all        # open flask
```

Path grants are symlink-resolved; a grant that normalizes to the empty string is rejected. A `.cell` config grants capabilities only when loaded explicitly with `--cell` — an auto-detected `operon.cell` cannot widen its own sandbox. Imports inside the project or the stdlib are always allowed; anything else needs a read grant. Resource ceilings are part of the same contract: recursion 10,000, step budget 200M (`--fuel N` lowers it), string repeat 512 MiB, edit-distance table 10M cells, JSON depth 512 — each a catchable Stress — and `sleep` is clamped to 60 s. Nothing crashes; see SPEC §9b.

## Toolchain

```
operon run f.op      [--entry g] [--variant v] [--cell c] [--rna r] [--frame n] [--ires] [--strict] [--fuel N]
                     [--allow-read p] [--allow-write p] [--allow-run prog] [--allow-net host:port] [--allow-env var] [--allow-all]
operon check f.op    [--nmd | --nmd=purge] [--json]   # 100-point grade + letter; --json: phantoms/nmd arrays
operon test [dirs]                                # proof-frame runner (26 files / 26 proofs / 281 assertions green)
operon fmt f.op     [--write]                     # precedence-correct canonical formatter
operon build f.op   [--variant v] [-o out.op]     # bake splices, strip proofs
operon profile f.op                               # per-gene calls, exclusive self-µs, flags, maturation, enhance candidates
operon crispr f.op  (--knockout gene | --matrix) [--json]
operon bench f.op   [--iters n]
operon-ls                                         # stdio LSP seed: diagnostics + gene-signature hover (SPEC §15)
operon version                                    # Operon 2.2.0 (rust-core, c-runtime, cpp-kernel) — banner matches SPEC 2.2.0
```

## Build from source

```console
$ ./scripts/build.sh          # gcc + g++ the kernels, rustc the core
OK: bin/operon
$ ./scripts/test.sh           # 22/22 proof files (262 assertions), C kernel smoke 14/14
$ python3 bootstrap/harness.py  # differential: 28/28 programs MATCH across implementations
$ bash scripts/redteam.sh       # adversarial containment: 59 attacks contained, 0 breached
$ bash scripts/bench.sh
```

Requires: rustc (≥1.70), gcc, g++. No crates, no network, no external dependencies.

## Language

- **Values**: null, bool, int(i64, overflow = catchable Stress), float, str (with `"interp {expr}"`), list, map (insertion-ordered), gene (closure), native.
- **Control**: `if/elif/else`, `while`, `loop`, `for…in`, `match/case` (literals, binding, wildcard), `for x in xs if cond collect body` comprehensions, `break/continue`, `return`.
- **Operators**: `**` (right-assoc pow), `& | ^ << >> ~` bitwise, `cond ? a : b` ternary, `int % int` → int.
- **Genes**: named, anonymous, lambdas, defaults, closures, recursion, `guard` clauses, marks — plus **phenotype classes** (`phenotype P { let f = 0; gene init() {…} }`, `new P(...)`, `self`, `phenotype C from P`) and **sequence generators** (`sequence s() { yield v }` with `.next()`/`.collect()`, lazy worker-cell pull).
- **Errors**: everything is a catchable `Stress{kind, message}` — `unfolded | missing | overflow | burned | interference` — plus Total-Grammar runtime notes. A program never crashes; worst case it narrates what it repaired.
- **Concurrency**: `spawn(gene, args)` / `join(id)` — real OS threads with value serialization; sequences run on worker cells; timed repressilator threads.
- **Modules**: `use std/bio;` — TAD-insulated, anchor-controlled exports, module cache, cycle-tolerant. Six stdlib modules today: `bio`, `collections`, `iter`, `math`, `seq`, `strings`.

## Repository layout

```
src/         Rust core (lexer, parser, interp, genes, tools, cli)
runtime/     C runtime kernel + C++ algorithm kernel
bootstrap/   Python oracle + differential harness (the verification layer)
std/         self-hosted Operon standard library (.op)
tools/       (reserved for .op tooling as self-hosting grows)
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
