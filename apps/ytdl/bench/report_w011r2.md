# ytdl deep benchmark — 2026-10-03 (post W011-r2 VM optimization)

CPU: Intel(R) Xeon(R) Processor x2 | kernel 5.10.134 | fixture apps/ytdl/test/mock/fixtures/meta_big.json
Binary: bin/operon on the W011-r2 call-funnel optimization round (see BENCH.md).
deno not installed on this box at re-run time (deno rows from the 2026-10-02
snapshot above remain valid for that host).

| workload | Operon VM | Python 3.12 | Bash 5 |
|---|---|---|---|
| startup | 28.2 ms | 55.1 ms | 26.3 ms |
| json | 35.9 ms | 43.1 ms | refused |
| table | 141.3 ms | 34.8 ms | refused |
| lines | 69.3 ms | 39.0 ms | 242.0 ms |
| spawn | 168.0 ms | 70.0 ms | 42.2 ms |
| queue1 | 1389.8 ms | 1373.7 ms | 1334.2 ms |
| queue8 | 221.9 ms | 250.0 ms | 1363.3 ms |
| json2k | 342.2 ms | 163.9 ms | refused |
| table300 | 1350.6 ms | 73.0 ms | refused |
| lines200k | 659.6 ms | 122.2 ms | 2412.4 ms |
| spawn200 | 1102.8 ms | 293.7 ms | 279.2 ms |

vs the 2026-10-02 pre-W011-r2 snapshot on the same host class:
table -13.7%, lines -17.8%, table300 -15.7%, lines200k -16.5%;
spawn/queue/startup unchanged (child-process / process-boot bound — the
call-funnel round targets interpreter work, not waits, exactly as designed).

checksums (flag-impls must agree per workload; bash is the honest subset):
  json: IDENTICAL, table: IDENTICAL, lines: IDENTICAL, spawn: IDENTICAL,
  queue1: IDENTICAL (bash sequential pinned: ok16,k16,c1),
  queue8: IDENTICAL (bash sequential pinned: ok16,k16,c1),
  json2k: IDENTICAL, table300: IDENTICAL, lines200k: IDENTICAL,
  spawn200: IDENTICAL
