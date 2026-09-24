# Operon v2.1 — Language Specification

**Status:** v2.2.0. This document is the single contract implemented identically by:

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
- `/` true division, always Float. `//` floor division (Int). `%` floored remainder, sign follows divisor; `int % int` returns Int.
- `**` power, right-associative (`2 ** 3 ** 2` → 512), binds tighter than unary minus on its left (`-2 ** 2` → −4); int base with a non-negative int exponent → Int, otherwise Float.
- Bitwise: `& | ^` (and, or, xor), `<< >>` shifts, `~` bitwise not — Int operands, Int results.
- Integer overflow wraps? NO — overflow raises Stress `overflow` (catchable).

## 3. Lexical

- Comments: `#` to end of line. `#!` shebang allowed on line 1.
- Strings: `"double"`; escapes `\n \t \\ \" \{ \}`; interpolation `"{expr}"` — any expression, evaluated at runtime, `str()`-coerced. No single-quoted strings in canonical form (a `'` in code is a wobble: treated as `"` with a note).
- Identifiers `[A-Za-z_][A-Za-z0-9_]*`.
- Numbers: `42`, `3.14`, `1e3` (float). Negative via unary minus.
- Newlines terminate statements; `;` allowed and ignored (also `;;`, stray). Blocks are `{ ... }`.
- Keywords (canonical, 51 — the parser's reserved set):
  `gene let if elif else while loop for in return break continue match case use tad anchor export import enhance silence stress rescue raise fate state regulate activates inhibits strength toggle repressilator period frame proof guard splice variant edit replace ires as collect enter phenotype sequence yield new threshold from self`
- Literal words `true false null` and the logical words `and or not` are recognized in expression positions (not part of the reserved keyword table).
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
for name in expr { }                # List→items, Str→1-char strs, Map→keys,
                                    # sequence → pulled values (§7b)
return expr?                        # bare return → null
break / continue
match expr { case p { } ... }       # patterns: literals (==, comma-sep allowed),
                                    # ident (binds), _ (wildcard). First match wins.
use path (as name)?                 # file import → binds module Map (see §8)
raise expr                          # raise Stress{kind:"unfolded", message: str(e)}
raise kind , expr                   # kind ∈ unfolded missing overflow burned
stress (kind)? { B } rescue (e)? { R }   # containment, see §9 (kinds incl.
                                    # `interference`, §9b — sandbox denials)

Name { B }                          # bare-name block: a gene definition with no
                                    # params (rung-4 note) — `main { }` is the
                                    # C-like entry idiom
```

## 6. Expressions (precedence low → high)

1. `cond ? a : b` — ternary, right-associative, lowest precedence
2. `or` (`||`) — short-circuits, returns operand value (`a or b` → a if truthy else b)
3. `and` (`&&`) — same semantics
4. unary `not` (`!`) — `not a == b` parses as `not (a == b)`
5. comparisons `== != < <= > >=` and `in` (left-assoc, no chaining); `x in xs`: List membership, Str substring, Map key membership
6. bitwise OR `|`
7. bitwise XOR `^`
8. bitwise AND `&`
9. shifts `<< >>`
10. `+ -`
11. `* / // %`
12. unary `-` and `~` (bitwise not)
13. `**` power — right-associative, binds tighter than unary minus on its left
14. postfix: call `f(x)`, index `a[i]`, member `a.k`, method call `a.k(args)`
15. primary: literal, ident, `(expr)`, list `[a, b]`, map `{k: v, "k2": v}`, lambda, `collect` (§7), `new Name(args)` (§7a)

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

## 7a. Phenotypes (classes)

```
phenotype Name {
    let field = value                # field with a default value (any expr)
    gene init(p1, ...) { self.f = p1 }   # constructor (optional)
    gene method(a) { return self.f + a } # methods read/write fields via self
}
phenotype Child from Parent { ... }  # inheritance: methods + fields
let o = new Name(args)               # construct (calls init if declared)
```

- `new Name(args)` builds an instance: field defaults apply lineage root-first (parent fields, then own overrides), then exactly one `init` runs with the args — the lineage is searched **root-first** and the first (least-derived) `init` found wins; a child's `init` only runs if no ancestor declares one. Without any `init`, fields keep their defaults. (`super` is not a binding: an unbound `super.init(...)` inside `init` reads null with a note.)
- `self.f` reads and writes fields inside methods. Field access from outside: `o.f`. Missing fields → `null` + note.
- Method dispatch: own methods first, then the parent chain. Missing method → `null` + note.
- `type(o)` returns the phenotype name (`"Counter"`); `o.f = v` assigns a field. Phenotype instances cross `spawn` boundaries by serialization (they travel as maps carrying a hidden `#phenotype` key).
- Marks may precede `gene` inside a phenotype (`@acetylate gene m() { ... }`).

## 7b. Sequences (generators)

```
sequence name(p1, p2 = default) {
    ...
    yield v                          # produce a value; body suspends until pulled
}
let s = name(3)
s.next()                             # next value, or null when exhausted
s.collect()                          # drain remaining values into a List
for v in name(3) { ... }             # sequences are directly iterable
```

- A sequence body runs on its own **worker cell** (a real OS thread); the consumer pulls values through a rendezvous channel. Pull is lazy — a sequence that yields forever is legal and only produces values on demand.
- Values cross the membrane by serialization. Named genes and lambdas may travel as arguments (they cross by definition); a running sequence object itself does not cross.
- Honesty note: the Rust core pulls lazily; the Python oracle models a sequence by running the body to completion on first pull (buffered). Output is identical for programs that do not print inside a sequence body, and the differential corpus avoids infinite sequences.
- `yield` outside a sequence is treated as `return` with a note.

## 8. Modules, TADs, anchors

- `use path;` — path like `std/bio`, `./util`, `util` (`.op` appended if absent). Resolution order: (1) relative to the importing file's dir; (2) the path as written, under CWD; (3) `std/` under CWD; (4) `$OPERON_STD` joined with the path as written (not double-joined with `std/`). Binds one name: basename, or `as name`. `use std/bio as b;` → `b.some_fn(...)`. Importing the same file twice executes it once (module cache).
- Import gating (§9b): imports that resolve inside the program's own managed trees — the importing file's project directory, the CWD, or the standard library — are always allowed; a `use` that reaches outside those trees requires a read capability.
- A module evaluates to a Map of its **exported** names. Export rule:
  - If the file contains any `anchor export a, b;` (at top level or inside a tad), ONLY those names are exported.
  - With zero `anchor export` anywhere, all top-level genes and lets are exported (default-open).
- `tad Name { ... }` — topologically-associating domain: an insulation boundary. Names inside a tad escape ONLY via `anchor export` **inside that tad**. Tads may not nest (nesting degrades to a merged tad + note). A file may have several tads.
- `anchor import x;` — declares an external name the domain expects; `operon check` verifies it exists in the file or in used modules (−2 and a finding if not).
- Cyclic `use` → second import returns the partial module Map + note (no hang).

## 9. Stress containment (errors)

Runtime failures raise a **Stress** value: Map `{"kind": Str, "message": Str}`. Kinds:
`unfolded` (type errors), `missing` (bad index/key/member/null-deref), `overflow` (int overflow, depth limit, resource ceilings), `burned` (assertion failures, resource errors), `interference` (capability-sandbox denials, §9b), `any` (catch-all position only).

```
stress { RISKY } rescue (e) { promote("contained: {e.message}") }
stress missing { ... } rescue { ... }    # kind filter: only catches `missing`
```

- Unspecified kind catches any. Filtered kind catches only that kind; others propagate outward.
- `rescue` binding optional: `rescue (e)`. No rescue clause → stress is contained to Null + note (Top-Grammar runtime law).
- Uncaught at top level → printed as containment note, run continues (or ends that entry call with Null).
- `assert(cond, msg?)` raises Stress `burned` on failure (this is what proof frames catch).

## 9b. Security — the capability sandbox

The runtime is **default-deny**: a program is an organism in a culture flask, and nothing outside the flask exists until the operator grants it. The builtins `read_file`, `write_file`, `append_file`, `exists`, `read_dir`, `run`, `http_get`, `serve`, and `env` raise catchable Stress `interference` when no grant covers the access — RNA-interference: the cell's antiviral machinery silences the operation instead of crashing. `recv_request`/`send_response` poll a queue that only `serve` fills, so they are inert without a granted server.

Grants (operator-side, CLI):

```
operon run app.op --allow-read /data --allow-write /tmp/out \
              --allow-run gzip --allow-net 127.0.0.1:8080 --allow-env API_KEY
operon run app.op --allow-all           # open flask (for scripts that mean it)
```

- Path grants (`read`/`write`) resolve symlinks: the requested path is canonicalized before comparison against the canonicalized grant. A grant that normalizes to the empty string (`/`, or `.` from `/`) would match everything and is **rejected** at startup.
- `--allow-net` takes `host:port`; `--allow-env` takes a variable name; `--allow-run` takes a program name.
- `.cell` grant keys (`allow.read = /data`, …) are honored **only** when the config is loaded explicitly via `--cell file.cell`. An auto-detected `operon.cell` cannot grant capabilities — its `allow.*` keys are ignored with an `[info]` note (a file that happens to sit in the project must not silently widen the sandbox).
- `use` imports: files inside the program's own project directory, the CWD, or the standard library are always importable (otherwise nothing imports under default-deny). A `use` that resolves outside those managed trees requires a read grant; the denial is Stress `interference`.

Resource ceilings (all raise catchable Stress):

| Resource | Ceiling |
|---|---|
| Recursion depth | 10,000 (`overflow`) |
| Step budget | 200,000,000 steps per run (`overflow` "step budget exhausted"); `--fuel N` lowers it |
| String `.repeat()` allocation | 512 MiB |
| `distance()` dynamic-programming table | 10,000,000 cells |
| `sleep()` | 60,000 ms (sleep escapes the step budget, so it is capped) |
| `json_parse` nesting | 512 levels |
| Integer arithmetic | i64, overflow → `overflow` (no wrap) |

## 10. Builtins and methods

**Builtins — core:** `promote(*a)` (print, space-joined, returns null) · `len` · `push(l,v)` · `pop(l)` · `insert(l,i,v)` · `remove(l,i)` · `keys(m)` · `values(m)` · `has(m,k)` · `del(m,k)` · `range(a, b?, step?)` (returns List) · `str` · `num` (fails → 0 + note) · `type` (`null bool int float str list map gene native sequence`, or the phenotype name for instances) · `abs min max sum` · `floor(x)` `ceil(x)` (→ Int) · `sqrt(x)` `pow(b, e)` (→ Float) · `clock()` (monotonic seconds, float) · `now()` (monotonic seconds, float — the same high-resolution timer under a briefer name) · `exit(n?)` (capability-gated, sec-r2: kills the host process, so it is default-deny — grant with `--allow-exit` or `.cell allow.exit = true`) · `assert(c, msg?)` · `codon(s)` (0–100 style score of an identifier) · `distance(a,b)` (edit distance, C++ bit-parallel kernel; 10M-cell ceiling and a 64 KiB per-operand cap — over-budget pairs never win a nearest-match contest) · `similar(a,b,maxd?)` (bool) · `transcribe(dna)` · `translate(rna)` (stops at stop codon) · `reverse_complement(dna)` · `gc_content(dna)` (0–100) · `find_orf(dna)` (list of ORF proteins) · `memory()` (map `arena_bytes, interns, allocs` from the in-process symbol table) · `methyl(key, default?)`.

**Builtins — randomness and dynamic dispatch:** `random()` (float in [0,1)) · `random(n)` (int in [0,n)) · `randomize(seed?)` (deterministic xorshift state, identical in both implementations) · `chr(i)` · `ord(c)` · `argv()` (List of the arguments after the script path) · `sleep(ms)` (≤ 60,000 ms) · `call(name_or_gene, args_list)` (dynamic dispatch — resolves builtins, named genes, or gene values).

**Builtins — JSON:** `json_parse(s)` (→ value; nesting > 512 → Stress) · `json_str(v)`.

**Builtins — capability-gated (§9b; denied → Stress `interference`):** `read_file(p)` · `write_file(p, s)` · `append_file(p, s)` · `exists(p)` · `read_dir(p)` (List of names) · `run(prog, args?)` (map `code stdout stderr ok`; the child runs under a wall-clock timeout — `.cell run.timeout_ms`, default 10 s, clamped 1..300 000 — and the child's wall time is charged as fuel exactly like `sleep`; output may be truncated when a grandchild holds the pipe past the 250 ms drain grace) · `http_get(host, port?, path?)` (response body) · `serve(port?)` · `recv_request()` (map `conn method path body`, or null) · `send_response(conn, status?, ctype?, body?)` (status/ctype containing control characters are refused — no header injection) · `env(name)`.

**Builtins — concurrency, telemetry, regulation:** `fingerprint()` (run telemetry, §14) · `spawn(f, args?)` → id · `join(id)` → value (default wait ceiling 300 s — timed-out tasks stay joinable, join returns null; explicit `join(id, ms)` unchanged) · `toggle_on(name)` · `toggle_state()` · `repressi_next()` · `repressi_state()` (fuel-charged: integrating new ring ticks costs 20 steps/tick) · `repressi_start(ms)` (one shared cancellable timer per run — restarting retires the old thread) · `grn_fire(name, decay?)` · `grn_state()` · `grn_set(name, v)` (write a node's level, clamped 0..1) · `grn_get(name)` (read a node's level) · `methylate(name)` / `demethylate(name)` (runtime methylation — same graded semantics as the `@methylate`/`@acetylate` marks: level +1 / saturating −1, gate applies at the next call; returns the gene's new level; keys are allocation-charged like any growth).

**Str methods:** `.upper() .lower() .trim() .split(sep) .join(list) .replace(a,b) .contains(x) .starts(x) .ends(x) .repeat(n) .slice(a,b) .len()`
**List methods:** `.map(f) .filter(f) .reduce(f, init) .each(f) .sort(cmp?) .reverse() .contains(x) .index_of(x) .slice(a,b) .join(sep) .len()` (cmp returns true when a before b)
**Map methods:** `.keys() .values() .items() .has(k) .del(k) .len()`

## 11. Gene-expression regulation layer (v2 core novelties)

All features are real, implemented, tested — none are decorative.

- **`tad` / `anchor`** — §8. Module insulation with export anchors.
- **`enhance a, b, c;`** — super-enhancer cluster: marks genes with an **activation boost** (v2.2). Under the GRN call gate, an enhanced gene lowers every incoming activating threshold by **0.25** (floored at 0) — an enhanced gene fires where an unenhanced one stays gated (e.g. regulator level 0.4: threshold 0.5 blocks the plain gene, passes the enhanced one). Genes without the mark are unaffected and edges without a threshold stay declarative, so old programs keep running (§5 default: the boost exists only where `enhance` was declared). `operon profile` shows the `enhanced` flag; `operon check` gives the file a codon-score bonus for enhanced hot genes; the NMD untranslated sweep skips enhanced genes.
- **`@acetylate`** — histone acetylation mark: gene is eager/priority; excluded from silence rewriting (active chromatin stays active); shown as `active` in profile.
- **`@methylate`** — histone methylation mark: gene is repressed, **graded** (v2.2). Every executed `@methylate`-marked `gene` definition deepens that gene's silencing level by 1; an `@acetylate`-marked definition relaxes it by 1 (histone marks compete on the same chromatin). A call to a gene whose level has reached the threshold (default **3**, tunable via `.cell` `methylate.threshold = n`) is **blocked**: it returns `null` with a fallback note ("methylation silences: … — call returns null"), never reaches the gene body, and does not count in `fingerprint()`/burst telemetry — repressed means repressed. Below the threshold, calls execute and the first call emits the soft note "methylated call" (later calls are silent — the cell does not narrate every repression; suppressed entirely when `.cell` sets `methylate.quiet = true`). Marked genes are exempt from the NMD untranslated sweep, and `@acetylate` genes are exempt from the methylation gate itself (open chromatin wins). One mark (the common case) never silences — old programs keep running.
- **`@m6a`** — m6A mark = dispatch priority + transcript stability: among same-name candidates (splice variants, shadowing), the m6a-marked one wins resolution (variant selection order: `.cell` > CLI > **m6a-marked** > first declared — a marked variant beats first-declared), and an m6a-marked binding **resists redefinition** — an unmarked re-`gene` of the same name is ignored with a note ("@m6a-stabilized; redefinition ignored"); mark the new copy too, to replace it.
- **`silence old -> new;`** — RISC-style silencing: after this statement, every call to gene `old` is redirected to `new` with a note "RISC: call silenced". `@acetylate` genes are immune. Same-name splice variants resolve first, then silencing applies to the resolved name.
- **NMD sweep** (`operon check --nmd`): "premature stops" = unreachable statements after an unconditional `return` (reported −4 each); "untranslated transcripts" = defined, never-called, non-exported, non-enhanced genes (info, −1). `--nmd purge` rewrites the file without them.
- **`splice name { variant a { } variant b { } }`** — alternative splicing: `name(...)` dispatches to the active variant. Selection order: `.cell` `variant.name=v` > CLI `--variant v` > **m6a-marked variant** (v2.2) > first declared. Variant declarations may carry histone marks (v2.2): `@m6a variant v { }` makes `v` the m6a-priority variant; `@acetylate`/`@methylate` on a variant ride on the resolved binding (acetylate immunity applies when a marked variant is active). Marks stack and can precede the variant keyword in any order; a mark not followed by `variant` is noted and skipped. Variants take parameters. `operon build --variant a` bakes one variant into a standalone file (build caveat: a variant's parameter list is not yet carried into the baked file — bake param-less variants, or keep the splice in the source).
- **`.rna` edit patches** — `edit target { replace "src" -> "dst"; }` where target is a file name or gene name; applied to that target's source text before parsing. CLI: `operon run app.op --rna hot.rna`. Each applied replacement emits a note.
- **`.cell` methylation config** — `key = value` lines, `[section]` headers, `#` comments. CLI `--cell f.cell`; else `operon.cell` auto-detected. Read via `methyl("k", d?)`. Impl-consumed keys: `variant.<splice>` (active variant), `methylate.quiet` (suppress the methylated-call note), `methylate.threshold` (graded-silencing gate, default 3), `grn.decay` (default GRN pulse decay, reg-r2), `run.timeout_ms` (child-process wall-clock timeout for `run()`, default 10000, clamped 1..300000), `wobble.strict = true` (a run with any rung ≥ 3 note exits 3, same as `--strict`), `entry` (cap-independent default entry, below CLI `--entry` and above `main`), and `allow.*` capability grants — honored only with explicit `--cell` (§9b; `allow.exit = true` grants the exit capability, sec-r2).
- **`ires name;`** — internal ribosome entry site: declares a cap-independent entry gene. `operon run --ires` overrides the canonical `main` entry and runs the first declared `ires` target; a file with no `main` uses it automatically. Entry precedence: CLI `--entry` > `.cell` `entry` > `main` > first `ires`.
- **`fate Name { state a -> b, c; state b -> a; enter a; }`** — fate landscape state machine. `let m = Name()` enters the `enter` state (default: first declared). Methods on instances: `.shift(s)` (true if transition allowed; invalid → note + false, state unchanged — the valley stay), `.state()` (current), `.can(s)` (bool). Instances are Maps with hidden `#fate`/`#state` keys.
- **`regulate { a activates b strength 0.8; c inhibits d; }`** — gene regulatory network (default strength 1.0). The network is **stateful**: by default levels persist across `grn_fire` calls (a latch — the honest default; real dilution needs the decay below), and each fire seeds additively, capped at 1.0. **Decay (reg-r2):** opt-in dilution — before every pulse, every existing level decays by a configurable fraction: `grn_fire(name, f)` takes the pulse's decay directly (0..1, clamped), or `.cell` `[grn] decay = f` configures it globally; unset/0 is byte-identical to the pre-decay behavior. (Term audit, reg-r3: persistence is a latch, decay-to-zero is dilution/degradation — neither is "homeostasis", which requires a regulated setpoint.) Each fire runs in two phases: (1) **activation** propagates in waves, `child = max(child, parent × strength^wave)` — hop attenuation over up to 10 waves; (2) **inhibition** is applied exactly once per fire, `child = max(0, child − parent × strength)`, based on the source's post-activation level — a repressor's concentration sets the output level, it does not compound across waves. An edge may carry an optional `threshold t` (dose-response, n = 2): influence = `strength × p² / (p² + t²)` where p is the parent level — weak below the threshold, saturating above it. `grn_state()` returns the level Map (all genes in the graph); `grn_set/grn_get` write/read single node levels (clamped 0..1) so hosts can steer gate state without re-firing. **Levels gate calls** (v2.2): a gene with an incoming activating edge carrying an explicit `threshold t` executes only while `level(source) ≥ t` — when several activating edges target one gene, every threshold must pass (conjunctive); an incoming inhibiting edge with an explicit threshold silences the callee while `level(inhibitor) ≥ t`. A gated call returns `null` with a fallback note ("grn gate: … call suppressed"). Edges **without** a threshold remain declarative (they shape `grn_fire` level dynamics only), and a threshold of `0` never blocks: declaring a network changes no call behavior unless a program opts in with explicit thresholds. The gate is checked at the gene-call funnel (named calls, higher-order calls, and RISC-redirected calls all pass through it) and precedes the call counters — a suppressed call is not expression and does not count in `fingerprint()`/burst telemetry.
- **`toggle a, b;`** — a **mutual-repression latch** (term audit, reg-r3: a declared boolean invariant, not rate-based bistability — no cooperativity, no hysteresis; the Gardner–Cantor–Collins toggle switch is the biological inspiration, not a simulated model). `toggle_on("a")` turns `a` on and `b` off; `toggle_state()` → Map of both. Exactly one may be on — and the pair gates **calls**: calling the repressed allele returns `null` with a note ("toggle repressed: … is the inactive allele"); `@acetylate` genes are immune (open chromatin wins).
- **`repressilator a -> b -> c;`** — repressive oscillation ring with **emergent mutual repression** (reg-r2): the ring integrates a **reduced, protein-only discrete variant inspired by the Elowitz–Leibler repressilator** (Elowitz & Leibler, Nature 2000; term audit, reg-r3: the published system is a two-state mRNA+protein model with Hill n=2, α≈250, β=5 and a basal term — this is the one-state protein-only reduction with h=4 and no basal term, which oscillates robustly in discrete time). Each substep, node level evolves as `dA/dt = α / (1 + R⁴) − γA` where R is the level of the repressor (ring `a -> b -> c` means a represses b, b represses c, c represses a), Euler-integrated with 20 substeps of `dt = 0.05` per ring tick (α = 10, γ = 1, init `[5, 0, 0]`). The oscillation — a ~6-tick limit cycle with one-tick production lag between a repressor's peak and its target's trough — **emerges from the loop itself; nothing is scheduled**, so gates can read the ring: a GRN edge sourced by a ring node reads the node's normalized level (`raw/α`, clamped 0..1) at the current tick. `repressi_state()` returns the level Map and is a **pure function of the tick count** — manual mode (`repressi_next()` advances one ring tick) and wall-clock mode (`repressilator … period s;` or `repressi_start(ms)`) fold the identical arithmetic and can never diverge; the Python oracle mirrors it op-for-op bit-identically (§17). The ring rides `spawn` snapshots frozen at the spawn tick (§13). The ring rides `spawn` snapshots frozen at the spawn tick (§13).

## 12. Frames, proofs, overlapping reading frames

- `frame proof { assert(...); ... }` — the **test reading frame** of the file. Skipped by `operon run`; executed by `operon test`. The same file encodes program + tests (two reading frames over one sequence).
- `frame name { ... }` — named frames (metadata/optional scenes); runnable via `operon run --frame name`.
- `operon test [paths...]` — default paths: `tests/` recursively. For each file: run its proof frames; a proof failure (Stress burned) is recorded; the suite continues (Total Grammar). A proof must **run to completion** — an early `return`/`break` inside a proof fails it ("exited early"), and a proof that exercises **zero assertions** fails it ("vacuous proof"). Exit code 1 if any failure. Report: files, proofs run, passed, failed, assertions exercised, wobble notes count. Current suite: 50 files / 44 proofs / 654 assertions, all green on both implementations.

## 13. Concurrency

- `spawn(f, args?)` — starts a real OS thread running gene `f`; returns task id (Int). `join(id)` waits and returns the result (second join → Null + note). Arguments and results cross by serialization (named genes, lambdas, and phenotype instances cross; a running sequence object does not).
- Thread panics are impossible by construction: any stress inside the thread is returned as a Stress Map value.
- Memory model note (honesty): values are reference-counted; tasks communicate by args/results, not shared mutable state. Data races on shared globals are prevented by design (closures capture is by value at spawn time for non-local references).
- **Worker cells inherit regulation state (reg-r1).** A spawned task or sequence cell starts with a copy of the parent's GRN edges + levels, methylation counters + threshold, toggle pairs, and enhance marks, frozen at spawn time. Worker calls dispatch through the same funnel as the host, so a toggle-repressed allele, a silenced (level ≥ threshold) gene, or a GRN-vetoed call returns null inside the cell exactly as it does outside — regulation is part of the cell, not a host-side illusion. Later parent-side regulation changes do NOT propagate to already-running cells (snapshot semantics).
- Sequences (§7b) run on the same worker-cell substrate: each sequence body is a worker thread pulling through a rendezvous channel.

## 14. Telemetry — the single-cell layer

- `fingerprint()` returns Map:
  - `calls` — Map gene → call count (phenotype methods count as `Name.method`).
  - `mature` — genes called at least once.
  - `nascent` — genes defined but never called.
  - `maturation` — mature / total defined genes (the transcript-maturation share).
  - `burst` — the aggregate **burst index**: mean over genes of (variance / mean) of per-gene call counts across complete 20-call bins of the run's call clock.
  - `burst_by_gene` — the per-gene burst indices (0 for a gene with ≤ 1 call or one bin).
- Method: the run's global call clock is sliced into windows of 20 gene calls; each gene's count per window is a sample. A gene fired in bursts has a high variance/mean ratio; a constitutively expressed one sits near 0. Only complete bins count (a trailing partial bin is dropped).
- `operon profile f.op` — runs instrumented, prints table: gene, calls, **exclusive self-time µs** (children subtracted), flags (`enhanced active repressed`), then a `mature · nascent · maturation` summary and **enhance candidates** — hot genes (called ≥ 10% as often as the most-called gene) that carry no `enhance` annotation.
- The v2.0 telemetry keys `spliced` / `unspliced` / `velocity` (and the per-gene variance/mean noise key) are retired; `mature`/`nascent`/`maturation` carry the same biology honestly (maturation share, not velocity).

## 15. Toolchain (Rust binary `operon`)

```
operon run f.op    [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires]
                   [--strict] [--quiet] [--fuel N]
                   [--allow-read p] [--allow-write p] [--allow-run prog]
                   [--allow-net host:port] [--allow-env var] [--allow-all]   # §9b
operon check f.op  [--nmd] [--nmd=purge] [--json]
operon test [paths...]
operon fmt f.op    [--write]        # canonical formatter; wobble-corrected output parses clean
operon build f.op  --variant v -o out.op
operon profile f.op
operon crispr f.op (--knockout gene | --matrix) [--json]
                   # knockout: body → return null; then run proofs; report survivors.
                   # matrix: knock out EVERY top-level gene; viability table (ESSENTIAL if a proof fails)
operon bench f.op  [--iters n]
operon version
```

**`operon-ls`** — language-server seed (stdio LSP): `initialize` / `shutdown` / `exit`, full-text document sync, `textDocument/publishDiagnostics` (Total Grammar parse notes by rung + `tools check` phantom calls), and `textDocument/hover` with gene/splice signatures. Programmer-first by D-008: hovering `boost` shows `gene boost(x)` plus its marks (`@acetylate`, `@methylate`, `@m6a`, `enhance`) and a one-line analogy — gene vocabulary is an intuition aid, never a prerequisite. Zero external dependencies: request JSON is parsed by the language's own `json_parse`.

- Grading (`operon check`): start 100; wobble note −2; fallback note −3; NMD premature stop −4; untranslated transcript −1; phantom call −2; unverified `anchor import` −2; hot `enhance`d genes with codon-optimal names earn up to +6 back; floor 50. Letter: A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60, else F.
- `check --json` emits valid JSON: score, letter, note counts, `phantoms` (undefined called genes), `nmd` findings array.
- `--fuel N` caps the interpreter's step budget (default 200,000,000); exhaustion raises catchable Stress `overflow`.
- `--strict` → exit code 3 if any rung ≥ 3 note occurred (`wobble.strict = true` in `.cell` does the same per run).
- Stdout discipline: `promote` prints program output; notes/reports go to **stderr** (so `operon run f.op > out.txt` is clean). `--json` on check/test/crispr emits machine-readable JSON to stdout.

## 16. Biology ↔ feature map (for docs; no scientist names)

| Mechanism (real molecular biology) | Operon feature |
|---|---|
| Wobble base pairing (redundant codon recognition) | 4-rung Total Grammar + synonym table |
| Codon optimality | `codon()` scoring in check grading |
| Overlapping reading frames | `frame proof` — tests and code in one sequence |
| Alternative splicing | `splice { variant }` + `--variant` / `.cell` selection |
| Phenotypic state and differentiation | `phenotype` classes: `new`, `init`, `self`, inheritance `from` |
| Polypeptide elongation (values produced one at a time) | `sequence` generators + `yield` / `.next()` / `.collect()` on worker cells |
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
| Single-cell transcriptomics / burst index | `fingerprint()` calls / burst / mature / nascent / maturation telemetry |
| Transcript maturation share (honest replacement for velocity) | `maturation` = mature / total genes |
| RNA interference (antiviral silencing) | capability sandbox: default-deny, Stress `interference` (§9b) |
| CRISPR knockout screens | `operon crispr --knockout` / `--matrix` |

## 17. Version

### §19 — Wave-3 hardening and surface expansion (2.1.1)

**Security model upgrades (§9b amendments):**

1. **Strict symlink resolution.** If a path EXISTS, its fully-resolved form
   (all symlinks followed) must lie inside a grant. The parent-chain probe is
   used only for not-yet-existing targets. A symlink planted in a granted
   directory pointing outside the sandbox resolves to its target and is
   rejected. File effects open the canonicalized path (what was checked is
   what is touched), and on Unix a write is refused when the target inode has
   more than one link (hardlink defense).
2. **Safe-base child environment.** `run()` spawns children with an EMPTY
   environment plus OS essentials (PATH, TEMP/SystemRoot family) and the
   explicitly `--allow-env`-granted variables. Parent secrets (CI tokens,
   credentials) cannot leak into effectors.
3. **Thread budget.** At most 256 live worker cells per run; exceeding the
   cap raises catchable `overflow`. Workers get 256 MiB stacks (matching the
   toolchain's depth headroom) and inherit the host's capabilities AND fuel.
   A worker panic fails the run with a non-zero exit — never a silent success.
4. **Run-wide fuel.** `spawn`/sequences no longer mint fresh budgets: host and
   every worker drain ONE pool (default 500M steps per run). `sleep` charges
   wall-time-proportional fuel (1 step per µs), and entering a `rescue` block
   charges 64 steps — neither a sleep loop nor a rescue retry-spin can outrun
   the budget.
5. **Memory ceilings.** String concat and `repeat`/`str * int` share the
   512 MiB ceiling; list concat is capped at 64M elements; live tasks at 4096
   (`join()` them). All raise catchable `overflow`.
6. **Server hardening.** `serve()` applies a 10 s read timeout per connection
   and caps the live connection table at 256; `http_get` bounds responses at
   64 MiB.
7. **Parser containment.** Expression nesting is capped at 4096 (deeper input
   truncates with a rung-4 note); token peeking is bounds-clamped. Adversarial
   inputs (million-paren bombs, quote storms) produce diagnostics, never
   native stack exhaustion.
8. **Cycle safety.** Self-referential lists/maps render with `[...]`/`{...}`
   markers (repr), serialize the repeated branch as `null` (JSON), and compare
   with identity short-circuit (a structure equals itself). Spawn arguments
   deeper than 100k fail the spawn with catchable `overflow`.

**New surface (§10 additions):**

9. **Regex (zero-dependency).** `re_match(pattern, s)` — anchored prefix test.
   `re_find(pattern, s, start?)` — leftmost match as
   `{text, start, end, groups}` or null. `re_groups(pattern, s)` — capture
   list or null. Syntax: literals, `.`, classes `[a-z0-9^]`, `\d \w \s \D \W
   \S`, `* + ? {m,n}`, alternation `|`, groups `( )` and `(?: )`, anchors
   `^ $`. The backtracking matcher has a hard 2M-step cap: catastrophic
   patterns raise catchable `overflow` (ReDoS-proof by construction). In
   double-quoted strings quantifier braces must be escaped (`\{2,3\}`) because
   `{..}` is interpolation.
10. **Time (UTC civil calendar).** `unix_time()` — seconds since the epoch.
    `date_parts(ts)` — `{year, month, day, hour, min, sec, wday}` (Sunday=0).
    `date_fmt(ts, fmt)` — `%Y %m %d %H %M %S` expansion.
11. **String repetition.** `"ab" * 3` and `3 * "ab"` — Python parity, capped
    by the 512 MiB ceiling.
12. **Join deadline.** `join(id, timeout_ms?)` returns null and notes when the
    worker exceeds the deadline (the task stays joinable).
13. **REPL.** `operon repl` — persistent-expression shell; expressions print
    their value, definitions persist; `:quit` exits. Commands: `:help`,
    `:load f.op` (execute a file into the session), `:proof` (run every
    `frame proof` defined in the session against live state — a proof must
    complete and exercise ≥1 assertion), `:proof f.op` (run a file's proofs),
    `:genes`, `:vars`, `:reset`. Commands are recognized only at an empty
    code buffer; inside an open block, `:` text is code.

**Semantics pinning (2.1.1):** `%` follows the divisor's sign; `//` is floor
division and always yields `int`; comparisons involving NaN are false;
`floor`/`ceil` of out-of-i64 floats raise `overflow`; negative slice indexes
count from the end; `sort()` returns a new list (immutable-method contract
shared with `reverse`/`slice`/`map`); cyclic imports return the still-loading
module's placeholder map which fills when loading completes (with a rung-4
note); map/filter/reduce/each run callbacks over a snapshot of the source
list (callbacks may freely mutate the original).

This specification is **Operon 2.2.0**. `operon version` prints the implementation banner `Operon 2.2.0 (rust-core, cpp-kernel)`, which matches this document. (sec-r2: the C runtime kernel was deleted — audit A15 proved its intern table was write-only and its raw pointers were the project's one ASan-confirmed memory-safety class; interning now lives in Rust, and the banner no longer claims a c-runtime.)

## 18. Verification status (what the shipped suite proves)

- Proof frames: **50 files / 44 proofs / 654 assertions**, green on the Rust core and the Python oracle.
- Differential harness (Rust core vs Python oracle, program-level stdout): **38 programs, all MATCH**.
- Playground smoke: expression-core subset in the browser, spec-aligned (unbound reads → null + note).
