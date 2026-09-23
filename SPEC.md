# Operon v2.0 — Frozen Language Specification

**Status:** FROZEN for v2.0. This document is the single contract implemented identically by:

| Implementation | Language | Role |
|---|---|---|
| `src/` + `runtime/` | Rust + C + C++ | **Primary** compiler & runtime (the real thing) |
| `bootstrap/oracle.py` | Python | Reference oracle for differential testing + packaging |
| `web/playground/` | TypeScript/JS | Browser subset playground |

Three implementations MUST agree on every behavior below. Where they disagree, the differential test suite fails.

---

## 1. Identity

- Name: **Operon** (an operon = a cluster of genes transcribed as one unit under shared regulation).
- Extensions: `.op` source · `.cell` methylation config · `.rna` edit patches.
- Design law: **Total Grammar** — no `.op` file is ever rejected. Every token stream parses and runs.
  Parse problems degrade into **wobble notes** (logged repairs), never errors.
- Naming law: mechanism names from molecular biology only. **No scientist names anywhere** (code, docs, comments).

## 2. Values

`Null` · `Bool` · `Int(i64)` · `Float(f64)` · `Str` · `List` · `Map` (insertion-ordered) · `Gene` (function/closure) · `Native` (builtin function).

Truthiness (Python-style): `null`, `false`, `0`, `0.0`, `""`, `[]`, `{}` are falsy; all else truthy.

Equality `==` is deep (recurses into lists/maps). Int/Float compare numerically.
Ordering `< <= > >=` on numbers and strings; between incompatible types → raises Stress `unfolded` (catchable).

Arithmetic:
- `+ - *` numeric (int×float → float); `+` on two strings concatenates; `+` on two lists concatenates.
- `/` true division, always Float. `//` floor division (Int). `%` floored remainder, sign follows divisor.
- Integer overflow wraps? NO — overflow raises Stress `overflow` (catchable).
- `~` none. Bitwise ops: not in v2.0.

## 3. Lexical

- Comments: `#` to end of line. `#!` shebang allowed on line 1.
- Strings: `"double"`; escapes `\n \t \\ \" \{`; interpolation `"{expr}"` — any expression, evaluated at runtime, `str()`-coerced. No single-quoted strings in canonical form (a `'` in code is a wobble: treated as `"` with a note).
- Identifiers `[A-Za-z_][A-Za-z0-9_]*`.
- Numbers: `42`, `3.14`, `1e3` (float). Negative via unary minus.
- Newlines terminate statements; `;` allowed and ignored (also `;;`, stray). Blocks are `{ ... }`.
- Keywords (canonical):
  `gene let if elif else while loop for in return break continue match case use tad anchor export import enhance silence stress rescue raise fate state regulate activates inhibits strength toggle repressilator period frame proof guard splice variant edit replace apply true false null and or not collect ires`
- Marks: `@acetylate` `@methylate` `@m6a`.
- `#` inside a string does NOT start a comment.

## 4. Total Grammar — the 4-rung ladder

Every parse passes down the ladder; each rung below 1 emits **notes** (structured: line, rung, message, repair):

1. **Canonical** — exact keyword/grammar match. No notes.
2. **Synonym** — a known synonym table maps alternate spellings to canonical keywords:
   `fn func def fun sub lambda proc → gene` · `print echo say show → promote` (promote is builtin, synonyms map in parser to a `promote` call) · `var val const → let` · `elseif → elif` · `foreach each → for` · `import include require → use` · `ret → return` · `stop → break` · `continue next skip → continue` · `yes on → true` · `no off → false` · `null nil none nothing → null` · `&& → and` · `|| → or` · `! → not`.
3. **Wobble** — an identifier within edit distance ≤ 2 of exactly one keyword (≤ 1 if its length ≤ 4) is repaired to that keyword, with a note. Applies to marks too (`@acetylat` → `@acetylate`). Ambiguity (two keywords equidistant) → rung 4 for that token.
4. **Semantic fallback** — unknown bare identifier in expression position becomes the string literal of its own name + note ("unbound wobble"); stray tokens are skipped with notes; unclosed braces are auto-closed at EOF with notes; extra closers are skipped with notes; an unclosed string consumes to EOF with a note.

Runtime Total Grammar: reading an unbound variable → `Null` + note; calling a non-gene → `Null` + note. **A run never aborts on soft failures** — only `exit()` or an uncaught `raise`-style hard event ends it early (and even that prints a containment note first).

Unbound/undefined `gene` calls are reported by `operon check` as **phantom calls** (they run as Null at runtime).

## 5. Statements

