# Operon

**The gene-expression language.** A Total Grammar language where nothing you write is ever rejected — it wobbles, repairs, and runs — implemented on a native Rust core with a small C++ codon kernel (bit-parallel Myers distance) for the hottest algorithms, and verified by a Python oracle + differential harness that never ships in the binary.

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

- A new language implemented in a systems language is the mainstream pattern: CPython (C core), Lua (C), Ruby's YJIT (Rust), Rust v1 (hosted in OCaml before self-hosting). Operon is Rust-first for parser/VM performance from day one.
- C second was the plan — until the sec-r2 audit proved the runtime kernel was write-only AND carried the project's one memory-safety class. Evidence beats aesthetics: interning moved into Rust, the C kernel was deleted, and the C++ algorithm kernel (which IS hot and pointer-free) stayed.
- Python as bootstrap (not implementation) is the right reduction from v1's 100%-Python mistake.
- One point withheld: **Operon below Python is a snapshot, not a destiny.** Mainstream languages converge on self-hosting (Rust in Rust, Go in Go, TypeScript in TypeScript). Operon's share must grow release over release — the stdlib is already pure `.op`.

### Measured composition (v2.2.0, `bash scripts/stack_report.sh`)

| rank | language | lines | share | role |
|---|---|---|---|---|
| 1 | **Rust** | 15,937 | ~46% | lexer, Total Grammar parser, evaluator, capability sandbox, symbol table, HTTP/JSON, toolchain CLI, REPL, `operon-ls` LSP seed (`src/`) |
| 2 | **Operon** | 17,248 | ~35% | **self-hosted stdlib (15 modules incl. `std/motifs`, `std/set`, `std/testing`, `std/random`), proof tests, red-team suite, differential corpus, GenomeLab** (`std/ tests/ examples/ apps/`) |
| 3 | **Python** | 5,129 | ~15% | bootstrap: reference oracle + differential harness (`bootstrap/`) — test infrastructure only, nothing shipped depends on it |
| 4 | **JavaScript** | 1,834 | ~5% | browser playground subset interpreter (`web/playground/app.js`) |
| 5 | **HTML** | 1,263 | ~4% | documentation site (`docs/`) |
| 6 | **CSS** | 293 | ~1% | docs + playground styling |
| 7 | **Shell** | 311 | ~1% | build/test/bench/stack/install scripts (`scripts/`) |
| 8 | **C++** | 134 | <1% | algorithm kernel: bit-parallel edit distance, codon-usage scoring (`runtime/codon_kernel.cpp` + its smoke driver) |
| 9 | **TypeScript** | 18 | <1% | playground type surface (`app.d.ts`) |

**Honest deviations from the requested order, and why:**

1. **Python (2) > C++ (8).** The oracle is not "too much Python" — it is the differential engine that proves the Rust core correct (128/128 program-level output matches). Deleting it would save lines and lose verification.
2. **The C kernel was deleted on purpose (sec-r2, audit A15).** The audit proved it was write-only (the lexer discarded every intern result) and that its raw-pointer arena was the project's one ASan-confirmed memory-safety class. Interning now lives in Rust (`src/ffi.rs`): same stable-id semantics, `memory()` still reports table stats, and the entire UAF class is structurally impossible. The C++ codon kernel STAYED because it earned its place: bit-parallel Myers is genuinely hot (`distance()`, `similar()`, wobble repair, parser suggestions), allocation-free, and budget-guarded.
3. **Operon (2) has overtaken everything except Rust.** Between v2.1.0 and v2.2.0 the `.op` share grew from ~7% to ~27% (proof suite, red-team containment, differential corpus, stdlib). `std/` runs on the Rust core today; every release self-hosts more. That is exactly how Rust/Go/TS historically converged.

### The substrate statement (D-010, substrate-r1)

Operon is not built from zero, and never was. The language stands ON three ecosystems, each earning its layer with evidence:

- **Rust — the toolchain substrate.** Lexer, Total Grammar parser, evaluator, capability sandbox, REPL, LSP, release binaries. This is the CPython/Lua pattern (systems-language core) with memory safety included.
- **C++ — the algorithm kernel substrate.** The codon kernel survived two audits because it is genuinely hot, allocation-free, and budget-guarded (bit-parallel Myers edit distance). The C kernel that did NOT earn its place was deleted.
- **Python — the ecosystem substrate (new in substrate-r1).** `py(module, "func", [args])` is a capability-gated bridge to the scientific Python stack: one granted call to NumPy, SciPy, Biopython, pandas. Grant per module (`--allow-py numpy`), isolated-mode child, same containment contract as `run()` (timeout kill, fuel-charged wall time, 64 MiB output cap, scrubbed env). This is the architectural answer to "your libraries are too small": a stdlib does not need 500,000 modules of its own — it needs a hard, sandboxed door to the ecosystem that already has them. Operon joins computational biology's Python majority instead of fighting it.

