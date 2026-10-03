# ytdl deep benchmark — 2026-10-03 (v2.8.0 release verification)

CPU: Intel(R) Xeon(R) Processor x2 | kernel 5.10.134 | fixture apps/ytdl/test/mock/fixtures/meta_big.json
Binary: bin/operon at the v2.8.0 release commit (post-W011-r2 + compat-r2 round).
deno not installed on this box (deno rows from the 2026-10-02 snapshot remain valid for that host).

| workload | Operon VM | Python 3.12 | Bash 5 |
|---|---|---|---|
| startup | 27.7 ms | 54.3 ms | 26.7 ms |
| json | 36.4 ms | 41.5 ms | refused |
| table | 140.1 ms | 34.9 ms | refused |
| lines | 75.1 ms | 40.0 ms | 244.0 ms |
| spawn | 169.2 ms | 71.9 ms | 44.3 ms |
| queue1 | 1391.5 ms | 1370.5 ms | 1338.7 ms |
| queue8 | 218.2 ms | 252.5 ms | 1363.8 ms |
| json2k | 350.9 ms | 167.2 ms | refused |
| table300 | 1364.0 ms | 76.2 ms | refused |
| lines200k | 666.4 ms | 122.0 ms | 2390.7 ms |
| spawn200 | 1109.2 ms | 283.2 ms | 268.0 ms |

vs the 2026-10-03 13:40 UTC post-W011-r2 re-run on the same host class:
every workload within noise (±3%); no regression from the compat-r2 round
(ffi test move, test-runner VM-default flip, harness UTF-8 stdout — none of
them touch the run-path hot loops, and the numbers say exactly that).

checksums (flag-impls must agree per workload; bash is the honest subset):
  json: IDENTICAL, table: IDENTICAL, lines: IDENTICAL, spawn: IDENTICAL,
  queue1: IDENTICAL (bash sequential pinned: ok16,k16,c1),
  queue8: IDENTICAL (bash sequential pinned: ok16,k16,c1),
  json2k: IDENTICAL, table300: IDENTICAL, lines200k: IDENTICAL,
  spawn200: IDENTICAL.

functional battery: apps/ytdl/test/run_tests.sh = 35/35 (operon e2e mocks,
cross-language differential byte-identical operon == python, bash honest-refusal
+ get e2e). Machine-readable: results_v280.json.

INFRA FIX recorded here: mockspawn/mocksleep had lost their exec bits at every
sandbox clone (index said 100644; prior sessions chmod'd locally and
core.fileMode=false masked it) — the spawn/queue workloads ERRORED for every
language until git update-index --chmod=+x landed them (the 534cba6 lesson,
reapplied to the two stubs it missed).
