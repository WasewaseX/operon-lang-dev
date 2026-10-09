# Operon

**The gene-expression language.** A Total Grammar language where nothing you write is ever rejected, it wobbles, repairs, and runs, implemented on a native Rust core with a small C++ codon kernel (bit-parallel Myers distance) for the hottest algorithms, and verified by a Python oracle + differential harness that never ships in the binary.

```operon
gene greet(name) {
    return "hello, {name}!"
}
promote(greet("operon"))        # hello, operon!
```

---

## The stack, rated honestly

The mandate: *"Rust, then C, then Python, then Operon, then C++, then HTML, then TS/JS, then others, rate it honestly, improve it, build it that way."*

**Verdict on that stack order before building: 9/10.** Evidence:

- A new language implemented in a systems language is the mainstream pattern: CPython (C core), Lua (C), Ruby's YJIT (Rust), Rust v1 (hosted in OCaml before self-hosting). Operon is Rust-first for parser/VM performance from day one.
- C second was the plan, until the sec-r2 audit proved the runtime kernel was write-only AND carried the project's one memory-safety class. Evidence beats aesthetics: interning moved into Rust, the C kernel was deleted, and the C++ algorithm kernel (which IS hot and pointer-free) stayed.
- Python as bootstrap (not implementation) is the right reduction from v1's 100%-Python mistake.
- One point withheld: **Operon below Python is a snapshot, not a destiny.** Mainstream languages converge on self-hosting (Rust in Rust, Go in Go, TypeScript in TypeScript). Operon's share must grow release over release, the stdlib is already pure `.op`.

### Measured composition (main @ 34bb34d, `bash scripts/stack_report.sh`)

| rank | language | lines | share | role |
|---|---|---|---|---|
| 1 | **Operon** | 435,834 | ~85% | **self-hosted stdlib (35 modules, generated inventory in [docs/STATS.md](docs/STATS.md)), proof tests, red-team suite, differential corpus, GenomeLab + the ytdl app (incl. the cross-language bench surface)** (`std/ tests/ examples/ apps/`) |
| 2 | **Rust** | 61,460 | ~12% | lexer, Total Grammar parser, evaluator, capability sandbox, symbol table, HTTP/JSON, toolchain CLI, REPL, `operon-ls` LSP (`src/`) |
| 3 | **Python** | 9,904 | ~1.9% | bootstrap: reference oracle + differential harness (`bootstrap/`), test infrastructure only, nothing shipped depends on it |
| 4 | **Shell** | 3,329 | <1% | build/test/bench/stack/install scripts (`scripts/`) |
| 5 | **JavaScript** | 2,062 | ~0.4% | browser playground subset interpreter (`web/playground/app.js`) |
| 6 | **HTML** | 1,347 | ~0.3% | documentation site (`docs/`) |
| 7 | **CSS** | 311 | <1% | docs + playground styling |
| 8 | **C++** | 148 | <1% | algorithm kernel: bit-parallel edit distance, codon-usage scoring (`runtime/codon_kernel.cpp` + its smoke driver) |
| 9 | **TypeScript** | 23 | <1% | playground type surface (`app.d.ts`) |

**Honest deviations from the requested order, and why:**

1. **Python (rank 3) > C++ (rank 8).** The oracle is not "too much Python", it is the differential engine that proves the Rust core correct (differential harness, generated counts in [docs/STATS.md](docs/STATS.md)). Deleting it would save lines and lose verification.
2. **The C kernel was deleted on purpose (sec-r2, audit A15).** The audit proved it was write-only (the lexer discarded every intern result) and that its raw-pointer arena was the project's one ASan-confirmed memory-safety class. Interning now lives in Rust (`src/ffi.rs`): same stable-id semantics, `memory()` still reports table stats, and the entire UAF class is structurally impossible. The C++ codon kernel STAYED because it earned its place: bit-parallel Myers is genuinely hot (`distance()`, `similar()`, wobble repair, parser suggestions), allocation-free, and budget-guarded.
3. **Operon (2) has overtaken everything except Rust.** Between v2.1.0 and v2.2.0 the `.op` share grew from ~7% to ~27% (proof suite, red-team containment, differential corpus, stdlib). `std/` runs on the Rust core today; every release self-hosts more. That is exactly how Rust/Go/TS historically converged.

