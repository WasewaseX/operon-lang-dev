# Operon, Language Specification

**Status:** v2.6.0, post-2.4 language amendments are landing incrementally (the generated inventory in [docs/STATS.md](docs/STATS.md) is the countable truth; D-009: version strings move only with the milestone). Last tagged release: 2.5.0 (2.6.0 = the W09 A6 VM-default flip, untagged). This document is the single contract implemented identically by:

| Implementation | Language | Role |
|---|---|---|
| `src/` + `runtime/` | Rust + C++ | **Primary** compiler & runtime (the real thing) |
| `bootstrap/oracle.py` | Python | Reference oracle for differential testing + packaging |
| `web/playground/` | TypeScript/JS | Browser subset playground |

Three implementations MUST agree on every behavior below. Where they disagree, the differential test suite fails.

---

## 1. Identity

- Name: **Operon** (an operon = a cluster of genes transcribed as one unit under shared regulation).
- Extensions: `.op` source · `.cell` methylation config · `.rna` edit patches.
- Design law: **Total Grammar**, no `.op` file is ever rejected. Every token stream parses and runs.
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
- Bitwise: `& | ^` (and, or, xor), `<< >>` shifts, `~` bitwise not, Int operands, Int results.
- Integer overflow wraps? NO, overflow raises Stress `overflow` (catchable).

## 3. Lexical

- Comments: `#` to end of line. `#!` shebang allowed on line 1.
- Strings: `"double"`; escapes `\n \t \\ \" \{ \}`; interpolation `"{expr}"`, any expression, evaluated at runtime, `str()`-coerced. No single-quoted strings in canonical form (a `'` in code is a wobble: treated as `"` with a note). **Raw strings** `r"..."`, content verbatim, NO escape processing, NO interpolation (newlines allowed). **Multiline strings** `"""..."""`, escapes and interpolation processed, quotes (`"` / `""`) allowed inside, content verbatim (no implicit indent stripping; `operon fmt` may normalize later). **Byte strings** `b"..."` / `b'...'` produce the bytes type, see the W029 bytes contract below (W029 LANDED).
- **Byte vs char vs grapheme indexing (W28, SPEC §3)**, one table, both engines:
  | unit | what it is | where it applies |
  |---|---|---|
  | byte | one UTF-8 byte | `byte_width` (std/unicode) reports the UTF-8 size; raw byte access arrives with the bytes type (W029) |
  | char | one Unicode scalar value (Rust `char` / Python str code point) | `len(str)`, `s[i]`, `s.slice(i,j)`, `char_at(s,i)`, `char_slice(s,i,j)`, `ord`, `for c in s`, ALL string indexing is char-indexed, on BOTH cores (pinned by tests/unicode.op, tests/differential/unicode.op) |
  | grapheme | one user-perceived character | `grapheme_len(s)` over a DOCUMENTED subset (no external deps): base + combining marks (U+0300–036F, U+1AB0–1AFF, U+1DC0–1DFF, U+20D0–20FF, U+FE20–FE2F), ZWJ (U+200D) glues the previous and following char, regional-indicator pairs (flags) are one cluster; a lone RI is its own cluster |
  Example: `"👨‍👩‍👧"` is 5 chars / 1 grapheme; `"🇺🇸"` is 2 chars / 1 grapheme; `"日本語"` is 3 chars / 6 UTF-8 bytes / 3 graphemes.
  **Case folding**: `fold_case(s)` implements a documented SUBSET, ASCII A–Z, Latin-1 Supplement (À–Ö, Ø–Þ), Latin Extended-A (U+0100–U+0137 even→odd), Greek (U+0391–U+03A9, full-range monotone; no final-sigma context rule). Everything else is unchanged. The identical table logic runs in both engines (byte-identical output); full Unicode case folding / normalization is stage 2 of W28 and is NOT claimed by `fold_case` (it is `casefold` / `norm_nfc` / `norm_nfd`, next bullet).
