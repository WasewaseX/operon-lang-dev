# Operon stdlib

The standard library is pure `.op` — it runs on the Rust core like any
program and is exercised by the proof suite (`tests/modules_std.op`,
`tests/stdlib_selfhost.op`, and the per-module files `tests/std_*.op`:
args, bigint, bio, collections, csv, fmt, fs, iter, json, math, random,
set, strings, testing). Import with `use std/<module> as <alias>` and
call with dot access: `s.capital("operon")`. The loader resolves `std/`
exe-relative (installed trees) or from the interpreter's own tree; a
`std/` next to your program wins.

| module | what it gives you |
|---|---|
| `std/args.op` | command-line argument shaping: `args_norm`, `args_positional`, `args_flag`, `args_has`, `args_value`, `args_get`, `args_number`, `args_subcommand` |
| `std/bigint.op` | exact arbitrary-precision integers over sign-magnitude base-10^4 digit lists (pure `.op`, deterministic, byte-identical on both engines; the sanctioned path past the i64 no-wrap overflow contract): `big_from_int`, `big_from_str` (null on malformed input), `big_to_str`, `big_to_int` (null when the value does not fit i64 — never a clamp), `big_is_zero`, `big_neg` (zero canonicalizes, so `-0 == 0`), `big_abs`, `big_cmp` (full signed ordering), `big_add`, `big_sub`, `big_mul`, `big_pow` (int exponent ≥ 0, else null), `big_fact` |
| `std/bio.op` | in-silico sequence utilities: `codon_usage`, `is_palindromic_site`, `melting_point`, `gc_skew` |
| `std/collections.op` | list-shaped data work: `chunk`, `zip`, `group_by`, `take`, `flatten`, `count` |
| `std/csv.op` | delimited data: `csv_escape`, `csv_row`, `csv_parse`, `csv_parse_line`, `csv_records`, `csv_column`, `csv_count_fields` |
| `std/deque.op` | double-ended queue + FIFO queue over plain lists (pure: every function returns a NEW list or a `[value, rest]` pair — arguments are never mutated): `deque_from`, `deque_push_back`, `deque_push_front`, `deque_pop_back`, `deque_pop_front`, `deque_peek_front`, `deque_peek_back`, `queue_new`, `queue_push`, `queue_pop`, `queue_len` |
| `std/fmt.op` | output formatting: `fmt_fixed`, `fmt_thousands`, `fmt_pct`, `fmt_bytes`, `fmt_cell`, `fmt_pad_cell`, `fmt_table`, `fmt_bool` |
| `std/fs.op` | capability-safe file helpers (Stress-returning, never panicking): `fs_read_or`, `fs_read`, `fs_lines_or`, `fs_lines`, `strings_lines`, `fs_write_text`, `fs_write_lines`, `fs_append_line`, `fs_read_json_or`, `fs_write_json`, `fs_list_dir_or`, `fs_size_or` |
| `std/heap.op` | deterministic binary min-heap over plain lists (array layout, pure copies; optional comparator gene with the `xs.sort` convention — `cmp(a,b)` true when a belongs before b; priority-queue idiom: `[priority, value]` pairs + `gene (x, y) => x[0] < y[0]`): `heap_from`, `heap_push`, `heap_pop` (returns `[value, rest]`), `heap_peek`, `heap_sorted`, `heap_len` |
| `std/iter.op` | iterator adapters over lists: `take`, `drain`, `enumerate_pairs`, `zip`, `flatten`, `unique`, `chunk`, `range_step`, `map`, `filter`, `fold`, `scan`, `any`, `all`, `take_while`, `drop_while`, `find_first`, `index_of`, `intersperse`, `sliding`, `sort_by_key`, `reversed`, `concat_all` |
| `std/json.op` | JSON navigation: `json_type`, `json_is_object`, `json_is_array`, `json_get_or`, `json_get`, `json_merge`, `json_pick`, `json_omit`, `json_flatten`, `json_flatten_into`, `json_compact` |
| `std/math.op` | numeric helpers: `clamp`, `lerp`, `mean`, `variance`, `stddev`, `median`, `hill`, `sigmoid`, `digits`, `round`, `round_to`, `sign`, `gcd`, `lcm`, `factorial`, `isqrt`, `divmod`, `wrap` |
| `std/motifs.op` | canonical regulatory-network circuits over the `regulate` layer: `motif_install` wires them, `motif_hill` is the engine's own dose-response transfer function, `motif_autoreg` / `ar` (negative autoregulation; `ar` is the short alias), `fc`/`motif_ffl` (coherent feed-forward persistence filter), `ic`/`motif_pulse` (incoherent feed-forward pulse compression), `tx`/`ty`/`motif_flip` (mutual-repression toggle), `motif_states` (the whole board's levels) |
| `std/path.op` | pure lexical path shaping (NO filesystem access, no capabilities — fs I/O stays in std/fs): `path_base`, `path_dir`, `path_ext`, `path_is_abs`, `path_join`, `path_norm` (lexical; relative `..` kept, absolute-root `..` ignored), `path_split_ext` |
| `std/random.op` | deterministic randomness over the core `random()`/`randomize(seed)` stream (mirrored xorshift64*, byte-identical under a fixed seed): `rand_below`, `rand_int` (inclusive both ends), `rand_pick`, `rand_shuffle` (Fisher-Yates, new list), `rand_weighted` (cumulative weights over the sum — weights MUST be non-negative; negative weights are undefined), `rand_chance` |
| `std/seq.op` | sequence (worker-cell generator) combinators: `map_seq`, `filter_seq`, `take_seq`, `concat_seq`, `fib_seq`, `range_seq` — combinators take the SOURCE as a sequence-gene NAME (transcripts do not cross membranes); transformation genes travel freely |
| `std/serialize.op` | unified serialization front door (W34): `to_json`, `from_json` (Result-shaped — `ok(value)`/`err(message)`, never a silent null), `to_csv_rows`, `from_csv_rows`, `serialize`/`deserialize` (format-generic dispatch), `roundtrip` (to→from→equality check, per-type table incl. the honest non-finite-float gap), `supported_formats`; stage 2 (2026-09-27): phenotype instances ride the canonical spawn wire shape (field map + hidden `"#phenotype"` key, SPEC §10 builtins `is_object`/`object_fields`/`object_from_map`) — instances round-trip to REAL instances (data restore, methods included), unknown class names are err values; the W04 `Serializable.to_map()` trait hook stays spec'd in docs/specs/SERIALIZATION.md as the future custom-projection override |
| `std/set.op` | set algebra over plain lists (a set is a list with unique members; equality is order-independent; every function is pure and returns a NEW list): `set_from`, `set_has`, `set_add`, `set_del`, `set_union`, `set_intersect`, `set_diff`, `set_symdiff`, `set_subset`, `set_eq`, `set_count` |
| `std/strings.op` | everyday string shaping: `words`, `capital`, `pad_left`, `pad_right`, `starts_any`, `pad`, `strip_prefix`, `strip_suffix`, `is_blank`, `chars`, `lines`, `title_case`, `to_snake`, `to_camel`, `to_kebab`, `ellipsis`, `unquote` |
| `std/testing.op` | minimal deterministic test harness (no I/O, nothing raised): `expect_eq`, `expect_true`, `expect_false`, `expect_near` (inclusive float tolerance), `expect_throws` (pass a zero-arg gene; any raise counts), `test_summary` — checks accumulate into the module's own tally, a failed expect is data and returns false |
| `std/time.op` | pure duration & instant arithmetic over the L1d clock builtins (UTC-only contract — no timezone support, W89): `time_add`, `time_diff`, `time_days_between`, `time_midnight`, `time_date_only`, `time_is_leap`, `time_days_in_month`, `dur_hms` ("HH:MM:SS"), `dur_human` ("1d 2h 3m 4s") |

