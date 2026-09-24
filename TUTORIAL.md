# Learn Operon in an Hour

**Operon is a programming language for people who already think in functions, variables, and
tests — no domain knowledge required.** Its vocabulary borrows from gene expression as an
intuition aid, nothing more. Every term in the language maps to something you already know:

| Operon term | You already know it as | What it actually does |
|---|---|---|
| `gene f(x) { ... }` | a function | defines callable code |
| `promote(v)` | `print(v)` | prints a value |
| `splice s { variant a {...} }` | overloaded functions | one name, several bodies, one is selected per call |
| `regulate a activates b` | a feature flag with a threshold | gate gene calls behind runtime levels |
| `@methylate` | "mute this function" | calls are silenced after N marks |
| `silence old -> new` | monkey-patching | redirect every future call of `old` to `new` |
| `frame proof { assert(...) }` | a test function | runs under `operon test` |
| `use std/math as m` | `import` | module import |
| `.cell` file | a config file (`Cargo.toml`-ish) | per-directory settings + capability grants |
| `.rna` file | a patch script | text edits applied to a program before it runs |

That's the whole metaphor budget. Everything else is ordinary programming.

---

## 1. Install

**From a release** (Linux / macOS / Windows binaries):

```sh
curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh | sh
```

**From source** (needs Rust):

```sh
git clone https://github.com/WasewaseX/operon-lang-dev
cd operon-lang-dev && cargo build --release
# binary at target/release/operon
```

Check it:

```sh
operon version
```

## 2. Hello, Operon

Save this as `hello.op`:

```operon
gene main() {
    promote("hello, operon!")
}
```

Run it:

```sh
operon run hello.op
```

`gene main()` is the entry point (≈ `fn main` in Rust, `def main` in Python). `promote` prints.
That's it — you're running Operon.

**The one rule that makes Operon different:** the parser *never rejects* your code. Typos and
unknown syntax don't abort the run — they "wobble" into a nearest sensible interpretation and
emit a note explaining what was adjusted. A program always runs *something*, and the toolchain
always tells you what it changed. (More in §8 — including how to make the wobble honest.)

## 3. The ten-minute tour

```operon
gene main() {
    # variables — dynamic, like Python
    let name = "computer science engineer"
    let year = 2026

    # string interpolation — "{expr}" inside double quotes
    promote("hello, {name}, welcome to {year}")

    # lists and maps
    let langs = ["python", "rust", "operon"]
    let stars = {"python": 4, "rust": 5}
    stars["operon"] = 5
    let operon_stars = stars["operon"]
    promote("langs: {langs}")
    promote("operon stars: {operon_stars}")

    # control flow — braces, no parens needed
    let n = 7
    if n > 5 {
        promote("{n} is big")
    } else if n > 0 {
        promote("{n} is small")
    } else {
        promote("{n} is not positive")
    }

    # while loops
    let i = 0
    while i < 3 {
        promote("tick {i}")
        i += 1
    }

    # for-in with collect — a list comprehension, in keyword form
    let doubled = for x in [1, 2, 3, 4] collect x * 2
    promote("doubled: {doubled}")
}
```

Numbers are i64 integers and floats; integer overflow raises a *catchable* stress instead of
wrapping silently (more in §8). Comparisons are `< <= > >= == !=`; string ordering is
lexicographic; mixing types in an ordering comparison is a catchable error, not a crash.

## 4. Genes = functions

```operon
gene fib(n) {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

gene main() {
    promote("fib(20) = {fib(20)}")
}
```

Genes are first-class: store them in variables, pass them to other genes, return them.

**Guard clauses** — a precondition with a fallback body. The first block runs when the guard
holds; the second block runs when it fails. It reads exactly like it behaves:

```operon
gene divide(a, b) guard (b != 0) else {
    return 0        # fallback arm: runs when the expectation FAILS (b == 0)
} {
    return a / b    # body: runs when the expectation HOLDS
}

gene main() {
    promote("10/2 = {divide(10, 2)}")     # 5.0 — `/` is float division; `//` is integer
    promote("10/0 = {divide(10, 0)}")     # 0 — expectation failed, fallback ran
}
```

Read it as: `guard (expectation) else { what-if-false } { what-if-true }`.
Always keep the `else` — the arm order without it will surprise you.

## 5. Splice = one name, several bodies

A `splice` is a family of `variant` bodies for one call name. Each call resolves to one
variant — by explicit choice, by config, or by declaration order:

```operon
splice fetch {
    variant fast {
        return "fast pathway"
    }
    variant safe {
        return "safe pathway"
    }
}

gene main() {
    promote("default: {fetch()}")          # first-declared variant: "fast pathway"
    # selection is configuration, not call-site magic:
    #   operon run f.op --variant safe     (CLI wins)
    #   or a .cell file with:  variant.fetch = safe
    promote("override via .cell or --variant: fetch() \"safe pathway\"")
}
```

Variants may take parameters and defaults:

```operon
splice sized {
    variant v(a, b = 10) {
        return a + b
    }
}

gene main() {
    promote("{sized(5)} and {sized(5, 20)}")    # 15 and 25
}
```

Think "overload resolution you can control at runtime" — useful for strategy selection,
A/B paths, or graceful-degradation wrappers.

## 6. Regulation = runtime feature flags

`regulate` wires genes to *levels* (0.0–1.0). A gene with an incoming activation edge only
runs while its activator's level is at or above the edge threshold; inhibition blocks instead:

```operon
regulate {
    beta activates rollout threshold 0.5
    killswitch inhibits rollout
}

