# ytdl deep benchmark — 2026-10-02 23:59 UTC

CPU: Intel(R) Xeon(R) Processor x2 | kernel 5.10.134-013.15.kangaroo.al8.x86_64 | fixture apps/ytdl/test/mock/fixtures/meta_big.json

| workload | Operon VM | Python 3.12 | Deno 2.9 (TS) | Bash 5 |
|---|---|---|---|---|
| startup | 27.2 ms | 61.6 ms | 36.5 ms | 26.8 ms |
| json | 34.8 ms | 41.9 ms | 19.4 ms | refused |
| table | 163.8 ms | 36.7 ms | 30.6 ms | refused |
| lines | 84.3 ms | 37.7 ms | 18.4 ms | 244.0 ms |
| spawn | 166.8 ms | 67.8 ms | 57.7 ms | 41.0 ms |
| queue1 | 1389.8 ms | 1370.0 ms | 1349.9 ms | 1336.1 ms |
| queue8 | 219.2 ms | 249.4 ms | 223.5 ms | 1362.6 ms |
| json2k | 342.0 ms | 167.0 ms | 98.3 ms | refused |
| table300 | 1602.8 ms | 91.1 ms | 44.0 ms | refused |
| lines200k | 789.7 ms | 121.4 ms | 34.7 ms | 2422.2 ms |
| spawn200 | 1095.7 ms | 277.7 ms | 277.9 ms | 246.7 ms |

| impl | runtime | app source | runtime+source | app LOC |
|---|---|---|---|---|
| Operon VM | 3.6 MB | 23.8 KB | 3.6 MB | 825 |
| Python 3.12 | 29.5 MB | 17.7 KB | 29.5 MB | 495 |
| Deno 2.9 (TS) | 91.2 MB | 21.4 KB | 91.2 MB | 608 |
| Bash 5 | 1.2 MB | 7.6 KB | 1.2 MB | 185 |

checksums (flag-impls must agree per workload; bash is the honest subset):
  json: IDENTICAL
  table: IDENTICAL
  lines: IDENTICAL
  spawn: IDENTICAL
  queue1: IDENTICAL (bash sequential pinned: ok16,k16,c1)
  queue8: IDENTICAL (bash sequential pinned: ok16,k16,c1)
  json2k: IDENTICAL
  table300: IDENTICAL
  lines200k: IDENTICAL
  spawn200: IDENTICAL