And one deliberate NON-dependency: **Racket is not in the stack — by evidence and by choice.** An external review claimed Operon depended on Racket; the measured stack (`scripts/stack_report.sh`) contains zero `.rkt` lines. We keep it that way: depending on a small-ecosystem language as infrastructure would import exactly the risk that review warned about. What we take from Racket is its idea — language-oriented programming, a tower of notations — which Total Grammar already embodies natively (four parse rungs, wobble repair, proof frames). Ideas travel; fragile dependencies don't.

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

## Cookbook — small real programs to steal from

`examples/cookbook/` holds 18 runnable programs (14 everyday recipes + 4 gotcha tours),
each ≤80 lines with a header that says what it teaches. Every output is deterministic
and **verified by `bash scripts/cookbook.sh` on the Rust core AND the Python oracle** —
an example that rots or diverges fails CI instead of lying to you.

| program | you already know it as | teaches |
|---|---|---|
| [word_freq](examples/cookbook/word_freq.op) | `Counter(text.split()).most_common(3)` | maps as counters, `sort_by_key`, pad_left |
| [csv_report](examples/cookbook/csv_report.op) | csv module + groupby + tabulate | quote-aware parsing, `num()` for fields, fmt_table |
| [json_transform](examples/cookbook/json_transform.op) | `json.loads` + dict surgery | dot-path get, merge/pick/omit, flatten, brace-free JSON |
| [cli_greet](examples/cookbook/cli_greet.op) | argparse / clap | flags, `--key=value`, subcommands via std/args |
| [fsm_vending](examples/cookbook/fsm_vending.op) | a state machine as dict-of-dicts | transition tables, terminal-state reset |
| [monte_carlo_pi](examples/cookbook/monte_carlo_pi.op) | random sampling on the unit square | deterministic LCG, float math, fmt_fixed |
| [bank_account](examples/cookbook/bank_account.op) | a class + try/catch | guard clauses, `raise`, `stress/rescue` containment |
| [text_stats](examples/cookbook/text_stats.op) | `wc -lwc` + longest word | words/chars/lines, running max, vowels via chars() |
| [gradebook](examples/cookbook/gradebook.op) | pandas describe, minus the install | mean/median/stddev, group_by, top-score lookup |
| [lru_cache](examples/cookbook/lru_cache.op) | `functools.lru_cache` | map + recency list, eviction, hit/miss counters |
| [matrix_ops](examples/cookbook/matrix_ops.op) | nested-list matmul | transpose, dot products, row printing |
| [primes](examples/cookbook/primes.op) | a sieve + Goldbach pairs | boolean-array sieves, list `contains` scans |
| [roman](examples/cookbook/roman.op) | an encoder/decoder pair | parallel-table greedy encode, subtractive decode |
| [caesar](examples/cookbook/caesar.op) | `str.translate` ROT13 | substitution tables from parallel strings, round-trips |
| [gotcha_braces](examples/cookbook/gotcha_braces.op) | f-string escaping | every `{` interpolates; raw braces via `chr(123)`; no `\r` escape |
| [gotcha_synonyms](examples/cookbook/gotcha_synonyms.op) | reserved words | `off/next/type/show…` repair to keywords/aliases — safe naming |
| [gotcha_builtins_not_values](examples/cookbook/gotcha_builtins_not_values.op) | `print` is a value in Python | builtins read as null; wrap them in a gene |
| [gotcha_division](examples/cookbook/gotcha_division.op) | Python `/` vs `//` vs `%` | floored modulo, cyclic indexing, money math |

Run any of them: `operon run examples/cookbook/word_freq.op`. Re-verify all:
`bash scripts/cookbook.sh`.

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