Native kernels back the hot parts and are builtins, not imports:
`distance(a, b)` (bit-parallel Myers edit distance, C++), `codon(seq)`
(codon-usage style score, C++), `json_parse`/`json_str`, and the
capability-gated `re_*` regex family. Every stdlib gene is pure `.op`
over those builtins — the library ships no native code of its own.

### Core builtins: first-class Option/Result (W06, D-014 — SPEC §9)

Expected failures are values, not stress. The tag IS the contract — the
Option family (`some`/`none`) and the Result family (`ok`/`err`) are
distinct even with equal payloads:

| builtin | contract |
|---|---|
| `some(v)` / `none()` | build an Option (type `option`) |
| `ok(v)` / `err(e)` | build a Result (type `result`) |
| `is_some(v)` / `is_none(v)` / `is_ok(v)` / `is_err(v)` | tag predicates |
| `unwrap_or(v, default)` | safe extraction — never stresses; None/Err/plain all yield `default` |
| `unwrap(v)` | unsafe extraction — Some/Ok payload; otherwise Stress kind `unwrap` (the exceptional tier; rescue-catchable) |
| `e?!` (postfix) | propagation — Some/Ok unwrap to the payload; None/Err return FROM the enclosing gene with that variant; plain values pass through; never contained by rescue (SPEC §9) |

JSON view: `{"ok":1}` / `{"err":"x"}` / `{"some":1}` / `null` for None.

Discovery rules for agents and humans:
- module genes are documented by their one-line headers in each file;
- anything the module exports is importable — nothing is hidden;
- new modules MUST land with a row here and proof coverage (SPEC §17).
