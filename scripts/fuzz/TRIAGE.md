# TRIAGE.md — what to do when fuzz.py finds something

fuzz.py (W051) is a mutation-based black-box runner over the parser/tooling
surface (`check`, `ast --json`, `fmt`). Its ONLY products that matter are
**crash** and **hang** findings. Everything else is noise by design:

| kind      | meaning                                                            |
|-----------|--------------------------------------------------------------------|
| clean     | rc 0, no panic text                                                 |
| contained | rc 1–3 = the language's own diagnostic (uncaught stress / parse /   |
|           | check failure). Redteam seeds land here ON PURPOSE. Not a finding.  |
| crash     | signal, exit 101, any other exit code, or panic/fatal text — BUG    |
| hang      | per-input timeout exceeded — BUG                                    |

**The rule: a finding is a BUG to fix, not a number to brag about.** "10,000
execs, 0 findings" is a property hold; "1 finding" is the fuzzer doing its job.
Never report a contained Stress (rc 1–3) as a crash — that is the Total Grammar
containment working as specified (tests/redteam/ conventions: contained = expected).

## Where findings live

```
fuzz_corpus/
  crash_<seed>_<exec>.op     the (truncation-minimized) input
  hang_<seed>_<exec>.op      same, for hangs
  MANIFEST.jsonl             one JSON line per unique finding:
                             kind, input, bytes, seed, exec, seed_file,
                             mutation_chain, argv, exit_code, stderr_head,
                             minimized
```

## How to reproduce a finding

1. Take `seed`, `seed_file`, `mutation_chain`, `argv` from the manifest line.
2. Either replay the exact input (`operon check fuzz_corpus/crash_*.op` —
   deterministic, the input is saved) or rebuild it:
   `python3 - <<'EOF'` with `scripts/fuzz/fuzz.py`'s `mutate()` and
   `random.Random(seed)` — re-derive the chain if you need the un-minimized
   ancestor.
3. Confirm the classification with the raw exit code / stderr: a finding must
   reproduce on a clean build (`cargo build`), not only on your tree mid-edit.

## How to turn a finding into a redteam payload

1. Minimize further by hand if the truncation sweep left obvious fat.
2. Copy it to `tests/redteam/rt_wNN_<short_name>.op` (`rt_w` = fuzz/wave-derived,
   next free NN; keep the existing `rt_p*` naming style: `rt_w01_deepfmt.op`).
3. Add a header comment stating the EXPECTED containment (e.g. "parser depth
   storm: must exit 2 with a diagnostic, never panic/hang").
4. If the payload needs capabilities (it usually must not — parser payloads run
   default-deny), match the `run_one` case pattern in `scripts/redteam.sh`
   (grant keywords: *grant*, *spawn*, *cell*, ...) and wire it there.
5. The payload becomes permanent: redteam.sh must report `ok` (contained) on it
   from then on.

## Then what

- File the finding against the owning lane (parser/CLI), attach the manifest
  line + minimal input + exact exit code/stderr.
- Fix the BUG; add the payload (steps above); re-run
  `python3 scripts/fuzz/fuzz.py --seed <same seed>` to confirm the class is gone.
- Honest scope note: this fuzzer is mutation-based, not coverage-guided, and
  runs three parse-only targets. In-process libFuzzer targets (S7/S9) remain
  the deeper layer; this script is the zero-setup daily driver.
