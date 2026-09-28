# CRITIQUE LOOP, what is left on the 100 item board

v1.0 · 2026-09-28 · main @ 7b9395d · written by builder-A on the owner directive:
"finish all remaining items, write a critique loop of what is left, deploy five agents,
work in bulk, five by five, never one by one"

This file is the loop. Each pass has two steps. Step one, critique the remaining board
honestly: what is open, why it is still open, what it will really take, and what could go
wrong. Step two, deploy a batch of five agents against five disjoint file domains, integrate
their work, re run the gates, commit, push, and write the next critique. The loop ends when
the board reads 100 of 100 or when an item is honestly closed as deferred with its gate met.

---

## Baseline truth, measured 2026-09-28 on main @ 7b9395d

- proofs: 154 files, 117 proofs, all green (cargo test)
- differential harness: 190 of 190, plus vm lane 184 of 184
- redteam: 104 payloads, 0 breached
- clippy 0, fmt clean, toolchain rust 1.98.1 reinstalled after environment reset 5

Board state by lane, measured against this commit:

- dev-1 (W001 to W033): 16 done, 16 open or partial, 1 deferred (W012 JIT, gate met by the
  design note and the audit ordering)
- dev-2 (W034 to W066): 5 done or verified, 5 partial, 20 open or claimed, 2 deferred
- dev-3 (W067 to W100): 27 done, 3 partial, 4 deferred with design notes landing their gate

The deferred items (W012 JIT, W071 hot reload, W077 C ABI, W080 native FFI, W087 bundle
implementation body) are closed as specced: each has an adopted design note, an honest
sequencing reason, and an owner sign off requirement. This loop does not reopen them. The
one exception is W065, the migrator, which was deferred only because W064 and W047 were
open. Once those two land, W065 comes off the deferred list and gets built.

---

## The critique, item by item

### dev-1 lane (core language)

W001 static types, stage 2. Critique: the soft annotation layer works and caught its own
author three times while writing proofs, which is the best evidence it does something. What
is missing is the quiet part: check time inference so `operon check` reports annotation
mismatches before a run, `List<T>` and `Map<K,V>` sugar at container boundaries, and type
aliases. The hard part is not the sugar, it is keeping the check honest without a full
Hindley Milner pass. The staged plan in the roadmap already says boundary checking only, so
the scope is controlled. Risk: check time duplicates the call funnel logic that the runtime
already has, and the two must agree, so the differential corpus needs programs where check
and runtime disagree on purpose.

W002 match v2 stage 2. Critique: the pattern language is in and battle tested by rt_p17a,
but unreachable arm detection is still absent, so a match where every arm is dead after arm
one passes silently. This is a check side analysis, not a runtime change, which makes it the
cheapest remaining core item. It feeds W042 directly. Risk: false positives on guard arms,
since a guard makes an arm conditionally reachable. The rule has to be: an arm with a guard
is never reported unreachable.

W003 generics stage 2. Critique: stage 1 delivered callable generic std APIs with zero
per-type duplication, which was the honest 80 percent. Stage 2 type parameter declarations
ride W001. Risk: scope creep, because explicit monomorphization was already deliberately
unscheduled. Keep it that way.