### The substrate statement (D-010, substrate-r1)

Operon is not built from zero, and never was. The language stands ON three ecosystems, each earning its layer with evidence:

- **Rust, the toolchain substrate.** Lexer, Total Grammar parser, evaluator, capability sandbox, REPL, LSP, release binaries. This is the CPython/Lua pattern (systems-language core) with memory safety included.
- **C++, the algorithm kernel substrate.** The codon kernel survived two audits because it is genuinely hot, allocation-free, and budget-guarded (bit-parallel Myers edit distance). The C kernel that did NOT earn its place was deleted.
- **Python, the ecosystem substrate (new in substrate-r1).** `py(module, "func", [args])` is a capability-gated bridge to the scientific Python stack: one granted call to NumPy, SciPy, Biopython, pandas. Grant per module (`--allow-py numpy`), isolated-mode child, same containment contract as `run()` (timeout kill, fuel-charged wall time, 64 MiB output cap, scrubbed env). This is the architectural answer to "your libraries are too small": a stdlib does not need 500,000 modules of its own, it needs a hard, sandboxed door to the ecosystem that already has them. Operon joins computational biology's Python majority instead of fighting it.

And one deliberate NON-dependency: **Racket is not in the stack, by evidence and by choice.** An external review claimed Operon depended on Racket; the measured stack (`scripts/stack_report.sh`) contains zero `.rkt` lines. We keep it that way: depending on a small-ecosystem language as infrastructure would import exactly the risk that review warned about. What we take from Racket is its idea, language-oriented programming, a tower of notations, which Total Grammar already embodies natively (four parse rungs, wobble repair, proof frames). Ideas travel; fragile dependencies don't.

### Measured performance (Rust core vs Python oracle, same programs)

| bench | rust | oracle | speedup |
|---|---|---|---|
| fib(25), 242k recursive calls | 127 ms | 3,119 ms | **24.5×** |
| 200k-iteration loop with arithmetic | 59 ms | 863 ms | **14.5×** |
| 16k string interpolations | 24 ms | 112 ms | **4.6×** |

(Reproduce: `bash scripts/bench.sh`.) Startup overhead included in both, the interpreter-only deltas are larger.

