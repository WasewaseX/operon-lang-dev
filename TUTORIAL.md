# Learn Operon in an Hour

**Operon is a programming language for people who already think in functions, variables, and
tests, no domain knowledge required.** Its vocabulary borrows from gene expression as an
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
That's it, you're running Operon.

**The one rule that makes Operon different:** the parser *never rejects* your code. Typos and
unknown syntax don't abort the run, they "wobble" into a nearest sensible interpretation and
emit a note explaining what was adjusted. A program always runs *something*, and the toolchain
always tells you what it changed. (More in §8, including how to make the wobble honest.)

## 3. The ten-minute tour

```operon
gene main() {
    # variables, dynamic, like Python
    let name = "computer science engineer"
    let year = 2026

    # string interpolation, "{expr}" inside double quotes
    promote("hello, {name}, welcome to {year}")

    # lists and maps
    let langs = ["python", "rust", "operon"]
    let stars = {"python": 4, "rust": 5}
    stars["operon"] = 5
    let operon_stars = stars["operon"]
    promote("langs: {langs}")
    promote("operon stars: {operon_stars}")

    # control flow, braces, no parens needed
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

    # for-in with collect, a list comprehension, in keyword form
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

**Guard clauses**, a precondition with a fallback body. The first block runs when the guard
holds; the second block runs when it fails. It reads exactly like it behaves:

```operon
gene divide(a, b) guard (b != 0) else {
    return 0        # fallback arm: runs when the expectation FAILS (b == 0)
} {
    return a / b    # body: runs when the expectation HOLDS
}

gene main() {
    promote("10/2 = {divide(10, 2)}")     # 5.0, `/` is float division; `//` is integer
    promote("10/0 = {divide(10, 0)}")     # 0, expectation failed, fallback ran
}
```

Read it as: `guard (expectation) else { what-if-false } { what-if-true }`.
Always keep the `else`, the arm order without it will surprise you.

## 5. Splice = one name, several bodies

A `splice` is a family of `variant` bodies for one call name. Each call resolves to one
variant, by explicit choice, by config, or by declaration order:

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

Think "overload resolution you can control at runtime", useful for strategy selection,
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
    promote("before: {rollout()}")          # null, level 0.0 is below threshold 0.5
    grn_fire("beta")                        # raise beta's level (+1.0, capped at 1.0)
    promote("after: {rollout()}")           # gated on, 1.0 >= 0.5
}
```

`grn_fire(name)` is the manual lever that raises a source's level. Genes with no incoming
edges are never gated, regulation is opt-in, so plain programs behave plainly.

The companion marks, without mystique:

- **`@methylate`**, a mute button. Marked genes accumulate a silencing counter; calls are
  blocked once the count reaches the threshold (default 3). `@acetylate` un-mutes.
- **`silence old -> new;`**, monkey-patching with an audit trail: every future call to `old`
  runs `new` instead (marked genes with `@acetylate` are immune).
- **`enhance`**, marks a gene as a hot path for the profiler; under regulation it also
  strengthens activation, so it behaves like a priority hint, not decoration.

You can also steer the mute button **at runtime**, without redefining anything,
`methylate()`/`demethylate()` use the exact same graded counter as the marks:

```operon
gene worker() {
    return "doing work"
}

methylate("worker")                    # level 1, still under the threshold
worker()                               # runs
methylate("worker")
methylate("worker")                    # level 3, gate closes
worker()                               # null, with a silencing note
demethylate("worker")                  # level 2, gate reopens
worker()                               # runs again
```

The same idea applies to the regulatory network: `grn_set(node, v)` / `grn_get(node)` read and
write gate levels directly, and `grn_fire(node, decay)` takes an optional decay fraction.
Decay is **opt-in** dilution, levels persist forever when unset, so pass a decay per pulse
or set `[grn] decay` in your `.cell` if you want levels that bleed off between pulses.

### Inside an operon: polarity and Rho termination (opt-in)

When several genes share one `operon` unit, a failure upstream is felt downstream, that is
transcriptional polarity. Two layers model it, and both are OFF unless you ask:

- **Polarity (always on inside units, deterministic):** a silenced or methylated upstream
  cistron derates the downstream protein yield by an expected factor (default `0.5` per
  blocked member, tune with `[operon] polarity` in your `.cell`).