Real mechanisms, real semantics — the professor-level feature set (fidelity is per-row; SPEC §16 carries the term audits):

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
| histone marks | `@acetylate` (permissiveness: silence-immune, not priority) · `@methylate` (graded marks) |
| nonsense-mediated decay | `operon check --nmd` finds premature stops & dead transcripts; `--nmd=purge` rewrites |
| miRNA → RISC | `silence old_api -> new_api;` — calls redirect, noted |
| m6A modification | `@m6a` — dispatch priority among splice variants |
| IRES cap-independent entry | `ires alt_main;` + `--ires` |
| UPR / ISR stress programs | `stress { RISKY } rescue (e) { CONTAIN }` — kind-filtered, catchable overflow |
| fate landscapes | `fate Cycle { state a -> b; enter a; }` — `.shift()/.state()/.can()`, valley semantics |
| phenotypic differentiation | `phenotype Cell { let f = 0; gene init(v) { self.f = v } }` — `new Cell(1)`, inheritance `from`, `self` |
| polypeptide elongation | `sequence gen() { yield v }` — lazy `.next()` / `.collect()` on worker cells |
| gene regulatory networks | `regulate { a activates b strength 0.9; c inhibits d; }` — stateful, two-phase fire, optional `threshold` dose–response |
| mutual-repression latch (toggle switch) | `toggle a, b;` — exactly one on; calls to the repressed allele return null with a note |
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
operon test [dirs]                                # proof-frame runner (80 files / 74 proofs / 930 assertions green)
operon fmt f.op     [--write]                     # precedence-correct canonical formatter
operon build f.op   [--variant v] [-o out.op]     # bake splices, strip proofs
operon profile f.op                               # per-gene calls, exclusive self-µs, flags, maturation, enhance candidates
operon crispr f.op  (--knockout gene | --matrix) [--json]
operon bench f.op   [--iters n]
operon-ls                                         # stdio LSP: diagnostics, hover, definition, symbols, completion, formatting (SPEC §15)
operon version                                    # Operon 2.2.0 (rust-core, cpp-kernel) — banner matches SPEC 2.2.0
```

## Connect your editor (operon-ls)

`operon-ls` is a zero-dependency stdio language server shipped in every release archive (and in the Windows zip since lsp-r1). It wires the real Total Grammar parser and the `check` engine into your editor — the same diagnostics you get from `operon check`, no second implementation:

- **Diagnostics on open/change** — parse notes per Total Grammar rung (canonical → hint, synonym → info, wobble → warning, fallback → error) plus phantom-call detection that resolves `use`d modules **independently of the launch directory** (document-relative → CWD → `std/` → exe-relative `std/`).
- **Hover** — gene/seq signatures with regulation marks (`@acetylate`, `@methylate`, `@m6a`, `enhance`), splice variant tables, builtin signatures.
- **Go-to-definition** for genes, sequences, and splice roots.
- **Document symbols** — the file's callable inventory in the outline view.
- **Completion** — in-file genes with signatures, builtins, keywords, top-level bindings.
- **Formatting** — the same canonical formatter as `operon fmt`.

Neovim (nvim-lspconfig, any version ≥ 0.8):

```lua
vim.lsp.start({
  name = 'operon-ls',
  cmd = { 'operon-ls' },
  filetypes = { 'operon' },
})
```

VS Code (minimal client via any LSP client extension, e.g. "LSP Support"):

```json
{ "operon-ls": { "command": "operon-ls", "args": [], "languages": ["operon"] } }
```

Helix (`~/.config/helix/languages.toml`):

```toml
[[language]]
name = "operon"
scope = "source.operon"
file-types = ["op"]
language-servers = ["operon-ls"]