**Honest CPython comparison:** on the call-heavy fib(25) micro-benchmark the Rust tree-walking interpreter runs **~12× slower than CPython 3.12** (≈125 ms vs ≈10.5 ms compute-only, startup excluded on both sides; CPython's frame machinery is C). Tree-walkers lose to bytecode VMs; a bytecode compiler + VM in the Rust core is the v3 performance path and is why the differential harness exists, it will prove the VM against the tree-walker before it replaces it.

---

## New here? Learn it in an hour

Operon is built for programmers, not biologists, the gene vocabulary is flavor, not a
prerequisite. **[TUTORIAL.md](TUTORIAL.md)** takes you from `hello.op` to proof frames,
splice variants, regulation-as-feature-flags and the REPL, with every example verified
against the toolchain. Then try `operon repl`, `:help` is your friend.

Want types? Operon runs **dynamic** (scripting, REPL) and adds an opt-in
**typed mode**: `gene add(a: Int, b: Int) -> Int`, generics (`gene first<T>
(items: list[T]) -> T?`), trait contracts, Option/Result with `?!`, and
match-exhaustiveness — all checked at compile time (`operon run --typed`,
`operon check --typed`) while the dynamic side stays byte-identical. The
flagship catch: `x = 10; x.name()` is rejected before the program starts.
See [docs/design/TYPED-MODE.md](docs/design/TYPED-MODE.md) and SPEC §16a.

---

## Cookbook, small real programs to steal from

`examples/cookbook/` holds 24 runnable programs (20 everyday recipes + 4 gotcha tours),
each ≤80 lines with a header that says what it teaches, plus the [packages chapter](examples/cookbook/packages.sh),
a shell transcript that walks the package CLI end to end. Every output is deterministic
and **verified by `bash scripts/cookbook.sh` on the Rust core AND the Python oracle**
(the packages chapter runs on the Rust core only; the oracle is an interpreter, not a
package manager). An example that rots or diverges fails CI instead of lying to you.

| program | you already know it as | teaches |
|---|---|---|
| [greeting](examples/cookbook/greeting.op) | argparse "hello world" | positional args, flags, defaults via `std/args` |
| [wordcount](examples/cookbook/wordcount.op) | `wc -w` + `Counter` top-3 | `std/strings` + `std/iter`, `sort_by_key`, `fold` |
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
| [gotcha_synonyms](examples/cookbook/gotcha_synonyms.op) | reserved words | `off/next/type/show…` repair to keywords/aliases, safe naming |
| [gotcha_builtins_not_values](examples/cookbook/gotcha_builtins_not_values.op) | `print` is a value in Python | builtins read as null; wrap them in a gene |
| [gotcha_division](examples/cookbook/gotcha_division.op) | Python `/` vs `//` vs `%` | floored modulo, cyclic indexing, money math |
| [option_pipeline](examples/cookbook/option_pipeline.op) | Rust's `Option`/`Result` + `?` | expected failures as values, `?!` propagation, `unwrap_or` defaults |
| [json_safety_net](examples/cookbook/json_safety_net.op) | `json.loads` inside try/except per record | `try_json_parse`, `try_get` field gates, rejections counted by reason |
| [log_sieve](examples/cookbook/log_sieve.op) | `re.match` groups + guarded slicing | `try_re_groups` captures, `try_char_at` positional reads, fixed range Errs |
| [edge_stack](examples/cookbook/edge_stack.op) | `stack.pop()` with bounds checks | the edge `try_*` family driving one machine; failures as loop-branch values |

The packages chapter is a transcript, not a `.op` program: [examples/cookbook/packages.sh](examples/cookbook/packages.sh)
executes the whole workflow for real (`operon new` → `add` → `tree` → `run` → `test` → `verify`)
inside a throwaway sandbox (`HOME`/`OPERON_DEPS`/`OPERON_REGISTRY_HOME` pointed at a temp
dir, the same pattern as `scripts/pkg_e2e.sh`) and diffs the frozen session transcript.
Same gate, applied to the CLI.

Run any of them: `operon run examples/cookbook/word_freq.op` (or
`bash examples/cookbook/packages.sh` to print the package session). Re-verify all:
`bash scripts/cookbook.sh`.

---

## Reading the errors

Diagnostics are part of the language contract, not an afterthought: stable
error codes (E1xxx for fatals, E2xxx for repairs), located blocks with
width-correct carets, did-you-mean suggestions that mirror the runtime
wobble ladder, machine-applicable fixes, and `--json-errors` for tools.
The guide is [docs/errors.md](docs/errors.md); the byte-exact fixtures are
part of the standard gate suite.

## Debugging

`operon debug` is a statement-level interactive debugger: line AND
conditional breakpoints, step-into/over/out, one-shot run-to-line, live
breakpoint management, variable assignment (`set`), frame variables and
expression evaluation — plus a machine surface for tools: `--protocol=json`
(NDJSON on stdio) and `operon dap` (the Debug Adapter Protocol adapter, so
VS Code and every other DAP client debug Operon natively — with
`stopOnEntry`, conditional breakpoints and `setVariable`; the extension
lives in `editors/vscode/`). Stack frames report real call-site lines. The
guide is [docs/DEBUGGER.md](docs/DEBUGGER.md); all three surfaces have
standing e2e gates.

## Fuzzing

The Total Grammar promise ("every input must not crash") is held against
inputs nobody wrote yet, not only the ones people wrote: three
deterministic-seed fuzz lanes (mutation-based, differential at scale,
exec-surface with capability-escape generation) run locally in seconds and
in CI on every push. A finding is a bug to fix, never a number to brag
about. The lane contract, the tool table, and the recorded baselines live
in [docs/FUZZING.md](docs/FUZZING.md); triage is
[scripts/fuzz/TRIAGE.md](scripts/fuzz/TRIAGE.md).

## Total Grammar, the 4-rung ladder

No `.op` file is ever rejected. Parse problems become **notes**:

1. **Canonical**, exact match.
2. **Synonym**, `fn/func/def → gene`, `print/echo → promote`, `var/val → let`, `yes/on → true`, `&& → and`, …
3. **Wobble**, a word at a keyword-required position within edit distance ≤ 2 of exactly one keyword is repaired (`les x = 2` runs as `let`). Powered by the C++ bit-parallel edit-distance kernel.
4. **Semantic fallback**, unknown identifiers read as `null`, phantom calls return `null`, unclosed braces auto-close, single quotes repair to double, all noted, all noted on stderr.

```operon
les x = 21              # wobble: repaired to 'let', still runs
promote("x = {x * 2}")  # x = 42
```

```console
$ operon run bad.op
[fallback] unbound 'undefined_thing' read as null
$ operon check bad.op
repair:
  bad.op: 1 note(s), 1 wobble(s), 0 fallback(s); run `operon explain bad.op` for the play-by-play
summary: 0 error(s), 0 warning(s), 0 style, 1 repair note(s)
```

Deprecating a gene (W064): mark it once, callers get a check warning with your
migration text, the runtime never changes:

```operon
@deprecated("use twice() instead", since="2.4")
gene old_double(n) { return n * 2 }
```

```console
$ operon check app.op
warning  app.op: 7: call to deprecated gene 'old_double' (since 2.4), use twice() instead, silence with '// allow: deprecated-use'
```

## The gene-expression regulation layer

Real mechanisms, real semantics — the professor-level feature set (fidelity is per-row; docs/spec/BIO-CONTRACT.md grades every mechanism, docs/spec/MODELING-NOTES.md carries the term audits, docs/GENE-EXPRESSION-PARITY.md is the end-to-end comparison against real molecular biology):

| mechanism | feature |
|---|---|
| wobble base pairing | 4-rung Total Grammar + codon scoring |
| overlapping reading frames | `frame proof { }`, tests encoded in the same file |
| alternative splicing | `splice name { variant a { } variant b { } }` + `--variant` / `.cell` |
| RNA editing | `.rna` hot patches: `edit target { replace "a" -> "b"; }` |
| upstream ORF repression | `gene f(x) guard (x > 0) else { … } { … }` |
| DNA methylation / epigenetics | `.cell` config layer + `methyl("k")` |
| TADs + CTCF anchors | `tad Name { anchor export f; }`, module insulation |
| super-enhancers | `enhance f, g;`, profile flags + grading bonus |
| histone marks | `@acetylate` (permissiveness: silence-immune, not priority) · `@methylate` (graded marks) |
| nonsense-mediated decay | `operon check --nmd` finds premature stops & dead transcripts; `--nmd=purge` rewrites |
| miRNA → RISC | `silence old_api -> new_api;`, calls redirect, noted |
| m6A modification | `@m6a`, dispatch priority among splice variants |
| IRES cap-independent entry | `ires alt_main;` + `--ires` |
| UPR / ISR stress programs | `stress { RISKY } rescue (e) { CONTAIN }`, kind-filtered, catchable overflow |
| fate landscapes | `fate Cycle { state a -> b; enter a; }`, `.shift()/.state()/.can()`, valley semantics |
| phenotypic differentiation | `phenotype Cell { let f = 0; gene init(v) { self.f = v } }`, `new Cell(1)`, inheritance `from`, `self` |
| polypeptide elongation | `sequence gen() { yield v }`, lazy `.next()` / `.collect()` on worker cells |
| gene regulatory networks | `regulate { a activates b strength 0.9; c inhibits d; }`, stateful, two-phase fire, optional `threshold` dose–response |
| mutual-repression latch (toggle switch) | `toggle a, b;`, exactly one on; calls to the repressed allele return null with a note |
| repressilator oscillation | `repressilator a -> b -> c period 3;`, manual ring or timed OS thread (`repressi_start(ms)`) |
| single-cell telemetry | `fingerprint()`, calls, burst index, mature / nascent / maturation |
| RNA interference (antiviral silencing) | capability sandbox, default-deny file/net/env/run; violations raise catchable Stress `interference` |
| CRISPR knockout screens | `operon crispr app.op --knockout fetch_data`, which proofs survive? (`--matrix` knocks out every gene) |

## Security model, the culture flask

An Operon program is an organism in a flask: the runtime is **default-deny** about the world outside. `read_file`, `write_file`, `append_file`, `exists`, `read_dir`, `run`, `http_get`, `serve`, and `env` raise a catchable Stress `interference` (the cell's RNA-interference machinery silencing an untrusted operation) unless the operator grants access:

```console
$ operon run app.op --allow-read /data --allow-write /tmp/out \
      --allow-run gzip --allow-net 127.0.0.1:8080 --allow-env API_KEY
$ operon run app.op --allow-all        # open flask
```

Path grants are symlink-resolved; a grant that normalizes to the empty string is rejected. A `.cell` config grants capabilities only when loaded explicitly with `--cell`, an auto-detected `operon.cell` cannot widen its own sandbox. Imports inside the project or the stdlib are always allowed; anything else needs a read grant. Resource ceilings are part of the same contract: recursion 10,000, step budget 200M (`--fuel N` lowers it), string repeat 512 MiB, edit-distance table 10M cells, JSON depth 512, each a catchable Stress, and `sleep` is clamped to 60 s. Nothing crashes; see SPEC §9b.

## Toolchain

```
operon run f.op      [--entry g] [--variant v] [--cell c] [--rna r] [--frame n] [--ires] [--strict] [--fuel N]
                     [--vm | --vm-opt]             # bytecode VM / VM + semantics-preserving optimizer
                     [--allow-read p] [--allow-write p] [--allow-run prog] [--allow-net host:port] [--allow-env var] [--allow-all]
operon check f.op    [--nmd | --nmd=purge] [--json]   # 100-point grade + letter; --json: phantoms/nmd arrays
operon lint f.op [f2.op ...]
                     [--strict] [--allow r1,r2] [--cell c] [--json]
                     # W48 split: style/quality only — unused genes/imports/bindings, dead consts,
                     # shadowed bindings, constant-condition, infinite-loop-suspect, unreachable code,
                     # duplicate/unreachable match-arms; --strict: any finding = exit 3
operon test [dirs]                                # proof-frame runner (files/proofs/assertions: docs/STATS.md)
operon fmt f.op     [--write]                     # precedence-correct canonical formatter
operon fix f.op     [--write] [--json]            # legacy-surface migrator (s:: → dot, synonym canonicalization), dry-run default
operon ast f.op     [--json]                      # s-expression AST dump (the formatter's printer)
operon keywords     [--json]                      # the reserved-word × behavior table (W55)
operon doc f.op|dir [-o outdir] [--json]          # ## doc comments → markdown (W73/W74; docs/api/ is generated this way)
operon graph f.op   [--json]                      # regulation network → graphviz dot (activates/inhibits edges, strength/threshold)
operon rna f.op patch.rna
                     [--write] [--check] [--json]
                     # AST patch; --check validates only (would_apply, ambiguity, comment preflight — W68, writes nothing);
                     # `syntax: v2` patches are node-addressed (W067)
operon watch f.op   [args...]                     # re-run on every file change
operon build f.op   [--variant v] [-o out.op]     # bake splices, strip proofs
operon profile f.op [--chrome trace.json]         # per-gene calls, exclusive self-µs, flags, maturation, enhance candidates; --chrome writes a Chrome-trace .json of per-call spans (docs/PROFILING.md)
operon crispr f.op  (--knockout gene | --matrix) [--json]
operon bench f.op   [--iters n]
operon disasm f.op                                # bytecode listing of compiled gene bodies
operon-ls                                         # stdio LSP: diagnostics, hover, definition, references, rename, semantic tokens, inlay hints, symbols, completion, formatting (SPEC §15)
operon version                                    # Operon 2.10.0-vm (rust-core, cpp-kernel), banner matches SPEC 2.7.0; -vm = the bytecode machine is the run default (W09 A6)
```

## Reliability — the compatibility contract

Reliability outranks features. The rule: **every optimization must
preserve semantics**, and every engine pairing must agree **byte-for-byte**
(stdout, stderr, exit code), including diagnostics. The machinery that
enforces this (docs/COMPAT.md):

- **5 engine axes**: tree-walk / bytecode VM / optimized VM / debug build /
  Python oracle — 1,200-program generated corpus + 217 pinned programs,
  all identical across every pairing (`scripts/compat_matrix.sh`).
- **3-way VM parity**: every repo program byte-identical on tree-walk,
  `--vm`, and `--vm-opt` (`scripts/vm_parity.sh`).
- **Differential harness**: 1,343 programs Rust-vs-oracle
  (`bootstrap/harness.py`).
- **Parser fuzzer**: 1,000 hostile inputs per run (random bytes, unicode
  salad, token salad, corpus mutations, depth bombs) — no panics, no hangs,
  no containment breaches, engines agree (`scripts/fuzz_parser.py`).
- **CI matrix**: Linux/macOS/Windows × release/debug × x86-64/aarch64/i686
  (`.github/workflows/compat.yml`).

The optimizer gate earned its keep the day it shipped: the first `--vm-opt`
build panicked on a redteam payload (a legal jump-to-end target was
indexed past the end insn); the corpus caught it, the fix landed, the
corpus re-greened. That is the intended workflow.

## Packages — the first-class workflow

The day-one verbs a developer expects, all backed by `operon.toml` +
`operon.lock` (resolved revs + sha256 checksums, byte-stable across
machines; docs/specs/REGISTRY.md):

```
operon new myapp          # scaffold: manifest, src/main.op, a green smoke test
operon test               # runs the smoke proof
operon add http           # pull a package from the registry — zero flags
operon add web            # web depends on http: the closure handles it
operon run src/main.op
operon tree               # what is resolved, at which rev, from where
operon verify             # every vendored tree matches its checksum
operon remove web
operon update             # re-resolve the whole closure
operon install            # cold-start: fetch + materialize every pinned dep (operon.toml + operon.lock; seed-registry path re-materializes offline)
operon mod …              # the same package verbs, explicit spelling (mod add/install/verify/tree/…)
operon publish            # append your package to a registry index
```

`operon add NAME` needs no setup: a seed registry (http, json, postgres,
web — real, tested, pure-Operon packages) ships inside the binary and
materializes on first use, so the first add works offline. Point
`OPERON_REGISTRY` at your own index (a file, or an http(s) URL) or set
`[registry] path` in operon.toml to publish and consume your own. CI pins
`operon run --locked`, which fails on any manifest-lockfile drift, and
`operon registry serve DIR` stands up a read-only HTTP registry in one
command.

The whole flow, every command run for real and gate-verified:
[examples/cookbook/packages.sh](examples/cookbook/packages.sh), the cookbook's
packages chapter. Depth lives in docs/specs/REGISTRY.md (registry format,
resolution chain, hosting) and docs/PACKAGING.md (how the `operon` binary
itself reaches machines).

## Connect your editor (operon-ls)

`operon-ls` is a zero-dependency stdio language server shipped in every release archive (and in the Windows zip since lsp-r1). It wires the real Total Grammar parser and the `check` engine into your editor, the same diagnostics you get from `operon check`, no second implementation:

- **Diagnostics on open/change**, parse notes per Total Grammar rung (canonical → hint, synonym → info, wobble → warning, fallback → error) plus phantom-call detection that resolves `use`d modules **independently of the launch directory** (document-relative → CWD → `std/` → exe-relative `std/`).
- **Hover**, gene/seq signatures with regulation marks (`@acetylate`, `@methylate`, `@m6a`, `enhance`), splice variant tables, builtin signatures.
- **Go-to-definition** for genes, sequences, and splice roots.
- **Find references**, word-boundary occurrences that skip strings and comments, declaration included.
- **Rename** (via prepareRename), all-or-nothing: invalid, reserved, or already-taken names refuse the whole rename with a clear error.
- **Semantic tokens**, a fixed six-type legend (keyword/function/variable/string/number/comment).
- **Inlay hints**, the checker's inferred types on un-annotated `let`/`const` bindings (concrete types only — a dynamic `any` binding stays hint-free by design).
- **Document symbols**, the file's callable inventory in the outline view.
- **Completion**, in-file genes with signatures, builtins, keywords, top-level bindings.
- **Formatting**, the same canonical formatter as `operon fmt`.

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

Ranged (incremental) edits are ignored by design while `textDocumentSync = 1` (full-text) is advertised, well-behaved editors always send full text on change.

## Install

Every channel below carries an honest validation mark (the full ledger lives in [docs/PACKAGING.md](docs/PACKAGING.md)):

| channel | how | mark |
|---|---|---|
| GitHub release (linux x64+arm64, macos x64+arm64, windows) | download `operon-<v>-<target>.tar.gz` / `.zip` + verify the companion `.sha256` | **validated**, per-artifact release smoke in CI |
| install script | `curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh \| sh` | community (runs on your machine) |
| from source | `./scripts/build.sh` or `cargo install --path .` | **validated**, the CI cargo gate builds this exact path |
| cargo-binstall | `cargo binstall operon` (metadata in Cargo.toml) | community, template contract pinned by the standing gate (docs/PACKAGING.md) |
| Homebrew formula | `packaging/homebrew/operon.rb` | community draft, builds the version tarball from source |
| source release archive | `scripts/release.sh` → `dist/operon-<v>.tar.gz` + `SHA256SUMS` (+ `--verify`) | validated; binary releases also ship a whole-release `SHA256SUMS` (workflow-enforced, `install.sh --verify` consumes it) |
| Scoop (Windows) | `packaging/scoop/operon.json` | community draft |
| AUR (release / git) | `packaging/aur/PKGBUILD` · `packaging/aur/PKGBUILD.git` | community drafts |
| Nix | `packaging/nix/default.nix` | community draft |
| hosted registry server (W19-r2) | `packaging/registry/app.py` · `packaging/registry/requirements.txt` · `packaging/registry/render.yaml` | community draft, self-hosted tier (docs/specs/REGISTRY.md); HTTP + WSGI surfaces pinned by the standing gate |
| deb / rpm | `[package.metadata.deb]` / `[package.metadata.generate-rpm]` in Cargo.toml | community drafts |

Every archive ships `operon`, `operon-ls`, and the self-hosted `std/` library. `validated` means a CI job or smoke script in this repo exercises the channel today; `community` means a maintainer must pin the checksum and verify at publish time, "should work" is not a state we write down.

## Build from source

```console
$ ./scripts/build.sh          # gcc + g++ the kernels, rustc the core
OK: bin/operon
$ ./scripts/test.sh           # proof suite (generated counts: docs/STATS.md), C++ kernel smoke
$ python3 bootstrap/harness.py  # differential corpus (counts: docs/STATS.md), must MATCH across implementations
$ bash scripts/redteam.sh       # adversarial containment (payload count: docs/STATS.md), 0 breached is the gate
$ bash scripts/bench.sh
```

Requires: rustc (≥1.70), gcc, g++ (builds the C++ codon kernel). Zero runtime crates, the only build dependency is `cc` for the kernel build step; nothing touches the network unless you grant it (`py()` bridge, `run()`).

## Language

- **Values**: null, bool, int(i64, overflow = catchable Stress), float, str (with `"interp {expr}"`), list, map (insertion-ordered), gene (closure), native, **Option/Result variants** (`some/none/ok/err`, expected failures are values, D-014).
- **Control**: `if/elif/else`, `while`, `loop`, `for…in`, `match/case` (W02 match-v2: variant payload patterns `Some(x)`/`Ok(v)`, list patterns `[a, *rest]`, map patterns `{k, j: p}`, or-patterns `a | b`, guards `p if cond`, a shape that cannot match falls through, never fails), `for x in xs if cond collect body` comprehensions, `break/continue`, `return`.
- **Regulation as execution**: `regulate` networks gate calls; `operon` units give polycistronic transcription with polarity, and opt-in Rho-dependent termination with a ribosome-queue coupling shield (`rho.termination` in a `.cell`).
- **Operators**: `**` (right-assoc pow), `& | ^ << >> ~` bitwise, `cond ? a : b` ternary, `e?!` Result/Option propagation (Rust-style: Some/Ok unwraps, None/Err returns from the gene, a return, never a failure), `int % int` → int.
- **Genes**: named, anonymous, lambdas, defaults, closures, recursion, `guard` clauses, marks, plus **soft type annotations** (`gene f(x: int) -> int`, `let n: float`, unions `int | str`, optionals `T?`, boundary-checked as catchable Stress, W01/SPEC §7c) and **phenotype classes** (`phenotype P { let f = 0; gene init() {…} }`, `new P(...)`, `self`, `phenotype C from P`) and **sequence generators** (`sequence s() { yield v }` with `.next()`/`.collect()`, lazy worker-cell pull).
- **Immutability**: `const x = e` is a deep-frozen binding (W05/SPEC §7d), every reachable container is registered frozen at bind time; mutation through ANY alias (reassignment, index/member set, `push/pop/insert/remove/del`, `unwrap`-ed payloads) burns catchable Stress `frozen`. `let mut x = e` is the documentation-only mutability marker. Reads never burn; read-only programs run byte-identically.
- **Traits**: `trait Show { gene display() }` + `phenotype User implements Show` (W04/SPEC §8b), required methods (contract-checked at construction, notes never fatal) and default methods with virtual dispatch; composes with `from` inheritance.
- **Errors**: a four-tier hierarchy (SPEC §9), null+note (soft miss) → **Option/Result values** (`some/none/ok/err`, `?!` propagation, `unwrap_or` defaults) → catchable `Stress{kind, message}`, `unfolded | missing | overflow | burned | interference | unwrap | frozen`, plus Total-Grammar runtime notes. Expected failures stay values; Stress is for contract violations. A program never crashes; worst case it narrates what it repaired.
- **Concurrency**: `spawn(gene, args)` / `join(id)`, real OS threads with value serialization; sequences run on worker cells; timed repressilator threads.
- **Modules**: `use std/bio;`, TAD-insulated, anchor-controlled exports, module cache, cycle-tolerant. 35 stdlib modules today (generated per-module inventory: [docs/STATS.md](docs/STATS.md)): `args`, `arrays`, `bigint`, `binary`, `bio`, `biocore`, `collections`, `csv`, `deque`, `env`, `fmt`, `fs`, `graph`, `hashing`, `heap`, `iter`, `json`, `logging`, `math`, `motifs`, `path`, `process`, `random`, `result`, `seq`, `serialize`, `set`, `strings`, `try_numeric`, `typed`, `terminal`, `testing`, `time`, `unicode`, `url`, plus the capability-gated `py()` bridge for the scientific-Python deep end. `std/bigint` is the sanctioned escape past the i64 no-wrap overflow contract: exact arbitrary-precision arithmetic over digit lists (20! fits i64, 21! does not, `bigint.big_fact` answers both exactly).

## Repository layout

```
src/         Rust core (lexer, parser, interp, genes, tools, cli)
runtime/     C++ codon kernel (codon_kernel.cpp, built by cc; the legacy C kernel was deleted in sec-r2, audit A15, Rust FFI bridge lives in src/ffi.rs)
bootstrap/   Python oracle + differential harness (the verification layer)
std/         self-hosted Operon standard library (.op)
tools/       (reserved for .op tooling as self-hosting grows)
tests/       proof-frame test suite (.op) + codon kernel smoke test
apps/        GenomeLab demo + ytdl, a downloader app in pure .op (language-comparison builds: apps/ytdl-compare, docs/APP-COMPARISON.md)
examples/    tour programs + cookbook recipes + embed/typed examples (.op)
docs/        documentation site (HTML/CSS)
web/         browser playground (JS + TS declarations)
scripts/     build.sh · test.sh · bench.sh · stack_report.sh
packaging/   distribution channels: Scoop manifest, AUR PKGBUILD (release+git), Nix derivation (ledger: docs/PACKAGING.md)
```

## License

MIT, see [LICENSE](LICENSE).