```
let name = expr                     # definition (re-let with note "rebinding")
name = expr                         # assign (auto-let at top scope with note)
name += -= *= //= expr              # compound (also on a[i], a.k targets)
if expr { } elif expr { } else { }  # elif/else optional, elif chainable
while expr { }
loop { }                            # infinite; break/continue
for name in expr { }                # List→items, Str→1-char strs, Map→keys
return expr?                        # bare return → null
break / continue
match expr { case p { } ... }       # patterns: literals (==, comma-sep allowed),
                                    # ident (binds), _ (wildcard). First match wins.
use path (as name)?                 # file import → binds module Map (see §8)
raise expr                          # raise Stress{kind:"unfolded", message: str(e)}
raise kind , expr                   # kind ∈ unfolded missing overflow burned
stress (kind)? { B } rescue (e)? { R }   # containment, see §9
```

## 6. Expressions (precedence low → high)

1. `or` (`||`) — short-circuits, returns operand value (`a or b` → a if truthy else b)
2. `and` (`&&`) — same semantics
3. unary `not` (`!`) — `not a == b` parses as `not (a == b)`
4. comparisons `== != < <= > >=` and `in` (left-assoc, no chaining); `x in xs`: List membership, Str substring, Map key membership
5. `+ -`
6. `* / // %`
7. unary `-`
8. postfix: call `f(x)`, index `a[i]`, member `a.k`, method call `a.k(args)`
9. primary: literal, ident, `(expr)`, list `[a, b]`, map `{k: v, "k2": v}`, lambda, `collect` (§7)

Member access on Map → key lookup (missing → Null + note). Methods (see §10) are native.
`a.k = v` assigns map key. `a[i] = v` assigns list index (out of range → Stress `missing`, catchable).

## 7. Genes (functions) and lambdas

```
marks* gene name(p1, p2 = default) guard (cond) else { B } { body }
marks* gene (p1) => expr            # anonymous lambda
marks* gene (p1) { body }           # anonymous block lambda
let f = gene (x) => x * 2
```

- `marks` = `@acetylate | @methylate | @m6a` (§11).
- `guard` (uORF leading guard) sits between signature and body. If `cond` false: run `B`; if `B` returned a value other than null that value is the call result; otherwise result is Null + note "guard tripped".
- Closures capture the defining environment (by reference, like Python).
- Calls: positional only + defaults. Extra args → note, ignored. Missing args without default → Null + note (never a crash).
- Recursion allowed. Depth limit 10_000 → Stress `overflow`.

**collect expression** (comprehension): `for x in xs collect x * 2` is an expression evaluating to a List. Optional filter: `for x in xs if x > 0 collect x`.

## 8. Modules, TADs, anchors

- `use path;` — path like `std/bio`, `./util`, `util` (`.op` appended if absent). Resolution order: (1) relative to importing file's dir; (2) `./std/` under CWD; (3) `$OPERON_STD`. Binds one name: basename, or `as name`. `use std/bio as b;` → `b.some_fn(...)`. Importing the same file twice executes it once (module cache).
- A module evaluates to a Map of its **exported** names. Export rule:
  - If the file contains any `anchor export a, b;` (at top level or inside a tad), ONLY those names are exported.
  - With zero `anchor export` anywhere, all top-level genes and lets are exported (default-open).
- `tad Name { ... }` — topologically-associating domain: an insulation boundary. Names inside a tad escape ONLY via `anchor export` **inside that tad**. Tads may not nest (nesting degrades to a merged tad + note). A file may have several tads.
- `anchor import x;` — declares an external name the domain expects; purely declarative + `operon check` verifies it exists (note if not).
- Cyclic `use` → second import returns the partial module Map + note (no hang).

## 9. Stress containment (errors)

Runtime failures raise a **Stress** value: Map `{"kind": Str, "message": Str}`. Kinds:
`unfolded` (type errors), `missing` (bad index/key/member/null-deref), `overflow` (int overflow, depth limit), `burned` (assertion failures, resource errors), `any` (catch-all position only).

```
stress { RISKY } rescue (e) { promote("contained: {e.message}") }
stress missing { ... } rescue { ... }    # kind filter: only catches `missing`
```

- Unspecified kind catches any. Filtered kind catches only that kind; others propagate outward.
- `rescue` binding optional: `rescue (e)`. No rescue clause → stress is contained to Null + note (Top-Grammar runtime law).
- Uncaught at top level → printed as containment note, run continues (or ends that entry call with Null).
- `assert(cond, msg?)` raises Stress `burned` on failure (this is what proof frames catch).

## 10. Builtins and methods