gene beta() { return 1 }
gene killswitch() { return 1 }
gene rollout() { return "shipping!" }

gene main() {
    promote("before: {rollout()}")          # null — level 0.0 is below threshold 0.5
    grn_fire("beta")                        # raise beta's level (+1.0, capped at 1.0)
    promote("after: {rollout()}")           # gated on — 1.0 >= 0.5
}
```

`grn_fire(name)` is the manual lever that raises a source's level. Genes with no incoming
edges are never gated — regulation is opt-in, so plain programs behave plainly.

The companion marks, without mystique:

- **`@methylate`** — a mute button. Marked genes accumulate a silencing counter; calls are
  blocked once the count reaches the threshold (default 3). `@acetylate` un-mutes.
- **`silence old -> new;`** — monkey-patching with an audit trail: every future call to `old`
  runs `new` instead (marked genes with `@acetylate` are immune).
- **`enhance`** — marks a gene as a hot path for the profiler; under regulation it also
  strengthens activation, so it behaves like a priority hint, not decoration.

You can also steer the mute button **at runtime**, without redefining anything —
`methylate()`/`demethylate()` use the exact same graded counter as the marks:

```operon
gene worker() {
    return "doing work"
}

methylate("worker")                    # level 1 — still under the threshold
worker()                               # runs
methylate("worker")
methylate("worker")                    # level 3 — gate closes
worker()                               # null, with a silencing note
demethylate("worker")                  # level 2 — gate reopens
worker()                               # runs again
```

The same idea applies to the regulatory network: `grn_set(node, v)` / `grn_get(node)` read and
write gate levels directly, and `grn_fire(node, decay)` takes an optional decay fraction —
regulation is homeostasis, not a latch, so levels bleed off between pulses unless you keep
re-firing them.

## 7. Proof frames = tests built into the language

A `frame proof` is a block of assertions the toolchain runs:

```operon
gene add(a, b) {
    return a + b
}

frame proof {
    assert(add(1, 2) == 3, "basic addition")
    assert(add(-1, 1) == 0, "cancels out")
}

gene main() {
    promote("add(2, 3) = {add(2, 3)}")
}
```

```sh
operon test f.op          # runs every proof frame in the file
```

Proof discipline is strict on purpose: a proof that never exercises an assertion, or exits
early, counts as FAILED — the suite catches tests that test nothing. The REPL can run proofs
too (§9).

## 8. Total Grammar — errors that inform, not abort

Every program runs *something*. Malformed or unknown input degrades to a nearest-run
interpretation with a "wobble note" explaining the repair:

```operon
gene main() {
    promote("this still runs")
    promote(missing_function(1, 2))     # unknown name → Null + a note, not a crash
    promote("and execution continues")
}
```

When you want the opposite behavior — strictness for CI, pipelines, grading — the toolchain
has you covered:

```sh
operon run f.op --strict     # any wobble/fallback becomes a failure (exit 3)
operon check f.op            # static diagnostics: dead code (NMD), scores, style
```

Hard failures still exist for real faults: `raise` throws a catchable stress
(`raise "overflow", "value too big"`), and `stress ... rescue ...` handles it — Operon's
try/catch, with more honest naming than most.

## 9. The REPL

```sh
operon repl
```

Expressions print their value; definitions persist across lines. Multi-line definitions
accumulate until braces close, then execute:

```text
op> 1 + 2 * 3
7
op> gene double(x) { return x * 2 }
op> double(21)
42
op> frame proof { assert(double(2) == 4, "d") }
op> :proof
  session proof: 1/1 passed (0 failed)
op> :genes
  double
op> :quit
```

Commands: `:help`, `:load f.op`, `:proof` (session or file), `:genes`, `:vars`, `:reset`.

## 10. Modules and configuration

**`use` imports** a module (a `.op` file); `as` names it. The `std/` library ships with the
toolchain and is written in Operon itself — read it, it's meant to be read:

```operon
use std/strings as s
use std/math as m

gene main() {
    promote(s::capital("operon"))            # "Operon"
    promote("mean = {m::mean([1, 2, 3, 4])}")
}
```

Bundled modules: `strings`, `collections`, `iter`, `math`, `seq`, `bio` (sequence utilities —
also useful as plain string/list exercises). Native kernels back the heavy parts:
`distance(a, b)` is a bit-parallel edit distance in C++, `codon(seq)` is a usage scorer —
call them like any function.

**`.cell` files** are per-directory configuration (entry gene, thresholds, capability grants
like `allow.read`). **`.rna` files** are scripted text patches applied before a run — handy
for mechanical edits across many programs. Both are plain text; see SPEC §9 for their keys.

## 11. Where to go next

- **Read the standard library**: `std/*.op` — six small modules, all Operon.
- **Read the proof suite**: `tests/` — 32 files, every language behavior asserted.
- **Run the app**: `apps/genomelab/genomelab.op` — a small DNA-toolbox CLI built entirely in Operon.
- **The spec**: `SPEC.md` — the full contract, organized by feature.
- **The roadmap**: where the language goes next (a bytecode VM for speed, more self-hosting).

The fastest way to learn the philosophy: delete a brace somewhere in `hello.op` and run it.
Watch the wobble note. Operon never leaves you guessing what it did with your mistake.
