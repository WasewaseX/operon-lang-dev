# Operon — Language Specification

**Status:** v2.2.0 + post-2.2 language amendments (L1a null-safety/destructuring/iteration builtins, L1c regex builtins, L1d wall-clock time builtins). Implementation version stays 2.2.0 until the next milestone tag (D-009: version moves only at milestones). This document is the single contract implemented identically by:

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
- Keywords (canonical, 58 — the parser's reserved set; the 6 quorum words joined in reg-bio-3 C8):
  `gene let if elif else while loop for in return break continue match case use tad anchor export import enhance silence stress rescue raise fate state regulate activates inhibits strength toggle repressilator period frame proof guard splice variant edit replace ires as collect enter phenotype sequence yield new threshold from self decoy ligand autoinducer bind inducer cofactor operon`
- Literal words `true false null` and the logical words `and or not` are recognized in expression positions (not part of the reserved keyword table).
- Marks: `@acetylate` `@methylate` `@m6a` `@copies` `@riboswitch` `@burst`.
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
let [a, b] = expr                   # destructuring definition (v2.3) — list
                                    # pattern; *rest captures the tail; strings
                                    # destructure by char; patterns nest; soft
                                    # miss → null + note (never a hard failure)
let {x, y} = expr                   # map pattern (v2.3): each name reads that
                                    # key (bare names only; missing → null+note)
let a, b = e1, e2                   # multi-define (v2.3): every RHS is
                                    # evaluated before any name binds
name = expr                         # assign (auto-let at top scope with note)
a, b = b, a                         # multi-assign / swap (v2.3): all RHS
                                    # evaluated (left→right) first, then targets
                                    # written in order; targets may be names,
                                    # a[i] or a.k; shorter RHS → null + note
name += -= *= //= expr              # compound (also on a[i], a.k targets)
if expr { } elif expr { } else { }  # elif/else optional, elif chainable
while expr { }
loop { }                            # infinite; break/continue
for name in expr { }                # List→items, Str→1-char strs, Map→keys,
                                    # sequence → pulled values (§7b)
for [k, v] in expr { }              # destructuring loop (v2.3) — any pattern
                                    # accepted; each item binds the pattern in a
                                    # fresh child scope
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

**Pattern soft-miss law (v2.3):** a destructuring pattern never hard-fails a
run. Destructuring a non-container (or null) binds all its names to null with
one note; a missing element/key binds null for that piece only; extra elements
are dropped with a note. `*rest` on an exhausted list binds `[]`.

## 6. Expressions (precedence low → high)

1. `cond ? a : b` — ternary, right-associative, lowest precedence
2. `or` (`||`) — short-circuits, returns operand value (`a or b` → a if truthy else b)
3. `and` (`&&`) — same semantics
3.5 `a ?? b` (v2.3) — null coalescing: sits between `or` and `and`
   (`a or b ?? c` reads `a or (b ?? c)`); short-circuits; coalesces **Null
   only** — falsy-but-non-null values (`0`, `""`, `[]`, `false`) pass through.
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
14. postfix: call `f(x)`, index `a[i]`, member `a.k`, method call `a.k(args)`,
    and the null-safe forms `a?.k`, `a?.k(args)` (v2.3): a Null receiver yields
    Null **silently** — no note; a non-Null receiver behaves exactly like `.`
    (missing keys still note). Chains compose: `a?.b?.c`.
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

- `marks` = `@acetylate | @methylate | @m6a | @copies n` (§11). Sequence definitions gate at CREATION like gene calls (§11 gate order) — a silenced or vetoed sequence returns null instead of starting its worker (reg-r4).
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

The runtime is **default-deny**: a program is an organism in a culture flask, and nothing outside the flask exists until the operator grants it. The builtins `read_file`, `write_file`, `append_file`, `exists`, `read_dir`, `file_size`, `fs_delete`, `fs_rename`, `fs_mkdir`, `run`, `py`, `http_get`, `serve`, `env`, and `exit` raise catchable Stress `interference` when no grant covers the access — RNA-interference: the cell's antiviral machinery silences the operation instead of crashing. `recv_request`/`send_response` poll a queue that only `serve` fills, so they are inert without a granted server.

Grants (operator-side, CLI):

```
operon run app.op --allow-read /data --allow-write /tmp/out \
              --allow-run gzip --allow-net 127.0.0.1:8080 --allow-env API_KEY
operon run app.op --allow-py math --allow-py numpy   # substrate-r1: per-module Python bridge grants
operon run app.op --allow-all           # open flask (for scripts that mean it)
```

- Path grants (`read`/`write`) resolve symlinks: the requested path is canonicalized before comparison against the canonicalized grant. A grant that normalizes to the empty string (`/`, or `.` from `/`) would match everything and is **rejected** at startup.
- **sec-r5 (anti-TOCTOU): file I/O verifies the opened handle, not the path.** The old check → canonicalize → write sequence was raceable with a symlink swap (proven live: a flipped link landed attacker-controlled bytes outside the grant). Now `read_file`/`write_file`/`append_file` open the file FIRST, verify the handle's true identity (Unix: the fully-resolved `/proc/self/fd/<n>` target; elsewhere: symlink/reparse handles are refused), and do all I/O **through the handle** — there is no later path re-resolution to race. A dangling outside-pointing symlink is resolved (chains up to 40 hops) and rejected **before** open, so it cannot be created-through as an empty file. Residual (documented): a race that flips a link to a dangling outside target inside the check-to-open window can create an empty file outside the grant — no content ever crosses.
- **sec-r5: `read_file` reads regular files only.** FIFOs (which would block `open()` forever) and device files like `/dev/zero` (an infinite byte well that OOM-killed the host) are refused with Stress `interference` before open; the stat'd size is charged against the aggregate allocation ceiling before the read.
- `--allow-net` takes `host:port`; `--allow-env` takes a variable name; `--allow-run` takes a program name; `--allow-py` takes a Python **module** name (substrate-r1: grant granularity is per module — `--allow-py math` runs `math.*` calls and nothing else; granting a module is an explicit trust act, the module executes with the interpreter's OS privileges).
- **substrate-r1 — the `py` bridge (D-010):** `py(module, "dotted.func", args?)` calls into the Python ecosystem through an isolated-mode child (`python3 -X utf8 -I -B`) running an embedded one-line-JSON runner. The child gets the same scrubbed environment as `run()` children; the request is one JSON line on stdin (then the pipe closes); the response is the LAST non-empty stdout line, so modules that print during import or call cannot corrupt the protocol. Result is a map `{ok, value, error, code}`: Python exceptions are DATA (`ok:false`, `code:1`, the exception line in `error`), protocol failures are `code:-2`, a timeout kill is `code:-1` with a note — the interpreter never crashes on a misbehaving module. Container ceilings mirror `run()`: wall time is fuel (1000 steps/ms), `.cell py.timeout_ms` (default 10000, clamped 1..300000), output capped at 64 MiB per stream. Marshal rules: Operon values ride the language's own JSON serializer (Seq → base string); Python side converts numpy scalars via `.item()`, ndarrays via `.tolist()`, tuples/sets to lists, bytes via UTF-8 decode, dates/Decimals to ISO/float; non-finite floats are refused by the runner (`ok:false`). The bridge is how Operon joins the scientific-Python wall (§1) instead of fighting it: the stdlib stays small because NumPy/SciPy/Biopython are one granted call away.
- `.cell` grant keys (`allow.read = /data`, …) are honored **only** when the config is loaded explicitly via `--cell file.cell`. An auto-detected `operon.cell` cannot grant capabilities — its `allow.*` keys are ignored with an `[info]` note (a file that happens to sit in the project must not silently widen the sandbox).
- `use` imports: files inside the program's own project directory, the CWD, or the standard library are always importable (otherwise nothing imports under default-deny). A `use` that resolves outside those managed trees requires a read grant; the denial is Stress `interference`.

Resource ceilings (all raise catchable Stress):

| Resource | Ceiling |
|---|---|
| Recursion depth | 10,000 (`overflow`) |
| Step budget | 200,000,000 steps per run (`overflow` "step budget exhausted"); `--fuel N` lowers it |
| String `.repeat()` allocation | 512 MiB |
| Aggregate run allocation | 2 GiB (push/concat/repeat/interp assembly all charged; sec-r5: large-string variable reads and writes/reads of file bytes are charged too — a 500 MB string read three times IS 1.5 GB of real allocation) |
| Collected `run()` child output | 64 MiB per stream (collected prefix is returned; the child is still timeout-killed) |
| Collected `py()` response (substrate-r1) | 64 MiB per stream, same contract as `run()`; wall time charged as fuel |
| Parse/lex notes | 10,000 per parse (further notes suppressed) |
| `distance()` dynamic-programming table | 10,000,000 cells |
| `sleep()` | 60,000 ms (sleep escapes the step budget, so it is capped) |
| `json_parse` nesting | 512 levels |
| Integer arithmetic | i64, overflow → `overflow` (no wrap) |
| Map non-scalar-key scan (sec-r5) | 512 entries per lookup/upsert — beyond that a non-scalar (list/map) key is treated as absent; scalar keys keep exact semantics via the hash memo |

sec-r5 containment semantics worth stating plainly:

- **DAG-shaped values serialize and compare in linear time.** `json_str`, structural display (`print`), and `deep_eq` (`==`) keep their visited/comparison sets for the whole walk instead of unwinding them. A shared (aliased) subtree is rendered once; later references render as `null` (JSON) or the `[...]`/`{...}` marker (display) — the same containment CPython applies to cycles, extended to aliased DAGs, which previously re-walked exponentially (`l = [l, l]` chains: 2^45 node visits for one builtin call).
- **`json_str` emits valid JSON only (RFC 8259).** Non-finite floats serialize as `null` instead of bare `inf`/`nan` tokens that no JSON parser accepts.
- **The aggregate allocation ceiling and the step budget are monotonic.** Once crossed, the counter stays over the limit — every later charge fails, including charges inside a `rescue` handler, so a run that breached a resource ceiling ends with the top-level containment note rather than resuming. Resource-exhaustion overflow is the one stress a rescue cannot recover from; the contract is a bounded note, never an OOM kill or allocator abort.
- **CI/supply chain:** every GitHub Action in both workflows is pinned to a commit SHA (checkout, toolchain, gh-release — D-6 closed); `Cargo.lock` has zero runtime dependencies (build-time only: cc, shlex, find-msvc-tools).

## 10. Builtins and methods

**Builtins — core:** `promote(*a)` (print, space-joined, returns null) · `len` · `push(l,v)` · `pop(l)` · `insert(l,i,v)` · `remove(l,i)` · `keys(m)` · `values(m)` · `has(m,k)` · `del(m,k)` · `range(a, b?, step?)` (returns List) · `str` · `num` (fails → 0 + note) · `type` (`null bool int float str list map gene native sequence`, or the phenotype name for instances) · `abs min max sum` · `floor(x)` `ceil(x)` (→ Int) · `sqrt(x)` `pow(b, e)` (→ Float) · `clock()` (monotonic seconds, float) · `now()` (monotonic seconds, float — the same high-resolution timer under a briefer name) · `exit(n?)` (capability-gated, sec-r2: kills the host process, so it is default-deny — grant with `--allow-exit` or `.cell allow.exit = true`) · `assert(c, msg?)` · `codon(s)` (0–100 style score of an identifier) · `distance(a,b)` (edit distance, C++ bit-parallel kernel; 10M-cell ceiling and a 64 KiB per-operand cap — over-budget pairs never win a nearest-match contest) · `similar(a,b,maxd?)` (bool) · `transcribe(dna)` · `translate(rna)` (stops at stop codon) · `reverse_complement(dna)` · `gc_content(dna)` (0–100) · `find_orf(dna)` (list of ORF proteins) · `memory()` (map `arena_bytes, interns, allocs` from the in-process symbol table) · `methyl(key, default?)`.

**Builtins — randomness and dynamic dispatch:** `random()` (float in [0,1)) · `random(n)` (int in [0,n)) · `randomize(seed?)` (deterministic xorshift state, identical in both implementations) · `chr(i)` · `ord(c)` · `argv()` (List of the arguments after the script path) · `sleep(ms)` (≤ 60,000 ms) · `call(name_or_gene, args_list)` (dynamic dispatch — resolves builtins, named genes, or gene values).

**Builtins — iteration and numeric (v2.3):** `enumerate(x)` (List of `[i, v]` pairs; List or Str) · `zip(a, b)` (List of `[va, vb]` pairs, min length) · `sorted(l, cmp?)` (new list; same comparator contract as `.sort` — cmp true when a belongs before b — or the default mixed-type order) · `reversed(l)` (new list) · `any(l)` / `all(l)` (truthiness; empty → `false` / `true`) · `first(l)` / `last(l)` (element or char; empty → Null + note) · `take(l, n)` / `drop(l, n)` (clamped slices; List or Str) · `unique(l)` (deep-equality dedup, first occurrence kept) · `flatten(l)` (one level) · `chunk(l, n)` (size-n groups, last partial; n ≤ 0 → note) · `round(x, d?)` (d omitted/0 → Int, else Float; **half away from zero** at the d-th decimal, computed in the same f64 formula in both implementations — `round(1.005, 2) == 1.0` because 1.005 is stored as 1.00499…) · `clamp(v, lo, hi)` (numbers; non-number → Null + note) · `divmod(a, b)` (`[q, r]` with the exact `//` and `%` semantics — floored, sign follows divisor). Wrong-type inputs return Null (or `false` for `any`/`all`) with a note, never a crash.

**Safe accessors (v2.3):** `m.get(k, d?)` (Map; deep-equality key lookup; missing → default when given, else Null + note) · `l.get(i, d?)` (List; negative index from the end; out of range → default or Null + note) · `s.at(i, d?)` (Str char; same contract). The bare forms (`m.k`, `l[i]`) keep their existing behavior — `.get`/`.at` add an explicit default, they do not change any existing read.

**Builtins — JSON:** `json_parse(s)` (→ value; nesting > 512 → Stress) · `json_str(v)`.

**Builtins — capability-gated (§9b; denied → Stress `interference`):** `read_file(p)` (regular files only — FIFOs/devices are refused `interference`, sec-r5) · `write_file(p, s)` · `append_file(p, s)` · `exists(p)` · `read_dir(p)` (List of names) · `fs_delete(p)` (files and EMPTY dirs; symlinks are refused outright — dx-r6) · `fs_rename(from, to)` (both paths need a write grant) · `fs_mkdir(p)` (create_dir_all semantics) · `run(prog, args?)` (map `code stdout stderr ok`; the child runs under a wall-clock timeout — `.cell run.timeout_ms`, default 10 s, clamped 1..300 000 — and the child's wall time is charged as fuel exactly like `sleep`; output may be truncated when a grandchild holds the pipe past the 250 ms drain grace) · `py(module, "dotted.func", args?)` (substrate-r1 Python bridge; map `ok value error code`; per-module grant `--allow-py m`; isolated-mode child, timeout `.cell py.timeout_ms`, wall time is fuel, 64 MiB output cap; Python exceptions are data `ok:false`, never crashes — full contract in §9b) · `http_get(host, port?, path?)` (response body) · `serve(port?)` · `recv_request()` (map `conn method path body`, or null) · `send_response(conn, status?, ctype?, body?)` (status/ctype containing control characters are refused — no header injection) · `env(name)`.

**Builtins — concurrency, telemetry, regulation:** `fingerprint()` (run telemetry, §14) · `spawn(f, args?)` → id · `join(id)` → value (default wait ceiling 300 s — timed-out tasks stay joinable, join returns null; explicit `join(id, ms)` unchanged) · `toggle_on(name)` · `toggle_state()` · `repressi_next()` · `repressi_state()` (fuel-charged: integrating new ring ticks costs 20 steps/tick) · `repressi_start(ms)` (one shared cancellable timer per run — restarting retires the old thread) · `grn_fire(name, decay?)` · `grn_state()` · `grn_set(name, v)` (write a node's level, clamped 0..1) · `grn_get(name)` (read a node's level) · `methylate(name)` / `demethylate(name)` (runtime methylation — same graded semantics as the `@methylate`/`@acetylate` marks: level +1 / saturating −1, gate applies at the next call; returns the gene's new level; keys are allocation-charged like any growth) · `m6a_write(name, n?)` / `m6a_erase(name, n?)` (reg-bio-3: quantitative m6A site density 0..=3 — writer/eraser dose, default 1; resistance to redefinition requires level ≥ 1) · `passage(n)` (reg-bio-3: n cell divisions — methylation levels dilute by `.cell methyl.maintenance` with half-down rounding, `generation` advances; returns the generation) · `expr_on(kon?, koff?)` / `expr_off()` (reg-bio: the telegraph promoter layer — §11; in-source switch for stochastic expression, seeded via `randomize`).

**Str methods:** `.upper() .lower() .trim() .split(sep) .join(list) .replace(a,b) .contains(x) .starts(x) .ends(x) .repeat(n) .slice(a,b) .len()`
**List methods:** `.map(f) .filter(f) .reduce(f, init) .each(f) .sort(cmp?) .reverse() .contains(x) .index_of(x) .slice(a,b) .join(sep) .len()` (cmp returns true when a before b)
**Map methods:** `.keys() .values() .items() .has(k) .del(k) .len()`

## 11. Gene-expression regulation layer (v2 core novelties)

All features are real, implemented, tested — none are decorative.

- **`tad` / `anchor`** — §8. Module insulation with export anchors.
- **`enhance a, b, c;`** — super-enhancer cluster: marks genes with an **activation boost** (v2.2). Under the GRN call gate, an enhanced gene lowers every incoming activating threshold by **0.25** (floored at 0; the dose is tunable via `.cell enhance.delta`, reg-bio: real enhancer strength varies with binding-site number and affinity) — an enhanced gene fires where an unenhanced one stays gated (e.g. regulator level 0.4: threshold 0.5 blocks the plain gene, passes the enhanced one). Genes without the mark are unaffected and edges without a threshold stay declarative, so old programs keep running (§5 default: the boost exists only where `enhance` was declared). `operon profile` shows the `enhanced` flag; `operon check` gives the file a codon-score bonus for enhanced hot genes; the NMD untranslated sweep skips enhanced genes.
- **`@acetylate`** — histone acetylation mark: excluded from silence rewriting and immune to the silencing gates (active chromatin stays active — open chromatin wins, D-005); shown as `active` in profile. (Term audit, reg-bio: acetylation is permissiveness — neutralized lysine charges open the chromatin — not dispatch priority; the mark's honest effect is immunity, nothing else.)
- **`@methylate`** — histone methylation mark: gene is repressed, **graded** (v2.2). Every executed `@methylate`-marked `gene` definition deepens that gene's silencing level by 1; an `@acetylate`-marked definition relaxes it by 1 (histone marks compete on the same chromatin). A call to a gene whose level has reached the threshold (default **3**, tunable via `.cell` `methylate.threshold = n`) is **blocked**: it returns `null` with a fallback note ("methylation silences: … — call returns null"), never reaches the gene body, and does not count in `fingerprint()`/burst telemetry — repressed means repressed. Below the threshold, calls execute and the first call emits the soft note "methylated call" (later calls are silent — the cell does not narrate every repression; suppressed entirely when `.cell` sets `methylate.quiet = true`). Marked genes are exempt from the NMD untranslated sweep, and `@acetylate` genes are exempt from the methylation gate itself (open chromatin wins). One mark (the common case) never silences — old programs keep running.
- **`@m6a`** — m6A mark = dispatch priority + redefinition resistance (term audit, reg-r4: the honest analogy for "the marked original wins over the unmarked new copy" is prokaryotic **DNA** m6A — Dam methylation marks the parent strand so mismatch repair knows which base is the error; eukaryotic RNA-m6A "stability" is reader-dependent and the wrong analogy): among same-name candidates (splice variants, shadowing), the m6a-marked one wins resolution (variant selection order: `.cell` > CLI > **m6a-marked** > first declared — a marked variant beats first-declared), and an m6a-marked binding **resists redefinition** — an unmarked re-`gene` of the same name is ignored with a note ("@m6a-stabilized; redefinition ignored"); mark the new copy too, to replace it.
- **`silence old -> new strength s sites n;` / `silence old;`** — RISC-style silencing, now STOICHIOMETRIC (reg-bio-3, C9). The redirect form: every call to gene `old` is redirected to `new` with a note "RISC: call silenced". The target-less form (reg-bio) is **pure degradation** — real RISC/miRNA destroys the transcript, there is no replacement gene: every call to `old` returns null with a "RISC degraded" note, and a degraded call is not expression (it never reaches the call counters or burst telemetry). **Dose (reg-bio-3):** `strength s` is the per-site capture probability (clamped 0..=1, default 1.0) and each `silence` statement for the same target is one binding site (`sites n` composes multiplicatively, 1..=64): the per-call capture probability is `1 − Π(1−sᵢ)^sitesᵢ`. Real RNAi is dose-dependent — limiting RISC complexes give fractional knockdown, and multiple target sites compound. A sub-1.0 capture draws ONCE per call attempt on the shared mirrored xorshift64* stream (the telegraph-promoter discipline; the oracle mirrors it op-for-op); a captured call takes the redirect/degrade path, an escaped call proceeds through the pinned funnel with a one-time "RISC escape" note. `strength 1.0` with one site is the legacy binary redirect and consumes NO entropy — the `random()` stream is untouched for every legacy program. `@acetylate` genes are immune, and the immune check precedes any draw (immunity costs no randomness). Same-name splice variants resolve first, then silencing applies to the resolved name.
- **NMD sweep** (`operon check --nmd`): "premature stops" = unreachable statements after an unconditional `return` (reported −4 each); "untranslated transcripts" = defined, never-called, non-exported, non-enhanced genes (info, −1). `--nmd purge` rewrites the file without them.
- **`splice name { variant a { } variant b { } }`** — alternative splicing: `name(...)` dispatches to the active variant. Selection order: `.cell` `variant.name=v` > CLI `--variant v` > **m6a-marked variant** (v2.2) > first declared. Variant declarations may carry histone marks (v2.2): `@m6a variant v { }` makes `v` the m6a-priority variant; `@acetylate`/`@methylate` on a variant ride on the resolved binding (acetylate immunity applies when a marked variant is active). Marks stack and can precede the variant keyword in any order; a mark not followed by `variant` is noted and skipped. Variants take parameters. `operon build --variant a` bakes one variant into a standalone file (build caveat: a variant's parameter list is not yet carried into the baked file — bake param-less variants, or keep the splice in the source).
- **`.rna` edit patches** — `edit target { replace "src" -> "dst"; }` where target is a file name or gene name; applied to that target's source text before parsing. CLI: `operon run app.op --rna hot.rna`. Each applied replacement emits a note.
- **`.cell` methylation config** — `key = value` lines, `[section]` headers, `#` comments. CLI `--cell f.cell`; else `operon.cell` auto-detected. Read via `methyl("k", d?)`. Impl-consumed keys: `variant.<splice>` (active variant), `methylate.quiet` (suppress the methylated-call note), `methylate.threshold` (graded-silencing gate, default 3), `grn.decay` (default GRN pulse decay, reg-r2), `run.timeout_ms` (child-process wall-clock timeout for `run()`, default 10000, clamped 1..300000), `py.timeout_ms` (Python-bridge wall-clock timeout for `py()`, default 10000, clamped 1..300000, substrate-r1), `wobble.strict = true` (a run with any rung ≥ 3 note exits 3, same as `--strict`), `entry` (cap-independent default entry, below CLI `--entry` and above `main`), and `allow.*` capability grants — honored only with explicit `--cell` (§9b; `allow.exit = true` grants the exit capability, sec-r2), `operon.polarity` (loop-9 D1 expected-value polarity weight, default 0.5), and the loop-10 Rho/queue knobs: `rho.termination` (the opt-in layer switch, default off), `rho.catch` (per-cistron catch probability, clamp 0..1, default 0.5), `rho.queue_floor` (shield threshold, default 0.5), `ribosome.queue_cap` (queue saturation, default 1.0), `ribosome.drain` (per-integration drain, default 0.5).
- **`ires name;`** — internal ribosome entry site: declares a cap-independent entry gene. `operon run --ires` overrides the canonical `main` entry and runs the first declared `ires` target; a file with no `main` uses it automatically. Entry precedence: CLI `--entry` > `.cell` `entry` > `main` > first `ires`.
- **`fate Name { state a -> b, c; state b -> a; enter a; }`** — fate landscape state machine. `let m = Name()` enters the `enter` state (default: first declared). Methods on instances: `.shift(s)` (true if transition allowed; invalid → note + false, state unchanged — the valley stay), `.state()` (current), `.can(s)` (bool). Instances are Maps with hidden `#fate`/`#state` keys.
- **`regulate { a activates b strength 0.8; c inhibits d; }`** — gene regulatory network (default strength 1.0). The network is **stateful**: by default levels persist across `grn_fire` calls (a latch — the honest default; real dilution needs the decay below), and each fire seeds additively, capped at 1.0. **Decay (reg-r2):** opt-in dilution — before every pulse, every existing level decays by a configurable fraction: `grn_fire(name, f)` takes the pulse's decay directly (0..1, clamped), or `.cell` `[grn] decay = f` configures it globally; unset/0 is byte-identical to the pre-decay behavior. (Term audit, reg-r3: persistence is a latch, decay-to-zero is dilution/degradation — neither is "homeostasis", which requires a regulated setpoint.) Each fire runs in two phases: (1) **activation** propagates in waves, `child = max(child, parent × strength^wave)` — hop attenuation over up to 10 waves; (2) **inhibition** is applied exactly once per fire, `child = max(0, child − parent × strength)`, based on the source's post-activation level — a repressor's concentration sets the output level, it does not compound across waves. An edge may carry an optional `threshold t` (dose-response): influence = `strength × pⁿ/(pⁿ + tⁿ)` where p is the parent level — weak below the threshold, saturating above it. The exponent `n` is the **Hill coefficient** (cooperative binding, reg-bio): `hill n` (integer 1..=8; canonical edge order `strength → threshold → hill → any`) overrides the default n = 2 per edge — `hill 1` is the graded Michaelis shape, higher n is ultrasensitive, and the exponent is computed by repeated multiplication (never `pow`) so both implementations stay bit-identical. An edge may also carry the keyword `any`: it becomes an **OR member** — an alternative activator that alone suffices. `hill`/`any` on an edge without a threshold, or `any` on an inhibitor, are noted and ignored (Total Grammar). `grn_state()` returns the level Map (all genes in the graph); `grn_set/grn_get` write/read single node levels (clamped 0..1) so hosts can steer gate state without re-firing. **Levels gate calls** (v2.2): a gene with an incoming activating edge carrying an explicit `threshold t` executes only while `level(source) ≥ t`; several activating edges form **cis-regulatory input functions** (reg-bio) — every AND member must pass, and an `any` (OR) member alone suffices: the gate opens iff (every AND member passes) OR (any OR member passes); an all-OR promoter with every member below its threshold vetoes with "no OR activator above threshold". An incoming inhibiting edge with an explicit threshold silences the callee while `level(inhibitor) ≥ t` — inhibitors veto independently of the OR group. A gated call returns `null` with a fallback note ("grn gate: … call suppressed"). Edges **without** a threshold remain declarative (they shape `grn_fire` level dynamics only), and a threshold of `0` never blocks: declaring a network changes no call behavior unless a program opts in with explicit thresholds. The gate is checked at the gene-call funnel (named calls, higher-order calls, and RISC-redirected calls all pass through it) and precedes the call counters — a suppressed call is not expression and does not count in `fingerprint()`/burst telemetry.
- **`toggle a, b;`** — a **mutual-repression latch** (term audit, reg-r3: a declared boolean invariant, not rate-based bistability — no cooperativity, no hysteresis; the Gardner–Cantor–Collins toggle switch is the biological inspiration, not a simulated model). `toggle_on("a")` turns `a` on and `b` off; `toggle_state()` → Map of both. Exactly one may be on — and the pair gates **calls**: calling the repressed allele returns `null` with a note ("toggle repressed: … is the inactive allele"); `@acetylate` genes are immune (open chromatin wins). Rate-based bistability IS expressible on the `regulate` layer: two mutual inhibiting edges with `hill ≥ 2` land the losing allele in a partial, cooperativity-dependent band (tests/grn_bistability.op proves the cooperativity-dependent partial bands; HYSTERESIS is not yet demonstrated by a sweep test — loop-8; `std/motifs` ships the circuit).
- **`repressilator a -> b -> c;`** — repressive oscillation ring with **emergent mutual repression** (reg-r2): the ring integrates a **reduced, protein-only discrete variant inspired by the Elowitz–Leibler repressilator** (Elowitz & Leibler, Nature 2000; term audit, reg-r3: the published system is a two-state mRNA+protein model with Hill n=2, α≈250, β=5 and a basal term — this is the one-state protein-only reduction with h=4 and no basal term, which oscillates robustly in discrete time). Each substep, node level evolves as `dA/dt = α / (1 + Rʰ) + basal − γA` where R is the level of the repressor (ring `a -> b -> c` means a represses b, b represses c, c represses a), Euler-integrated with 20 substeps of `dt = 0.05` per ring tick (α = 10, γ = 1, init `[5, 0, 0]`). The oscillation — a ~6-tick limit cycle with one-tick production lag between a repressor's peak and its target's trough — **emerges from the loop itself; nothing is scheduled**, so gates can read the ring: a GRN edge sourced by a ring node reads the node's normalized level (`raw/α`, clamped 0..1) at the current tick. `repressi_state()` returns the level Map and is a **pure function of the tick count** — manual mode (`repressi_next()` advances one ring tick) and wall-clock mode (`repressilator … period s;` or `repressi_start(ms)`) fold the identical arithmetic and can never diverge; the Python oracle mirrors it op-for-op bit-identically (§17). **Kinetics are load-bearing (reg-bio, F-5):** inline `repressilator a -> b -> c alpha 20 basal 0.5 noise 0.1 seed 42;` (canonical order alpha, gamma, hill, basal, noise, seed) or `.cell` keys `repressi.alpha/gamma/hill/basal/noise/seed` override the historical constants per-field (the last declaration wins per-field; `.cell` applies first, declarations layer on top). `basal` is the promoter leak real repressed promoters never lose; `noise` injects a per-substep kick — reg-bio-2 (12-d D6): the kick is MULTIPLICATIVE and dt-aware, `v *= 1 + noise·(1/√20)·(2u−1)` (real expression noise is multiplicative and strictly positive; the old additive kick rectified upward through the zero clamp) — drawn from a stream derived from the ABSOLUTE (tick, substep) position — so the fold-from-init path and the incremental cache path stay bit-identical and the oracle mirrors it op-for-op — and it never perturbs the program's `random()` stream. Defaults (α=10, γ=1, h=4, basal=0, noise=0) are byte-identical to the pre-parameterized ring. A second `repressilator` declaration replaces the ring. The ring rides `spawn` snapshots frozen at the spawn tick (§13).
- **Telegraph promoter layer (reg-bio, F-1) — stochastic expression.** Real promoters switch between active and inactive states; transcription happens in **bursts**, and noise is what lets bistable cells switch fate. Off by default (the deterministic contract is untouched for every existing program). `expr_on(kon?, koff?)` — or `.cell` `[expression] stochastic = true` with `kon`/`koff`/`seed` — turns it on: each gene carries a promoter state (initially active) and every call attempt draws once on the shared mirrored xorshift64* stream — an active promoter switches off with probability `koff`, an inactive one switches on with probability `kon`. A burst-off call returns `null` with a "promoter inactive" note, never reaches the call counters, and is recorded in `fingerprint().bursts`. The state persists across calls — that persistence IS the burst. `randomize(seed)` fixes the stream (reproducible bursting, identical in both implementations); `expr_off()` restores determinism mid-run. Bursting is universal (open chromatin bursts too): no `@acetylate` exemption. Worker cells inherit promoter states and burst counters in the regulation snapshot (§13). **Per-gene promoter identity (loop-9, F-2):** `@burst kon koff` marks a gene with its OWN telegraph rates (clamped 0..=1), overriding the global parameters for that gene alone — different promoters have different (kon, koff); that is their identity. The mark rides the gene definition (workers inherit via the Arc); draw count and stream order are unchanged. **Attempt telemetry (loop-9, F-3):** `promoter_telemetry(gene)` returns `{attempts, on_total, episodes, on_frac, burst_size}` — every call that reaches the promoter gate is an attempt; episodes are maximal ON-runs; `burst_size = on_total/episodes` is the mean ON-run length, ~1 under Poisson-like firing and >>1 under bursting. Telemetry rides the snapshot so worker cells never desync from the host mid-burst. Boundary honesty: episodes count COMPLETED ON-runs, so a run ending mid-ON slightly overestimates burst_size (the trailing episode is uncounted) — finite-sample bias, deterministic, documented here rather than hidden. tests/burst_identity.op pins the identity separation on both cores; tests/burst_modulate.op pins runtime modulation AND the `@burst > burst_set` precedence. **Runtime modulation (loop-9, R9 jury):** `burst_set(gene, kon, koff)` retunes ONE promoter's switching rates at runtime — regulators change promoter switching, not just transcript gating; `burst_set(gene)` clears the override. Precedence: `@burst` mark > `burst_set` override > global rates; rides the snapshot (workers freeze modulation at spawn). tests/burst_modulate.op.
- **`a translates p rate r decay d;` (reg-bio-2, C1) — the translation layer.** Real expression is TWO coupled tiers: transcripts accumulate fast and bursty; proteins accumulate slower (translation), lag, and smooth the bursts. A `translates` edge (inside `regulate`, verb `translates`; optional `rate` default 1.0, `decay` default 0.0) makes the target protein node integrate one Euler step of the classic two-tier ODE at every engine update point (each `grn_fire` pulse and each decay-clock tick): `p += rate·Δcalls − decay·p`, clamped 0..1, where Δcalls is the source gene's call-count delta since the last integration (checkpoints start at 0). Protein nodes live in the GRN level map, so any gate can be regulated by a PROTEIN instead of a transcript — the observable signature is the two-tier delay (mRNA present, protein still below threshold; tests/grn_translates.op proves the gate opens later than one-tier). (Term audit: Δcalls counts EXECUTED CALLS, not molecules — transcripts-as-calls is the language's own unit; the smoothing/lag/persistence phenomenology is what this layer models.)
- **`occupy` (reg-bio-2, D2b) — occupancy repression.** An inhibiting edge marked `occupy` composes MULTIPLICATIVELY: `child *= 1 − influence` — the thermodynamic Kⁿ/(Kⁿ+Rⁿ) survival form. Two partial repressors survive more than subtractive repression would allow (1 × 0.6 × 0.6 = 0.36 vs 1 − 0.4 − 0.4 = 0.2), repression can never overshoot, and near-full occupancy approaches (never crosses) zero. Canonical edge order: `strength → threshold → hill → any → occupy → sum`. Legacy edges keep the subtractive-once form (bit-identical default). (Term audit: occupancy shapes the fire-phase dose arithmetic; the call-gate veto remains threshold-based — occupancy is expressed through the levels it produces.)
- **`sum` (reg-bio-2, B7) — synergistic pooling.** `sum` on a thresholded activating edge makes it a POOLED member: edges targeting the same gene with the same (threshold, hill) pool their weighted inputs `P = min(1, Σ strength·level)` and ONE Hill function of P drives both the gate and the fire influence. Two sub-threshold inputs (0.3 each, threshold 0.5) fire together — enhanceosome synergy that individual AND members can never express — and `hill 2` makes the pooled response super-additive (pooled influence > sum of individual influences). Groups act as conjunctive members: the gate opens iff (every AND member passes) OR (any OR member passes), where a pooled group passes iff `P ≥ t` (with the enhance boost). Message order: individual members first, then pooled groups in declaration order.
- **`decoy d for tf capacity c;` (reg-bio-2, C11) — competitive titration.** A decoy node absorbs its regulator without producing output: every regulation read of `tf` sees the free fraction `max(0, level(tf) − c·level(d))`. Overexpressing the sponge closes gates that were open; emptying it restores the free fraction. Real TF sequestration/decoy-site titration, deterministic, no RNG.
- **`ligand x;` + `bind tf inducer lg k v;` / `bind tf cofactor lg k v;` (reg-bio-2, A4) — small-molecule allostery.** Ligands are metabolites, not genes: `ligand_set("iptg", 0.9)` sets a pool (clamped 0..1), `ligand("x")` reads it, and the `.cell` key `ligand.<name> = f` is the bath default (the runtime pool wins). A binding record modulates the regulator's DNA-available fraction at EVERY regulation read: `occ = L/(k+L)` (k defaults 0.1); an INDUCER reduces affinity — free × (1 − occ) (allolactose on LacI: binding relieves repression); a COFACTOR increases it — free × occ (tryptophan on TrpR: binding enables repression). THE POLARITY IS THE BIOLOGY: lac's inducer turns a repressor OFF, trp's corepressor turns it ON, and in both cases the regulator's LEVEL never changes — allostery, not dilution. A ligand named as an edge SOURCE gates calls straight from its pool — a riboswitch-style, protein-free gate (no `grn_fire` needed). tests/lac_gate.op proves the canonical IPTG + cAMP dual control; tests/trp_attenuator.op proves the corepressor polarity mirror.
- **`attenuates` (reg-bio-2, A5) — RNA-level attenuation.** An edge `x attenuates y threshold t;` vetoes like an inhibitor but reports the RNA-level mechanism: "attenuator … (leader terminated)" — the OUTCOME of ribosome-coupled leader attenuation (trp operon), not TF occlusion. (Term audit: the ribosome-stalling mechanics are not modeled; the metabolite-threshold outcome is.)
- **`decay_clock(n, f?)` (reg-bio-2, C2) — time-driven decay.** The call clock is the language's in-model timebase; `decay_clock(n, f)` fires one GRN decay step (fraction f, default from `.cell [grn] decay`) every n calls, and the translation layer integrates on the same ticks. Decay now runs as expression time passes — real transcripts decay per unit time, not only when someone calls `grn_fire`. `.cell [grn] decay_calls = N` is the config form; `decay_clock(0)` restores the event-driven contract (byte-identical default). (Term audit, C4: call-clock time is an explicit design choice — genes are closures the programmer invokes; a Gillespie SSA scheduler would invert the language's own metaphor. The ring remains the continuous-time enclave.)
- **`operon Name { cistron rbs r; cistron2; }` (reg-bio-3, A1/A7) — the polycistronic transcription unit.** The namesake construct: ONE promoter drives N cistrons on ONE polycistronic mRNA. A call to ANY cistron is a transcription attempt of the WHOLE unit: edges targeting the UNIT gate every member first (induction acts on the unit's promoter — one edge opens or blocks all cistrons), then the cistron's own edges apply as usual. The unit pass short-circuits: its message wins over per-cistron messages ("grn gate: 'lacZ' call suppressed (operon 'lac': …)"). An `attenuates` edge targeting the unit is the trpL leader: when it fires, NO cistron is transcribed (the whole polycistronic mRNA is lost). **Per-cistron `rbs r`** is the translation-efficiency multiplier (Shine-Dalgarno strength, clamped 0..=1, default 1.0 — distinct from edge `strength`, a binding weight): every `translates` edge sourced at a cistron has its rate scaled by the cistron's rbs, so the lacZYA stoichiometric gradient lives in the two-tier protein nodes (tests/operon_unit.op proves pz:py:pa = 1.0:0.6:0.3). **Transcriptional polarity** (reg-bio-3, re-weighted loop-9): an upstream cistron that is target-less RISC-degraded or methylated-past-threshold derates transcriptional read-through — every downstream cistron's protein integration is multiplied by the upstream members' expected factor (a fully-captured or methylated upstream member contributes the full `polarity`; a partially-captured one contributes proportionally — "blocks" would be the old binary rule). **Rho-dependent termination (loop-10, F-7 — SHIPPED, opt-in):** when `.cell rho.termination = true`, the pinned funnel extends to RISC → toggle → GRN → methylation → riboswitch → promoter → RHO. After the promoter gate, a called cistron scans its upstream members in transcriptional order: an upstream cistron whose translation fails on THIS attempt — methylated-past-threshold (deterministically naked) or a target-less RISC silence whose per-call capture draw fires — is NAKED RNA: Rho loads, chases, and terminates the rest of the transcript for THIS call with probability `1 − (1−catch)^d` (`.cell rho.catch`, clamp 0..1, default 0.5; d = the naked runway in cistrons between the failure and the reader — the FURTHER downstream the reader, the MORE time Rho has had to catch up, so the probability GROWS with distance; per-cistron compounding by repeated multiply — never powf). The terminated call returns null with a `rho terminated: transcript lost at '<g>'` note and counts NOTHING (no counters, no transcript, no queue — a terminated call is not expression). The first naked-AND-unshielded member decides; `catch = 0.0` is read-through-certain, `catch = 1.0` termination-certain. **Composition rule (R10 W2):** under Rho ON the D1 polarity factor is IDENTITY — D2 resolves the upstream failure per call (terminate or read-through), so a SURVIVING transcript is whole and translates at full rate; the D1 expected-value derate applies only when the Rho layer is off (it is the expectation-form stand-in for the same loss). (Not modeled: Rho loading kinetics, RNAP velocity, rut-site sequence strength, antitermination, tmRNA/SsrA rescue — the per-cistron threshold-draw abstraction is the model.) **Ribosome-queue coupling shield (loop-10, F-8):** every successful unit call grows EVERY member's queue by its rbs (the Shine–Dalgarno initiation propensity — one polycistronic transcript loads every cistron's ribosomes), capped at `.cell ribosome.queue_cap` (default 1.0); every integration tick drains each queue by `.cell ribosome.drain` (default 0.5); a queue ≥ `.cell rho.queue_floor` (default 0.5) occludes the rut sites — Rho cannot fire behind a translated cistron (the transcription–translation coupling trait; captured calls grow nothing — their transcript never existed). Honesty note (R10-M4/M6): the register is an INITIATION-FLUX abstraction, not elongation-ribosome pile-up — occupancy shields the failure POINT, not the naked runway past it; both boundaries are pinned in tests/granted/rho_queue_shield.op. The queue register and resolved knobs ride the snapshot (§13 — worker cells fold the parent's termination math exactly). ENTROPY: flag off = zero draws (bit-identical legacy); flag on draws only where 0 < p < 1 (member order) and where 0 < q < 1 (one catch-up draw) — the C9 p∈{0,1} no-draw discipline. The granted-lane proofs (tests/granted/rho_*.op under explicit operator cells) pin the deterministic extremes, the exact seeded distance-decay counts, the shield's ≥-floor boundary, drain re-exposure, and worker parity; harness.py's granted targets pin the byte-exact ENTROPY-STREAM parity on both cores; rt_p14e holds the 1500-cistron load shape contained. Methylation-past-threshold contributes `polarity` outright; a target-less RISC silence with per-call capture `p = 1 − Π(1−s_i)^sites_i` contributes `surv + (1−surv)·polarity` with `surv = Π(1−s_i)^sites_i` — a 1%-capture silence now derates by 1%, not by the full `0.5^1` the old binary-existence rule imposed (`polarity` default 0.5, `.cell operon.polarity`; silent at the note layer, observable through levels; no randomness consumed — this is the expected value). Cistron ORDER is load-bearing: it determines the RBS gradient and polarity exposure. Membership is by name (cistrons may be defined after the unit); redefinition replaces (last wins); a cistron owned by another unit is ignored with a note; a unit without targeting edges is inert (declarative). Each call that passes every gate makes ONE transcript of the unit (`fingerprint().transcripts`, §14 — a suppressed call is not expression). Worker cells inherit the unit registry (§13). (Term audit, 13a: documented as PROKARYOTE-specific — eukaryotes lack polycistronic transcription; the construct is not "call these genes together" sugar: the unit-level transcript counter, per-cistron gradient, and polarity are the biology.)
- **`passage(n)` (reg-bio-3, B2/B6) — cell divisions and epigenetic inheritance.** Real epigenetic marks are maintained across replication only by maintenance machinery (DNMT1-style copying); without it, marks dilute ~50% per generation — dilution is the null model. `passage(n)` advances the cell n divisions: every methylation level is multiplied by `.cell methyl.maintenance` (default **0.5** = pure dilution; 1.0 = perfect maintenance, marks stable; 0.0 = instant loss) with **half-down rounding** on the 0..=3 lattice — a diluted mark never reads as MORE repressed (3 → 1 → 0 at 0.5; 1.5 rounds down to 1). `generation` counts divisions (saturating u64; `fingerprint().generation`, §14) — spawn is a thread, not a division, and never touches it. `n` is clamped to 1,000,000 with a note (a culture that old is not a useful model). tests/passage_dilution.op proves the dilution ladder and the generation counter.
- **`m6a_write(g, n?)` / `m6a_erase(g, n?)` (reg-bio-3, B3) — quantitative m6A site density.** The @m6a mark is now a LEVEL 0..=3 per gene (prokaryotic Dam-style DNA-methylation analogy per the term audit above — no eukaryotic mRNA-reader drift): every executed `@m6a`-marked definition deepens the level (+1, capped 3), and dispatch/redefinition resistance requires **level ≥ 1** — the legacy bool is exactly the {0,1} sub-lattice, so mark/unmark programs are bit-identical while the builtins make the mark quantitative: write adds writer-complex dose (clamped 3), erase removes eraser dose, and an ERASED mark truly releases the redefinition (the old bool could not). `.cell m6a.decay f` erases site density on every decay-clock tick (`level × (1−f)`, half-down) — higher density persists longer. **Standalone cadence (loop-9):** with no GRN decay clock configured, `m6a.decay` runs on its own cadence — every `m6a.decay_calls` calls (default 1) — instead of being a silent no-op behind the GRN tick's early returns. Levels live on the regulation snapshot (workers inherit).
- **`splice_shift(root, variant)` (loop-9, F-4) — runtime splicing-factor regulation.** Splicing factors (SR proteins, hnRNPs) CHANGE which splice site wins at runtime. The shift slots into variant selection between the operator pins (`.cell variant.<root>` → `cli.variant`) and the @m6a mark: a bound factor beats a basal inclusion bias, the operator still has the final word. The builtin rebinds the splice root TRANS — up the environment chain, so every future transcript of the root uses the shifted variant (in-flight calls finish on their resolved definition — no mid-call body swap, ever); returns the now-active variant name; soft-fails (rung-4 note + Null/current name) on unknown root/variant; zero RNG; rides the snapshot (workers freeze the shift at spawn). tests/splice_shift.op.
- **`@riboswitch ligand off|on threshold t` (loop-9, F-5) — the CIS riboswitch.** The aptamer lives on the transcript it controls: the metabolite pool is cell-wide (`ligand_level`), the SENSOR is per-gene. `off` class (TPP/purine/SAM): ligand bound (level ≥ t) → terminator hairpin → transcription OFF. `on` class (adenine/glycine activators): unbound → RBS sequestered → OFF. Gate order extends the pinned funnel to RISC → toggle → GRN → methylation → riboswitch → promoter → RHO (loop-10, opt-in) (DNA-level chromatin first, then the RNA-level element, then coupling-dependent termination) on both call paths; a terminated transcript is not expression (null before the counters). `@acetylate` does NOT immunize — the RNA hairpin is downstream of chromatin. The trans ligand EDGE remains the protein-free metabolite-sensor analog (the old "riboswitch-style" label for it was wrong — the A2 agent's finding). tests/riboswitch_cis.op.
- **m6A READER fate (loop-9, F-6).** The mark is Dam-style resistance at density 1; at density ≥ 2 (`.cell m6a.reader.min_level`, default 2) eukaryotic READER fate engages on `translates` edges: YTHDF2-like decay routing multiplies the production term by `1 − m6a.reader.decay` (default 0.25) and YTHDF1/3-like attenuation adds `m6a.reader.translation` (default 0.10) to the protein decay — normative order `rate × rbs × polarity × (1−yd2)`. The {0,1} legacy lattice is bit-identical; resolved knobs ride the snapshot so workers fold the parent's math. tests/m6a_readers.op.
- **`autoinducer x;` + `secrete` / `quorum` / `quench` / `quorum_state` (loop-9, C8) — quorum sensing: the population layer.** Real bacteria share diffusible autoinducers (Vibrio fischeri LuxI → AHL; LuxR·AHL activates the lux operon at a density threshold; AiiA quenches; S. aureus agr, E. coli AI-2). The signal is EXTRACELLULAR and SHARED — my secretion raises YOUR activation — so the medium is process-global, not cell state: it is deliberately NOT part of the spawn snapshot (workers inherit frozen cytoplasm but a LIVE medium, handed off explicitly like the fuel pool). The pool stores INTEGER molecule counts (u64, saturating cap 1e9): addition is exact, associative and commutative, so concurrent worker secretions COMMUTE — thread interleaving cannot change the pool (the cross-thread answer to the burst_total float-sum lesson). `secrete("ahl", n)` charges (Int exact; Float floors — never rounds; negative/non-finite clamp to 0 with a note; species auto-registers, cap 64); `quorum("ahl")` reads the level `molecules/1e9` (ONE division — bit-identical both cores); `quorum("ahl", t)` compares `>= t`; `quench("ahl", f?)` degrades `floor(m × (1−f))` (AiiA lactonase; no f destroys all); `quorum_state()` is sorted-key telemetry. A species named as an EDGE SOURCE is the LuxR·AHL gate — full AND/OR/sum/occupy/Hill composition for free (resolved after explicit grn levels and ligand pools; `grn_set("ahl", v)` overrides the medium; `grn_fire("ahl")` refuses with a note — the level lives in the medium). Dilution: `passage(n)` multiplies every species by `.cell quorum.dilution` per division (default 0.5, binary-exact halving; 1.0 chemostat; 0.0 full exchange). Contract: cross-cell reads are deterministic once every secreting worker is JOINED (join-before-read; reads over unjoined workers are outside the contract). No capability gate — the medium dies with the process; there is no persistence and no path out of the sandbox (a future file-backed culture would be default-deny `allow.signal`). tests/quorum_basic.op (pool semantics), tests/quorum_lux.op (the LuxI positive-feedback all-or-none circuit — the synthase sits IN the operon it activates), tests/quorum_population.op (cross-cell commutation + dilution), tests/quorum_gate.op (AND/sum/resolution-order composition); redteam rt_p15a (species flood + saturation + hot loop) / rt_p15b (the unjoined-read class).
- **`@copies n` (reg-bio-3, C10) — gene dosage.** Copy-number variation: `@copies n` (clamped 1..=64 with a note) amplifies the CONCENTRATION the gene feeds its GRN edges — every regulation read of the gene's level sees `level × copies`, saturating on the 0..1 lattice like real transcript dose under titration. Copies change transcript AMOUNT, never the call's return value (a call is a transcription event; its return is the per-transcript product) and never the raw `grn_get` level (the dose is a read-side effect). tests/copies_dose.op proves 0.3×2 opens a 0.5-threshold gate and the return contract.
- **Determinism hardening (reg-bio-2, D9):** `grn_state()`/`grn_fire()`/`fingerprint().calls` emit in SORTED key order (HashMap iteration order varied per process — a proof-frame and oracle-parity hazard), `burst_total` accumulates in sorted order (float addition is not associative), and the variance uses explicit multiply (never `powi`). Worker cells derive their RNG stream from the task id (`DEFAULT_SEED ^ id·GOLDEN`) — the old shared default seed synchronized promoter bursts across cells (perfect correlation — the exact OPPOSITE of extrinsic noise, 12-c C3); sequence-generator cells still share the default stream (loop-8).
- **Level hygiene (reg-bio-2, D2c/D7):** edge `strength` clamps to 0..=1 at parse with a note (a binding weight is not an amplifier), and fire-phase influence clamps at 1.0 — a level is a concentration fraction; legacy programs (strength ≤ 1) are bit-identical. `strength > 1` previously pushed levels past saturation, breaking the `grn_set` clamp invariant.

- **Gate order (reg-r4, pinned; reg-bio extends the tail):** for every call — named, value-bound (higher-order), RISC-redirected, sequence-creation, and worker-cell — the gates apply in ONE order: **RISC redirect first** (silencing rewrites the callee, so it wins), then the **toggle gate**, then the **GRN veto**, then the **methylation gate**, then the **promoter gate** (the burst draw is the promoter's own stochastic dynamics, downstream of every trans/epigenetic gate). Value-bound gene calls pass the toggle gate like named calls (an unqualified "the pair gates calls"); a ring node named by an edge source contributes its normalized oscillation level only when no explicit `grn_set`/`grn_fire` level exists (explicit levels win).

## 12. Frames, proofs, overlapping reading frames

- `frame proof { assert(...); ... }` — the **test reading frame** of the file. Skipped by `operon run`; executed by `operon test`. The same file encodes program + tests (two reading frames over one sequence).
- `frame name { ... }` — named frames (metadata/optional scenes); runnable via `operon run --frame name`.
- `operon test [paths...]` — default paths: `tests/` recursively. For each file: run its proof frames; a proof failure (Stress burned) is recorded; the suite continues (Total Grammar). A proof must **run to completion** — an early `return`/`break` inside a proof fails it ("exited early"), and a proof that exercises **zero assertions** fails it ("vacuous proof"). Exit code 1 if any failure. Report: files, proofs run, passed, failed, assertions exercised, wobble notes count. Current suite: 99 files / 92 proofs / 1171 assertions, all green on both implementations, plus 8 granted-lane proofs under explicit operator cells (py bridge, m6A decay cadence, and the 5 loop-10 Rho/queue proofs) — the differential harness verifies 128 byte-exact targets (123 zero-grant sweep + 5 granted-with-cell).

## 13. Concurrency

- `spawn(f, args?)` — starts a real OS thread running gene `f`; returns task id (Int). `join(id)` waits and returns the result (second join → Null + note). Arguments and results cross by serialization (named genes, lambdas, and phenotype instances cross; a running sequence object does not).
- Thread panics are impossible by construction: any stress inside the thread is returned as a Stress Map value.
- Memory model note (honesty): values are reference-counted; tasks communicate by args/results, not shared mutable state. Data races on shared globals are prevented by design (closures capture is by value at spawn time for non-local references).
- **Worker cells inherit regulation state (reg-r1; reg-r4 inventory; reg-bio-3 extends).** A spawned task or sequence cell starts with a copy of the parent's GRN edges + levels, silences (stoichiometric RISC incl. escape bookkeeping), polycistronic operon units (membership, order, rbs, transcript counters), m6A levels, generation counter, gene-dosage registry, methylation counters + threshold, toggle pairs, enhance marks + enhancer dose (reg-bio), the telegraph promoter states + burst counters (reg-bio), **and the repressilator ring (node names + the raw ODE levels frozen at the spawn tick + the ring's kinetic parameters — the cell does not live-tick)**, frozen at spawn time. Worker calls dispatch through the same funnel as the host, so a toggle-repressed allele, a silenced (level ≥ threshold) gene, a GRN-vetoed call, or a burst-off promoter returns null inside the cell exactly as it does outside — regulation is part of the cell, not a host-side illusion. Later parent-side regulation changes do NOT propagate to already-running cells (snapshot semantics).
- Sequences (§7b) run on the same worker-cell substrate: each sequence body is a worker thread pulling through a rendezvous channel.

## 14. Telemetry — the single-cell layer

- `fingerprint()` returns Map:
  - `calls` — Map gene → call count (phenotype methods count as `Name.method`).
  - `mature` — genes called at least once.
  - `nascent` — genes defined but never called.
  - `maturation` — mature / total defined genes (the transcript-maturation share).
  - `burst` — the aggregate **burst index**: mean over genes of (variance / mean) of per-gene call counts across complete 20-call bins of the run's call clock.
  - `burst_by_gene` — the per-gene burst indices (0 for a gene with ≤ 1 call or one bin).
  - `bursts` — Map gene → promoter **burst-off count** (reg-bio): calls suppressed by the telegraph promoter layer. Empty unless stochastic expression is on; with it on, `burst` measures promoter-driven variance in real expression rather than call patterns alone.
  - `transcripts` — Map unit → polycistronic transcript count (reg-bio-3, A1/A7): one successful cistron call is one transcript of the unit; suppressed calls count nothing. Empty unless an `operon` unit is declared. Sorted-key emission (D9).
  - `generation` — the division counter (reg-bio-3, B2/B6): advanced by `passage(n)` only (spawn is a thread, not a division).
- Method: the run's global call clock is sliced into windows of 20 gene calls; each gene's count per window is a sample. A gene fired in bursts has a high variance/mean ratio; a constitutively expressed one sits near 0. Only complete bins count (a trailing partial bin is dropped).
- `operon profile f.op` — runs instrumented, prints table: gene, calls, **exclusive self-time µs** (children subtracted), flags (`enhanced active repressed`), then a `mature · nascent · maturation` summary and **enhance candidates** — hot genes (called ≥ 10% as often as the most-called gene) that carry no `enhance` annotation.
- The v2.0 telemetry keys `spliced` / `unspliced` / `velocity` (and the per-gene variance/mean noise key) are retired; `mature`/`nascent`/`maturation` carry the same biology honestly (maturation share, not velocity).

## 15. Toolchain (Rust binary `operon`)

```
operon run f.op    [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires]
                   [--strict] [--quiet] [--fuel N]
                   [--allow-read p] [--allow-write p] [--allow-run prog]
                   [--allow-py module]                                   # substrate-r1
                   [--allow-net host:port] [--allow-env var] [--allow-all]   # §9b
                   [-- --args...]   # dx-r6: everything after `--` is program argv
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

**`operon-ls`** — language server (stdio LSP; lsp-r1 v2): `initialize` / `shutdown` / `exit`, full-text document sync (ranged edits are ignored, never mis-applied), `textDocument/publishDiagnostics` (Total Grammar parse notes by rung + `tools check` phantom calls, resolved **CWD-independently**: document-relative → CWD → `std/` → exe-relative `std/` → cargo-manifest `std/`), `textDocument/hover` with gene/splice signatures, `textDocument/definition` (genes, sequences, splice roots), `textDocument/documentSymbol`, `textDocument/completion` (in-file genes with signatures, builtins, keywords, top-level bindings), and `textDocument/formatting` (the canonical `operon fmt` engine). Analysis is cached per document version; `didClose` drops the document and clears diagnostics. Programmer-first by D-008: hovering `boost` shows `gene boost(x)` plus its marks (`@acetylate`, `@methylate`, `@m6a`, `enhance`) and a one-line analogy — gene vocabulary is an intuition aid, never a prerequisite. Zero external dependencies: request JSON is parsed by the language's own `json_parse`. Shipped in every release archive including the Windows zip; editor setup (Neovim / VS Code / Helix) is in README "Connect your editor".

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
| Runtime splice regulation (splicing factors) | `splice_shift(root, variant)` — trans-acting shift between the operator pins and the @m6a bias |
| m6A reader fate (YTHDF2 decay / YTHDF1-3 attenuation) | density >= 2 engages reader factors on `translates` (`.cell m6a.reader.*`) |
| Phenotypic state and differentiation | `phenotype` classes: `new`, `init`, `self`, inheritance `from` |
| Polypeptide elongation (values produced one at a time) | `sequence` generators + `yield` / `.next()` / `.collect()` on worker cells |
| RNA editing | `.rna` hot patches (`edit/replace`) |
| Upstream ORF repression | `guard (cond) else { }` leading clauses |
| DNA methylation / epigenetics | `.cell` config layer + `methyl()` |
| TADs + CTCF anchors | `tad` domains + `anchor export/import` |
| Super-enhancers | `enhance` clusters (dose via `.cell enhance.delta`) |
| Histone acetylation / methylation | `@acetylate` (permissiveness, not priority) / `@methylate` (graded marks) |
| Nonsense-mediated decay | NMD sweep (`--nmd`) |
| miRNA → RISC silencing / transcript degradation | `silence old -> new;` (allele replacement) / `silence old;` (pure degradation) |
| Two-tier expression (transcription → translation) | `a translates p rate r decay d;` — protein nodes lag & smooth transcript bursts |
| Small-molecule allostery (lac inducer / trp corepressor) | `ligand x;` + `bind tf inducer|cofactor lg k v;` — affinity modulation, level untouched |
| Riboswitches / protein-free metabolite gating | a ligand named as an edge source reads its pool directly |
| Quorum sensing (LuxI/LuxR AHL, agr, AI-2) | `autoinducer ahl;` + `secrete`/`quorum`/`quench` — process-global integer molecule pool, signal species as an edge source |
| Transcription attenuation (trp leader) | `x attenuates y threshold t;` — RNA-level veto with leader-termination report |
| TF sequestration / decoy binding sites | `decoy d for tf capacity c;` — free-TF titration |
| Enhanceosome synergy (cooperative pooling) | `sum` edge keyword — pooled Hill input, super-additive with hill > 1 |
| Thermodynamic occupancy repression | `occupy` edge keyword — multiplicative Kⁿ/(Kⁿ+Rⁿ) survival |
| Time-based degradation (half-lives) | `decay_clock(n, f)` / `.cell grn.decay_calls` — decay on the call clock |
| Extrinsic noise (cell-to-cell variation) | worker RNG streams derived from task id (decorrelated bursting) |
| m6A modification (quantitative site density) | `@m6a` dispatch priority + `m6a_write`/`m6a_erase` levels 0..=3 + `.cell m6a.decay` (Dam-style analogy — §11 term audit) |
| Polycistronic operons (lacZYA / trpEDCBA) | `operon lac { lacZ rbs 1.0; lacY rbs 0.6; }` — unit-level gate, one transcript, cistron order load-bearing |
| RBS strength gradient (per-cistron translation efficiency) | per-cistron `rbs r` multiplier on `translates` rates — the lacZYA stoichiometric ratio |
| Transcriptional polarity (expected read-through loss) | upstream degraded/methylated cistrons scale downstream yield — per-member expected factor `surv + (1−surv)·polarity` (loop-9 weighted rule; methylation contributes `polarity` outright) |
| Rho-dependent termination + ribosome-queue coupling shield | a failed upstream translation leaves naked RNA; Rho loads and chases — termination probability compounds per cistron of naked runway `1−(1−catch)^d` (loop-10, opt-in); ribosome occupancy (queue ≥ floor) occludes rut sites — translated cistrons shield, drained queues re-expose |
| Epigenetic maintenance vs dilution (DNMT1 / passaging) | `passage(n)` + `.cell methyl.maintenance` — half-down dilution, `generation` counter |
| Gene copy-number variation (dosage) | `@copies n` — read-side dose amplification, saturating on the 0..1 lattice |
| Stoichiometric RISC (dose-dependent knockdown, multi-site) | `silence old -> new strength s sites n;` — per-call capture `1 − Π(1−s)^n` |
| IRES cap-independent entry | `ires name;` + `--ires` |
| UPR / ISR stress programs | `stress { } rescue { }` containment |
| Fate landscapes (valley semantics) | `fate` state machines |
| Gene regulatory networks | `regulate` + `grn_fire/grn_state` |
| Cooperative binding (Hill exponent / ultrasensitivity) | per-edge `hill n` dose-response + `motif_hill` |
| Cis-regulatory input functions (AND / OR promoters) | thresholded edges: conjunctive by default, `any` = alternative activator |
| Network motifs (autoregulation, coherent/incoherent FFL, toggle) | `std/motifs` runnable circuits |
| Mutual-repression latch (toggle switch) | `toggle a, b;` (+ emergent rate-based bistability via mutual `hill ≥ 2` edges) |
| Repressilator oscillation | `repressilator a -> b -> c` (parameterized: α/γ/n/basal/noise) |
| Transcriptional bursting (two-state telegraph promoter) | `expr_on`/`expr_off` stochastic layer + `bursts` telemetry |
| Promoter identity (per-gene kon/koff) | `@burst kon koff` + `promoter_telemetry(g)` — on_frac / burst_size separation |
| Single-cell transcriptomics / call-pattern burst index | `fingerprint()` calls / burst / bursts / mature / nascent / maturation telemetry |
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
   list or null. `re_replace(pattern, s, repl)` — global literal substitution
   (no `$1` refs — captures come from `re_groups`; empty patterns are
   rejected like `re_match`; the result is capped at 64 MiB and every scan
   is step-charged — dx-r6). Syntax: literals, `.`, classes `[a-z0-9^]`, `\d \w \s \D \W
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

- Proof frames: **99 files / 92 proofs / 1,171 assertions**, green on the Rust core and the Python oracle.
- Differential harness (Rust core vs Python oracle, program-level stdout): **128 programs, all MATCH**, plus the oracle runs the same proof suite (both implementations green, enforced in CI).
- Red-team suite: **95 payloads, 0 breaches** (note-cap, fuel-charge, and output-cap containment verified live on the stochastic-expression, reg-bio-3, and Rho-termination surfaces).
- Playground smoke: expression-core subset in the browser, spec-aligned (unbound reads → null + note).