[language-server.operon-ls]
command = "operon-ls"
```

Ranged (incremental) edits are ignored by design while `textDocumentSync = 1` (full-text) is advertised — well-behaved editors always send full text on change.

## Install

Every channel below carries an honest validation mark (the full ledger lives in [docs/PACKAGING.md](docs/PACKAGING.md)):

| channel | how | mark |
|---|---|---|
| GitHub release (linux x64+arm64, macos x64+arm64, windows) | download `operon-<v>-<target>.tar.gz` / `.zip` + verify the companion `.sha256` | **validated** — per-artifact release smoke in CI |
| install script | `curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh \| sh` | community (runs on your machine) |
| from source | `./scripts/build.sh` or `cargo install --path .` | **validated** — the CI cargo gate builds this exact path |
| cargo-binstall / Homebrew / winget | metadata + drafts | staged — land with the B5 stack merge (docs/PACKAGING.md) |
| Scoop (Windows) | `packaging/scoop/operon.json` | community draft |
| AUR (release / git) | `packaging/aur/PKGBUILD` · `packaging/aur/PKGBUILD.git` | community drafts |
| Nix | `packaging/nix/default.nix` | community draft |
| deb / rpm | `[package.metadata.deb]` / `[package.metadata.generate-rpm]` in Cargo.toml | community drafts |

Every archive ships `operon`, `operon-ls`, and the self-hosted `std/` library. `validated` means a CI job or smoke script in this repo exercises the channel today; `community` means a maintainer must pin the checksum and verify at publish time — "should work" is not a state we write down.

## Build from source

```console
$ ./scripts/build.sh          # gcc + g++ the kernels, rustc the core
OK: bin/operon
$ ./scripts/test.sh           # proof suite (generated counts: docs/STATS.md), C++ kernel smoke
$ python3 bootstrap/harness.py  # differential corpus (counts: docs/STATS.md) — must MATCH across implementations
$ bash scripts/redteam.sh       # adversarial containment (payload count: docs/STATS.md) — 0 breached is the gate
$ bash scripts/bench.sh
```

Requires: rustc (≥1.70), gcc, g++ (builds the C++ codon kernel). Zero runtime crates — the only build dependency is `cc` for the kernel build step; nothing touches the network unless you grant it (`py()` bridge, `run()`).

## Language

- **Values**: null, bool, int(i64, overflow = catchable Stress), float, str (with `"interp {expr}"`), list, map (insertion-ordered), gene (closure), native, **Option/Result variants** (`some/none/ok/err` — expected failures are values, D-014).
- **Control**: `if/elif/else`, `while`, `loop`, `for…in`, `match/case` (W02 match-v2: variant payload patterns `Some(x)`/`Ok(v)`, list patterns `[a, *rest]`, map patterns `{k, j: p}`, or-patterns `a | b`, guards `p if cond` — a shape that cannot match falls through, never fails), `for x in xs if cond collect body` comprehensions, `break/continue`, `return`.
- **Regulation as execution**: `regulate` networks gate calls; `operon` units give polycistronic transcription with polarity, and opt-in Rho-dependent termination with a ribosome-queue coupling shield (`rho.termination` in a `.cell`).
- **Operators**: `**` (right-assoc pow), `& | ^ << >> ~` bitwise, `cond ? a : b` ternary, `e?!` Result/Option propagation (Rust-style: Some/Ok unwraps, None/Err returns from the gene — a return, never a failure), `int % int` → int.
- **Genes**: named, anonymous, lambdas, defaults, closures, recursion, `guard` clauses, marks — plus **soft type annotations** (`gene f(x: int) -> int`, `let n: float`, unions `int | str`, optionals `T?` — boundary-checked as catchable Stress, W01/SPEC §7c) and **phenotype classes** (`phenotype P { let f = 0; gene init() {…} }`, `new P(...)`, `self`, `phenotype C from P`) and **sequence generators** (`sequence s() { yield v }` with `.next()`/`.collect()`, lazy worker-cell pull).
- **Errors**: a four-tier hierarchy (SPEC §9) — null+note (soft miss) → **Option/Result values** (`some/none/ok/err`, `?!` propagation, `unwrap_or` defaults) → catchable `Stress{kind, message}` — `unfolded | missing | overflow | burned | interference | unwrap` — plus Total-Grammar runtime notes. Expected failures stay values; Stress is for contract violations. A program never crashes; worst case it narrates what it repaired.
- **Concurrency**: `spawn(gene, args)` / `join(id)` — real OS threads with value serialization; sequences run on worker cells; timed repressilator threads.
- **Modules**: `use std/bio;` — TAD-insulated, anchor-controlled exports, module cache, cycle-tolerant. Fifteen stdlib modules today: `args`, `bio`, `collections`, `csv`, `fmt`, `fs`, `iter`, `json`, `math`, `motifs`, `random`, `seq`, `set`, `strings`, `testing` — plus the capability-gated `py()` bridge for the scientific-Python deep end.

## Repository layout

```
src/         Rust core (lexer, parser, interp, genes, tools, cli)
runtime/     C++ codon kernel (codon_kernel.cpp, built by cc; the legacy C kernel was deleted in sec-r2 — audit A15 — Rust FFI bridge lives in src/ffi.rs)
bootstrap/   Python oracle + differential harness (the verification layer)
std/         self-hosted Operon standard library (.op)
tools/       (reserved for .op tooling as self-hosting grows)
tests/       proof-frame test suite (.op) + codon kernel smoke test
apps/        GenomeLab demo (pure .op)
examples/    tour programs (.op)
docs/        documentation site (HTML/CSS)
web/         browser playground (JS + TS declarations)
scripts/     build.sh · test.sh · bench.sh · stack_report.sh
packaging/   distribution channels: Scoop manifest, AUR PKGBUILD (release+git), Nix derivation (ledger: docs/PACKAGING.md)
```

## License

MIT — see [LICENSE](LICENSE).