- **Rho termination (opt-in):** set `[rho] termination = true` in a `.cell` file and a failed
  upstream cistron becomes *naked RNA*: Rho loads, chases, and with probability
  `1 − (1−catch)^d`, `d` = how many cistrons downstream the reader is, the rest of the
  transcript is lost for that call (the call returns null and counts nothing). Further
  downstream readers give Rho more time, so the probability grows with distance. Healthy
  cistrons push back: every successful unit call fills a per-cistron ribosome queue
  (`[ribosome] queue_cap`, drained by `[ribosome] drain` per integration tick), and a queue
  at or above `[rho] queue_floor` shields the cistron, ribosome occupancy hides the sites
  Rho needs. Full semantics: SPEC §11; runnable proofs: `tests/granted/rho_*.op`.

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
early, counts as FAILED, the suite catches tests that test nothing. The REPL can run proofs
too (§9).

## 8. Total Grammar, errors that inform, not abort

Every program runs *something*. Malformed or unknown input degrades to a nearest-run
interpretation with a "wobble note" explaining the repair:

```operon
gene main() {
    promote("this still runs")
    promote(missing_function(1, 2))     # unknown name → Null + a note, not a crash
    promote("and execution continues")
}
```

When you want the opposite behavior, strictness for CI, pipelines, grading, the toolchain
has you covered:

```sh
operon run f.op --strict     # any wobble/fallback becomes a failure (exit 3)
operon check f.op            # static diagnostics: errors, warnings, style (W41)
```

Hard failures still exist for real faults: `raise` throws a catchable stress
(`raise "overflow", "value too big"`), and `stress ... rescue ...` handles it, Operon's
try/catch, with more honest naming than most.

### 8a. Expected failures are values: Option/Result