- **Normalization, full case folding, categories (W28 stage 2)**, generated tables, both engines:
  **Tables are GENERATED, never hand-typed** (the W028 critique): `scripts/gen_unicode_tables.py` (Python stdlib only) emits `src/unicode_tables.rs` FROM Python's `unicodedata`, and the oracle calls that very same stdlib module, so the two cores agree by construction instead of by hand-copied data. The generator verifies BEFORE emitting (and refuses to write anything on mismatch): it runs the exact runtime algorithm against `unicodedata` over all 1,114,112 codepoints (`NFD`, `NFC`, and the round-trip bar `NFC(NFD(ch)) == unicodedata NFC(ch)`), a combining-mark pair sweep, a composite+mark sweep, and Hangul jamo sweeps. **Unicode version: unicodedata 15.0.0** (generator header, `unicode_tables::UNIDATA_VERSION`, and the oracle's interpreter must be the same Python); regenerating under a newer Python re-pins both cores at once.
  `norm_nfd(s)` (canonical decomposition, UAX #15): per-codepoint canonical decompositions FULLY EXPANDED (Hangul syllables AC00–D7A3 are algorithmic, never tabulated), then ONE canonical-ordering pass over the whole sequence (each maximal run of nonzero combining class is stable-sorted by ccc). Expansions are stored already ordered per codepoint; the runtime ordering pass exists because combining runs span codepoint boundaries.
  `norm_nfc(s)` (canonical composition, UAX #15): NFD, then left-to-right composition with the blocking rule (a combining char composes with the pending starter when nothing of class ≥ its own sits between them; the `last_cc == 0` admission is also what lets Hangul L+V and LV+T compose, algorithmically, under the same rule). Composition pairs were DERIVED EMPIRICALLY by the generator (every codepoint whose `NFC(NFD(c)) == c` contributes the pairs its re-composition walk needs, taken from ONE-LEVEL decompositions so multi-level forms like U+01D5 = U+00DC + U+0304 resolve through their intermediates), so Full_Composition_Exclusion is honored by construction (U+0344 never recomposes).
  **Canonical only**: compatibility decompositions are OUT of scope (`norm_nfd("ﬁ") == "ﬁ"`; ligatures, circled, superscript, CJK compatibility forms never expand — that is NFKD/NFKC territory, not claimed by stage 2). Lone surrogates cannot reach the tables (Rust `str` cannot hold them).
  `casefold(s)`: FULL Unicode case folding (the C+F tables, i.e. `str.casefold`), 1:many (`ß` → `ss`, `ﬁ` → `fi`), NOT locale-aware (Turkish: `İ` folds to `i` + U+0307, bytes `0x69 0xCC 0x87`; there is no Turkish İ→i special case), and CONTEXT-FREE: folding owns NO final-sigma rule (that rule belongs to lowercasing, not case folding), so `Σ`, `σ` and `ς` all fold to `σ` unconditionally in every position — `casefold("ΑΣ")` is `ασ`, never `ασς`. This is not a deviation from `str.casefold`; it IS `str.casefold` (the generator proves per-char application equals whole-string `casefold()` over a deterministic sample plus every sigma context). `fold_case` keeps its documented subset contract above; the two coexist.
  `char_category(s)`: ONE pinned shape, string in, the two-letter general category (`Lu`, `Ll`, `Nd`, `Lo`, `Mn`, `Zs`, `Cn` for unassigned, …) of its FIRST CHAR out; empty string is the soft tier (null + note). The full category table ships run-length encoded (all 30 categories over the whole codepoint space).
  **Error shape**: a non-string argument to any of the four is the standard catchable `unfolded` type Stress (`"norm_nfc(s) needs a string"`, `"norm_nfd(s) needs a string"`, `"casefold(s) needs a string"`, `"char_category(s) needs a string"`), the same family as `char_at`'s index Stress. A `char_decompose` helper was considered and dropped: `char_codes(norm_nfd(s))` (std/unicode) already expresses it, the surface stays small.
  Pins: `tests/unicode_depth.op` + `tests/differential/unicode_depth.op` (byte-identical); proof corpus: the generator's printed verification run.
- **The W029 bytes contract.** `bytes` is a first-class immutable value kind (Rust `Rc<Vec<u8>>`, Python `bytes`): literals `b"..."` / `b'...'` (a `'` form is a wobble-repair with a note), escapes `\n \t \r \\ \" \' \0 \xNN` (exactly two hex digits; a malformed `\x` is kept verbatim with a note), NO interpolation ever, non-ASCII source chars UTF-8-encoded with a note (Total Grammar: repair, never reject). `type()` → `bytes`; truthiness follows emptiness; equality is byte content; `len()` is the BYTE count. Indexing `b[i]` (negative from the end) yields the byte as an `int` 0–255; out-of-range is catchable `missing`. `b.slice(a, b)` uses Python-style clamped negatives and returns NEW bytes, bytes have NO mutators by design. `+` concatenates bytes (512 MiB per-op ceiling, `"bytes concat exceeds the 512 MiB ceiling"`), `*` repeats (same ceiling family); bytes and str NEVER coerce into each other (`b"x" + "y"` is catchable `unfolded`). Iteration (`for x in b`) yields ints 0–255. Bytes are legal SCALAR map keys (content equality, exact on both engines). Conversions are UTF-8-only with the encoding named explicitly: `bytes_from_str(s, enc?)` encodes infallibly (an Operon str IS valid UTF-8; unknown encoding names are `unfolded`), `str_from_bytes(b, enc?)` decodes and reports invalid UTF-8 as the SOFT tier (null + note naming the first bad byte), `bytes_from_list`/`bytes_to_list` bridge int lists (out-of-range values are `unfolded`). JSON view: bytes serialize as the lossless int list (`json_str(b"AB")` → `[65,66]`); JSON has no bytes type. File I/O mirrors the text forms' armor exactly (regular-files-only, charge-before-read, TOCTOU-verified handle, hardlink defense, same capability gates): `read_file_bytes(path)` / `write_file_bytes(path, b)`. Bytes cross the spawn membrane losslessly (SendValue::Bytes). The whole contract is pinned by `tests/bytes.op` + `tests/differential/bytes.op` (byte-identical) and attacked by `tests/redteam/rt_p19a_bytes.op` (ceilings, monotonic aggregate interaction, coercion, domain, index, capability gates).
- Identifiers `[A-Za-z_][A-Za-z0-9_]*`.
- Numbers: `42`, `3.14`, `1e3` (float). Negative via unary minus. Radix forms `0xFF`, `0b101010`, `0o755` (case-insensitive prefix; canonical value is the same Int). `_` digit separators allowed inside any numeric literal (`1_000_000`, `0xFF_FF`, `1_000.5`) and are stripped before parsing, the printed value is unaffected (canonical form stays decimal). A radix prefix with no valid digit after it lexes as decimal `0` followed by identifiers (`0x` → `0`, `x`). Out-of-range literals keep the existing saturate-to-0 note contract (f0fe2ec).
- Newlines terminate statements; `;` allowed and ignored (also `;;`, stray). Blocks are `{ ... }`.
- Keywords (canonical, the parser's reserved set; the generated inventory with per-keyword programmer analogies lives in [docs/KEYWORDS.md](docs/KEYWORDS.md) and is never hand-typed here).
- Literal words `true false null` and the logical words `and or not` are recognized in expression positions (not part of the reserved keyword table).
- Marks: `@acetylate` `@methylate` `@m6a` `@copies` `@riboswitch` `@burst` `@deprecated`. The six declaration marks of the biology layer: lexical presence here, attachment in §7, semantics in §11 (part contract §11a). The seventh mark is the language-surface deprecation mark (W64, below).
- `@deprecated("migration text", since="2.4")` (W64) marks a gene declaration as deprecated. The mark rides the gene as **pure metadata**: the interpreter never reads it, a marked gene runs, composes and dispatches exactly like any other gene (`tests/deprecate.op` pins runtime neutrality), and no differential parity is claimed (the oracle consumes the mark and drops it, like doc comments). The tooling surface owns the behavior: `operon check` reports a `deprecated-use` warning (stable code `W12`) at every static free-gene call site of a marked gene in the same file, carrying the migration text and the `since` gate; `--strict` escalates it per the strict table; `// allow: deprecated-use` on the call line silences it. Method calls, dynamic dispatch through a bare gene reference, and cross-file uses are the documented escape hatches (the check is file-local by design). Malformed payloads (`@deprecated(42)`, missing message) degrade to a rung-4 note and drop the mark, never a rejection. `fmt` re-emits the mark canonically and `fmt∘fmt = fmt` holds (`tests/deprecate_lint.rs` pins the warning, the escapes and the fmt fixpoint). Removal scheduling follows the compatibility ladder (docs/specs/COMPATIBILITY.md, W63). Honest inventory: NO core or std gene carries the mark today (`const` is live semantics per W05, its migration was retired), the machinery is future-proofing.
- `#` starts a line comment; `##` starts a **doc comment** (W074): consecutive `##` lines form a doc block that attaches to the declaration (`gene`/`sequence`/`phenotype`/`splice`/`fate`, including `@mark` lines above them) it immediately hugs, the doc's last line is exactly one line above the declaration keyword line. A doc block at the very top of the file that is separated from everything by a blank line becomes the **module doc**. Doc text is captured verbatim after the `##` marker (one leading space stripped). Docs are **pure metadata**: they never affect evaluation, repair rungs, or differential parity, the Python oracle ignores them as comments, and parity is not required for doc content (`tests/differential/doc_comments.op` pins neutrality). Docs survive `fmt` byte-exact (`fmt∘fmt = fmt` holds corpus-wide), surface in LSP hover and REPL `:doc name`, and are rendered by `operon doc` (W073).
- `#` inside a string does NOT start a comment.

## 4. Total Grammar, the 4-rung ladder

Every parse passes down the ladder; each rung below 1 emits **notes** (structured: line, rung, message, repair):

1. **Canonical**, exact keyword/grammar match. No notes.
2. **Synonym**, a known synonym table maps alternate spellings to canonical keywords:
   `fn func def fun sub lambda proc → gene` · `print echo say show → promote` (promote is builtin, synonyms map in parser to a `promote` call) · `var val → let` · `elseif → elif` · `foreach each → for` · `import include require → use` · `ret → return` · `stop → break` · `continue next skip → continue` · `yes on → true` · `no off → false` · `null nil nothing → null` · `&& → and` · `|| → or` · `! → not`. (W06/D-014: `none` was RETIRED from the null synonyms, it is now the Option constructor `none()`; bare `none` degrades to an unbound-ident note, never a silent null. W05: `const` was RETIRED from the let synonyms, it is its own immutable-binding form, §7d.)
3. **Wobble**, an identifier within edit distance ≤ 2 of exactly one keyword (≤ 1 if its length ≤ 4) is repaired to that keyword, with a note. Applies to marks too (`@acetylat` → `@acetylate`). Ambiguity (two keywords equidistant) → rung 4 for that token.
4. **Semantic fallback**, unknown bare identifier in expression position becomes the string literal of its own name + note ("unbound wobble"); stray tokens are skipped with notes; unclosed braces are auto-closed at EOF with notes; extra closers are skipped with notes; an unclosed string consumes to EOF with a note.

Runtime Total Grammar: reading an unbound variable → `Null` + note; calling a non-gene → `Null` + note. **A run never aborts on soft failures**, only `exit()` or an uncaught `raise`-style hard event ends it early (and even that prints a containment note first).

Unbound/undefined `gene` calls are reported by `operon check` as **phantom calls** (they run as Null at runtime).

## 5. Statements

```
let name = expr                     # definition (re-let with note "rebinding")
let [a, b] = expr                   # destructuring definition (v2.3), list
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
scope { }                           # structured concurrency (§13): spawns
                                    # inside are joined at block exit
for name in expr { }                # List→items, Str→1-char strs, Map→keys,
                                    # sequence → pulled values (§7b)
for [k, v] in expr { }              # destructuring loop (v2.3), any pattern
                                    # accepted; each item binds the pattern in a
                                    # fresh child scope
return expr?                        # bare return → null
break / continue
match expr { case p { } ... }       # patterns (W02 match-v2, see §5a). First
                                    # match wins; a shape that cannot match
                                    # falls through, matching never rejects a
                                    # run (Total Grammar, §4)
use path (as name)?                 # file import → binds module Map (see §8)
raise expr                          # raise Stress{kind:"unfolded", message: str(e)}
raise kind , expr                   # kind ∈ unfolded missing overflow burned
stress (kind)? { B } rescue (e)? { R }   # containment, see §9 (kinds incl.
                                    # `interference`, §9b, sandbox denials)

Name { B }                          # bare-name block: a gene definition with no
                                    # params (rung-4 note), `main { }` is the
                                    # C-like entry idiom
```

**Pattern soft-miss law (v2.3):** a destructuring pattern never hard-fails a
run. Destructuring a non-container (or null) binds all its names to null with
one note; a missing element/key binds null for that piece only; extra elements
are dropped with a note. `*rest` on an exhausted list binds `[]`.

### 5a. Pattern matching, match-v2 (W02)

`match` arms are patterns `p`, tried top to bottom; the first arm whose
pattern hits executes its body in a fresh child scope holding the pattern's
captures, and the statement ends. A pattern that cannot match (wrong tag,
wrong length, missing key, non-container subject) simply misses, it never
fails the run; with no hit and no catch-all the `match` falls through
silently. New in W02 (all mirrored op-for-op by the Python oracle, and by
`operon fmt` roundtrip):

- **Variant patterns**, `Some(p)`, `None`, `Ok(p)`, `Err(p)` where `p` is
  itself a pattern (nestable: `Some(Some(y))`). The tag IS the contract
  (§9): a `Some` pattern never matches an `Ok` value. The bare tag form
  (`Some`) matches the tag with any payload; `Some(p)` against the
  payload-less `None` value misses. Tags are the four built-in variant
  constructors only, any other capitalized name in pattern position is a
  note + a whole-subject binding (a parenthesized payload is consumed and
  ignored so the token stream stays aligned).
- **List patterns**, `[p1, p2, *rest]`. Element patterns nest; the subject
  must be a List of exactly that length, or (with `*rest`) at least that
  length; `rest` binds the remaining tail as a fresh List. `[]` matches
  only the empty list.
- **Map patterns**, `{k1, k2: p}`. Each named key must be present on the
  subject (a Map); the bare-key form binds the value, the `k: p` form runs
  the sub-pattern against it. Non-string keys are only reachable via
  indexing, not patterns.
- **Or-patterns**, `p1 | p2 | ...`; alternatives are tried in order and
  the FIRST hitting alternative provides the bindings (an alternative that
  hits without binding yields unbound-name lookups downstream, soft nulls).
  A failed alternative's partial captures never leak into the arm.
  Newlines are allowed before any `|` (multi-line chains).
- **Guards**, `p if cond`: the condition evaluates after `p`'s captures,
  in the arm scope, so it sees the bindings (including after an or-chain,
  the winning alternative's). Guard false ⇒ the arm misses and matching
  continues. A stress inside the guard is contained to a note + miss;
  the ONE escape is `?!` propagation (§9), which is a return, never a
  failure, it leaves the match and unwinds to the gene boundary.
- **Legacy forms, unchanged**, literal patterns (`1`, `"s"`, `true`,
  `null`), identifier binds, `_`, and the literal comma-run (`case 1, 2`
  = matches either). Pattern literal expressions cannot call or reference
  names (they are literals by construction), so guards are the supported
  way to test computed conditions.

**Unreachable-arm report (W002 stage 2, check-side).** The static engine
(`src/lint.rs`, rule `unreachable-match-arm`, stable code `W06`) reports an
arm that can never run under first-match-wins: an arm placed after an
UNGUARDED catch-all (`_`, a binding pattern, or an or-pattern containing
either; the legacy comma-run holds plain literals only, so it can never
hide a catch-all), or after a strictly broader arm whose shape provably
covers every value the later arm can match (`Some(_)` before `Some(x)`,
the bare tag before a payload arm, `[a, b]` before `[1, 2]`, `[1, *t]`
before `[1, 2]`, `{x}` before `{x: 1}`, a literal inside an earlier
or-/comma-run, nested payloads likewise). The report is a semantic WARNING
on the lint stream (`operon lint`; `check --style` inlines it), advisory
like every lint rule: Total Grammar never rejects a program, and the
interpreter and the Python oracle are untouched, so runtime semantics and
differential parity cannot drift from this check. Conservatism is the
design bar (zero false positives): a GUARD on the earlier arm makes it
fallible, so a guarded arm is never treated as covering anything, and a
guard on the later (dead) arm does not save it, because first-match-wins
means that guard can never evaluate; structural coverage is claimed only
where the pattern algebra proves it (floats are excluded entirely, NaN
never deep-equals itself; byte-string keys compare per byte, so distinct
byte strings are never conflated; identical plain literal arms are left to
the separate `duplicate-match-arm` rule, `W05`, which owns that symptom);
or-alternatives must ALL be covered before the arm is reported. Findings
carry file and line (the first line-bearing statement of the arm body;
the AST keeps statement spans only on expression statements, so a column
is not reported, and an arm whose body opens with spanless statements,
a plain assignment say, anchors at the file's line 1, where only a
line-1 allow comment can suppress it). Exhaustiveness
(is some value class unhandled?) is intentionally NOT claimed here: that
needs the full pattern algebra and stays with the semantic lane.

## 6. Expressions (precedence low → high)

1. `cond ? a : b`, ternary, right-associative, lowest precedence
2. `or` (`||`), short-circuits, returns operand value (`a or b` → a if truthy else b)
3. `and` (`&&`), same semantics
3.5 `a ?? b` (v2.3), null coalescing: sits between `or` and `and`
   (`a or b ?? c` reads `a or (b ?? c)`); short-circuits; coalesces **Null
   only**, falsy-but-non-null values (`0`, `""`, `[]`, `false`) pass through.
4. unary `not` (`!`), `not a == b` parses as `not (a == b)`
5. comparisons `== != < <= > >=` and `in` (left-assoc, no chaining); `x in xs`: List membership, Str substring, Map key membership
6. bitwise OR `|`
7. bitwise XOR `^`
8. bitwise AND `&`
9. shifts `<< >>`
10. `+ -`
11. `* / // %`
12. unary `-` and `~` (bitwise not)
13. `**` power, right-associative, binds tighter than unary minus on its left
14. postfix: call `f(x)`, index `a[i]`, member `a.k`, method call `a.k(args)`,
    the propagation form `e?!` (§9, D-014, binds tighter than every binary/
    ternary operator, repeats compose), and the null-safe forms `a?.k`,
    `a?.k(args)` (v2.3): a Null receiver yields
    Null **silently**, no note; a non-Null receiver behaves exactly like `.`
    (missing keys still note). Chains compose: `a?.b?.c`.
15. primary: literal, ident, `(expr)`, list `[a, b]`, map `{k: v, "k2": v}`, lambda, `collect` (§7), `new Name(args)` (§7a)

Member access on Map → key lookup (missing → Null + note). Methods (see §10) are native.
`a.k = v` assigns map key. `a[i] = v` assigns list index (out of range → Stress `missing`, catchable).

## 7. Genes (functions) and lambdas

```
marks* gene name(p1: T, p2: T = default) -> T guard (cond) else { B } { body }
marks* gene (p1) => expr            # anonymous lambda
marks* gene (p1) { body }           # anonymous block lambda
let f = gene (x) => x * 2
```

- `marks` = `@acetylate | @methylate | @m6a | @copies n` (§11). Sequence definitions gate at CREATION like gene calls (§11 gate order), a silenced or vetoed sequence returns null instead of starting its worker (reg-r4).
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

- `new Name(args)` builds an instance: field defaults apply lineage root-first (parent fields, then own overrides), then exactly one `init` runs with the args, the lineage is searched **root-first** and the first (least-derived) `init` found wins; a child's `init` only runs if no ancestor declares one. Without any `init`, fields keep their defaults. (`super` is not a binding: an unbound `super.init(...)` inside `init` reads null with a note.)
- `self.f` reads and writes fields inside methods. Field access from outside: `o.f`. Missing fields → `null` + note.
- Method dispatch: own methods first, then the parent chain. Missing method → `null` + note.
- `type(o)` returns the phenotype name (`"Counter"`); `o.f = v` assigns a field. Phenotype instances cross `spawn` boundaries by serialization (they travel as maps carrying a hidden `#phenotype` key).
- **Instance equality (dev1 ruling, closes the W34-stage-2 parity finding):** `==` on two instances is `true` iff the class names are equal AND every field deep-equals (`a == a` is always `true`; subclasses are distinct values, the class name is part of the value). Instances are DATA; genes (closures) and sequences stay identity-based, they are behavior handles. Instance-keyed maps (`m[a] = v`, `m.get(b)`) use the same deep equality in both implementations. Pins: `tests/pheno_equality.op` + byte-identical `tests/differential/pheno_equality.op` (mutual-instance cycles stay cycle-safe).
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

- A sequence body runs on its own **worker cell** (a real OS thread); the consumer pulls values through a rendezvous channel. Pull is lazy, a sequence that yields forever is legal and only produces values on demand.
- Values cross the membrane by serialization. Named genes and lambdas may travel as arguments (they cross by definition); a running sequence object itself does not cross.
- Honesty note: the Rust core pulls lazily; the Python oracle models a sequence by running the body to completion on first pull (buffered). Output is identical for programs that do not print inside a sequence body, and the differential corpus avoids infinite sequences.
- `yield` outside a sequence is treated as `return` with a note.

### 7c. Soft type annotations (W01, L2c)

Annotations document and enforce contracts at the boundaries, in the spirit of
mypy/TypeScript-gradual-typing rather than a static checker:

```
gene clip(x: int, lo: int, hi: int) -> int { ... }   # param + return annotations
let ratio: float = 0.5                               # annotated definition
let name: str? = null                                # optional (accepts null)
gene handle(v: int | str) -> any { ... }             # union
```

- **Grammar**: a type is a name, a name with `?` (optional), or a `|`
  union of those. Known names match by value type: `int float str bool
  list map gene sequence phenotype option result`. An unknown name parses
  fine and matches nothing today (user-defined phenotype annotations are a
  later stage), never a parse rejection (Total Grammar, §4).
- **Widening rule**: `float` accepts int (safe numeric widening); `int`
  REFUSES float, no silent narrowing. `any` accepts everything.
  `option`/`result` match the variant FAMILIES (Some/None vs Ok/Err, the
  tag is the contract, §9); families are distinct.
- **Enforcement is a SOFT contract**: violations raise catchable Stress
  kind `unfolded` (§9), never a hard failure and never a parse rejection.
  - **Params** are checked at the call funnel: `argument 'x' for gene
    'g' expects int, got str`. Default-value expressions are checked the
    same way. Phenotype methods run the same checks (`...for method
    'm'...`).
  - **Returns** are checked when the gene produces its value, including a
    `?!`-propagated variant and a guard-branch return: `return of gene
    'g' expects int, got str`. Falling off the end without a return is an
    implicit null and violates a non-optional annotation: `...got null (no
    return statement ran)`; declare `-> T?` when null is legitimate.
  - **`let x: T = e`** checks at binding: `type annotation violated: 'x'
    expects int, got str`. On a mismatch the binding does NOT happen.
- **Staged model** (this stage = stage 1): annotations are runtime
  contracts on gene calls and definitions. `operon check`-time inference
  and reporting, typed collections (`List<T>` sugar), and type aliases are
  later stages of W01; sequencing is tracked in ROADMAP-100.
- **W026, the collections sugar first slice:** the typed-collections
  layer lives as pure `.op` std modules over map/list (`std/set`,
  `std/deque`, `std/heap`, `std/graph`), and `std/graph.op` is the first
  std module whose public genes carry these stage-1 annotations (the W003
  sugar slice; set/deque/heap stay untyped until the mechanical pass
  lands). Its determinism contract is the pinned node order: node order IS
  the `nodes` map's insertion order (both cores keep map insertion order,
  §19a), every traversal and tie-break follows it, ids are never sorted;
  pinned by `tests/std_graph.op` and byte-exactly by
  `tests/differential/graph.op`.

### 7d. Immutability, `const` bindings + deep freeze (W05)

`const NAME = expr;` binds a name immutably. Two walls, one stress kind:

```
const K = 10                  # immutable binding
const L = [1, 2, [3]]         # the value is DEEP-FROZEN
const M = {"a": 1}            # maps freeze the same way
K = 5                         # frozen stress: cannot reassign const 'K'
push(L, 9)                    # frozen stress: cannot modify frozen list
L[0] = 9                      # frozen stress: cannot modify frozen list
M["b"] = 2                    # frozen stress: cannot modify frozen map
del(M, "a")                   # frozen stress: cannot modify frozen map
L.push(4) / M.del("a")        # method forms stress identically
```

- **Deep freeze**: freezing walks the value, every list and map reachable
  from it (through nesting, aliases, variant payloads, and phenotype field
  values) becomes immutable for the rest of the run. Cycle-safe (a
  self-referencing container freezes once). The freeze itself draws
  nothing: the entropy stream is untouched (byte-identical legacy).
- **Catchable, never fatal**: every wall raises Stress kind `frozen`
  (§9), `rescue` handles it like any other stress. `--strict` runs and
  `operon lint` surface statically-detectable reassignments first
  (`const-reassign` finding; name-based, conservative, any `let`
  re-binding of the name retires the finding).
- **Scope of the walls**: the NAME wall covers `=`, `op=`, and multi-
  assign rebinding; a re-DEFINITION (`const x = 1; const x = 2`) rebinds
  like `let` does (with the usual rebinding note). The CONTAINER wall
  covers index/member writes, `push/pop/insert/remove/del`, and their
  method forms. Phenotype member stores stay mutable (methods keep
  working); lists/maps held in fields are frozen. `sort`/`reverse`/
  concatenation return NEW containers, they are reads, not writes.
- **The spawn boundary**: frozen-ness does not cross `spawn` (§13), a
  worker receives serialized copies, and mutating a frozen argument inside
  the worker body is engine-defined territory the corpus pins around (the
  oracle parks the reachable frozen ids for the inline body to mirror the
  worker's fresh-container semantics).
- **Back-compat**: programs whose `const` bindings are never reassigned
  and whose containers are never mutated run byte-identically to the old
  `const → let` synonym (freeze draws nothing; corpus-verified). The
  `operon fix` const→let migration is RETIRED (red-main r5 hotfix, W05):
  `const` is live semantics, a fixer rewrite to `let` would unfreeze
  bindings and change program meaning (fix_corpus law 1). The
  `const_to_let` report field stays (always 0) for `--json` shape
  stability.
- **`let mut x`** parses: the `mut` annotation is documentation-only in
  v2.x (contextually consumed when followed by the binding name, a
  variable literally named `mut` keeps working).

## 8. Modules, TADs, anchors

- `use path;`, path like `std/bio`, `./util`, `util` (`.op` appended if absent). Binds one name: basename, or `as name`. `use std/bio as b;` → `b.some_fn(...)`. Importing the same file twice executes it once (module cache).
- **Namespace spellings (W25).** `::` is the Rust-style separator, `use std::bio;`, `use bio::sequence`, and multi-segment `use a::b::c` are EXACT sugar for the `/` form (`a::b::c` resolves as `a/b/c`); every separator (`/`, `.`, `-`, `::`) may be mixed within one path. The imported module is a Map, so qualified access (`bio.some_fn(...)`) is the namespace read, and, because the `use` also flat-binds every export beside the alias, the old flat spelling (`some_fn(...)`) keeps working beside it (back-compat is a PINNED behavior, not an accident: tests/differential/namespaces.op pins both spellings byte-identically on both engines, plus aliased namespaces where the same module is reachable under two binding names). Stage 2 (W025) extends the surface three ways, all mirrored op-for-op by the oracle and pinned byte-identically by tests/differential/namespaces2.op: (1) **nested declarations**: a module file (or any block) may declare `module name { ... }`; the body runs ONCE in a fresh child scope and its names become the export table (sorted like the file-module loader sorts them; data lets export beside genes; a gene in the table closes over the table's own scope). `module` is a CONTEXTUAL word: only `module NAME {` is a declaration, any other use of the identifier parses exactly as before, so no pre-existing program changes meaning. Last declaration wins: a later `module name` (or gene) re-binds the name. (2) **path descent**: a multi-segment `use` stays FILE-FIRST (W069); when the full path is not a file, each shorter prefix is tried as a file, longest first, and the remaining segments descend the loaded table's exported nested tables (a missing segment notes `segment '<seg>' is not a nested module table; import binds nothing` and the use falls through to the next prefix, Total Grammar: it never rejects). The tail-against-std resolution shortcut is BARE-NAME only: `use mylib::seq` never hijacks `std/seq.op` by its tail, descent is the only multi-segment fallback. (3) **wildcard tail**: `use mylib::seq::*` flat-binds the target table's exports without binding the table itself under a name (an explicit `as` alias still binds it); later imports overwrite earlier flat binds, first-match-wins at call time. Qualified reads and calls may be spelled with `::` (`mylib::seq::tag()`), which is EXACT sugar for the dot form and carries identical AST.
- **Visibility model (W24).** `pub` is a CONTEXTUAL marker, not a keyword: at top level, `pub gene f()`, `pub let x = …`, `pub const c = …`, and `pub phenotype P { … }` mark the name public; an identifier literally named `pub` (or `pub` anywhere else) parses exactly as before. Default mode: `pub` is INERT, zero behavior change (pinned byte-identically by tests/differential/visibility_default.op, including `pub` used as an ordinary variable). Opt-in strict mode, `.cell modules.visibility = strict`, changes the export rule for that load: a module with pub marks exports ONLY those names (anchor export still wins when present); every other top-level name stays module-private (module-local calls are unaffected, what changes is what the export map carries, so both the qualified read and the flat alias binding see the hidden names as the SOFT tier: null + note, never a crash). The loader notes how many private names were hidden (`strict visibility: N private name(s) hidden in '<mod>'`). Migration path: a strict module with ZERO pub marks keeps default-open and notes it once, turning strict on never silently empties a legacy module. Both engines byte-identical under the granted cell (tests/granted/visibility_strict.op + .cell in the harness's granted targets).
- **Resolution algorithm (W069, pinned).** The path is tried against candidate roots **in order; first regular file wins**:
  | # | candidate root | class | notes |
  |---|---|---|---|
  | 1 | the **importing file's directory** (`interp.base_dir` = entry-file dir, tools.rs) | doc-relative | the CWD never participates here, editors/CI may run from anywhere |
  | 2 | the path **as written**, under the CWD | CWD | plain names resolve here when no doc-relative shadow exists |
  | 3 | `std/` under the CWD | std | dev-tree layout |
  | 4 | exe-dir joined with the path (and with exe-parent) | exe-relative | bundled layout `…/bin/operon + …/bin/std` and install/dev layout `…/bin/operon + …/std` (dx-r5) |
  | 5 | exe-relative `std/` trees by **bare file name** (non-`std/` paths only) | exe-relative std | lets `use x` find the shipped `std/x.op` |
  | 6 | `$OPERON_STD` joined with the path as written (not double-joined with `std/`) | override | operator override of the managed std tree |
  | 7 | the **vendored dependency cache** (`operon.lock` name→rev→dir; `$OPERON_DEPS` or `~/.operon/deps/<name>-<rev12>`), the leading path segment maps to the pinned checkout: `use my-lib/util` tries `<cache>/util.op` | deps (W19/W23) | deterministic: the lock pins name→rev→dir, no globbing; the standard roots above win first, so a local checkout shadows the vendored copy |
  - Failure classes (W070): a miss reports every attempted root **for the plain-name class only**; the traversal class (absolute/`..`/`~`/drive paths) keeps ONE unified "denied or nonexistent" message, attempted-root detail there would resurrect the C-7 filesystem existence oracle (sec-r1).
  - **Parity note (honest scope)**: the Python oracle implements roots 1–3 + 6; roots 4–5 are runtime-only (the oracle never ships beside a `std/` tree). Differential coverage pins roots 1–2 (`tests/differential/mod_res.op`); root 3 is exercised corpus-wide by every `use std/…` program; roots 4–6 are pinned by the redteam/granted suites where the layouts exist (`scripts/install.sh`, release artifacts).
  - **LSP divergence (allowed, justified)**: `operon-ls` resolves with the document's directory as root #1 and is deliberately **CWD-independent** (lsp-r1), editors launch language servers from arbitrary CWDs; CWD-rooted candidates would make diagnostics machine-dependent. Runtime keeps CWD roots because `operon run`'s CWD is part of the operator contract.
- Import gating (§9b): imports that resolve inside the program's own managed trees, the importing file's project directory, the CWD, the standard library, or a **vendored dependency from the lockfile** (root 7, the operator resolved + installed it, so it is sanctioned; an arbitrary cache path never enters the mapping), are always allowed; a `use` that reaches outside those trees requires a read capability.
- **The package system (W19/W20/W23).** Package metadata lives in `operon.toml`, `[package] name/version/operon-version` + `[deps] <name> = { git = "<url>", rev = "<pin?>" }`, a minimal TOML subset, parser-reported line-by-line on anything outside it (a manifest that lies would corrupt the lockfile; the W22 rule keeps `.cell` runtime-only, the two never merge). CLI: `operon mod init` (write the manifest) · `add <url> [--rev r] [--as name]` (clone via the git CLI, pin the resolved rev, vendor under the cache, lock) · `remove <name>` · `update` (re-resolve the closure) · `install` (materialize every lock entry, offline whenever the cache is warm) · `tree` (the resolved graph) · `verify` (recompute every vendored checksum; nonzero exit on tampering). Resolution walks the TRANSITIVE closure (each dep's own operon.toml), cycle-safe, in sorted-name order, the same manifest yields the byte-identical `operon.lock` on every machine. The lockfile (`operon.lock`) pins `name/git/rev/checksum` where the checksum is a sha256 content digest of the vendored checkout (in-house FIPS 180-4, zero crates). `--locked` fails the run on manifest↔lock drift (missing dep, changed URL, rev mismatch, or a lock entry justified by nothing, direct or transitive). Proof: `scripts/pkg_e2e.sh` builds a two-package transitive dependency tree from local repos, locks it, wipes the cache, re-installs FROM THE LOCKFILE ONLY, runs offline, and exercises `verify` tamper-detection and `--locked` drift rejection.
- A module evaluates to a Map of its **exported** names. Export rule:
  - If the file contains any `anchor export a, b;` (at top level or inside a tad), ONLY those names are exported.
  - With zero `anchor export` anywhere, all top-level genes and lets are exported (default-open).
- `tad Name { ... }`, topologically-associating domain: an insulation boundary. Names inside a tad escape ONLY via `anchor export` **inside that tad**. Tads may not nest (nesting degrades to a merged tad + note). A file may have several tads.
- `anchor import x;`, declares an external name the domain expects; `operon check` verifies it exists in the file or in used modules (−2 and a finding if not).
- Cyclic `use` → second import returns the partial module Map + note (no hang).

### 8b. Traits, shared behavior for phenotypes (W04)

A trait is a named set of method contracts. A phenotype can `implements` any number of
traits (after the optional `from Parent` in the header); traits and inheritance compose:

```
trait Show {
    gene describe() { return "shape:" + self.display() }   # DEFAULT method (has a body)
}
trait Big {
    gene area()                                            # REQUIRED method (no body)
}
phenotype Rect implements Show, Big {
    let w = 0
    let h = 0
    gene init(w, h) { self.w = w; self.h = h }
    gene area() { return self.w * self.h }
    gene display() { return "rect(" + str(self.w) + "x" + str(self.h) + ")" }
}
```

- **Required vs default**: a trait method with no body is REQUIRED, the implementing
  phenotype (or an ancestor) must provide it; a trait method with a body is a DEFAULT,
  used when the phenotype's own lineage has no method of that name.
- **Contract check at construction**: `new Rect(...)` verifies every REQUIRED method of
  every implemented trait. A missing one is a rung-4 note,
  `phenotype 'Rect' implements 'Big' but does not provide 'area()'`, the instance is
  still built (Total Grammar); the eventual method call wobbles null per §9.
  Implementing a trait that was never declared notes `trait 'X' not declared; contract
  on 'Y' ignored`.
- **Dispatch order**: phenotype's own lineage (child → root) first, then implemented
  traits in declaration order (first default body with the name wins), then the
  field-as-callable fallback. Dispatch is VIRTUAL: a default body calling
  `self.display()` dispatches back through the phenotype's own methods first, that is
  the point of the `Show.describe` example.
- **Method semantics are ordinary gene semantics**: defaults take params, close over
  nothing (they are plain genes with `self` bound), and ride the full call funnel
  (regulation gates apply, §11).
- **Redefinition**: re-declaring a trait replaces it (last wins, noted), mirroring
  phenotype redefinition. Trait declarations inside tads contribute their names to the
  module's member list (§8 export rules unchanged).
- Evidence: tests/traits.op (10 assertions, both cores),
  tests/differential/traits.op (byte-identical), fmt round-trips `trait` blocks and the
  `implements` clause canonically.

## 9. Stress containment (errors)

**The error hierarchy (W06, D-014).** Operon separates four tiers of "went wrong", each
with its own mechanism, everyday failures never touch Stress:

1. **Null + note**, a soft miss (`?.` on null, a member that isn't there, a burst-off
   call). The value is `null`, a rung-4 note explains. Expected, routine, unwrappable
   with `??`.
2. **Option / Result**, an *expected* failure as a first-class value (below). The caller
   decides: propagate with `?!`, default with `unwrap_or`, or inspect with `is_ok`/`is_err`.
3. **Stress**, an *exceptional* failure: a contract violation, resource ceiling, or
   capability denial. Contained by `stress/rescue`.
4. **Hard exit**, uncaught stress at the entry boundary renders the traceback and exits 1
   (§9a).

**First-class Option/Result (D-014).** Four constructors build variant values: `some(v)`,
`none()` (the Option family), `ok(v)`, `err(e)` (the Result family). The tag IS the
contract, families are distinct (`some(1) != ok(1)`) and `type()` returns `option` or
`result`. Payloads may be any value, including other variants (`some(some(9))`).

- **Predicates**: `is_some/is_none/is_ok/is_err(v)`.
- **Safe extraction**: `unwrap_or(v, default)` never stresses, payloads win, everything
  else (None/Err/plain values) yields the default.
- **Unsafe extraction**: `unwrap(v)` returns the payload of Some/Ok; on None/Err (or a
  plain value) it raises Stress kind `unwrap`, a *programmer-contract* violation, i.e.
  tier 3, deliberately rescue-catchable. Expected failures belong in the value layer
  (check `is_ok` first, or propagate).
- **The `try_*` family (W06 stage 2)**: Result-returning variants of the failure-prone
  core builtins. `try_num(s)` (parse as int, then float, same rules as `num`; the err
  payload renders the RAW input: `Err("num('x9') failed")`), `try_index(l, i)`
  (`Err("index 3 out of range for list of length 3")`), `try_get(m, k)`
  (`Err("no key 'zz'")`), `try_pop(l)` (`Err("pop from an empty list")`), the latter
  still mutating. Non-matching argument TYPES are `Err` payloads (not stresses); wrong
  ARITY stays an `unfolded` stress (the ok/err rule). Pinned on both engines:
  tests/differential/try_family.op + tests/try_family.op.
- - **Repr**: `Ok(3)`, `Err("x")`, `Some(3)`, `None`. **Truthiness**: Some/Ok are truthy,
  None/Err falsy (`if (result)` reads naturally). **JSON**: `{"ok":1}`, `{"err":"x"}`,
  `{"some":1}`; None serializes as `null` (in-memory lossless, JSON is a Map-shaped view).
- **Equality**: same tag + equal payloads; `None == None`.

**Propagation `?!` (D-014).** A postfix operator, `e?!`:

- `e` is `Some(v)`/`Ok(v)` → the expression IS `v` (payload unwrapped).
- `e` is `None`/`Err(x)` → the enclosing gene **returns** that variant value, unwinding
  immediately (statement-level, like a `return`). This is a *return*, not a failure: it
  crosses `stress/rescue` boundaries untouched, gains no traceback chain frames, and can
  never be contained by rescue (not even `rescue any`).
- `e` is a plain value → silent identity (the `?.`-on-non-null precedent).
- Outside any gene (top level, proof frames, REPL): the signal is contained with a
  rung-4 note `propagation reached top level: <value> passes through`, Total Grammar
  never rejects, and the "propagate" kind never leaks as a failure anywhere (the marker
  rides an interpreter-only payload field no user path can set; the `raise` grammar's
  kind whitelist cannot name it either).
- Inside a sequence body: the stream ends cleanly with the note `propagation ended the
  sequence` (sequences are streams, not answers, the variant has no return path).
- Repeats compose: `some(some(9))?!?!` is `9`. Binds tighter than every binary/ternary
  operator; the formatter round-trips it.
- Grammatically `?!` is one token, lexed before the ternary's bare `?`, the two never
  collide.

**Compatibility note (stage-2 roadmap, in progress)**: std functions and builtins that
fail today return `null` + tier-1 notes or raise tier-3 stresses. They migrate to Result
returns per-function, each documented at migration time; the null/stress contracts stay
the default for 2.x so existing programs keep their byte-identical behavior. First
migrated family (v2.6.0): the `try_*` core builtins above — additions, not replacements;
the legacy `num`/index/key-read/pop contracts are pinned UNCHANGED in tests/try_family.op.

Runtime failures raise a **Stress** value: Map `{"kind": Str, "message": Str}`. Kinds:
`unfolded` (type errors), `missing` (bad index/key/member/null-deref), `overflow` (int overflow, depth limit, resource ceilings), `burned` (assertion failures, resource errors), `interference` (capability-sandbox denials, §9b), `unwrap` (D-014: `unwrap` on None/Err or a plain value), `any` (catch-all position only).

```
stress { RISKY } rescue (e) { promote("contained: {e.message}") }
stress missing { ... } rescue { ... }    # kind filter: only catches `missing`
```

- Unspecified kind catches any. Filtered kind catches only that kind; others propagate outward.
- `rescue` binding optional: `rescue (e)`. No rescue clause → stress is contained to Null + note (Top-Grammar runtime law).
- Uncaught at top level → printed as containment note, run continues (or ends that entry call with Null).
- `assert(cond, msg?)` raises Stress `burned` on failure (this is what proof frames catch).

### 9a. Stress tracebacks (W007)

Every Stress carries a **call chain**: the gene frames it unwound through, innermost
first. The chain is captured during unwinding at the call funnel (`call_gene`), each frame
being `(gene name, call-site line)`, the line of the call expression that invoked that
frame. The frame capture costs nothing on the happy path (append-on-error only) and is
capped at **64 frames** (note-cap discipline: a bounded chain is a contained chain; deeper
chains keep the 64 innermost frames).

- **Rescue binding surface**: `e.chain` is a List of Maps `{"gene": Str, "line": Int}`,
  innermost frame first, in that field order. `e.kind` / `e.message` unchanged. (The
  Stress's own origin line is a Rust-side stderr-rendering field and is deliberately not
  part of the rescue-map contract.)
- **Uncaught rendering** (exit-1 path, e.g. `--entry`): the primary diagnostic renders
  `file:line` of the raise (raise statements carry their own line), followed by the chain,
  one frame per line, then `at main`:

  ```
  [contained] [overflow] app.op:2: detonating at depth
    at boom (app.op:5)
    at go
    at main
  ```

  A frame with line 0 (the runtime-invoked entry gene) renders as a bare `at <gene>`.
  The renderer is also capped at 64 frames (`… N more frame(s)`).
- **Containment**: the chain leaks nothing beyond the script path already printed in the
  primary diagnostic, no environment, no cwd, no host paths (redteam rt_p15a–c).
- The Python oracle mirrors the capture op-for-op: identical `e.chain` values (gene names
  AND call-site lines), differential-pinned in `tests/differential/traceback_chain.op`
  and shape-pinned in `tests/traceback_shape.op`.

## 9b. Security, the capability sandbox

The runtime is **default-deny**: a program is an organism in a culture flask, and nothing outside the flask exists until the operator grants it. The builtins `read_file`, `write_file`, `append_file`, `exists`, `read_dir`, `file_size`, `fs_delete`, `fs_rename`, `fs_mkdir`, `run`, `py`, `http_get`, `serve`, `env`, and `exit` raise catchable Stress `interference` when no grant covers the access, RNA-interference: the cell's antiviral machinery silences the operation instead of crashing. `recv_request`/`send_response` poll a queue that only `serve` fills, so they are inert without a granted server.

Grants (operator-side, CLI):

```
operon run app.op --allow-read /data --allow-write /tmp/out \
              --allow-run gzip --allow-net 127.0.0.1:8080 --allow-env API_KEY
operon run app.op --allow-py math --allow-py numpy   # substrate-r1: per-module Python bridge grants
operon run app.op --allow-all           # open flask (for scripts that mean it)
```

- Path grants (`read`/`write`) resolve symlinks: the requested path is canonicalized before comparison against the canonicalized grant. A grant that normalizes to the empty string (`/`, or `.` from `/`) would match everything and is **rejected** at startup.
- **sec-r5 (anti-TOCTOU): file I/O verifies the opened handle, not the path.** The old check → canonicalize → write sequence was raceable with a symlink swap (proven live: a flipped link landed attacker-controlled bytes outside the grant). Now `read_file`/`write_file`/`append_file` open the file FIRST, verify the handle's true identity (Unix: the fully-resolved `/proc/self/fd/<n>` target; elsewhere: symlink/reparse handles are refused), and do all I/O **through the handle**, there is no later path re-resolution to race. A dangling outside-pointing symlink is resolved (chains up to 40 hops) and rejected **before** open, so it cannot be created-through as an empty file. Residual (documented): a race that flips a link to a dangling outside target inside the check-to-open window can create an empty file outside the grant, no content ever crosses.
- **sec-r5: `read_file` reads regular files only.** FIFOs (which would block `open()` forever) and device files like `/dev/zero` (an infinite byte well that OOM-killed the host) are refused with Stress `interference` before open; the stat'd size is charged against the aggregate allocation ceiling before the read.
- `--allow-net` takes `host:port`; `--allow-env` takes a variable name; `--allow-run` takes a program name; `--allow-py` takes a Python **module** name (substrate-r1: grant granularity is per module, `--allow-py math` runs `math.*` calls and nothing else; granting a module is an explicit trust act, the module executes with the interpreter's OS privileges).
- **substrate-r1, the `py` bridge (D-010):** `py(module, "dotted.func", args?)` calls into the Python ecosystem through an isolated-mode child (`python3 -X utf8 -I -B`) running an embedded one-line-JSON runner. The child gets the same scrubbed environment as `run()` children; the request is one JSON line on stdin (then the pipe closes); the response is the LAST non-empty stdout line, so modules that print during import or call cannot corrupt the protocol. Result is a map `{ok, value, error, code}`: Python exceptions are DATA (`ok:false`, `code:1`, the exception line in `error`), protocol failures are `code:-2`, a timeout kill is `code:-1` with a note, the interpreter never crashes on a misbehaving module. Container ceilings mirror `run()`: wall time is fuel (1000 steps/ms), `.cell py.timeout_ms` (default 10000, clamped 1..300000), output capped at 64 MiB per stream. Marshal rules: Operon values ride the language's own JSON serializer (Seq → base string); Python side converts numpy scalars via `.item()`, ndarrays via `.tolist()`, tuples/sets to lists, bytes via UTF-8 decode, dates/Decimals to ISO/float; non-finite floats are refused by the runner (`ok:false`). The bridge is how Operon joins the scientific-Python wall (§1) instead of fighting it: the stdlib stays small because NumPy/SciPy/Biopython are one granted call away.
- **The bridge is OPTIONAL (W078).** Operon is fully functional with zero Python present, `py()` is the only surface that touches it, and the language, stdlib, toolchain, and every non-`py` builtin are unaffected by its absence. Default state is OFF: no grant → `py()` raises catchable Stress `interference` (capability fence, §9b) before any interpreter is probed. With a grant but no interpreter on PATH, the bridge reports a clean operational error, Stress `missing`, message `python interpreter not found (tried python3, python)`, never a crash, never a hang. A Python-absent machine must pass the entire non-`py` test corpus byte-identically (the differential harness itself runs the Python oracle, which is a TOOLCHAIN dependency, not a runtime one, the shipped `operon` binary needs no Python to run, check, fmt, build, doc, or watch).
- **Python version contract (W079, §15b).**

  | Question | Contract |
  |---|---|
  | Supported floor | **Python ≥ 3.10**. The bridge child reports its version (`py` key in the protocol response); a run whose interpreter is below 3.10 emits a rung-4 note **once per run**: `py bridge: interpreter X.Y is below the documented support floor (3.10); behavior may drift`, the call itself still proceeds. |
  | Newer interpreters | Supported forward-as-is: 3.11/3.12/3.13+ are expected to work; any break is a bridge bug to file, not a new support tier. |
  | Third-party packages | No guarantee is made for NumPy/pandas/SciPy beyond what the marshal rules below state (numpy scalars/ndarrays convert via `.item()`/`.tolist()`); if a package exists in the environment it is importable, full stop. Package absence is `ok:false` data, not a crash. |
  | Startup cost | Each `py()` call spawns a fresh isolated-mode child (isolation over speed, deliberately); wall time is charged as fuel. Callers batching many calls should amortize at the Python side (one `py("module", "batch_driver", [args])` per workload, not one per element). |
  | Windows caveat | `python3` is probed first, then `python` (the Windows launcher name); if neither is on PATH the bridge reports the same clean `missing` Stress as any other absent interpreter. |

- `.cell` grant keys (`allow.read = /data`, …) are honored **only** when the config is loaded explicitly via `--cell file.cell`. An auto-detected `operon.cell` cannot grant capabilities, its `allow.*` keys are ignored with an `[info]` note (a file that happens to sit in the project must not silently widen the sandbox).
- `use` imports: files inside the program's own project directory, the CWD, or the standard library are always importable (otherwise nothing imports under default-deny). A `use` that resolves outside those managed trees requires a read grant; the denial is Stress `interference`.

Resource ceilings (all raise catchable Stress):

| Resource | Ceiling |
|---|---|
| Recursion depth | 10,000 (`overflow`) |
| Step budget | 200,000,000 steps per run (`overflow` "step budget exhausted"); `--fuel N` lowers it |
| String `.repeat()` allocation | 512 MiB |
| Aggregate run allocation | 2 GiB (push/concat/repeat/interp assembly all charged; sec-r5: large-string variable reads and writes/reads of file bytes are charged too, a 500 MB string read three times IS 1.5 GB of real allocation) |
| Collected `run()` child output | 64 MiB per stream (collected prefix is returned; the child is still timeout-killed) |
| Collected `py()` response (substrate-r1) | 64 MiB per stream, same contract as `run()`; wall time charged as fuel |
| Parse/lex notes | 10,000 per parse (further notes suppressed) |
| `distance()` dynamic-programming table | 10,000,000 cells |
| `sleep()` | 60,000 ms (sleep escapes the step budget, so it is capped) |
| `json_parse` nesting | 512 levels |
| Integer arithmetic | i64, overflow → `overflow` (no wrap) |
| Map non-scalar-key scan (sec-r5) | 512 entries per lookup/upsert, beyond that a non-scalar (list/map) key is treated as absent; scalar keys keep exact semantics via the hash memo |

**The W32 overflow contract, stated once and pinned everywhere.** There is no silent wrap on any integer surface: `+`, `-`, `*`, `**`, `//`, `%`, unary negation, and `abs` raise catchable `overflow` at the exact i64 boundary with per-op messages (`int overflow in '+'` etc.); shifts require the amount to be in `0..=63`; `//` and `%` guard the `i64::MIN / -1` corner on both engines (the oracle materializes the same corner stress the Rust core raises); `/` by zero is `unfolded`, never `overflow`; float `*` and `/` reach `inf`/`nan` as ordinary VALUES, while float `**` overflow raises; mixed int/float arithmetic widens and never overflows. The whole contract is pinned with exact kind+message asserts in `tests/overflow_contract.op` (both engines) and byte-identical output in `tests/differential/overflow_contract.op`. **BigInt / Decimal:** a native BigInt/Decimal type is deliberately deferred until a real use-case forces it (a crypto std would); the sanctioned escape hatch is `std/bigint`, exact arbitrary-precision integers over sign-magnitude base-10^4 digit lists (`big_from_int`, `big_from_str`, `big_to_str`, `big_to_int`, `big_cmp`, `big_add`, `big_sub`, `big_mul`, `big_pow`, `big_fact`, `big_neg`, `big_abs`, `big_is_zero`), pure `.op`, deterministic, byte-identical on both engines. `big_to_int` returns null (never a clamp) when the value does not fit i64; parse functions return null on malformed input; zero is always canonical (`-0` normalizes, so structural `==` works).

sec-r5 containment semantics worth stating plainly:

- **DAG-shaped values serialize and compare in linear time.** `json_str`, structural display (`print`), and `deep_eq` (`==`) keep their visited/comparison sets for the whole walk instead of unwinding them. A shared (aliased) subtree is rendered once; later references render as `null` (JSON) or the `[...]`/`{...}` marker (display), the same containment CPython applies to cycles, extended to aliased DAGs, which previously re-walked exponentially (`l = [l, l]` chains: 2^45 node visits for one builtin call).
- **`json_str` emits valid JSON only (RFC 8259).** Non-finite floats serialize as `null` instead of bare `inf`/`nan` tokens that no JSON parser accepts.
- **The aggregate allocation ceiling and the step budget are monotonic.** Once crossed, the counter stays over the limit, every later charge fails, including charges inside a `rescue` handler, so a run that breached a resource ceiling ends with the top-level containment note rather than resuming. Resource-exhaustion overflow is the one stress a rescue cannot recover from; the contract is a bounded note, never an OOM kill or allocator abort.
- **CI/supply chain:** every GitHub Action in both workflows is pinned to a commit SHA (checkout, toolchain, gh-release, D-6 closed); `Cargo.lock` has zero runtime dependencies (build-time only: cc, shlex, find-msvc-tools).

## 10. Builtins and methods

**Builtins, core:** `promote(*a)` (print, space-joined, returns null) · `len` · `push(l,v)` · `pop(l)` · `insert(l,i,v)` · `remove(l,i)` · `keys(m)` · `values(m)` · `has(m,k)` · `del(m,k)` · `range(a, b?, step?)` (returns List) · `str` · `num` (fails → 0 + note) · `type` (`null bool int float str list map gene native sequence`, or the phenotype name for instances) · `abs min max sum` · `floor(x)` `ceil(x)` (→ Int) · `sqrt(x)` `pow(b, e)` (→ Float) · `clock()` (monotonic seconds, float) · `now()` (monotonic seconds, float, the same high-resolution timer under a briefer name) · `exit(n?)` (capability-gated, sec-r2: kills the host process, so it is default-deny, grant with `--allow-exit` or `.cell allow.exit = true`) · `assert(c, msg?)` · `codon(s)` (0–100 style score of an identifier) · `distance(a,b)` (edit distance, C++ bit-parallel kernel; 10M-cell ceiling and a 64 KiB per-operand cap, over-budget pairs never win a nearest-match contest) · `similar(a,b,maxd?)` (bool) · `transcribe(dna)` · `translate(rna)` (stops at stop codon) · `reverse_complement(dna)` · `gc_content(dna)` (0–100) · `find_orf(dna)` (list of ORF proteins) · `memory()` (W097 accounting, map `arena_bytes, interns, allocs` from the in-process symbol table: **arena_bytes** = live bytes held by the Rust-owned intern table (symbol spellings; NOT the process RSS, NOT interpreter values, refcounted values live in ordinary Rust allocation that this counter does not see); **interns** = count of interned canonical spellings (one per distinct identifier the process has lexed, oldest-first in `:symbols`); **allocs** = allocation operations served by the table since process start (monotonic, never reset). All three are process-wide, cross-run cumulative for one binary invocation, and thread-safe (mutex-guarded). Honest limit: this is a symbol-table gauge, not a heap profiler, per-value/per-gene accounting is the MEM-PROFILER design (docs/design/MEM-PROFILER.md, deferred W097-v2); **cycles** (W013/D-013) = live detected reference cycles, a fourth process-cumulative key with its own contract (registration at the edge that closes a cycle, prune only on PROVEN death or breakage, bounded deterministic walks, §19e) and the weak-handle API around it (§19f)) · `weak(v)` / `strengthen(w)` (W013 weak references: non-owning handles to list/map/phenotype values, dead derefs answer null, refusals and membrane rules in §19f) · `methyl(key, default?)`.

**Builtins, randomness and dynamic dispatch:** `random()` (float in [0,1)) · `random(n)` (int in [0,n)) · `randomize(seed?)` (deterministic xorshift state, identical in both implementations) · `chr(i)` · `ord(c)` · `argv()` (List of the arguments after the script path) · `sleep(ms)` (≤ 60,000 ms) · `call(name_or_gene, args_list)` (dynamic dispatch, resolves builtins, named genes, or gene values).

**Builtins, iteration and numeric (v2.3):** `enumerate(x)` (List of `[i, v]` pairs; List or Str) · `zip(a, b)` (List of `[va, vb]` pairs, min length) · `sorted(l, cmp?)` (new list; same comparator contract as `.sort`, cmp true when a belongs before b, or the default mixed-type order) · `reversed(l)` (new list) · `any(l)` / `all(l)` (truthiness; empty → `false` / `true`) · `first(l)` / `last(l)` (element or char; empty → Null + note) · `take(l, n)` / `drop(l, n)` (clamped slices; List or Str) · `unique(l)` (deep-equality dedup, first occurrence kept) · `flatten(l)` (one level) · `chunk(l, n)` (size-n groups, last partial; n ≤ 0 → note) · `round(x, d?)` (d omitted/0 → Int, else Float; **half away from zero** at the d-th decimal, computed in the same f64 formula in both implementations, `round(1.005, 2) == 1.0` because 1.005 is stored as 1.00499…) · `clamp(v, lo, hi)` (numbers; non-number → Null + note) · `divmod(a, b)` (`[q, r]` with the exact `//` and `%` semantics, floored, sign follows divisor). Wrong-type inputs return Null (or `false` for `any`/`all`) with a note, never a crash.

**Safe accessors (v2.3):** `m.get(k, d?)` (Map; deep-equality key lookup; missing → default when given, else Null + note) · `l.get(i, d?)` (List; negative index from the end; out of range → default or Null + note) · `s.at(i, d?)` (Str char; same contract). The bare forms (`m.k`, `l[i]`) keep their existing behavior, `.get`/`.at` add an explicit default, they do not change any existing read.

**Builtins, JSON:** `json_parse(s)` (→ value; nesting > 512 → Stress) · `json_str(v)`.

**Builtins, capability-gated (§9b; denied → Stress `interference`):** `read_file(p)` (regular files only, FIFOs/devices are refused `interference`, sec-r5) · `write_file(p, s)` · `append_file(p, s)` · `exists(p)` · `read_dir(p)` (List of names) · `fs_delete(p)` (files and EMPTY dirs; symlinks are refused outright, dx-r6) · `fs_rename(from, to)` (both paths need a write grant) · `fs_mkdir(p)` (create_dir_all semantics) · `run(prog, args?)` (map `code stdout stderr ok`; the child runs under a wall-clock timeout, `.cell run.timeout_ms`, default 10 s, clamped 1..300 000, and the child's wall time is charged as fuel exactly like `sleep`; output may be truncated when a grandchild holds the pipe past the 250 ms drain grace) · `py(module, "dotted.func", args?)` (substrate-r1 Python bridge; map `ok value error code`; per-module grant `--allow-py m`; isolated-mode child, timeout `.cell py.timeout_ms`, wall time is fuel, 64 MiB output cap; Python exceptions are data `ok:false`, never crashes, full contract in §9b) · `http_get(host, port?, path?)` (response body) · `serve(port?)` · `recv_request()` (map `conn method path body`, or null) · `send_response(conn, status?, ctype?, body?)` (status/ctype containing control characters are refused, no header injection) · `env(name)`.

**Builtins, concurrency, telemetry, regulation:** `fingerprint()` (run telemetry, §14) · `spawn(f, args?)` → id · `join(id)` → value (default wait ceiling 300 s, timed-out tasks stay joinable, join returns null; explicit `join(id, ms)` unchanged) · `cancel(id)` / `task_state(id)` / `cancelled()` (W18 cooperative cancellation, full contract §13) · `wait_all(ids)` / `wait_any(ids, ms?)` (W15 task groups, §13) · `channel()` / `send(ch, v)` / `recv(ch)` / `close(ch)` / `select(ch...)` (W015 channels, full contract §13) · `toggle_on(name)` · `toggle_state()` · `repressi_next()` · `repressi_state()` (fuel-charged: integrating new ring ticks costs 20 steps/tick) · `repressi_start(ms)` (one shared cancellable timer per run, restarting retires the old thread) · `grn_fire(name, decay?)` · `grn_state()` · `grn_set(name, v)` (write a node's level, clamped 0..1) · `grn_get(name)` (read a node's level) · `methylate(name)` / `demethylate(name)` (runtime methylation, same graded semantics as the `@methylate`/`@acetylate` marks: level +1 / saturating −1, gate applies at the next call; returns the gene's new level; keys are allocation-charged like any growth) · `m6a_write(name, n?)` / `m6a_erase(name, n?)` (reg-bio-3: quantitative m6A site density 0..=3, writer/eraser dose, default 1; resistance to redefinition requires level ≥ 1) · `passage(n)` (reg-bio-3: n cell divisions, methylation levels dilute by `.cell methyl.maintenance` with half-down rounding, `generation` advances; returns the generation) · `expr_on(kon?, koff?)` / `expr_off()` (reg-bio: the telegraph promoter layer, §11; in-source switch for stochastic expression, seeded via `randomize`).

**Str methods:** `.upper() .lower() .trim() .split(sep) .join(list) .replace(a,b) .contains(x) .starts(x) .ends(x) .repeat(n) .slice(a,b) .len()`
**List methods:** `.map(f) .filter(f) .reduce(f, init) .each(f) .sort(cmp?) .reverse() .contains(x) .index_of(x) .slice(a,b) .join(sep) .len()` (cmp returns true when a before b)
**Map methods:** `.keys() .values() .items() .has(k) .del(k) .len()`

**Bio-layer note (§11a):** the codon-kernel family and sequence utilities in the core row above (`codon` `distance` `similar` `transcribe` `translate` `reverse_complement` `gc_content` `find_orf`) and the bio half of the regulation row (`toggle_*` `repressi_*` `grn_*` `methylate` `demethylate` `m6a_write` `m6a_erase` `passage` `expr_*`) are the biology layer's runtime surface; the mechanisms live in §11 and the crossings are stated once in §11a.

## 11. Gene-expression regulation layer (v2 core novelties)

### 11a. The biology layer: contract header, freeze, and boundary (W091)

**Scope.** §11 (this section) is the semantics home of the biology layer, and §16 is its
vocabulary map for docs. The layer: the gene regulatory network and its call gates, the
marks, stoichiometric silencing, alternative splicing and runtime variant swap, fate
machines, signal pools and quorum, polycistronic `operon` units (Rho termination, ribosome
queues), the telegraph promoter and bursting, the repressilator ring, the decay clock,
two-tier translation, epigenetic dilution, and the `.rna` / `.cell` surfaces that configure
them. The layer can gate, redirect, degrade, and meter calls; it cannot redefine what a
value, a call, or a closure is: no statement in §11 or §16 changes the evaluation rules of
§1-§10, the verification rules of §12, the concurrency rules of §13, the toolchain rules of
§15, or the memory model of §19.

**Freeze (W036).** No new biology-flavored syntax enters the core language: new mechanisms
land as `std/*.op` libraries, as `.cell` keys, or as builtins with a DECISIONS entry. The
governance, the frozen inventory, and the enforcement story (including its honest gap) live
in `docs/specs/CORE-BIO-BOUNDARY.md`; the compatibility classes are
`docs/specs/COMPATIBILITY.md` (W63/W64). What that freezes, and what this section
documents: the 25 keywords frozen at W036 (regulation 10, signal pools 4, operons and
oscillators 3, variant swap 3, fate machines 3, entry and editing 2), the 6 marks
(`@acetylate` `@methylate` `@m6a` `@copies` `@riboswitch` `@burst`), the contextual
statement spellings (`translates` `attenuates` `secrete` `quorum` `quench`, the edge
modifiers `sum` `any` `occupy` `hill`, per-cistron `rbs`), and the `.cell` tuning keys.
The live keyword table is generated ([docs/KEYWORDS.md](docs/KEYWORDS.md)) and is never
counted by hand here.

**Modeling contract.** Every mechanism below carries an honesty grade (REAL / APPROX /
ABSTRACTION / SIMPLIFICATION) and an output-meaning statement in
`docs/spec/BIO-CONTRACT.md`; a change confined to §11/§16 plus that contract is a
bio-modeling change, never a language change (BIO-CONTRACT governance rule 4).

**Determinism.** Bio evaluation is deterministic and seeded: same binary version + same
source + same seed + same flags + same `.cell` gives byte-identical output, core and bio
surface alike (`docs/spec/DETERMINISM.md`). Every stochastic mechanism in this layer draws
from a deterministic seeded stream: the shared mirrored xorshift stream (silence capture,
promoter bursts), worker streams derived from the task id (the D9 bullet below), or
streams keyed to absolute position so a cached replay cannot diverge (repressilator
noise); platform entropy never enters, and the p∈{0,1} no-draw discipline keeps legacy
programs bit-identical.

**Voice (D-008).** Within this specification, §11 and §16 are the one place the metaphor is
load-bearing; every other section of this document, and the documents outside this part,
keep the D-008 voice: zero biology assumed, gene vocabulary is an intuition aid with
one-line programmer analogies ([docs/KEYWORDS.md](docs/KEYWORDS.md)), never a
prerequisite. Each §11 bullet carries its own term audit where the name and the honest
effect diverge.

**The boundary, in one paragraph (a newcomer may quote this).** Operon has two layers. The
core layer (§1-§10, §12, §13, §15, §19) is an ordinary dynamic language: values, genes
(functions), phenotypes (classes), modules, stress containment, concurrency, toolchain,
memory. The biology layer (§11, plus the vocabulary map in §16) is a frozen, optional set
of bio-named mechanisms that ride on the core: they keep their own state, and they may
gate, redirect, degrade, or meter calls, but they never change what a value, a call, or a
closure is. The test: a construct is bio-layer iff its behavior cannot be predicted
without reading §11; if the core sections alone predict it, it is core however biological
its name (`fate` is a state machine, `splice`/`variant` is an implementation swap,
`tad`/`anchor` is module insulation, `frame`/`proof` is verification). With no regulation
declared and no marks attached, every program evaluates exactly as the core sections say,
and all three implementations agree on that byte-for-byte.

**Boundary map** (every bio construct with its anchor):

| Constructs | Anchor | Layer |
|---|---|---|
| Frozen bio keywords: `regulate` `activates` `inhibits` `strength` `threshold` `enhance` `silence` `decoy` `bind` `toggle` / `ligand` `inducer` `cofactor` `autoinducer` / `operon` `period` `repressilator` / `splice` `variant` `replace` / `fate` `state` `enter` / `ires` `edit` | semantics §11, statement forms declared in §11, CLI §15 | bio (frozen) |
| Marks `@acetylate` `@methylate` `@m6a` `@copies` `@riboswitch` `@burst` | lexical §3, attachment §7/§7a, semantics §11 | bio (frozen) |
| Contextual spellings `translates` `attenuates` `secrete` `quorum` `quench`, edge modifiers `sum` `any` `occupy` `hill`, per-cistron `rbs` | §11 | bio (frozen) |
| GRN evaluation: levels, waves, once-per-fire inhibition, thresholds, Hill coefficients, `occupy`, `sum` pooling, `decoy`, decay | §11 (`regulate` bullet) | bio |
| Oscillator: `repressilator` ring, kinetics, seeded noise | §11 | bio |
| Signal pools: `ligand`/`bind` allostery, `autoinducer` quorum, `quench` | §11 | bio |
| Polycistronic units: `operon`, `rbs`, polarity, Rho termination, ribosome queues | §11 | bio |
| Two-tier translation: `translates`, m6A reader fate | §11 | bio |
| Epigenetics: methylation levels, `passage` dilution, `generation` | §11, counters §14 | bio |
| Stochastic expression: telegraph promoter, `@burst`, `burst_set`, `promoter_telemetry` | §11, counters §14 | bio |
| Variant swap: `splice`/`variant`, `splice_shift` | §11, builtin §10, CLI §15 | bio (semantically generic, grandfathered) |
| Fate machines: `fate`/`state`/`enter` | §11 | bio (semantically generic, grandfathered) |
| Entry and editing: `ires`, `.rna` `edit` patches | §11, CLI §15 | bio |
| Check-time bio analysis: NMD sweep, codon-score grading | §11, §15 | bio |
| `.cell` bio keys: `methylate.*`, `grn.*`, `rho.*`, `ribosome.*`, `m6a.*`, `enhance.delta`, `ligand.*`, `quorum.dilution`, `repressi.*`, `[expression]`, `operon.polarity` | §11 (`.cell` bullets), schema `docs/specs/CELL-SCHEMA.md` | bio tuning surface |
| Bio builtins named in §10: `toggle_on` `toggle_state` `repressi_next` `repressi_state` `repressi_start` `grn_fire` `grn_state` `grn_set` `grn_get` `methylate` `demethylate` `m6a_write` `m6a_erase` `passage` `expr_on` `expr_off` | §10 regulation row, mechanisms §11 | bio |
| Bio builtins introduced with their mechanisms: `burst_set`, `promoter_telemetry`, `splice_shift`, `secrete`, `quorum`, `quench`, `quorum_state`, `ligand`/`ligand_set` pool reads | §11 | bio |
| Codon-kernel family: `distance` `similar` (kernel `runtime/codon_kernel.cpp`), `codon` scoring; sequence utilities `transcribe` `translate` `reverse_complement` `gc_content` `find_orf` | §10 core row, kernel `runtime/codon_kernel.cpp` | bio (kernel is the Rule-5 walled foreign-function surface) |
| Bio telemetry: `bursts`, `transcripts`, `generation`, profile flags | §14 | bio counters inside a core map |
| Bio CLI: `--variant`, `--rna`, `--ires`, `--cell`, `--trace-grn`, `check --nmd`, `operon crispr`, `build --variant` | §15 | bio tooling on the core CLI |
| Bio state on workers: the regulation snapshot | §13 worker bullet | bio state via the core membrane |
| Bio-NAMED core (name only, no bio semantics): `tad`/`anchor` (§8 module insulation), `frame`/`proof` (§12 verification), guard-as-uORF (§7), RNAi as the sandbox naming (§9b) | their own sections | CORE |

**Crossings** (each stated once, one sentence):

- Calls: every bio gate (RISC redirect, toggle, GRN veto, methylation, riboswitch,
  promoter, opt-in Rho) executes on the core evaluator's gene-call funnel at one pinned
  order (§11 gate-order bullet), after which the call is an ordinary core call or a null
  return with a note.
- Scheduling: bio-only scheduling is the gate ordering around a call; no bio gate spawns a
  thread and no bio mechanism alters the fuel, join, or cancellation rules of §13 (the
  repressilator's wall-clock timer is a clock input to the ring, §11, not a worker cell).
- Snapshot: bio state reaches a worker only through the core spawn snapshot membrane
  (§13, §19d), copied at spawn, never live-shared; the one documented exception is the
  quorum medium, a process-global pool outside the snapshot with a join-before-read
  contract (§11 quorum bullet).
- Codon kernel: `distance()`/`similar()` are a foreign-function surface into the C++
  kernel with its own budget guard (10M-cell DP ceiling, 64 KiB per-operand cap,
  over-budget pairs never win a contest); `codon()` and the sequence builtins around them
  are ordinary total functions.
- Entropy: every bio draw comes from a deterministic seeded stream (the shared mirrored
  xorshift stream, a worker stream derived from the task id, or a position-keyed stream for
  cached ring noise), never from platform entropy, so bio and core randomness stay
  replayable (`docs/spec/DETERMINISM.md`).
- Errors: a bio gate never introduces a new failure channel; suppression is the core soft
  tier (null + note) and bio failures surface through the ordinary §9 stress kinds.
- Config: bio `.cell` keys ride the one runtime-only `.cell` loader (§11 W22 bullet) and,
  like every key, can never grant capabilities (§9b).
- Telemetry: bio counters (`bursts` `transcripts` `generation`) are keys in the core-owned
  `fingerprint()` map (§14), emitted in sorted order (§11 D9 bullet).
- Toolchain: every bio CLI verb calls the same parser, evaluator, and checker; there is no
  separate bio runtime and no second evaluator (§15).

All features are real, implemented, tested, none are decorative.

- **`tad` / `anchor`**, §8. Module insulation with export anchors. (Core module semantics listed here because it shipped in the same v2 wave; the biology layer only borrows the name, §11a.)
- **`enhance a, b, c;`**, super-enhancer cluster: marks genes with an **activation boost** (v2.2). Under the GRN call gate, an enhanced gene lowers every incoming activating threshold by **0.25** (floored at 0; the dose is tunable via `.cell enhance.delta`, reg-bio: real enhancer strength varies with binding-site number and affinity), an enhanced gene fires where an unenhanced one stays gated (e.g. regulator level 0.4: threshold 0.5 blocks the plain gene, passes the enhanced one). Genes without the mark are unaffected and edges without a threshold stay declarative, so old programs keep running (§5 default: the boost exists only where `enhance` was declared). `operon profile` shows the `enhanced` flag; `operon check` gives the file a codon-score bonus for enhanced hot genes; the NMD untranslated sweep skips enhanced genes.
- **`@acetylate`**, histone acetylation mark: excluded from silence rewriting and immune to the silencing gates (active chromatin stays active, open chromatin wins, D-005); shown as `active` in profile. (Term audit, reg-bio: acetylation is permissiveness, neutralized lysine charges open the chromatin, not dispatch priority; the mark's honest effect is immunity, nothing else.)
- **`@methylate`**, histone methylation mark: gene is repressed, **graded** (v2.2). Every executed `@methylate`-marked `gene` definition deepens that gene's silencing level by 1; an `@acetylate`-marked definition relaxes it by 1 (histone marks compete on the same chromatin). A call to a gene whose level has reached the threshold (default **3**, tunable via `.cell` `methylate.threshold = n`) is **blocked**: it returns `null` with a fallback note ("methylation silences: …, call returns null"), never reaches the gene body, and does not count in `fingerprint()`/burst telemetry, repressed means repressed. Below the threshold, calls execute and the first call emits the soft note "methylated call" (later calls are silent, the cell does not narrate every repression; suppressed entirely when `.cell` sets `methylate.quiet = true`). Marked genes are exempt from the NMD untranslated sweep, and `@acetylate` genes are exempt from the methylation gate itself (open chromatin wins). One mark (the common case) never silences, old programs keep running.
- **`@m6a`**, m6A mark = dispatch priority + redefinition resistance (term audit, reg-r4: the honest analogy for "the marked original wins over the unmarked new copy" is prokaryotic **DNA** m6A, Dam methylation marks the parent strand so mismatch repair knows which base is the error; eukaryotic RNA-m6A "stability" is reader-dependent and the wrong analogy): among same-name candidates (splice variants, shadowing), the m6a-marked one wins resolution (variant selection order: `.cell` > CLI > **m6a-marked** > first declared, a marked variant beats first-declared), and an m6a-marked binding **resists redefinition**, an unmarked re-`gene` of the same name is ignored with a note ("@m6a-stabilized; redefinition ignored"); mark the new copy too, to replace it.
- **`silence old -> new strength s sites n;` / `silence old;`**, RISC-style silencing, now STOICHIOMETRIC (reg-bio-3, C9). The redirect form: every call to gene `old` is redirected to `new` with a note "RISC: call silenced". The target-less form (reg-bio) is **pure degradation**, real RISC/miRNA destroys the transcript, there is no replacement gene: every call to `old` returns null with a "RISC degraded" note, and a degraded call is not expression (it never reaches the call counters or burst telemetry). **Dose (reg-bio-3):** `strength s` is the per-site capture probability (clamped 0..=1, default 1.0) and each `silence` statement for the same target is one binding site (`sites n` composes multiplicatively, 1..=64): the per-call capture probability is `1 − Π(1−sᵢ)^sitesᵢ`. Real RNAi is dose-dependent, limiting RISC complexes give fractional knockdown, and multiple target sites compound. A sub-1.0 capture draws ONCE per call attempt on the shared mirrored xorshift64* stream (the telegraph-promoter discipline; the oracle mirrors it op-for-op); a captured call takes the redirect/degrade path, an escaped call proceeds through the pinned funnel with a one-time "RISC escape" note. `strength 1.0` with one site is the legacy binary redirect and consumes NO entropy, the `random()` stream is untouched for every legacy program. `@acetylate` genes are immune, and the immune check precedes any draw (immunity costs no randomness). Same-name splice variants resolve first, then silencing applies to the resolved name.
- **NMD sweep** (`operon check --nmd`): "premature stops" = unreachable statements after an unconditional `return` (reported −4 each); "untranslated transcripts" = defined, never-called, non-exported, non-enhanced genes (info, −1). `--nmd purge` rewrites the file without them.
- **`splice name { variant a { } variant b { } }`**, alternative splicing: `name(...)` dispatches to the active variant. Selection order: `.cell` `variant.name=v` > CLI `--variant v` > **m6a-marked variant** (v2.2) > first declared. Variant declarations may carry histone marks (v2.2): `@m6a variant v { }` makes `v` the m6a-priority variant; `@acetylate`/`@methylate` on a variant ride on the resolved binding (acetylate immunity applies when a marked variant is active). Marks stack and can precede the variant keyword in any order; a mark not followed by `variant` is noted and skipped. Variants take parameters. `operon build --variant a` bakes one variant into a standalone file (build caveat: a variant's parameter list is not yet carried into the baked file, bake param-less variants, or keep the splice in the source).
- **`.rna` edit patches**, `edit target { replace "src" -> "dst"; }` where target is a file name or gene name; applied to that target's source text before parsing. CLI: `operon run app.op --rna hot.rna`. Each applied replacement emits a note. **Node-addressed v2 (W067)**: a patch whose first content line is `syntax: v2` dispatches to the AST editor instead, `rename <path> -> <new>`, `delete <path>`, `body gene NAME[#N] { ... }` over paths `gene NAME[#N]` / `splice ROOT` / `variant ROOT.NAME` / `phenotype NAME` / `method PHENO.GENE` / `fate NAME` / `regulate #N`; edits apply to a freshly parsed AST and the result is reprinted through the canonical formatter (apply output is fmt-stable by construction). All-or-nothing: any missed/ambiguous target refuses the whole patch (exit 1, per-rule fate report, nothing written); bare names shared by 2+ declarations require an ordinal (`gene foo#2`). The reprint drops plain `#` comments, so v2 refuses such sources unless `--allow-comment-drop`. CLI: `operon rna app.op patch.rna [--write] [--json] [--allow-comment-drop]`; header-less patches keep the v1 text semantics byte-for-byte. **v1 deprecation (W067 stage 3, W63 step 1, info since 2.2.0)**: on the `operon rna` editor path a header-less patch emits an info note on stderr and `--json` reports `"engine":"v1","deprecated":true`; the patched FILE output is unchanged. Severity escalates per W63 (warning, then removal one minor later); the `run --rna` pre-parse path is deliberately note-free (differential stderr parity). **Check mode (W068)**: `operon rna app.op patch.rna --check` validates the patch against its target and writes NOTHING, ever (`--check --write` refuses, exit 2): engine detection (v1/v2), per-rule span/node resolution + ambiguity fate, the v2 comment-preflight result (`ok|refused|allowed`), and whether a real apply with the same flags would succeed (`would_apply` in `--json`); exit 0 clean, 1 on any validation failure, 2 on usage errors. Design + as-built: `docs/design/RNA-V2.md`.
- **`.cell` methylation config**, `key = value` lines, `[section]` headers, `#` comments. CLI `--cell f.cell`; else `operon.cell` auto-detected.
- **`.cell` is RUNTIME configuration ONLY (W22, audit item 22)**, the hard boundary: `.cell` keys configure the interpreter for one run (behavior knobs, capability grants, feature flags). Package/project metadata, `name`, `version`, `operon-version`, `dependencies`/`deps`, `authors`, `license`, belongs in the **`operon.toml` manifest** (W19/W022, track L3c), never in `.cell`. The two files never merge: a package installed by a registry must be able to ship its own runtime config while the parent project's manifest stays authoritative. The `cell/package-keys` lint (W48) will warn when package-like keys appear in a `.cell`; until then, unknown keys are inert data readable via `methyl()` (the W66 formal schema, dev-3, defines the full key inventory). Read via `methyl("k", d?)`. Impl-consumed keys: `variant.<splice>` (active variant), `methylate.quiet` (suppress the methylated-call note), `methylate.threshold` (graded-silencing gate, default 3), `grn.decay` (default GRN pulse decay, reg-r2), `run.timeout_ms` (child-process wall-clock timeout for `run()`, default 10000, clamped 1..300000), `py.timeout_ms` (Python-bridge wall-clock timeout for `py()`, default 10000, clamped 1..300000, substrate-r1), `wobble.strict = true` (a run with any rung ≥ 3 note exits 3, same as `--strict`), `entry` (cap-independent default entry, below CLI `--entry` and above `main`), and `allow.*` capability grants, honored only with explicit `--cell` (§9b; `allow.exit = true` grants the exit capability, sec-r2), `operon.polarity` (loop-9 D1 expected-value polarity weight, default 0.5), and the loop-10 Rho/queue knobs: `rho.termination` (the opt-in layer switch, default off), `rho.catch` (per-cistron catch probability, clamp 0..1, default 0.5), `rho.queue_floor` (shield threshold, default 0.5), `ribosome.queue_cap` (queue saturation, default 1.0), `ribosome.drain` (per-integration drain, default 0.5).
- **`ires name;`**, internal ribosome entry site: declares a cap-independent entry gene. `operon run --ires` overrides the canonical `main` entry and runs the first declared `ires` target; a file with no `main` uses it automatically. Entry precedence: CLI `--entry` > `.cell` `entry` > `main` > first `ires`.
- **`fate Name { state a -> b, c; state b -> a; enter a; }`**, fate landscape state machine. `let m = Name()` enters the `enter` state (default: first declared). Methods on instances: `.shift(s)` (true if transition allowed; invalid → note + false, state unchanged, the valley stay), `.state()` (current), `.can(s)` (bool). Instances are Maps with hidden `#fate`/`#state` keys.
- **`regulate { a activates b strength 0.8; c inhibits d; }`**, gene regulatory network (default strength 1.0). The network is **stateful**: by default levels persist across `grn_fire` calls (a latch, the honest default; real dilution needs the decay below), and each fire seeds additively, capped at 1.0. **Decay (reg-r2):** opt-in dilution, before every pulse, every existing level decays by a configurable fraction: `grn_fire(name, f)` takes the pulse's decay directly (0..1, clamped), or `.cell` `[grn] decay = f` configures it globally; unset/0 is byte-identical to the pre-decay behavior. (Term audit, reg-r3: persistence is a latch, decay-to-zero is dilution/degradation, neither is "homeostasis", which requires a regulated setpoint.) Each fire runs in two phases: (1) **activation** propagates in waves, `child = max(child, parent × strength^wave)`, hop attenuation over up to 10 waves; (2) **inhibition** is applied exactly once per fire, `child = max(0, child − parent × strength)`, based on the source's post-activation level, a repressor's concentration sets the output level, it does not compound across waves. An edge may carry an optional `threshold t` (dose-response): influence = `strength × pⁿ/(pⁿ + tⁿ)` where p is the parent level, weak below the threshold, saturating above it. The exponent `n` is the **Hill coefficient** (cooperative binding, reg-bio): `hill n` (integer 1..=8; canonical edge order `strength → threshold → hill → any`) overrides the default n = 2 per edge, `hill 1` is the graded Michaelis shape, higher n is ultrasensitive, and the exponent is computed by repeated multiplication (never `pow`) so both implementations stay bit-identical. An edge may also carry the keyword `any`: it becomes an **OR member**, an alternative activator that alone suffices. `hill`/`any` on an edge without a threshold, or `any` on an inhibitor, are noted and ignored (Total Grammar). `grn_state()` returns the level Map (all genes in the graph); `grn_set/grn_get` write/read single node levels (clamped 0..1) so hosts can steer gate state without re-firing. **Levels gate calls** (v2.2): a gene with an incoming activating edge carrying an explicit `threshold t` executes only while `level(source) ≥ t`; several activating edges form **cis-regulatory input functions** (reg-bio), every AND member must pass, and an `any` (OR) member alone suffices: the gate opens iff (every AND member passes) OR (any OR member passes); an all-OR promoter with every member below its threshold vetoes with "no OR activator above threshold". An incoming inhibiting edge with an explicit threshold silences the callee while `level(inhibitor) ≥ t`, inhibitors veto independently of the OR group. A gated call returns `null` with a fallback note ("grn gate: … call suppressed"). Edges **without** a threshold remain declarative (they shape `grn_fire` level dynamics only), and a threshold of `0` never blocks: declaring a network changes no call behavior unless a program opts in with explicit thresholds. The gate is checked at the gene-call funnel (named calls, higher-order calls, and RISC-redirected calls all pass through it) and precedes the call counters, a suppressed call is not expression and does not count in `fingerprint()`/burst telemetry.
- **`toggle a, b;`**, a **mutual-repression latch** (term audit, reg-r3: a declared boolean invariant, not rate-based bistability, no cooperativity, no hysteresis; the Gardner–Cantor–Collins toggle switch is the biological inspiration, not a simulated model). `toggle_on("a")` turns `a` on and `b` off; `toggle_state()` → Map of both. Exactly one may be on, and the pair gates **calls**: calling the repressed allele returns `null` with a note ("toggle repressed: … is the inactive allele"); `@acetylate` genes are immune (open chromatin wins). Rate-based bistability IS expressible on the `regulate` layer: two mutual inhibiting edges with `hill ≥ 2` land the losing allele in a partial, cooperativity-dependent band (tests/grn_bistability.op proves the cooperativity-dependent partial bands; HYSTERESIS is not yet demonstrated by a sweep test, loop-8; `std/motifs` ships the circuit).
- **`repressilator a -> b -> c;`**, repressive oscillation ring with **emergent mutual repression** (reg-r2): the ring integrates a **reduced, protein-only discrete variant inspired by the Elowitz–Leibler repressilator** (Elowitz & Leibler, Nature 2000; term audit, reg-r3: the published system is a two-state mRNA+protein model with Hill n=2, α≈250, β=5 and a basal term, this is the one-state protein-only reduction with h=4 and no basal term, which oscillates robustly in discrete time). Each substep, node level evolves as `dA/dt = α / (1 + Rʰ) + basal − γA` where R is the level of the repressor (ring `a -> b -> c` means a represses b, b represses c, c represses a), Euler-integrated with 20 substeps of `dt = 0.05` per ring tick (α = 10, γ = 1, init `[5, 0, 0]`). The oscillation, a ~6-tick limit cycle with one-tick production lag between a repressor's peak and its target's trough, **emerges from the loop itself; nothing is scheduled**, so gates can read the ring: a GRN edge sourced by a ring node reads the node's normalized level (`raw/α`, clamped 0..1) at the current tick. `repressi_state()` returns the level Map and is a **pure function of the tick count**, manual mode (`repressi_next()` advances one ring tick) and wall-clock mode (`repressilator … period s;` or `repressi_start(ms)`) fold the identical arithmetic and can never diverge; the Python oracle mirrors it op-for-op bit-identically (§17). **Kinetics are load-bearing (reg-bio, F-5):** inline `repressilator a -> b -> c alpha 20 basal 0.5 noise 0.1 seed 42;` (canonical order alpha, gamma, hill, basal, noise, seed) or `.cell` keys `repressi.alpha/gamma/hill/basal/noise/seed` override the historical constants per-field (the last declaration wins per-field; `.cell` applies first, declarations layer on top). `basal` is the promoter leak real repressed promoters never lose; `noise` injects a per-substep kick, reg-bio-2 (12-d D6): the kick is MULTIPLICATIVE and dt-aware, `v *= 1 + noise·(1/√20)·(2u−1)` (real expression noise is multiplicative and strictly positive; the old additive kick rectified upward through the zero clamp), drawn from a stream derived from the ABSOLUTE (tick, substep) position, so the fold-from-init path and the incremental cache path stay bit-identical and the oracle mirrors it op-for-op, and it never perturbs the program's `random()` stream. Defaults (α=10, γ=1, h=4, basal=0, noise=0) are byte-identical to the pre-parameterized ring. A second `repressilator` declaration replaces the ring. The ring rides `spawn` snapshots frozen at the spawn tick (§13).
- **Telegraph promoter layer (reg-bio, F-1), stochastic expression.** Real promoters switch between active and inactive states; transcription happens in **bursts**, and noise is what lets bistable cells switch fate. Off by default (the deterministic contract is untouched for every existing program). `expr_on(kon?, koff?)`, or `.cell` `[expression] stochastic = true` with `kon`/`koff`/`seed`, turns it on: each gene carries a promoter state (initially active) and every call attempt draws once on the shared mirrored xorshift64* stream, an active promoter switches off with probability `koff`, an inactive one switches on with probability `kon`. A burst-off call returns `null` with a "promoter inactive" note, never reaches the call counters, and is recorded in `fingerprint().bursts`. The state persists across calls, that persistence IS the burst. `randomize(seed)` fixes the stream (reproducible bursting, identical in both implementations); `expr_off()` restores determinism mid-run. Bursting is universal (open chromatin bursts too): no `@acetylate` exemption. Worker cells inherit promoter states and burst counters in the regulation snapshot (§13). **Per-gene promoter identity (loop-9, F-2):** `@burst kon koff` marks a gene with its OWN telegraph rates (clamped 0..=1), overriding the global parameters for that gene alone, different promoters have different (kon, koff); that is their identity. The mark rides the gene definition (workers inherit via the Arc); draw count and stream order are unchanged. **Attempt telemetry (loop-9, F-3):** `promoter_telemetry(gene)` returns `{attempts, on_total, episodes, on_frac, burst_size}`, every call that reaches the promoter gate is an attempt; episodes are maximal ON-runs; `burst_size = on_total/episodes` is the mean ON-run length, ~1 under Poisson-like firing and >>1 under bursting. Telemetry rides the snapshot so worker cells never desync from the host mid-burst. Boundary honesty: episodes count COMPLETED ON-runs, so a run ending mid-ON slightly overestimates burst_size (the trailing episode is uncounted), finite-sample bias, deterministic, documented here rather than hidden. tests/burst_identity.op pins the identity separation on both cores; tests/burst_modulate.op pins runtime modulation AND the `@burst > burst_set` precedence. **Runtime modulation (loop-9, R9 jury):** `burst_set(gene, kon, koff)` retunes ONE promoter's switching rates at runtime, regulators change promoter switching, not just transcript gating; `burst_set(gene)` clears the override. Precedence: `@burst` mark > `burst_set` override > global rates; rides the snapshot (workers freeze modulation at spawn). tests/burst_modulate.op.
- **`a translates p rate r decay d;` (reg-bio-2, C1), the translation layer.** Real expression is TWO coupled tiers: transcripts accumulate fast and bursty; proteins accumulate slower (translation), lag, and smooth the bursts. A `translates` edge (inside `regulate`, verb `translates`; optional `rate` default 1.0, `decay` default 0.0) makes the target protein node integrate one Euler step of the classic two-tier ODE at every engine update point (each `grn_fire` pulse and each decay-clock tick): `p += rate·Δcalls − decay·p`, clamped 0..1, where Δcalls is the source gene's call-count delta since the last integration (checkpoints start at 0). Protein nodes live in the GRN level map, so any gate can be regulated by a PROTEIN instead of a transcript, the observable signature is the two-tier delay (mRNA present, protein still below threshold; tests/grn_translates.op proves the gate opens later than one-tier). (Term audit: Δcalls counts EXECUTED CALLS, not molecules, transcripts-as-calls is the language's own unit; the smoothing/lag/persistence phenomenology is what this layer models.)
- **`occupy` (reg-bio-2, D2b), occupancy repression.** An inhibiting edge marked `occupy` composes MULTIPLICATIVELY: `child *= 1 − influence`, the thermodynamic Kⁿ/(Kⁿ+Rⁿ) survival form. Two partial repressors survive more than subtractive repression would allow (1 × 0.6 × 0.6 = 0.36 vs 1 − 0.4 − 0.4 = 0.2), repression can never overshoot, and near-full occupancy approaches (never crosses) zero. Canonical edge order: `strength → threshold → hill → any → occupy → sum`. Legacy edges keep the subtractive-once form (bit-identical default). (Term audit: occupancy shapes the fire-phase dose arithmetic; the call-gate veto remains threshold-based, occupancy is expressed through the levels it produces.)
- **`sum` (reg-bio-2, B7), synergistic pooling.** `sum` on a thresholded activating edge makes it a POOLED member: edges targeting the same gene with the same (threshold, hill) pool their weighted inputs `P = min(1, Σ strength·level)` and ONE Hill function of P drives both the gate and the fire influence. Two sub-threshold inputs (0.3 each, threshold 0.5) fire together, enhanceosome synergy that individual AND members can never express, and `hill 2` makes the pooled response super-additive (pooled influence > sum of individual influences). Groups act as conjunctive members: the gate opens iff (every AND member passes) OR (any OR member passes), where a pooled group passes iff `P ≥ t` (with the enhance boost). Message order: individual members first, then pooled groups in declaration order.
- **`decoy d for tf capacity c;` (reg-bio-2, C11), competitive titration.** A decoy node absorbs its regulator without producing output: every regulation read of `tf` sees the free fraction `max(0, level(tf) − c·level(d))`. Overexpressing the sponge closes gates that were open; emptying it restores the free fraction. Real TF sequestration/decoy-site titration, deterministic, no RNG.
- **`ligand x;` + `bind tf inducer lg k v;` / `bind tf cofactor lg k v;` (reg-bio-2, A4), small-molecule allostery.** Ligands are metabolites, not genes: `ligand_set("iptg", 0.9)` sets a pool (clamped 0..1), `ligand("x")` reads it, and the `.cell` key `ligand.<name> = f` is the bath default (the runtime pool wins). A binding record modulates the regulator's DNA-available fraction at EVERY regulation read: `occ = L/(k+L)` (k defaults 0.1); an INDUCER reduces affinity, free × (1 − occ) (allolactose on LacI: binding relieves repression); a COFACTOR increases it, free × occ (tryptophan on TrpR: binding enables repression). THE POLARITY IS THE BIOLOGY: lac's inducer turns a repressor OFF, trp's corepressor turns it ON, and in both cases the regulator's LEVEL never changes, allostery, not dilution. A ligand named as an edge SOURCE gates calls straight from its pool, a riboswitch-style, protein-free gate (no `grn_fire` needed). tests/lac_gate.op proves the canonical IPTG + cAMP dual control; tests/trp_attenuator.op proves the corepressor polarity mirror.
- **`attenuates` (reg-bio-2, A5), RNA-level attenuation.** An edge `x attenuates y threshold t;` vetoes like an inhibitor but reports the RNA-level mechanism: "attenuator … (leader terminated)", the OUTCOME of ribosome-coupled leader attenuation (trp operon), not TF occlusion. (Term audit: the ribosome-stalling mechanics are not modeled; the metabolite-threshold outcome is.)
- **`decay_clock(n, f?)` (reg-bio-2, C2), time-driven decay.** The call clock is the language's in-model timebase; `decay_clock(n, f)` fires one GRN decay step (fraction f, default from `.cell [grn] decay`) every n calls, and the translation layer integrates on the same ticks. Decay now runs as expression time passes, real transcripts decay per unit time, not only when someone calls `grn_fire`. `.cell [grn] decay_calls = N` is the config form; `decay_clock(0)` restores the event-driven contract (byte-identical default). (Term audit, C4: call-clock time is an explicit design choice, genes are closures the programmer invokes; a Gillespie SSA scheduler would invert the language's own metaphor. The ring remains the continuous-time enclave.)
- **`operon Name { cistron rbs r; cistron2; }` (reg-bio-3, A1/A7), the polycistronic transcription unit.** The namesake construct: ONE promoter drives N cistrons on ONE polycistronic mRNA. A call to ANY cistron is a transcription attempt of the WHOLE unit: edges targeting the UNIT gate every member first (induction acts on the unit's promoter, one edge opens or blocks all cistrons), then the cistron's own edges apply as usual. The unit pass short-circuits: its message wins over per-cistron messages ("grn gate: 'lacZ' call suppressed (operon 'lac': …)"). An `attenuates` edge targeting the unit is the trpL leader: when it fires, NO cistron is transcribed (the whole polycistronic mRNA is lost). **Per-cistron `rbs r`** is the translation-efficiency multiplier (Shine-Dalgarno strength, clamped 0..=1, default 1.0, distinct from edge `strength`, a binding weight): every `translates` edge sourced at a cistron has its rate scaled by the cistron's rbs, so the lacZYA stoichiometric gradient lives in the two-tier protein nodes (tests/operon_unit.op proves pz:py:pa = 1.0:0.6:0.3). **Transcriptional polarity** (reg-bio-3, re-weighted loop-9): an upstream cistron that is target-less RISC-degraded or methylated-past-threshold derates transcriptional read-through, every downstream cistron's protein integration is multiplied by the upstream members' expected factor (a fully-captured or methylated upstream member contributes the full `polarity`; a partially-captured one contributes proportionally, "blocks" would be the old binary rule). **Rho-dependent termination (loop-10, F-7, SHIPPED, opt-in):** when `.cell rho.termination = true`, the pinned funnel extends to RISC → toggle → GRN → methylation → riboswitch → promoter → RHO. After the promoter gate, a called cistron scans its upstream members in transcriptional order: an upstream cistron whose translation fails on THIS attempt, methylated-past-threshold (deterministically naked) or a target-less RISC silence whose per-call capture draw fires, is NAKED RNA: Rho loads, chases, and terminates the rest of the transcript for THIS call with probability `1 − (1−catch)^d` (`.cell rho.catch`, clamp 0..1, default 0.5; d = the naked runway in cistrons between the failure and the reader, the FURTHER downstream the reader, the MORE time Rho has had to catch up, so the probability GROWS with distance; per-cistron compounding by repeated multiply, never powf). The terminated call returns null with a `rho terminated: transcript lost at '<g>'` note and counts NOTHING (no counters, no transcript, no queue, a terminated call is not expression). The first naked-AND-unshielded member decides; `catch = 0.0` is read-through-certain, `catch = 1.0` termination-certain. **Composition rule (R10 W2):** under Rho ON the D1 polarity factor is IDENTITY, D2 resolves the upstream failure per call (terminate or read-through), so a SURVIVING transcript is whole and translates at full rate; the D1 expected-value derate applies only when the Rho layer is off (it is the expectation-form stand-in for the same loss). (Not modeled: Rho loading kinetics, RNAP velocity, rut-site sequence strength, antitermination, tmRNA/SsrA rescue, the per-cistron threshold-draw abstraction is the model.) **Ribosome-queue coupling shield (loop-10, F-8):** every successful unit call grows EVERY member's queue by its rbs (the Shine–Dalgarno initiation propensity, one polycistronic transcript loads every cistron's ribosomes), capped at `.cell ribosome.queue_cap` (default 1.0); every integration tick drains each queue by `.cell ribosome.drain` (default 0.5); a queue ≥ `.cell rho.queue_floor` (default 0.5) occludes the rut sites, Rho cannot fire behind a translated cistron (the transcription–translation coupling trait; captured calls grow nothing, their transcript never existed). Honesty note (R10-M4/M6): the register is an INITIATION-FLUX abstraction, not elongation-ribosome pile-up, occupancy shields the failure POINT, not the naked runway past it; both boundaries are pinned in tests/granted/rho_queue_shield.op. The queue register and resolved knobs ride the snapshot (§13, worker cells fold the parent's termination math exactly). ENTROPY: flag off = zero draws (bit-identical legacy); flag on draws only where 0 < p < 1 (member order) and where 0 < q < 1 (one catch-up draw), the C9 p∈{0,1} no-draw discipline. The granted-lane proofs (tests/granted/rho_*.op under explicit operator cells) pin the deterministic extremes, the exact seeded distance-decay counts, the shield's ≥-floor boundary, drain re-exposure, and worker parity; harness.py's granted targets pin the byte-exact ENTROPY-STREAM parity on both cores; rt_p14e holds the 1500-cistron load shape contained. Methylation-past-threshold contributes `polarity` outright; a target-less RISC silence with per-call capture `p = 1 − Π(1−s_i)^sites_i` contributes `surv + (1−surv)·polarity` with `surv = Π(1−s_i)^sites_i`, a 1%-capture silence now derates by 1%, not by the full `0.5^1` the old binary-existence rule imposed (`polarity` default 0.5, `.cell operon.polarity`; silent at the note layer, observable through levels; no randomness consumed, this is the expected value). Cistron ORDER is load-bearing: it determines the RBS gradient and polarity exposure. Membership is by name (cistrons may be defined after the unit); redefinition replaces (last wins); a cistron owned by another unit is ignored with a note; a unit without targeting edges is inert (declarative). Each call that passes every gate makes ONE transcript of the unit (`fingerprint().transcripts`, §14, a suppressed call is not expression). Worker cells inherit the unit registry (§13). (Term audit, 13a: documented as PROKARYOTE-specific, eukaryotes lack polycistronic transcription; the construct is not "call these genes together" sugar: the unit-level transcript counter, per-cistron gradient, and polarity are the biology.)
- **`passage(n)` (reg-bio-3, B2/B6), cell divisions and epigenetic inheritance.** Real epigenetic marks are maintained across replication only by maintenance machinery (DNMT1-style copying); without it, marks dilute ~50% per generation, dilution is the null model. `passage(n)` advances the cell n divisions: every methylation level is multiplied by `.cell methyl.maintenance` (default **0.5** = pure dilution; 1.0 = perfect maintenance, marks stable; 0.0 = instant loss) with **half-down rounding** on the 0..=3 lattice, a diluted mark never reads as MORE repressed (3 → 1 → 0 at 0.5; 1.5 rounds down to 1). `generation` counts divisions (saturating u64; `fingerprint().generation`, §14), spawn is a thread, not a division, and never touches it. `n` is clamped to 1,000,000 with a note (a culture that old is not a useful model). tests/passage_dilution.op proves the dilution ladder and the generation counter.
- **`m6a_write(g, n?)` / `m6a_erase(g, n?)` (reg-bio-3, B3), quantitative m6A site density.** The @m6a mark is now a LEVEL 0..=3 per gene (prokaryotic Dam-style DNA-methylation analogy per the term audit above, no eukaryotic mRNA-reader drift): every executed `@m6a`-marked definition deepens the level (+1, capped 3), and dispatch/redefinition resistance requires **level ≥ 1**, the legacy bool is exactly the {0,1} sub-lattice, so mark/unmark programs are bit-identical while the builtins make the mark quantitative: write adds writer-complex dose (clamped 3), erase removes eraser dose, and an ERASED mark truly releases the redefinition (the old bool could not). `.cell m6a.decay f` erases site density on every decay-clock tick (`level × (1−f)`, half-down), higher density persists longer. **Standalone cadence (loop-9):** with no GRN decay clock configured, `m6a.decay` runs on its own cadence, every `m6a.decay_calls` calls (default 1), instead of being a silent no-op behind the GRN tick's early returns. Levels live on the regulation snapshot (workers inherit).
- **`splice_shift(root, variant)` (loop-9, F-4), runtime splicing-factor regulation.** Splicing factors (SR proteins, hnRNPs) CHANGE which splice site wins at runtime. The shift slots into variant selection between the operator pins (`.cell variant.<root>` → `cli.variant`) and the @m6a mark: a bound factor beats a basal inclusion bias, the operator still has the final word. The builtin rebinds the splice root TRANS, up the environment chain, so every future transcript of the root uses the shifted variant (in-flight calls finish on their resolved definition, no mid-call body swap, ever); returns the now-active variant name; soft-fails (rung-4 note + Null/current name) on unknown root/variant; zero RNG; rides the snapshot (workers freeze the shift at spawn). tests/splice_shift.op.
- **`@riboswitch ligand off|on threshold t` (loop-9, F-5), the CIS riboswitch.** The aptamer lives on the transcript it controls: the metabolite pool is cell-wide (`ligand_level`), the SENSOR is per-gene. `off` class (TPP/purine/SAM): ligand bound (level ≥ t) → terminator hairpin → transcription OFF. `on` class (adenine/glycine activators): unbound → RBS sequestered → OFF. Gate order extends the pinned funnel to RISC → toggle → GRN → methylation → riboswitch → promoter → RHO (loop-10, opt-in) (DNA-level chromatin first, then the RNA-level element, then coupling-dependent termination) on both call paths; a terminated transcript is not expression (null before the counters). `@acetylate` does NOT immunize, the RNA hairpin is downstream of chromatin. The trans ligand EDGE remains the protein-free metabolite-sensor analog (the old "riboswitch-style" label for it was wrong, the A2 agent's finding). tests/riboswitch_cis.op.
- **m6A READER fate (loop-9, F-6).** The mark is Dam-style resistance at density 1; at density ≥ 2 (`.cell m6a.reader.min_level`, default 2) eukaryotic READER fate engages on `translates` edges: YTHDF2-like decay routing multiplies the production term by `1 − m6a.reader.decay` (default 0.25) and YTHDF1/3-like attenuation adds `m6a.reader.translation` (default 0.10) to the protein decay, normative order `rate × rbs × polarity × (1−yd2)`. The {0,1} legacy lattice is bit-identical; resolved knobs ride the snapshot so workers fold the parent's math. tests/m6a_readers.op.
- **`autoinducer x;` + `secrete` / `quorum` / `quench` / `quorum_state` (loop-9, C8), quorum sensing: the population layer.** Real bacteria share diffusible autoinducers (Vibrio fischeri LuxI → AHL; LuxR·AHL activates the lux operon at a density threshold; AiiA quenches; S. aureus agr, E. coli AI-2). The signal is EXTRACELLULAR and SHARED, my secretion raises YOUR activation, so the medium is process-global, not cell state: it is deliberately NOT part of the spawn snapshot (workers inherit frozen cytoplasm but a LIVE medium, handed off explicitly like the fuel pool). The pool stores INTEGER molecule counts (u64, saturating cap 1e9): addition is exact, associative and commutative, so concurrent worker secretions COMMUTE, thread interleaving cannot change the pool (the cross-thread answer to the burst_total float-sum lesson). `secrete("ahl", n)` charges (Int exact; Float floors, never rounds; negative/non-finite clamp to 0 with a note; species auto-registers, cap 64); `quorum("ahl")` reads the level `molecules/1e9` (ONE division, bit-identical both cores); `quorum("ahl", t)` compares `>= t`; `quench("ahl", f?)` degrades `floor(m × (1−f))` (AiiA lactonase; no f destroys all); `quorum_state()` is sorted-key telemetry. A species named as an EDGE SOURCE is the LuxR·AHL gate, full AND/OR/sum/occupy/Hill composition for free (resolved after explicit grn levels and ligand pools; `grn_set("ahl", v)` overrides the medium; `grn_fire("ahl")` refuses with a note, the level lives in the medium). Dilution: `passage(n)` multiplies every species by `.cell quorum.dilution` per division (default 0.5, binary-exact halving; 1.0 chemostat; 0.0 full exchange). Contract: cross-cell reads are deterministic once every secreting worker is JOINED (join-before-read; reads over unjoined workers are outside the contract). No capability gate, the medium dies with the process; there is no persistence and no path out of the sandbox (a future file-backed culture would be default-deny `allow.signal`). tests/quorum_basic.op (pool semantics), tests/quorum_lux.op (the LuxI positive-feedback all-or-none circuit, the synthase sits IN the operon it activates), tests/quorum_population.op (cross-cell commutation + dilution), tests/quorum_gate.op (AND/sum/resolution-order composition); redteam rt_p15a (species flood + saturation + hot loop) / rt_p15b (the unjoined-read class).
- **`@copies n` (reg-bio-3, C10), gene dosage.** Copy-number variation: `@copies n` (clamped 1..=64 with a note) amplifies the CONCENTRATION the gene feeds its GRN edges, every regulation read of the gene's level sees `level × copies`, saturating on the 0..1 lattice like real transcript dose under titration. Copies change transcript AMOUNT, never the call's return value (a call is a transcription event; its return is the per-transcript product) and never the raw `grn_get` level (the dose is a read-side effect). tests/copies_dose.op proves 0.3×2 opens a 0.5-threshold gate and the return contract.
- **Determinism hardening (reg-bio-2, D9):** `grn_state()`/`grn_fire()`/`fingerprint().calls` emit in SORTED key order (HashMap iteration order varied per process, a proof-frame and oracle-parity hazard), `burst_total` accumulates in sorted order (float addition is not associative), and the variance uses explicit multiply (never `powi`). Worker cells derive their RNG stream from the task id (`DEFAULT_SEED ^ id·GOLDEN`), the old shared default seed synchronized promoter bursts across cells (perfect correlation, the exact OPPOSITE of extrinsic noise, 12-c C3); sequence-generator cells still share the default stream (loop-8).
- **Level hygiene (reg-bio-2, D2c/D7):** edge `strength` clamps to 0..=1 at parse with a note (a binding weight is not an amplifier), and fire-phase influence clamps at 1.0, a level is a concentration fraction; legacy programs (strength ≤ 1) are bit-identical. `strength > 1` previously pushed levels past saturation, breaking the `grn_set` clamp invariant.

- **Gate order (reg-r4, pinned; reg-bio extends the tail):** for every call, named, value-bound (higher-order), RISC-redirected, sequence-creation, and worker-cell, the gates apply in ONE order: **RISC redirect first** (silencing rewrites the callee, so it wins), then the **toggle gate**, then the **GRN veto**, then the **methylation gate**, then the **promoter gate** (the burst draw is the promoter's own stochastic dynamics, downstream of every trans/epigenetic gate). Value-bound gene calls pass the toggle gate like named calls (an unqualified "the pair gates calls"); a ring node named by an edge source contributes its normalized oscillation level only when no explicit `grn_set`/`grn_fire` level exists (explicit levels win).

## 12. Frames, proofs, overlapping reading frames

- `frame proof { assert(...); ... }`, the **test reading frame** of the file. Skipped by `operon run`; executed by `operon test`. The same file encodes program + tests (two reading frames over one sequence).
- `frame name { ... }`, named frames (metadata/optional scenes); runnable via `operon run --frame name`.
- `operon test [paths...]`, default paths: `tests/` recursively. For each file: run its proof frames; a proof failure (Stress burned) is recorded; the suite continues (Total Grammar). A proof must **run to completion**, an early `return`/`break` inside a proof fails it ("exited early"), and a proof that exercises **zero assertions** fails it ("vacuous proof"). Exit code 1 if any failure. Report: files, proofs run, passed, failed, assertions exercised, wobble notes count. The current suite is all green on both implementations, files/proofs/assertions, the granted-lane cell count, and the differential-harness target split are GENERATED, never hand-typed: see docs/STATS.md (regenerate with `python3 scripts/gen_doc_stats.py`; `scripts/check_docs_sync.py` fails CI on any hand-typed drift).

## 13. Concurrency

- `spawn(f, args?)`, starts a real OS thread running gene `f`; returns task id (Int). `join(id)` waits and returns the result (second join → Null + note). Arguments and results cross by serialization (named genes, lambdas, and phenotype instances cross; a running sequence object does not).
- **Cooperative cancellation (W18).** `cancel(id)` asks a live task to stop: it sets the task's cancel flag and the task observes it at its next fuel tick boundary, raising the catchable stress `[cancelled] "task cancelled"`. Nothing is preempted and no data is touched; a task that already finished answers `false` with a note, and so does an unknown id (soft tier). `cancel(0)`/`cancel(negative)` are contained the same way. The observed boundary is exactly the fuel tick boundary, with two deliberate exceptions so a worker can act on its own flag: the entry tick of a `stress` construct does not raise (the handler gets its window), and while a rescue handler runs the boundary raise is suppressed (the step budget still applies, so a handler cannot spin forever). `cancelled()` polls the live flag chain (true iff this task or any ancestor was asked to stop); on the host it is always false. `task_state(id)` reads the lifecycle phase without joining: `running` | `done` | `cancelled` for a live task, the same answer from the post-join tombstone, and null + note for an unknown id (`task_state(0)` is `done`, the inline-closure task). Cancellation is inherited: a worker's chain carries its own flag plus every ancestor flag, so cancelling a cell stops its whole descent at tick boundaries. A task cancelled before its gene ever reached a tick dies with the `[cancelled]` stress; a worker inside a `stress { ... } rescue (e) { ... }` window can rescue it and decide its own exit (that completion counts as `done`). `join` of a cancelled task returns the standard stress map `{kind: "cancelled", message: "task cancelled"}`. Sequential-oracle note: the oracle runs worker bodies inline at spawn time, so the differential corpus pins only the ordering-free shapes (tests/differential/cancel.op); the timing shapes are Rust-lane evidence under tests/timing/ plus the rt_p20a storm.
- Thread panics are impossible by construction: any stress inside the thread is returned as a Stress Map value.
- Memory model note (honesty): values are reference-counted; tasks communicate by args/results, not shared mutable state. Data races on shared globals are prevented by design (closures capture is by value at spawn time for non-local references).
- **Worker cells inherit regulation state (reg-r1; reg-r4 inventory; reg-bio-3 extends).** A spawned task or sequence cell starts with a copy of the parent's GRN edges + levels, silences (stoichiometric RISC incl. escape bookkeeping), polycistronic operon units (membership, order, rbs, transcript counters), m6A levels, generation counter, gene-dosage registry, methylation counters + threshold, toggle pairs, enhance marks + enhancer dose (reg-bio), the telegraph promoter states + burst counters (reg-bio), **and the repressilator ring (node names + the raw ODE levels frozen at the spawn tick + the ring's kinetic parameters, the cell does not live-tick)**, frozen at spawn time. Worker calls dispatch through the same funnel as the host, so a toggle-repressed allele, a silenced (level ≥ threshold) gene, a GRN-vetoed call, or a burst-off promoter returns null inside the cell exactly as it does outside, regulation is part of the cell, not a host-side illusion. Later parent-side regulation changes do NOT propagate to already-running cells (snapshot semantics). Crossing (§11a): the inherited regulation state is bio-layer state riding the core snapshot membrane (§19d); the membrane rules themselves are unchanged by it.
- **Task groups (W15).** `wait_all(ids)` joins every id in input order and returns the results position-aligned with the input (join semantics per slot: already-joined or unknown ids contribute null + a note, so alignment never shifts). A failing child contributes its stress map like any join; the group call itself never raises. `wait_any(ids, ms?)` returns the id of the first task in the list whose worker has finished, in wall-clock completion order (ties resolve by scan order), or null + note when the timeout expires first (default 30 s, ceiling 300 s). Completion ORDER is inherently timing-dependent: the sequential oracle answers the first listed id, so the differential corpus pins only the ordering-free shapes (tests/differential/task_groups.op) and the ordering shape is Rust-lane evidence (tests/timing/wait_any.op). Mutexes/atomics remain open on the W15 board entry; channels landed below (W015).
- **Channels + select (W015).** `channel()` creates an unbounded FIFO channel, empty at creation, and returns a channel handle (type name `channel`, repr `<channel>` address-free, truthy, identity-compared like the other behavior handles: a copied handle IS the same buffer, two buffers never compare equal). `send(ch, v)` appends and returns null. `recv(ch)` returns the OLDEST value; on an empty-and-open channel it blocks; on a closed-and-empty channel it returns null (null is the only closed signal, a sent null is indistinguishable from closed-empty by design; programs that need the distinction use non-null sentinels or select). `close(ch)` marks the channel closed and returns null; close is idempotent (second close: null + soft note; Go panics here, we contain). `select(ch1, ch2, ...)` is a BUILTIN, not syntax (the `select { recv a => ... }` grammar stays frozen per W036): it polls its argument channels strictly in declaration order and returns the 0-based int index of the first one holding a ready value (a non-empty buffer; a closed channel with buffered values is still ready and receivable). Fairness is by declaration order, documented, never random: the leftmost ready channel always wins, and with several values buffered on one channel select only reports the channel. If nothing is ready and every channel is closed, select returns -1; an empty argument list is the vacuous case and answers -1 with a note. If nothing is ready and at least one channel is open, select blocks until one becomes ready.
  - **The wire membrane.** Every payload that enters a channel crosses the SendValue serialization the spawn boundary uses, EVEN on the same-thread buffered path (the queue stores the wire form, so the two cores agree op-for-op and the membrane holds by construction, SPEC §19d): map keys stringify through display (int key 1 is received as "1"), nested behavior handles (genes, sequences, channels) degrade to null, nested phenotype instances arrive as maps carrying the hidden `#phenotype` key, variants ride with their payload. A TOP-LEVEL behavior-handle payload (gene, sequence, phenotype instance, channel) is refused AT SEND TIME with the catchable `membrane` stress (`"send() refuses a phenotype payload; channels carry data, not handles"`), a deliberate tightening of the spawn wire where instances cross as maps: a thread that needs to pass behavior passes the gene name (a string), not the handle. A payload nested deeper than the SendValue cap (100k) fails the send with catchable `overflow` before anything is queued (same discipline as the spawn pre-flight), and every append is charged to the aggregate allocation ceiling exactly like `push`, so a send storm is a caught overflow, never an OOM.
  - **The stress families.** send/recv/close on a non-channel, and select over any non-channel argument, raise the standard type stress `unfolded`. `send` on a closed channel raises the dedicated catchable family `closed_channel` (`"send on a closed channel"`), never a crash and never silent loss; check order is membrane, depth, allocation, closed, so a handle payload on a closed channel reports `membrane`. Everything here is catchable by `stress { } rescue (e) { }` and by `?!`-style propagation boundaries.
  - **Threads and fuel.** Channel handles cross `spawn` boundaries LIVE (as a spawn argument, a captured global, or a closure capture): the worker re-binds the same shared buffer, so a host can send and a worker recv across a real OS thread. A channel nested INSIDE a container that crosses spawn degrades to null like any handle (the wire rule above), and a gene that RETURNS a channel across the join boundary degrades to null with the same rule. Blocking is fuel-accounted (wall time is fuel, the `sleep` shape, SPEC §9b): recv waits in 50 ms slices and select re-polls in 10 ms slices, each slice charging ms×1000 steps against the interpreter budget AND the run-wide shared pool, and each wake observes the cancel chain, so `cancel()` unblocks a parked cell within one slice (the blocked recv raises the catchable `cancelled` stress and the task exits, no leaked thread), and a never-fed channel drains the run budget into the catchable `overflow` stress instead of hanging forever. Same-thread buffered use charges nothing extra and is the deterministic path.
  - **Oracle parity.** The sequential oracle mirrors every builtin op-for-op on the buffered path (list FIFO + closed flag + the same wire transform at send time, `channel_wire`), so differential programs are byte-identical. The oracle is single-threaded BY DESIGN and never blocks: a program that would block (recv on empty-and-open, select with nothing ready) raises the oracle-only `blocked` stress; the differential corpus never writes one (buffered sends precede recvs/selects), and the blocking shapes are Rust-lane evidence under tests/redteam/rt_p21a_channels.op. Pinned by tests/channels.op + tests/differential/channels.op (byte-identical) plus the redteam payload.
- **Structured concurrency: `scope { ... }` (W17).** Tasks spawned inside a scope block register on the innermost active scope and are joined at block exit, in spawn order, with results discarded, on EVERY flow path: normal fall-through, `return`/`break`/`continue` crossing the block, and stress. When the block unwinds by stress, cancel-on-error (default on; `.cell scope.cancel_on_error = off` to disable) asks each registered task to stop (§13 cancellation) BEFORE the reap; the original stress then propagates after the reap, so the caller's rescue sees the kind and message it would have seen without the scope. A child's own failure is contained as always (its stress map is discarded by the reap, the tombstone says `done`); scope exit never raises because a child failed. Joining a child manually inside the scope still works; the reap's re-join of the same id is contained (null + note). Nested scopes register independently. Sequence cells (§7b) are NOT scope-tracked (they are pulled to exhaustion by their consumer, not joined). Pinned by `tests/scope.op` + `tests/differential/scope.op` (byte-identical) and the Rust-only timing shapes in `tests/timing/scope_timing.op` (cancel-on-error tombstones, blocking reap).
- Sequences (§7b) run on the same worker-cell substrate: each sequence body is a worker thread pulling through a rendezvous channel.

## 14. Telemetry, the single-cell layer

- `fingerprint()` returns Map:
  - `calls`, Map gene → call count (phenotype methods count as `Name.method`).
  - `mature`, genes called at least once.
  - `nascent`, genes defined but never called.
  - `maturation`, mature / total defined genes (the transcript-maturation share).
  - `burst`, the aggregate **burst index**: mean over genes of (variance / mean) of per-gene call counts across complete 20-call bins of the run's call clock.
  - `burst_by_gene`, the per-gene burst indices (0 for a gene with ≤ 1 call or one bin).
  - `bursts`, Map gene → promoter **burst-off count** (reg-bio): calls suppressed by the telegraph promoter layer. Empty unless stochastic expression is on; with it on, `burst` measures promoter-driven variance in real expression rather than call patterns alone.
  - `transcripts`, Map unit → polycistronic transcript count (reg-bio-3, A1/A7): one successful cistron call is one transcript of the unit; suppressed calls count nothing. Empty unless an `operon` unit is declared. Sorted-key emission (D9).
  - `generation`, the division counter (reg-bio-3, B2/B6): advanced by `passage(n)` only (spawn is a thread, not a division).
- Method: the run's global call clock is sliced into windows of 20 gene calls; each gene's count per window is a sample. A gene fired in bursts has a high variance/mean ratio; a constitutively expressed one sits near 0. Only complete bins count (a trailing partial bin is dropped).
- `operon profile f.op`, runs instrumented, prints table: gene, calls, **exclusive self-time µs** (children subtracted), flags (`enhanced active repressed`), then a `mature · nascent · maturation` summary and **enhance candidates**, hot genes (called ≥ 10% as often as the most-called gene) that carry no `enhance` annotation.
- The v2.0 telemetry keys `spliced` / `unspliced` / `velocity` (and the per-gene variance/mean noise key) are retired; `mature`/`nascent`/`maturation` carry the same biology honestly (maturation share, not velocity).
- Crossing (§11a): the `bursts`, `transcripts`, and `generation` keys are bio-layer counters (mechanisms in §11); the call clock, the bins, `mature`/`nascent`/`maturation`, and the profile table are core telemetry that touches bio state only where the corresponding mechanism is in play.
- The accounting gauges are a different lane: `memory()` (arena/intern/alloc counters plus the W013 live-cycle field) is process-cumulative for one binary invocation, not per-run call telemetry; the cycle field's contract lives in §19e.

## 15. Toolchain (Rust binary `operon`)

- **`operon debug f.op --break N` (W08 phase 1)**: a statement-level trap in the tree-walk interpreter with a REPL on break: `c`/`continue` resumes, `s`/`step` breaks after the next statement, `p EXPR` evaluates in the current frame (same notes and stresses as a run), `vars` dumps the frame chain (values display-truncated), `bt` prints the call chain, `q` leaves with exit 0. EOF on stdin resumes to completion, so piped sessions are scriptable and never wedge. Workers are separate interpreters and never break. VM-offset breakpoints (DAP adapter, phase 2) are deferred to the A-track.


- **`--vm` / `--interp` (W09 A2/A6)**: since v2.6.0 the bytecode machine is the DEFAULT engine for gene bodies (src/vm.rs; docs/vm-design.md §2a); `--vm` remains accepted for explicitness and `--interp` opts back to the tree-walking interpreter (the A6 escape hatch, docs/vm-design.md §9). Calls, the gate funnel, capabilities, notes and stress kinds are SHARED code, so output is byte-identical across engines by construction; the differential harness runs every corpus target on BOTH engines against the oracle (default lane + tree-walk lane, both must stay all-green). `operon ir f.op` prints the OIR1 listing, one line per instruction (`op idx | mnemonic | operands | line`), deterministic for the same source (a stability test compiles a mixed program twice and byte-compares the listing, W10). The opcode table, as executed by the machine and documented here:

| opcode | operands | meaning |
|---|---|---|
| `Push` | const idx | push a constant (null/bool/int/float/str) |
| `LoadName` | name idx | read a name (clone-charge + unbound note, the tree-walk read arm) |
| `LoadNameQuiet` | name idx | read without note or charge (compound-assign target read) |
| `StoreName` | name idx | `let` semantics: rebinding note + define |
| `AssignName` | name idx | assignment semantics: const check, set, auto-declare note |
| `Bin` | op | pop two, apply the shared `apply_binop` (exact kinds, messages, line stamps) |
| `JmpIfF` | target | pop one, jump when falsy (same truthy() order) |
| `Jmp` | target | unconditional jump |
| `EvalExpr` | expr idx | BRIDGE: evaluate the arena expression with the tree-walk, push the result |
| `BridgeStmt` | stmt idx | BRIDGE: execute the arena statement with the tree-walk |
| `BridgeStmtInLoop` | stmt, top, end | bridge inside a compiled loop; the bridged flow re-enters the loop |
| `Pop` | | discard one value (expression statements) |
| `Ret` | | return the value on the stack |
| `Brk`/`Cont` | target | break/continue out of the enclosing COMPILED loop (patched) |
| `EnterScope`/`ExitScope` | | enter/leave a block scope (fresh child env) |
| `CallNamed` | name idx, argc | native call (W09): pop argc args, run the SHARED named-call funnel (RISC gate included), push the result |

Bridging is the Total Grammar escape: a construct the compiler does not lower natively either evaluates via the tree-walk or degrades to a bridge, nothing is rejected at compile time. The native/bridge split is invisible to program output by construction (shared code paths), which is what the vm lane of the differential harness pins.


```
operon run f.op    [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires]
                   [--strict] [--quiet] [--fuel N]
                   [--allow-read p] [--allow-write p] [--allow-run prog]
                   [--allow-py module]                                   # substrate-r1
                   [--allow-net host:port] [--allow-env var] [--allow-all]   # §9b
                   [-- --args...]   # dx-r6: everything after `--` is program argv
operon check f.op  [--nmd] [--nmd=purge] [--json]
operon test [paths...]
operon fmt f.op    [--write] [--indent N] [--quotes single|double] [--width N] [--fmt-config f]
                   # canonical formatter; wobble-corrected output parses clean.
                   # --width: soft wrap at parser-safe comma points only (W47-v2;
                   # docs/specs/FMT-CONFIG.md safe-break contract; AST never changes)
operon fix f.op    [--write] [--json]  # legacy-surface migrator (W65): const→let, s::→dot, synonym canonicalization; dry-run default; meaning-preserving (canonical(fix(x)) == canonical(x), pinned corpus-wide)
operon build f.op  --variant v -o out.op
operon profile f.op
operon run f.op --trace-grn trace.jsonl
                   # W095 GRN tick-stream: every engine update point (grn_fire pulse /
                   # decay-clock tick) appends one JSONL frame to the file
                   # {"tick":N,"phase":"fire"|"decay","levels":{sorted map}},
                   # deterministic output (byte-sorted keys, 6-dp levels), 200k-frame
                   # cap, drained by the CLI after the run (success or contained
                   # failure); the interpreter itself performs no I/O. Feeds the
                   # live regulation visualizer (W095/GenomeLab) from the W094
                   # static graph export.
operon crispr f.op (--knockout gene | --matrix) [--json]
                   # knockout: body → return null; then run proofs; report survivors.
                   # matrix: knock out EVERY top-level gene; viability table (ESSENTIAL if a proof fails)
operon bench f.op  [--iters n]
operon version
```

Crossing (§11a): the bio-layer flags and verbs in the block above (`--variant`, `--rna`, `--ires`, `--cell`, `--trace-grn`, `check --nmd`, `operon crispr`, `build --variant`) enter the biology layer (§11); the CLI process rules around them are core.

**`operon-ls`**, language server (stdio LSP; lsp-r1 v2 → W45/W46): `initialize` / `shutdown` / `exit`, full-text document sync (ranged edits are ignored, never mis-applied), `textDocument/publishDiagnostics` (Total Grammar parse notes by rung + `tools check` phantom calls, resolved **CWD-independently**: document-relative → CWD → `std/` → exe-relative `std/` → cargo-manifest `std/`; repair-carrying diagnostics embed `relatedInformation` with the canonical interpretation, W46), `textDocument/hover` with gene/splice signatures **and repair provenance** (hovering a repaired token shows what the parser decided it meant), `textDocument/definition` (genes, sequences, splice roots), `textDocument/references` (W45: word-boundary occurrences, string/comment-aware, declaration included), `textDocument/semanticTokens/full` (W45: fixed six-type legend, keyword/function/variable/string/number/comment), `textDocument/prepareRename` + `textDocument/rename` (W45-v2: same scanner engine as references, every code occurrence becomes one WorkspaceEdit edit, strings/comments/`@marks` excluded, interpolated `{..}` expressions included because they are evaluated; identifier-precise, NOT scope-aware, `operon check` after rename is the honest follow-up; all-or-nothing: invalid/reserved/builtin/same-name/already-taken new names refuse the WHOLE rename as a JSON-RPC error -32001, mirroring W67's `operon rna` discipline), `textDocument/documentSymbol`, `textDocument/completion` (in-file genes with signatures, builtins, keywords, top-level bindings), and `textDocument/formatting` (the canonical `operon fmt` engine). Analysis is cached per document version; `didClose` drops the document and clears diagnostics. `operon-ls --explain FILE` prints the `operon explain` Total Grammar report (W38's second door). Contract version + feature list ride the `operonLsp` handshake block (W62; docs/specs/LSP-VERSIONING.md). Programmer-first by D-008: hovering `boost` shows `gene boost(x)` plus its marks (`@acetylate`, `@methylate`, `@m6a`, `enhance`) and a one-line analogy, gene vocabulary is an intuition aid, never a prerequisite. Zero external dependencies: request JSON is parsed by the language's own `json_parse`. Shipped in every release archive including the Windows zip; editor setup (Neovim / VS Code / Helix) is in README "Connect your editor".

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
| Overlapping reading frames | `frame proof`, tests and code in one sequence |
| Alternative splicing | `splice { variant }` + `--variant` / `.cell` selection |
| Runtime splice regulation (splicing factors) | `splice_shift(root, variant)`, trans-acting shift between the operator pins and the @m6a bias |
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
| Two-tier expression (transcription → translation) | `a translates p rate r decay d;`, protein nodes lag & smooth transcript bursts |
| Small-molecule allostery (lac inducer / trp corepressor) | `ligand x;` + `bind tf inducer|cofactor lg k v;`, affinity modulation, level untouched |
| Riboswitches / protein-free metabolite gating | a ligand named as an edge source reads its pool directly |
| Quorum sensing (LuxI/LuxR AHL, agr, AI-2) | `autoinducer ahl;` + `secrete`/`quorum`/`quench`, process-global integer molecule pool, signal species as an edge source |
| Transcription attenuation (trp leader) | `x attenuates y threshold t;`, RNA-level veto with leader-termination report |
| TF sequestration / decoy binding sites | `decoy d for tf capacity c;`, free-TF titration |
| Enhanceosome synergy (cooperative pooling) | `sum` edge keyword, pooled Hill input, super-additive with hill > 1 |
| Thermodynamic occupancy repression | `occupy` edge keyword, multiplicative Kⁿ/(Kⁿ+Rⁿ) survival |
| Time-based degradation (half-lives) | `decay_clock(n, f)` / `.cell grn.decay_calls`, decay on the call clock |
| Extrinsic noise (cell-to-cell variation) | worker RNG streams derived from task id (decorrelated bursting) |
| m6A modification (quantitative site density) | `@m6a` dispatch priority + `m6a_write`/`m6a_erase` levels 0..=3 + `.cell m6a.decay` (Dam-style analogy, §11 term audit) |
| Polycistronic operons (lacZYA / trpEDCBA) | `operon lac { lacZ rbs 1.0; lacY rbs 0.6; }`, unit-level gate, one transcript, cistron order load-bearing |
| RBS strength gradient (per-cistron translation efficiency) | per-cistron `rbs r` multiplier on `translates` rates, the lacZYA stoichiometric ratio |
| Transcriptional polarity (expected read-through loss) | upstream degraded/methylated cistrons scale downstream yield, per-member expected factor `surv + (1−surv)·polarity` (loop-9 weighted rule; methylation contributes `polarity` outright) |
| Rho-dependent termination + ribosome-queue coupling shield | a failed upstream translation leaves naked RNA; Rho loads and chases, termination probability compounds per cistron of naked runway `1−(1−catch)^d` (loop-10, opt-in); ribosome occupancy (queue ≥ floor) occludes rut sites, translated cistrons shield, drained queues re-expose |
| Epigenetic maintenance vs dilution (DNMT1 / passaging) | `passage(n)` + `.cell methyl.maintenance`, half-down dilution, `generation` counter |
| Gene copy-number variation (dosage) | `@copies n`, read-side dose amplification, saturating on the 0..1 lattice |
| Stoichiometric RISC (dose-dependent knockdown, multi-site) | `silence old -> new strength s sites n;`, per-call capture `1 − Π(1−s)^n` |
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
| Promoter identity (per-gene kon/koff) | `@burst kon koff` + `promoter_telemetry(g)`, on_frac / burst_size separation |
| Single-cell transcriptomics / call-pattern burst index | `fingerprint()` calls / burst / bursts / mature / nascent / maturation telemetry |
| Transcript maturation share (honest replacement for velocity) | `maturation` = mature / total genes |
| RNA interference (antiviral silencing) | capability sandbox: default-deny, Stress `interference` (§9b) |
| CRISPR knockout screens | `operon crispr --knockout` / `--matrix` |

## 17. Version

### §19, Wave-3 hardening and surface expansion (2.1.1)

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
   A worker panic fails the run with a non-zero exit, never a silent success.
4. **Run-wide fuel.** `spawn`/sequences no longer mint fresh budgets: host and
   every worker drain ONE pool (default 500M steps per run). `sleep` charges
   wall-time-proportional fuel (1 step per µs), and entering a `rescue` block
   charges 64 steps, neither a sleep loop nor a rescue retry-spin can outrun
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

9. **Regex (zero-dependency).** `re_match(pattern, s)`, anchored prefix test.
   `re_find(pattern, s, start?)`, leftmost match as
   `{text, start, end, groups}` or null. `re_groups(pattern, s)`, capture
   list or null. `re_replace(pattern, s, repl)`, global literal substitution
   (no `$1` refs, captures come from `re_groups`; empty patterns are
   rejected like `re_match`; the result is capped at 64 MiB and every scan
   is step-charged, dx-r6). Syntax: literals, `.`, classes `[a-z0-9^]`, `\d \w \s \D \W
   \S`, `* + ? {m,n}`, alternation `|`, groups `( )` and `(?: )`, anchors
   `^ $`. The backtracking matcher has a hard 2M-step cap: catastrophic
   patterns raise catchable `overflow` (ReDoS-proof by construction). In
   double-quoted strings quantifier braces must be escaped (`\{2,3\}`) because
   `{..}` is interpolation.
10. **Time (UTC civil calendar).** `unix_time()`, seconds since the epoch.
    `date_parts(ts)`, `{year, month, day, hour, min, sec, wday}` (Sunday=0).
    `date_fmt(ts, fmt)`, `%Y %m %d %H %M %S` expansion. `std/time.op` layers pure duration and calendar arithmetic over these (W33) and adds `time_parse_iso(s)`: parses "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM:SS" into an Int timestamp; fractional seconds are accepted then truncated, a trailing `Z` is accepted, offset suffixes like `+01:00` are rejected (UTC-only module, W89), and any malformed input returns null.
11. **String repetition.** `"ab" * 3` and `3 * "ab"`, Python parity, capped
    by the 512 MiB ceiling.
12. **Join deadline.** `join(id, timeout_ms?)` returns null and notes when the
    worker exceeds the deadline (the task stays joinable).
13. **REPL.** `operon repl`, persistent-expression shell; expressions print
    their value, definitions persist; `:quit` exits. Commands: `:help`,
    `:load f.op` (execute a file into the session), `:proof` (run every
    `frame proof` defined in the session against live state, a proof must
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

This specification is **Operon 2.6.0**. `operon version` prints the implementation banner `Operon 2.6.0 (rust-core, cpp-kernel)`, which matches this document. (sec-r2: the C runtime kernel was deleted, audit A15 proved its intern table was write-only and its raw pointers were the project's one ASan-confirmed memory-safety class; interning now lives in Rust, and the banner no longer claims a c-runtime.)

## 18. Verification status (what the shipped suite proves)

- Proof frames: green on the Rust core and the Python oracle, files/proofs/assertions are generated in docs/STATS.md.
- Differential harness (Rust core vs Python oracle, program-level stdout): every target byte-exact on both the zero-grant and granted-with-cell lanes, match/diverge counts are generated in docs/STATS.md. The oracle also runs the same proof suite (both implementations green, enforced in CI).
- Red-team suite: 0 breaches is the release gate; the payload count is generated in docs/STATS.md (note-cap, fuel-charge, and output-cap containment verified live on the stochastic-expression, reg-bio-3, and Rho-termination surfaces).
- Playground smoke: expression-core subset in the browser, spec-aligned (unbound reads → null + note).

## 19. Memory model, binding, sharing, cycles (W014)

The contract below states the CURRENT truth, verified behaviorally and mirrored by the
differential corpus (`tests/differential/memory_model.op`). One rule of thumb: **scalar
values copy; containers share.**

### 19a. Binding semantics (`let b = a`)

| Value kind | `let b = a` | Mutation through `b` |
|---|---|---|
| int, float, bool, null | value copy (independent) | n/a (immutable values) |
| str | value copy (immutable; the read allocates a fresh string and is mem-charged, sec-r5 F-9) | n/a |
| list | **shared handle** (Rc alias) | visible through `a` (`push(b, 3)` grows `a`) |
| map | **shared handle** (Rc alias, insertion-ordered) | visible through `a` |
| gene / phenotype / sequence | shared handle | method/state effects visible through both names |

There is no implicit copy-on-write and no implicit deep clone. Programs that need an
independent container copy it explicitly (element-wise or via stdlib helpers).

### 19b. Argument passing

Gene arguments follow the same rule: scalars copy, containers alias. A callee that
`push`es a caller's list mutates the caller's list. Callers who need isolation copy before
calling.

### 19c. Closure capture

Lambdas and inner genes capture the DEFINING environment by reference (the env chain is
shared). Mutations of captured variables inside a closure are visible outside it. There is
no by-value capture mode.

### 19d. Thread transfer (spawn), the snapshot membrane

`spawn` hands the worker cell a SNAPSHOT: container mutations inside the worker are never
visible in the parent and vice versa (verified: a worker pushing to a parent list leaves
the parent's length unchanged after `join`). This membrane is what makes `Rc` safe across
worker cells, nothing aliased crosses the boundary.

### 19e. Cycles (W013 decision)

Reference cycles (`let a = []; push(a, a)`) are legal values: equality, repr, and JSON
serialization are cycle-safe (sec-r5 DAG-memoized walks). **Lifetime truth: an `Rc` cycle
lives until interpreter teardown**, scripts and short-lived workers never notice; a
long-lived server building unbounded cycles would leak. Chosen strategy (DECISIONS.md
D-013, shipped in stages):

1. document the model (this section), no silent reclamation, no determinism surprises;
2. SHIPPED (W013): `memory().cycles` is the live-cycle gauge; the accounting contract is
   below;
3. reclamation is opt-in through the weak-reference API (§19f), the sanctioned
   cycle-breaking mechanism; a `break_cycle()` builtin is consciously NOT landed (§19f);
   a tracing GC is REJECTED for v3, it would break the fuel/mem charge determinism
   contract (§9b).

**Live-cycle accounting contract (W013).**
What counts: a container subgraph the engine has PROVEN reachable from itself, where a
container is a list, a map, or a phenotype instance's fields. Variant payloads are walked
through; map KEYS are not walked; gene/env capture cycles are not container edges and
never count. When it increments: at the container mutation that CLOSES the cycle
(push/insert/index write/map insert/field write), detected BEFORE the edge lands by a
bounded walk from the inserted value to the target container. One cycle group counts
once: registering a container whose subgraph already holds a registered member is a
no-op. When it decrements: only on PROOF of death or breakage, re-verified by a bounded
walk on every `memory()` call: a target whose container was reclaimed (last strong
reference gone) or whose cycle was broken by later mutation is pruned; everything else
persists (D-013: a leak is honest accounting, not a surprise).

Determinism: every detection and verification walk is capped at the same 100k-visit
budget in BOTH engines, visits in the same order, and cuts at the same node. On graphs
beyond the budget the cut is conservative in the gauge's direction: an inconclusive
DETECTION walk does not register (detection is best-effort), an inconclusive VERIFICATION
walk keeps the entry counted (death must be PROVEN, persistence is the default). The
budget is native-walk armor, not fuel: interpreter steps stay the charged currency
(§9b), the cap bounds the per-call native work of `memory()` and of container mutations
on adversarial graphs (redteam rt_p22a). The gauge is a PROCESS field: cumulative across
files in one binary invocation, like every `memory()` key, and leaked cycles legitimately
persist into later files' gauges, so proofs pin deltas, never absolutes.

### 19f. Weak references (W013), the sanctioned cycle-breaking mechanism

`weak(v)` hands out a non-owning handle to a container value (list, map, phenotype
instance); `strengthen(w)` returns the SAME value (same allocation: identity and later
mutations are shared, `type()` answers the phenotype kind) while any strong reference is
alive, and `null` (soft note) once the last strong reference is gone. There is no GC:
freeing is immediate at strong-count zero, the null answer fires at the deref, not at
the drop.

- Refusals: `weak()` on anything that is not a list, map, or phenotype instance (scalars,
  behavior handles, channels alike) is a catchable `unfolded` stress; so is `strengthen()`
  on a non-handle. Both messages are byte-identical across the engines (pinned in
  tests/differential/weak_refs.op).
- Membranes: a weak handle NEVER rides the wire. A top-level spawn argument and a channel
  payload are REFUSED with the catchable `membrane` stress (a handle's whole meaning is
  the allocation identity of its target in the creating cell; the snapshot wire could
  only deliver a lie, a null or a handle re-pointed at a copy). A handle nested inside a
  container degrades to null on the wire, the same rule every behavior handle has (§19d).
- Reclamation decision: weak refs are the sanctioned way to make a large object graph
  collectable under D-013: drop the last strong binding, or break the cycle edge yourself
  (pop/del/overwrite on a container you strongly hold), and the next `memory()` call
  PROVES the death and prunes the gauge (§19e). What is consciously NOT landed is a
  `break_cycle()` builtin: asking the engine to sever an edge mid-flight would silently
  mutate a graph that other live aliases can still reach, which breaks identity
  semantics (a value you hold must never change shape behind your back). Cycle breaking
  stays an explicit owner operation on edges the program itself controls.
- Engine truth vs oracle model: the Rust core answers from real strong counts; the Python
  oracle has no refcounter and models aliveness as "a name binding for the target is
  still live on the scope chain". The differential corpus pins only the shapes where the
  two models agree by construction; a target held ONLY by another container is a
  Rust-lane shape the corpus deliberately avoids (recorded divergence, not a bug).