**Builtins:** `promote(*a)` (print, space-joined, returns null) · `len` · `push(l,v)` · `pop(l)` · `insert(l,i,v)` · `remove(l,i)` · `keys(m)` · `values(m)` · `has(m,k)` · `del(m,k)` · `range(a, b?, step?)` (returns List) · `str` · `num` (fails → 0 + note) · `type` (`null bool int float str list map gene native`) · `abs min max sum` · `clock()` (seconds, float) · `exit(n?)` · `assert(c, msg?)` · `codon(s)` (0–100 style score of an identifier) · `distance(a,b)` (edit distance, C++ Myers kernel) · `similar(a,b,maxd?)` (bool) · `transcribe(dna)` · `translate(rna)` (stops at stop codon) · `reverse_complement(dna)` · `gc_content(dna)` (0–100) · `find_orf(dna)` (list of ORF proteins) · `memory()` (map `arena_bytes, interns, allocs` from the C runtime) · `fingerprint()` (run telemetry, §12) · `spawn(f, args?)` → id · `join(id)` → value · `toggle_on(name)` · `toggle_state()` · `repressi_next()` · `repressi_state()` · `grn_fire(name)` · `grn_state()` · `methyl(key, default?)`.

**Str methods:** `.upper() .lower() .trim() .split(sep) .join(list) .replace(a,b) .contains(x) .starts(x) .ends(x) .repeat(n) .slice(a,b) .len()`
**List methods:** `.map(f) .filter(f) .reduce(f, init) .each(f) .sort(cmp?) .reverse() .contains(x) .index_of(x) .slice(a,b) .join(sep) .len()` (cmp returns true when a before b)
**Map methods:** `.keys() .values() .items() .has(k) .del(k) .len()`

## 11. Gene-expression regulation layer (v2 core novelties)

All features are real, implemented, tested — none are decorative.

- **`tad` / `anchor`** — §8. Module insulation with export anchors.
- **`enhance a, b, c;`** — super-enhancer cluster: marks genes; `operon profile` shows the enhancement flag; `operon check` gives the file a codon-score bonus for enhanced hot genes.
- **`@acetylate`** — histone acetylation mark: gene is eager/priority; excluded from silence rewriting (active chromatin stays active); shown as `active` in profile.
- **`@methylate`** — histone methylation mark: gene is repressed; calls to it emit a soft note "methylated call" (suppressed when `.cell` sets `methylate.quiet = true`); excluded from docs.
- **`@m6a`** — m6A mark = dispatch priority: among same-name candidates (splice variants, shadowing), the m6a-marked one wins resolution.
- **`silence old -> new;`** — RISC-style silencing: after this statement, every call to gene `old` is redirected to `new` with a note "RISC: call silenced". `@acetylate` genes are immune. Same-name splice variants resolve first, then silencing applies to the resolved name.
- **NMD sweep** (`operon check --nmd`): "premature stops" = unreachable statements after an unconditional `return` (reported −4 each); "untranslated transcripts" = defined, never-called, non-exported, non-enhanced genes (info, −1). `--nmd purge` rewrites the file without them.
- **`splice name { variant a { } variant b { } }`** — alternative splicing: `name(...)` dispatches to the active variant. Selection order: `.cell` `variant.name=v` > CLI `--variant v` > `@m6a`-marked variant > first declared. `operon build --variant a` bakes one variant into a standalone file.
- **`.rna` edit patches** — `edit target { replace "src" -> "dst"; }` where target is a file name or gene name; applied to that target's source text before parsing. CLI: `operon run app.op --rna hot.rna`. Each applied replacement emits a note.
- **`.cell` methylation config** — `key = value` lines, `[section]` headers, `#` comments. CLI `--cell f.cell`; else `operon.cell` auto-detected. Read via `methyl("k", d?)`. Impl-consumed keys: `variant.<splice>`, `methylate.quiet`, `wobble.strict` (wobble ≥ rung 3 printed as warnings), `entry`.
- **`ires name;`** — internal ribosome entry site: declares a cap-independent entry gene. `operon run --ires` (or no `main`) runs the first declared `ires` target.
- **`fate Name { state a -> b, c; state b -> a; enter a; }`** — fate landscape state machine. `let m = Name()` enters the `enter` state (default: first declared). Methods on instances: `.shift(s)` (true if transition allowed; invalid → note + false, state unchanged — the valley stay), `.state()` (current), `.can(s)` (bool). Instances are Maps with hidden `#fate`/`#state` keys.
- **`regulate { a activates b strength 0.8; c inhibits d; }`** — gene regulatory network (default strength 1.0). `grn_fire("a")` sets `a = 1.0` and propagates in waves: activates → `child = max(child, parent × strength^wave)`, inhibits → `child = max(0, child − parent × strength^wave)`. `grn_state()` returns the level Map (all genes in the graph).
- **`toggle a, b;`** — bistable mutual-repression pair (genetic toggle switch). `toggle_on("a")` turns `a` on and `b` off; `toggle_state()` → Map of both. Exactly one may be on.
- **`repressilator a -> b -> c;`** — repressive oscillation ring. Manual mode (deterministic, for tests): `repressi_next()` advances the ring (each `next` represses the previous, activating the next), `repressi_state()` → Map of levels (on gene = 1.0, others 0.0). `repressilator a -> b -> c period 3;` + `repressi_start()` spawns a real OS thread flipping every 3 s (demo mode).