Most "errors" aren't failures at all, the key might be absent, the record might not
exist. Operon gives those the mainstream treatment (Rust's `Option`/`Result`, D-014):

```operon
gene main() {
    let maybe = some(42)        # or none()
    let r = ok("shipped")       # or err("why it failed")
    promote(unwrap_or(maybe, 0))    # 42, safe extraction, never stresses
    promote(is_ok(r), is_err(r))    # true false
    promote(r)                      # Ok("shipped"), the tag IS the contract
}
```

The workhorse is **`?!`**, propagation. A missing value returns *from your gene*, so the
happy path stays straight-line:

```operon
gene find(db, k) {
    if (not has(db, k)) {
        return err("missing: " + k)
    }
    return ok(db[k])
}
gene double_of(db, k) {
    let v = find(db, k)?!      # Err returns from double_of; Ok unwraps to v
    return ok(v * 2)
}
```

`double_of(db, "ghost")` IS `err("missing: ghost")`, the caller decides what a missing
key means, right at the boundary. Reserve `unwrap()` (stress kind `unwrap`) for genuine
programmer bugs, and `stress/rescue` for the exceptional tier. Hierarchy: null+note →
Option/Result → Stress → hard exit (SPEC §9).

### 8b. Pattern matching: shape-check and bind in one step (W02)

`match` is the structured way to consume Options/Results and other shaped data, each
`case` is a pattern that checks a shape AND binds its pieces in one step (SPEC §5a):

```operon
gene main() {
    let r = find(db, "k2")
    match r {
        case Ok(v) if v > 100 { promote("huge:", v) }   # guard sees the binding
        case Ok(v)            { promote("ok:", v) }
        case Err(m)           { promote("failed:", m) }
        case _                { promote("not even a Result") }
    }
    # list and map shapes work the same way
    let point = [3, 7]
    match point {
        case [0, y]    { promote("on the y axis at", y) }
        case [x, 0]    { promote("on the x axis at", x) }
        case [x, *rest]{ promote("general:", x, rest) }
    }
}
```

The important contract: a pattern that cannot match (a `Some` pattern against `None`,
the wrong list length, a missing map key) makes that arm miss, matching moves on and
never fails the run. `|` alternatives (`case Ok(n) | Ok(2)`) try in order, first hit
binds; a `case _` arm is the catch-all. Missing arms = silent fall-through, exactly
like Total Grammar everywhere else.

### 8c. `const`: values that refuse to change (W05)

Two bindings, two promises. `let` is the everyday binding; `const` freezes the whole
value, not just the name, for the rest of the run:

```operon
gene main() {
    const SPEED = 299_792              # km/s (underscores are numeric literals)
    const LANES = [1, 2, 3]
    stress {
        push(LANES, 4)                 # frozen, catchable, program continues
    } rescue (e) {
        print("denied:", e.kind)       # denied: frozen
    }
    print(SPEED, LANES)                # 299792 [1, 2, 3]
}
```

Mutation of anything reachable from a `const` burns a catchable Stress `frozen`, the
name itself (`SPEED = 5`), an element (`LANES[0] = 9`), a nested container, `push`/
`pop`/`del`, or the same value through a different variable. Freezing follows the
VALUE: copies escape (numbers always copy), and reads never burn. `let mut x = e`
is accepted as a documentation-only marker (v2.x parses it, fmt tidies it). Programs
that only read their consts run exactly as before, freezing is invisible until
something actually tries to mutate (SPEC §7d).

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
toolchain and is written in Operon itself, read it, it's meant to be read:

```operon
use std/strings as s
use std/math as m

gene main() {
    promote(s.capital("operon"))             # "Operon"
    promote("mean = {m.mean([1, 2, 3, 4])}")
}
```

Bundled modules: `strings`, `collections`, `iter`, `math`, `seq`, `bio` (sequence utilities,
also useful as plain string/list exercises), `env` + `process` (capability-gated reads of
environment variables and subprocesses; a denied grant answers your fallback instead of
crashing). Native kernels back the heavy parts:
`distance(a, b)` is a bit-parallel edit distance in C++, `codon(seq)` is a usage scorer,
call them like any function.

**`.cell` files** are per-directory configuration (entry gene, thresholds, capability grants
like `allow.read`). **`.rna` files** are scripted text patches applied before a run, handy
for mechanical edits across many programs. Both are plain text; see SPEC §9 for their keys.

## 11. Concurrency: spawn, cancel, and scope

Operon tasks are real OS threads. `spawn(gene, args)` starts one and returns a task id;
`join(id)` waits for the result:

```
gene slow(n) {
    run("sleep", ["0.05"])        # needs a run grant; see the capability sandbox
    return n * n
}
let t = spawn(slow, [12])
print(join(t))              # 144
```

Stress inside a worker never crashes anything: it comes back from `join` as a map
`{kind, message}`. Tasks talk by arguments and results, never by shared mutable state,
so there are no data races by construction.

**Cancel a task** with `cancel(id)`. Cancellation is cooperative: the worker notices at
its next fuel tick and dies with a catchable `cancelled` stress. A worker that wants to
clean up runs its body inside `stress { } rescue (e) { }` and decides its own exit when
the cancellation arrives. `task_state(id)` answers `"running"`, `"done"`, or
`"cancelled"` without joining, and `cancelled()` lets a worker (or its descendants, which
inherit the flag) poll the live request. Nothing is preempted, ever.

**Structure your tasks with `scope`.** A `scope { }` block joins everything spawned
inside it when the block exits, in spawn order, on every exit path. If the body raises,
the children are asked to stop first (cancel-on-error), the reap still happens, and then
the original stress propagates to your rescue. No orphaned threads, no bookkeeping:

```
scope {
    let a = spawn(fetch, ["x"])
    let b = spawn(fetch, ["y"])
    # ... either both are reaped here, or a failure cancels and reaps both
}
```

## 12. Where to go next

- **Read the standard library**: `std/*.op`, all Operon, meant to be read.
- **Read the proof suite**: `tests/`, 50 files, every language behavior asserted.
- **Run the app**: `apps/genomelab/genomelab.op`, a small DNA-toolbox CLI built entirely in Operon.
- **The spec**: `SPEC.md`, the full contract, organized by feature.
- **The roadmap**: where the language goes next (a bytecode VM for speed, more self-hosting).

The fastest way to learn the philosophy: delete a brace somewhere in `hello.op` and run it.
Watch the wobble note. Operon never leaves you guessing what it did with your mistake.
