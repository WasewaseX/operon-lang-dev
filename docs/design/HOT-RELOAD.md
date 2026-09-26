# HOT-RELOAD — design note (W071, deferred: depends on W072, which is LIVE)

Status: design paragraph set. `operon watch` (W072) ships the process-model
half; this note specifies what a reload would have to mean before any
implementation.

## The reload problem in Operon terms

A "reload" is a new Interp over new ASTs. The interesting question is never
the file I/O — it is WHICH state survives. Operon's state families:

1. **Process-external** (fuel pool, capability grants, RNG seeds): a reload
   MUST re-derive these from the CLI + `.cell` — silently carrying old caps
   into new code would be a sandbox escape by iteration.
2. **Module cache** (§8): keyed by resolved path; a reload invalidates entries
   whose mtime moved, transitively (an invalidated module invalidates its
   importers).
3. **Regulation state** (GRN levels, methylation, silences, operon units,
   repressilator ring, telegraph promoters): the v2.2 contract says worker
   cells get SNAPSHOTS and the parent holds live state. A reload is a new
   parent — the honest default is a FULL RESET to the fresh-parse state, with
   an explicit opt-in (`--keep-grn`) that re-fires `grn_fire` pulses recorded
   from the previous run (replay, not transfer — transferring live levels into
   a changed network is undefined behavior we do not want to define).
4. **REPL/session state**: the REPL (T4) already re-executes a session buffer
   against a fresh interpreter — that IS the reload primitive for interactive
   use, and it is the pattern to reuse.

## Shape (when built)

`operon watch --reload app.op` runs the program in-process; on change:
new parse → new Interp → re-derive caps/seed from CLI+cell → replay recorded
GRN pulses if `--keep-grn` → run. No partial AST patching (RNA-V2 is for file
edits, not for live reloads — different tools, different guarantees).

## GenomeLab integration sketch

GenomeLab's REPL-driven workflow would use the same primitive the REPL uses:
`:load` on change with the session buffer replayed. No special app hooks
needed — the reload contract lives in the toolchain, the app stays a program.

## Why deferred

The in-process runner needs fuel/caps isolation guarantees per iteration that
today's fresh-child-per-run model (W072) gives for free. Any in-process reload
must re-prove those guarantees (a leak between iterations is a correctness AND
security bug). Until a level demands reload latency < process spawn (~2 ms
startup, W084 — already excellent), the child-process model is strictly safer
at the same perceived speed.