W006 Result/Option stage 2. Critique: the language shape is done, the std module still
returns null where a Result would tell the truth. The migration has to sit behind a compat
note because existing programs pattern match on null today. Risk is low, reach is wide:
every std/*.op file touches this. It is mechanical work, not design work.

W008 debugger. Critique: the debug REPL exists and works, but stepping breadth is thin and
there is no DAP adapter, so no editor integration. The DAP adapter is the piece that turns a
demoship feature into a real one. Risk: DAP is a protocol with many messages, and half
measures produce an adapter that hangs VSCode. Scope it to the core set: launch,
setBreakpoints, stackTrace, scopes, variables, next, stepIn, stepOut, continue, evaluate.

W009 bytecode VM. Critique: the OIR1 machine runs and its lane is 184 of 184 against the
same oracle outputs, but the parity campaign is not complete across the full 190 program
corpus, and the fib25 at least 2x bench gate has not been confirmed. The machine either
replaces the tree walk or it stays a lane. This batch decides. Risk: some programs use
features the compiler does not lower yet, and the honest answer is to pin the not-yet list,
not to force parity by cheating the comparator.

W010 disassembler. Critique: stage 1 rides the VM, but there are no stability tests and not
every opcode is documented in SPEC. This is documentation discipline, not engineering. It is
cheap and it should never be the reason the VM lane looks unfinished.

W011 optimization pipeline. Critique: constant folding and jump threading exist behind
--opt 1. Dead code elimination, trivial gene inlining, and specialization remain, plus the
bench rows that prove each pass pays for itself. Critique of myself: shipping passes without
bench rows is how a pipeline becomes a rumor. No pass lands in this loop without a measured
row.

W013 memory cycles. Critique: the decision D-013 and the detection test landed, but
memory() does not report live cycle counts and there are no weak references. The audit
recommended options a plus c: document the leak honestly and provide a weak handle. Risk:
weak refs touch the value representation, which touches everything. Keep the handle opaque.

W015 channels and select. Critique: task groups landed, channels did not. Without
channel/send/recv/close and select, the concurrency story is fork join only. This is the
most user visible remaining core gap. Risk: the SendValue membrane already exists for
spawn, channels must respect it, and select over multiple channels needs a deterministic
fairness story, which for Operon means documented fixed order polling.

W016 async. Critique: the spec sketch says green threads over the VM loop with suspension
at builtin boundaries. That is the right architecture and it is still just prose. This is
the largest remaining item. Honest sequencing: land W015 first, then async rides the same
task infrastructure. Risk: half an async model is worse than none, so the gate is a working
async block with await on channel recv and sleep, integrated with structured scopes, or it
does not land.

W021 registry. Critique: the static git index registry landed, which was the cheap first
version the roadmap called for. A hosted service needs infrastructure decisions only the
owner can make. Stays closed as partial with the gate met.

W025 namespaces. Critique: the :: sugar works. Nested sub module declarations remain. Risk:
the module resolution roots table from W069 must not fork into two behaviors.

W026 typed collections. Critique: set, deque, heap exist and are deterministic. Graph with
typed vertices and the annotation sugar over containers remain. Risk: Graph<T> wants W001
sugar, so sequence it after, or ship the graph untyped first and annotate later.

W027 stdlib breadth. Critique: 22 modules exist, the audit list still names process, env,
logging, terminal, compression, hashing, url, http high level, walk, binary, db stub. That
is eleven modules. Critique of the audit itself: db stub and http high level are scope traps,
a stub that does nothing and a client that cannot be capability honest without the network
grant story settled. The loop builds the real ones (process, env, logging, hashing, walk,
terminal, compression, url, binary) and pins db stub behind the stub name it deserves.

W028 unicode. Critique: stage 1 covers grapheme semantics and a case fold subset. NFC and
NFD normalization, full case folding, and category queries remain. Risk: pulling in a
normalization table by hand is error prone, so the tables must be generated and the
generator committed next to them.

### dev-2 lane (tooling and truth)

W034 serialization. Critique: json and csv exist as separate std modules with different
conventions. The unified model is a small spec plus a round trip pin. Cheap, do it.

W036 core/bio boundary. Critique: the directive froze new biology syntax but the boundary
is a convention, not a written contract with a lint rule that fires when a PR crosses it.
The deliverable is the contract page plus the checker rule. Cheap.

W037 Total Grammar contract. Critique: the rungs exist and the contract page exists, but the
semantic half (nothing that parses may fail without a catchable Stress family name) is not
enforced by a test sweep. The deliverable is the sweep.

W038 explain. Critique: repair data already counts wobbles and fallbacks, so explain is a
presentation layer over data that exists. There is no excuse for this being open. Cheap.

W039 ast dump. Critique: the AST derives Debug, so the subcommand is a day of work including
the stability test. Open purely because it was never claimed. Cheap.

W040 ir dump. Critique: blocked on W009 historically, and the VM now exists, so the dump is
a print of the OIR1 program with a stability test. Cheap once W009 confirms parity.

W041 check rework. Critique: check reports a school grade starting at 100 with deductions,
which no professional tool does. The rework is diagnostics with codes, locations, and an
optional summary line. Medium. Risk: the cookbook gate and docs quote the grade, so the
number migration must land in the same commit.

W042 analysis depth. Critique: phantom calls, wobble, and NMD checks live. The audit asks
for more rules. Critique of the ask: rule lists grow forever. Pin the set: unreachable match
arms (from W002), unused binding, shadowed binding, dead const. Then stop.

W043 wrong arity. Critique: gene defs carry param counts, check does not compare call sites.
This is the highest value cheap item in the whole dev-2 lane, because wrong arity is the
most common beginner error in the wild. Cheap.

W044 to W046 LSP. Critique: five of seven depth items wired. Signature help, rename,
references, and the repair distinction remain. Risk: LSP tests are protocol heavy, so the
harness needs a fixtures approach, one JSON per message exchange.

W047 fmt config. Critique: the formatter has zero options. The deliverable is operon.toml
fmt section with width and indent, honored by fmt and pinned. Cheap. Unblocks W065.

W048 lint. Critique: check mixes correctness and style. Splitting them is a CLI surface
change plus moving the style rules. Medium, mostly plumbing.

W049 test filtering. Critique: operon test takes paths and nothing else. Name filters and
repeat counts are trivial. Cheap.

W050 property testing. Critique: the differential harness is example based. The deliverable
is a shrinkable generator harness over the std API surface, seeded, deterministic, and
wired as an opt in script, not a CI gate on day one.

W051 fuzzing. Critique: the redteam corpus is adversarial but fixed. A real fuzzer mutates.
The deliverable is a coverage guided or at least mutation based runner over the parser with
a time budget and a crash corpus, run manually, findings triaged.

W052 coverage. Critique: nothing exists. The deliverable is source based coverage of the
operon binary over the full test corpus, rendered as a text table plus a tracked baseline.
The existing coverage.sh stub says the intent was always there.

W053 to W058 truth items. Critique: five verify items (version source, keywords, C kernel
claims, stdlib inventory, dependency claim) plus generated numbers. These were the items the
owner first saw as contradictions. They are all cheap and they are first in batch one
because trust items go before feature items.

W059 windows CI. Critique: continue on error true is still the pattern, so the job is
permanently green and permanently useless. Make it blocking, accept that it will go red,
and fix what it finds. Risk: the runner may surface real Windows path bugs, which is the
point.

W060 release smoke. Critique: five targets build, none are proven to run. The deliverable
is a smoke run per target in the release workflow. Cheap but CI only, so it cannot be fully
verified from this environment. Land the workflow, mark the gate as CI verified when the
first release runs.

W061 distribution. Critique: binstall and brew draft landed. The remaining channels are
packaging recipes, which are cheap. Critique of the earlier plan: scoop and winget specs
rot if nothing validates them, so each recipe gets a lint check in CI where possible.

W062 and W063 compatibility policies. Critique: both are writing tasks with real content:
version floors, deprecation windows, and what counts as a breaking change. Cheap, high trust
value.

W064 deprecation. Critique: nothing marks a feature deprecated. The deliverable is the
mark, the warning surface, the removal ladder, and one real deprecated thing to prove it.
Unblocks W065.

W065 migrator. Critique: deferred only because W064 and W047 were open. When they land,
operon fix gains migrations for the deprecations. The earlier use path bug in fix is already
fixed and pinned.

W066 .cell schema. Critique: parse_cell returns a flat map and the schema lives in heads.
The deliverable is the formal schema doc plus a validating parser with honest errors.
Claimed by builder-B on the canonical board, owner arbitration pending. This loop treats it
as buildable by whoever reaches it first, one claimant rule enforced through TASKS.md.

### dev-3 lane (correctness surfaces)

W091 bio semantics separation. Critique: blocked historically on W054, which batch one
resolves. The work is restructuring SPEC so the bio layer is its own section with its own
contract, and pinning which parts of the interpreter are bio only. Medium, mostly honesty.

W095 GenomeLab wiring. Critique: the tick stream ships, the TUI does not read it. The
deliverable is a trace viewer pane in the existing genomelab app.

W096 chrome trace. Critique: json profiler output landed. Chrome trace needs per call spans,
which is an interpreter change, so it rides the semantic lane and then the formatter is
trivial.

W097 memory profiler. Critique: attribution was deferred until the VM existed. It exists
now. The counting allocator sketch in MEM-PROFILER.md gets built, or the honest scope is
reduced and documented.

W098 workers builtin. Critique: the telemetry design note is done and the builtin was left
to the dev-1 lane. It lands with the concurrency batch since it shares the worker pool.

---

## The loop protocol

One batch = five agents, deployed in parallel, each owning a disjoint file domain so no two
agents ever edit the same file in the same window:

- the semantic slot owns src core and the oracle mirror and SPEC language sections
- the CLI slot owns src/tools.rs and src/main.rs
- the LSP slot owns src/ls.rs
- the docs slot owns README and docs, never SPEC.md in the same window as another agent
- the infra slot owns tests, scripts, CI, and packaging

Agents implement, test, and commit nothing themselves. They report the exact files touched.
The integrator (builder-A) reviews, runs the full gate sweep, commits one unit at a time with
natural messages, pushes, and updates the board. After every batch the critique section above
gets its status lines updated and a short verdict paragraph, so the critique loop is
literally a loop, not a memo.

## Batch plan

- batch 1: truth and trust. W054 W055 W056 W057 W058 (docs slot), W039 W049 (CLI), W047
  (CLI fmt config, sequenced after W039 in the same agent), W062 W063 W036 (docs slot 2),
  W091 (SPEC slot), W052 coverage (infra).
- batch 2: W009 parity campaign (semantic), W038 explain W043 arity (CLI), W044 W045 W046
  (LSP), W034 serialization (docs), W050 property harness (infra).
- batch 3: W015 channels and select (semantic), W041 check rework (CLI), W048 lint (CLI
  slot 2, sequenced), W059 windows CI W060 smoke (infra), W066 .cell schema (docs/spec),
  W037 contract sweep (infra 2).
- batch 4: W016 async (semantic), W008 debugger DAP (CLI), W064 deprecation (semantic lite
  via interp warn surface), W061 distro (infra), W035 macros design (docs).
- batch 5: W013 weak refs (semantic), W011 opt passes (semantic 2, vm files), W065 migrator
  (CLI), W051 fuzzing (infra), W053 generated numbers (docs).
- batch 6: W001 W003 types stage 2 (semantic), W002 W042 unreachable arms (CLI check),
  W025 nested modules (semantic 2), W098 workers (semantic 3, small), W095 GenomeLab (infra).
- batch 7: W027 stdlib breadth wave 1 (semantic), W010 opcode docs (docs), W040 ir dump
  (CLI), W096 spans (semantic 2), W057 reverify (docs 2).
- batch 8: W027 wave 2, W026 graph, W028 unicode, W097 memory profiler, final truth sweep.

Batch composition may flex when a gate finds something real, but the shape holds: five at a
time, always.

---

## Pass 1 verdict (pre batch)

The board is not 60 percent done, it is roughly 75 percent done with a heavy tail: the
remaining work is a third large semantic items (async, channels, types stage 2, unicode,
stdlib breadth) and two thirds cheap unclaimed tooling and truth items. The cheap items are
open because nobody claimed them, not because they are hard, which is itself the sharpest
critique of the program so far: the hard things got built and the easy things waited. Batch
one attacks that inversion directly.

---

## Pass 2 verdict (after batch 1, main @ c95c434 + cli_ast pin)

Batch 1 closed 16 items: W036, W039, W044, W045, W046, W047, W049, W052, W053, W054,
W055, W056, W057, W058, W062, W063. Gates after integration: cargo test 131 green
(3 new ast pins, 5 fmt config pins, LSP smoke extended), clippy 0, fmt clean,
differential 190/190 + vm lane 184/184, docs checker exit 0 (v2.2.0, 60 keywords,
22 modules, 229 std funcs, 127 proof files, 101 redteam payloads).

Honest critique of the batch: the four agents that hit deadline errors still left
substantially complete work, which says the briefs were right but the agent time
budget is the scarce resource. The integrator had to finish the LSP capability wiring
was already complete (stale release binary was the real failure), write the ast dump
pin test the agent promised but did not deliver, and apply the W62 policy patch by
hand. Nothing semantic was touched, so the oracle never moved: the right shape for a
truth-and-trust batch.

What the batch exposed: lint.rs already carries arity, unused-gene, unreachable and
shadowing rules (W042/W043 substrate is real), and the LSP agent's explain door
already covers most of W038. The board was more done than the statuses said, which is
the same lesson as pass 1, from the other direction.

Batch 2 changes the protocol: the two semantic units (W015 channels and select, W028
unicode stage 2) work in separate git worktrees so both may touch src/interp.rs
without racing, and the integrator merges. The shared tree hosts the CLI check rework
(W041 plus W043), the SPEC bio separation (W091), and the testing infrastructure pair
(W050 plus W051).