## 12. Frames, proofs, overlapping reading frames

- `frame proof { assert(...); ... }` — the **test reading frame** of the file. Skipped by `operon run`; executed by `operon test`. The same file encodes program + tests (two reading frames over one sequence).
- `frame name { ... }` — named frames (metadata/optional scenes); runnable via `operon run --frame name`.
- `operon test [paths...]` — default paths: `tests/` recursively. For each file: run its proof frames; a proof failure (Stress burned) is recorded; the suite continues (Total Grammar). Exit code 1 if any failure. Report: files, proofs run, passed, failed, wobble notes count.

## 13. Concurrency

- `spawn(f, args?)` — starts a real OS thread running gene `f`; returns task id (Int). `join(id)` waits and returns the result (second join → Null + note).
- Thread panics are impossible by construction: any stress inside the thread is returned as a Stress Map value.
- Memory model note (honesty): values are reference-counted; tasks communicate by args/results, not shared mutable state. Data races on shared globals are prevented by design (closures capture is by value at spawn time for non-local references).

## 14. Telemetry — the single-cell layer

- `fingerprint()` returns Map: `calls` (Map gene→count), `fano` (Map gene→variance/mean over call-time buckets; 0 for ≤1 call), `spliced` (count of genes executed ≥1), `unspliced` (defined but never executed), `velocity` (unspliced / total, 0 if none).
- `operon profile f.op` — runs instrumented, prints table: gene, calls, self-time µs, flags (`enhanced active repressed`), then spliced/unspliced/velocity summary and **enhance suggestions** (top unspliced-but-hot candidates).

## 15. Toolchain (Rust binary `operon`)

```
operon run f.op    [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires] [--strict] [--quiet] [--json]
operon check f.op  [--nmd] [--nmd=purge] [--json]
operon test [paths...]
operon fmt f.op    [--write]        # canonical formatter; wobble-corrected output parses clean
operon build f.op  --variant v -o out.op
operon profile f.op
operon crispr f.op --knockout gene [--json]   # knockout: body → return null; then run proofs; report survivors
operon bench f.op  [--iters n]
operon version
```

- Grading (`operon check`): start 100; wobble note −2; fallback note −3; NMD premature stop −4; untranslated transcript −1; phantom call −2; floor 50. Letter: A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60, else F.
- `--strict` → exit code 3 if any rung ≥ 3 note occurred.
- Stdout discipline: `promote` prints program output; notes/reports go to **stderr** (so `operon run f.op > out.txt` is clean). `--json` on check/test/profile/crispr emits machine-readable JSON to stdout.

## 16. Biology ↔ feature map (for docs; no scientist names)

| Mechanism (real molecular biology) | Operon feature |
|---|---|
| Wobble base pairing (redundant codon recognition) | 4-rung Total Grammar + synonym table |
| Codon optimality | `codon()` scoring in check grading |
| Overlapping reading frames | `frame proof` — tests and code in one sequence |
| Alternative splicing | `splice { variant }` + `--variant` / `.cell` selection |
| RNA editing | `.rna` hot patches (`edit/replace`) |
| Upstream ORF repression | `guard (cond) else { }` leading clauses |
| DNA methylation / epigenetics | `.cell` config layer + `methyl()` |
| TADs + CTCF anchors | `tad` domains + `anchor export/import` |
| Super-enhancers | `enhance` clusters |
| Histone acetylation / methylation | `@acetylate` / `@methylate` marks |
| Nonsense-mediated decay | NMD sweep (`--nmd`) |
| miRNA → RISC silencing | `silence old -> new;` |
| m6A modification | `@m6a` dispatch priority |
| IRES cap-independent entry | `ires name;` + `--ires` |
| UPR / ISR stress programs | `stress { } rescue { }` containment |
| Fate landscapes (valley semantics) | `fate` state machines |
| Gene regulatory networks | `regulate` + `grn_fire/grn_state` |
| Toggle switch bistability | `toggle a, b;` |
| Repressilator oscillation | `repressilator a -> b -> c` |
| Single-cell transcriptomics / Fano factor | `fingerprint()` call/noise telemetry |
| RNA velocity | spliced/unspliced/velocity metrics |
| CRISPR knockout screens | `operon crispr --knockout` |

## 17. Version

`operon version` → `Operon 2.0.0 (rust-core, c-runtime, cpp-kernel)`.
