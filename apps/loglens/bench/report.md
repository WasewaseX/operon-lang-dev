# loglens bench report — test app #4 (Lane A)

## Purpose

loglens is the room's deliberate weak-spot workload: per-line string
splitting (P5 surface), map get/set + iteration (P1 surface), comparator
sorting (P2 surface), bulk line streaming, and 100k gene calls per run
(P4 surface). Four engines, one algorithm, byte-identical stdout
(sha256 contract on every run of every workload).

## Results v2 — 2026-10-04, after perf batch W-L1 (char_at/at/len fast
paths + MapStore allocation-free memo), vs v1 baseline (same machine,
same fixtures, same commit methodology)

| workload        | operon v1 | operon v2 | Δ      | python  | node    | rust (Bar C) |
|-----------------|-----------|-----------|--------|---------|---------|--------------|
| startup         | 2.41 ms   | 2.47 ms   | —      | 16.1 ms | 24.7 ms | 0.48 ms      |
| stats_big       | 1013.5    | 935.2     | −7.8%  | 154.5   | 126.8   | 35.5         |
| top_url_big     | 828.1     | 781.6     | −5.6%  | 132.0   | 109.0   | 30.2         |
| top_status_big  | 830.1     | 778.0     | −6.3%  | 130.1   | 106.9   | 28.3         |
| errors_big      | 841.2     | 778.5     | −7.5%  | 135.5   | 108.4   | 33.2         |
| table_big       | 769.8     | 698.1     | −9.3%  | 127.0   | 105.3   | 26.1         |
| stats_small     | 5.63      | 5.47      | −2.8%  | 16.5    | 28.4    | 0.70         |

- checksums (sha256 over stdout): IDENTICAL across all four engines on
  every workload, every run — the win is pure speed, zero semantics.
- Operon still LEADS Python and Node on startup (2.5 vs 16 vs 25 ms).
- The remaining 5.5–6.1x gap vs CPython is attributed by ablation
  (see below), dominated by per-op interpreter dispatch, not by any
  single primitive.

## Ablation (50k-record pipeline, medians of 3, scripts/ablate_loglens.py)

| stage delta (cumulative) | v1 ms | v2 ms | reading |
|---|---|---|---|
| read                     | 12.1  | 12.1  | fs read + startup |
| + split \n + copy        | +40.2 | +41.2 | split allocs |
| + quote split            | +51.2 | +50.4 | split allocs |
| + req/rest splits        | +68.4 | +72.0 | split allocs |
| + is_digits (char loops) | +402.0| +383.6| 100k gene calls + 300k char_at |
| + num() + record build   | +175.4| +171.9| list/Value allocs |
| + 3x project + count_map | +255.3| ~+260 | 450k map lookups |
| + sorts                  | ~0    | ~0    | sorts are free |

Micro-ablation of the digits stage (scripts/ab2_loglens.py): gene call +
loop skeleton ≈ 99 ms; char_at + compares ≈ 88 ms; per-iteration len()
≈ 6 ms (the len ASCII fast path already removed the O(n) count).

## Root-cause ledger (what exactly the 5.5–6x is)

1. Per-op interpreter dispatch (call frames, arg Vecs, Value moves) —
   the dominant term, visible as: 100k is_digits calls ≈ 99 ms ALONE.
   This is P4 territory (call overhead); the W011-s3 carve-out already
   took the bookkeeping slice of it. Next lever, needs its own batch.
2. char_at returning an allocated 1-char String (300k allocs/run) —
   partially mitigated in W-L1 (collect removal); full fix wants a
   VM-level fused "all-chars-are-digits" idiom or char-code builtin —
   language-surface change, gated on an owner verdict.
3. split() piece allocations + charge accounting (~160 ms) — inherent
   to producing real Vec<Value> pieces; semantics-visible surface.
4. MapStore lookups — memo already O(1); W-L1 removed the per-lookup
   String clone; the remaining cost is the 9 method/index dispatches
   per record, i.e. item 1 again.

## Gates at v2 (all green)

differential 3478/3478 x2 lanes, 0 diverge · vm_parity 3557/0
(3 documented load-sensitive) · redteam 109/0 · proofs 141f/2276a ·
cargo 330/0 · clippy 0 · fmt clean · ytdl 35/35 · csvstat e2e 24/24 ·
loglens e2e 37/37 · 4-way checksum contract identical everywhere.
