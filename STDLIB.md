# Operon stdlib

The standard library is pure `.op`, it runs on the Rust core like any
program and is exercised by the proof suite (`tests/modules_std.op`,
`tests/stdlib_selfhost.op`, and the per-module files `tests/std_*.op`).
The generated per-module inventory (module + function counts) lives in
[docs/STATS.md](docs/STATS.md) and is the countable truth for sizes.
Import with `use std/<module> as <alias>` and
call with dot access: `s.capital("operon")`. The loader resolves `std/`
exe-relative (installed trees) or from the interpreter's own tree; a
`std/` next to your program wins.

| module | what it gives you |
|---|---|
| `std/args.op` | command-line argument shaping: `args_norm`, `args_positional`, `args_flag`, `args_has`, `args_value`, `args_get`, `args_number`, `args_subcommand` |
| `std/bigint.op` | exact arbitrary-precision integers over sign-magnitude base-10^4 digit lists (pure `.op`, deterministic, byte-identical on both engines; the sanctioned path past the i64 no-wrap overflow contract): `big_from_int`, `big_from_str` (null on malformed input), `big_to_str`, `big_to_int` (null when the value does not fit i64, never a clamp), `big_is_zero`, `big_neg` (zero canonicalizes, so `-0 == 0`), `big_abs`, `big_cmp` (full signed ordering), `big_add`, `big_sub`, `big_mul`, `big_pow` (int exponent ≥ 0, else null), `big_fact`, `big_zero` |
| `std/binary.op` | byte-order packing and hex over the bytes type (pure, new-bytes results, short reads answer null): `bin_hex_encode` (lowercase), `bin_hex_decode` (null on odd length or non-hex), `bin_u16le`/`bin_u16be`/`bin_u32le`/`bin_u32be`, `bin_i64le` (two's complement; the bare 2^63 literal is out of the language's i64 range, the min value is spelled `(-9223372036854775807 - 1)`), `bin_read_u16le/be`, `bin_read_u32le/be`, `bin_read_i64le` (sign-extended, bit 63 kept out of shifts per the shift contract), `bin_size` |
| `std/bio.op` | in-silico sequence utilities: `codon_usage`, `is_palindromic_site`, `melting_point`, `gc_skew` |
| `std/collections.op` | list-shaped data work: `chunk`, `zip`, `group_by`, `take`, `flatten`, `count` |
| `std/csv.op` | delimited data: `csv_escape`, `csv_row`, `csv_parse`, `csv_parse_line`, `csv_records`, `csv_column`, `csv_count_fields` |
| `std/deque.op` | double-ended queue + FIFO queue over plain lists (pure: every function returns a NEW list or a `[value, rest]` pair, arguments are never mutated): `deque_from`, `deque_push_back`, `deque_push_front`, `deque_pop_back`, `deque_pop_front`, `deque_peek_front`, `deque_peek_back`, `queue_new`, `queue_push`, `queue_pop`, `queue_len` |
| `std/env.op` | capability-safe environment reads over the gated `env(name)` builtin (wrappers carry NO grants; a grant refusal stays a catchable `interference` Stress, graceful wrappers answer the fallback instead, the loud paths propagate the refusal untouched because hiding a denial would forge evidence): `env_get`, `env_get_or`, `env_has` (false on unset OR denial, one answer, documented), `env_granted` (value-free grant probe), `env_fetch` (Option, `some`/`none`), `env_require` (value or catchable `missing` when unset, refusal passes through as `interference`); GAP: there is no enumeration builtin, reads are by-name only, and any future `env_keys()` needs a DECISIONS entry for its grant model (enumerating names is itself a leak surface); environment writes are absent by contract |
| `std/fmt.op` | output formatting: `fmt_fixed`, `fmt_thousands`, `fmt_pct`, `fmt_bytes`, `fmt_cell`, `fmt_pad_cell`, `fmt_table`, `fmt_bool` |
| `std/fs.op` | capability-safe file helpers (Stress-returning, never panicking): `fs_read_or`, `fs_read`, `fs_lines_or`, `fs_lines`, `strings_lines`, `fs_write_text`, `fs_write_lines`, `fs_append_line`, `fs_read_json_or`, `fs_write_json`, `fs_list_dir_or`, `fs_size_or` |
| `std/graph.op` | deterministic directed weighted graphs over plain maps (canonical shape `{nodes: {}, adj: {}}`; pure: every function returns a NEW graph or list, arguments are never mutated; the DETERMINISM CONTRACT pins node order to the `nodes` map's insertion order and neighbor order to the row's insertion order, every traversal and tie-break follows it, ids are never sorted, only weights compare, via the optional strictly-before comparator gene or the default `<`; soft failures are null/false/unchanged-copy, never a Stress; stage-1 soft type annotations on every public gene, the W003 sugar first slice): `graph_new`, `graph_add_node`, `graph_add_edge` (auto-creates missing endpoints), `graph_remove_node`, `graph_remove_edge`, `graph_has_node`, `graph_has_edge`, `graph_weight`, `graph_nodes`, `graph_node_count`, `graph_neighbors`, `graph_edges`, `graph_edge_count`, `graph_bfs`, `graph_dfs`, `graph_reachable`, `graph_topo` (Kahn's, insertion-order tie-break, null on a cycle), `graph_shortest_path` (unweighted BFS path, null when unreachable), `graph_mst_weight` (undirected reading, Kruskal over pinned pair order, null when disconnected) |
| `std/heap.op` | deterministic binary min-heap over plain lists (array layout, pure copies; optional comparator gene with the `xs.sort` convention, `cmp(a,b)` true when a belongs before b; priority-queue idiom: `[priority, value]` pairs + `gene (x, y) => x[0] < y[0]`): `heap_from`, `heap_push`, `heap_pop` (returns `[value, rest]`), `heap_peek`, `heap_sorted`, `heap_len` |
| `std/hashing.op` | checksums + cryptographic hashing in pure Operon (deterministic integer arithmetic, byte-identical on both engines; 32-bit lanes only, a 64-bit hash would overflow the i64 no-wrap contract and is consciously out of scope): `hash_fnv1a32`, `hash_djb2`, `hash_sdbm`, `hash_adler32`, `hash_crc32` (the zip/png polynomial, bitwise, no table), `sha256` (FIPS 180-4 over 32-bit lanes, byte-list digest; pinned to the standard test vectors), `sha256_hex`; hashes accept bytes or a string (UTF-8) |
| `std/iter.op` | iterator adapters over lists: `take`, `drain`, `enumerate_pairs`, `zip`, `flatten`, `unique`, `chunk`, `range_step`, `map`, `filter`, `fold`, `scan`, `any`, `all`, `take_while`, `drop_while`, `find_first`, `index_of`, `intersperse`, `sliding`, `sort_by_key`, `reversed`, `concat_all` |
| `std/json.op` | JSON navigation: `json_type`, `json_is_object`, `json_is_array`, `json_get_or`, `json_get`, `json_merge`, `json_pick`, `json_omit`, `json_flatten`, `json_flatten_into`, `json_compact` |
| `std/logging.op` | leveled logging shapes (deterministic core; wall-clock reads are opt-in and shape-checked, never pinned absolute): `log_level`, `log_level_name` (round-trip; unknown names are info), `log_line` (the one format: `[LEVEL] message`), `log_stamp` (`[LEVEL epoch=<n>] message`, the only impure gene), `make_logger` (numeric floor; returns a map with `.debug`/`.info`/`.warn`/`.error` methods, quieter levels return null), `make_logger_named` (symbolic floor) |
| `std/math.op` | numeric helpers: `clamp`, `lerp`, `mean`, `variance`, `stddev`, `median`, `hill`, `sigmoid`, `digits`, `round`, `round_to`, `sign`, `gcd`, `lcm`, `factorial`, `isqrt`, `divmod`, `wrap` |
| `std/motifs.op` | canonical regulatory-network circuits over the `regulate` layer: `motif_install` wires them, `motif_hill` is the engine's own dose-response transfer function, `motif_autoreg` / `ar` (negative autoregulation; `ar` is the short alias), `fc`/`motif_ffl` (coherent feed-forward persistence filter), `ic`/`motif_pulse` (incoherent feed-forward pulse compression), `tx`/`ty`/`motif_flip` (mutual-repression toggle), `motif_states` (the whole board's levels) |
| `std/path.op` | pure lexical path shaping (NO filesystem access, no capabilities, fs I/O stays in std/fs): `path_base`, `path_dir`, `path_ext`, `path_is_abs`, `path_join`, `path_norm` (lexical; relative `..` kept, absolute-root `..` ignored), `path_split_ext` |
| `std/process.op` | capability-safe subprocess wrappers over the gated `run(prog, args)` builtin (result map `{code, stdout, stderr, ok}`; a grant refusal or an unspawnable program degrades to the fallback, a wall-clock timeout stays IN-BAND with `code -1, ok false`; children inherit a scrubbed environment and their wall time is charged as fuel): `process_run`, `process_run_or`, `process_output`, `process_output_or`, `process_lines_or`, `process_ok` (true only for a clean exit), `process_succeeded`/`process_code` (shape-checked result-map accessors, null on junk); GAP: no child cwd override, no stdin feed, no shell form, no fire-and-forget spawn, no child env override, no pid/kill surface, each needs a DECISIONS entry before a builtin lands |
| `std/random.op` | deterministic randomness over the core `random()`/`randomize(seed)` stream (mirrored xorshift64*, byte-identical under a fixed seed): `rand_below`, `rand_int` (inclusive both ends), `rand_pick`, `rand_shuffle` (Fisher-Yates, new list), `rand_weighted` (cumulative weights over the sum, weights MUST be non-negative; negative weights are undefined), `rand_chance` |
| `std/seq.op` | sequence (worker-cell generator) combinators: `map_seq`, `filter_seq`, `take_seq`, `concat_seq`, `fib_seq`, `range_seq`, combinators take the SOURCE as a sequence-gene NAME (transcripts do not cross membranes); transformation genes travel freely |
| `std/serialize.op` | unified serialization front door (W34): `to_json`, `from_json` (Result-shaped, `ok(value)`/`err(message)`, never a silent null), `to_csv_rows`, `from_csv_rows`, `serialize`/`deserialize` (format-generic dispatch), `roundtrip` (to→from→equality check, per-type table incl. the honest non-finite-float gap), `supported_formats`; stage 2 (2026-09-27): phenotype instances ride the canonical spawn wire shape (field map + hidden `"#phenotype"` key, SPEC §10 builtins `is_object`/`object_fields`/`object_from_map`), instances round-trip to REAL instances (data restore, methods included), unknown class names are err values; the W04 `Serializable.to_map()` trait hook stays spec'd in docs/specs/SERIALIZATION.md as the future custom-projection override |
| `std/set.op` | set algebra over plain lists (a set is a list with unique members; equality is order-independent; every function is pure and returns a NEW list): `set_from`, `set_has`, `set_add`, `set_del`, `set_union`, `set_intersect`, `set_diff`, `set_symdiff`, `set_subset`, `set_eq`, `set_count` |
| `std/strings.op` | everyday string shaping: `words`, `capital`, `pad_left`, `pad_right`, `starts_any`, `pad`, `strip_prefix`, `strip_suffix`, `is_blank`, `chars`, `lines`, `title_case`, `to_snake`, `to_camel`, `to_kebab`, `ellipsis`, `unquote` |
| `std/terminal.op` | ANSI escape builders + text hygiene (pure strings, ESC built via `chr(27)`, no capabilities): `t_reset`, `t_bold`, `t_dim`, `t_underline`, `t_fg`/`t_bg` by color name (unknown names fall back to white), `t_fg256` (clamped 0..255), `t_strip_ansi` (state-machine removal of ESC-[...-letter sequences), `t_truncate_width` (printed-cell budget, escapes ride free, `...` counts toward it), `t_has_ansi` |
| `std/typed.op` | W003 stage-2 typed wrappers over the callable-generic core (docs/specs/GENERICS.md is the normative contract; one wrapper per core gene, zero per-type duplication, `<T, U>` spellings descriptive, bare type parameters erase at runtime, the parameterized container annotations enforce the shallow is-a shape as a catchable unfolded Stress naming the annotation): `map` (`list[T]` → `list[U]`), `filter`, `fold`, `first` (`T?`), `zip`, `lookup` (`map[K, V]` read, named lookup because a module gene named `get` shadows the map method `m.get`), `sort_by_key` |
| `std/testing.op` | minimal deterministic test harness (no I/O, nothing raised): `expect_eq`, `expect_true`, `expect_false`, `expect_near` (inclusive float tolerance), `expect_throws` (pass a zero-arg gene; any raise counts), `test_summary`, checks accumulate into the module's own tally, a failed expect is data and returns false |
| `std/time.op` | pure duration & instant arithmetic over the L1d clock builtins (UTC-only contract, no timezone support, W89): `time_add`, `time_diff`, `time_days_between`, `time_midnight`, `time_date_only`, `time_is_leap`, `time_days_in_month`, `dur_hms` ("HH:MM:SS"), `dur_human` ("1d 2h 3m 4s"), `time_parse_iso` (ISO-8601 UTC subset parser: "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM:SS", fractional seconds truncated, trailing Z accepted, offsets rejected, null on malformed) |
| `std/unicode.op` | Unicode depth helpers over the language's documented fold subset (SPEC §3, pure string arithmetic, no capabilities, no external tables): `upper`, `is_upper`, `is_lower`, `is_digit`, `is_alpha`, `char_codes`, `from_char_codes`, `byte_width` |
| `std/url.op` | percent-encoding and URL shaping over plain strings (pure, canonical, lenient by contract): `url_encode` (RFC 3986 unreserved set, uppercase hex), `url_decode` (`%XX` and `+` to space, malformed escapes kept verbatim, never fails), `url_parse` (scheme/host/port/path/query map with decoded pairs), `url_query_encode` (sorted keys, canonical output) |

Native kernels back the hot parts and are builtins, not imports:
`distance(a, b)` (bit-parallel Myers edit distance, C++), `codon(seq)`
(codon-usage style score, C++), `json_parse`/`json_str`, and the
capability-gated `re_*` regex family. Every stdlib gene is pure `.op`
over those builtins, the library ships no native code of its own.

### Core builtins: first-class Option/Result (W06, D-014, SPEC §9)

Expected failures are values, not stress. The tag IS the contract, the
Option family (`some`/`none`) and the Result family (`ok`/`err`) are
distinct even with equal payloads:

| builtin | contract |
|---|---|
| `some(v)` / `none()` | build an Option (type `option`) |
| `ok(v)` / `err(e)` | build a Result (type `result`) |
| `is_some(v)` / `is_none(v)` / `is_ok(v)` / `is_err(v)` | tag predicates |
| `unwrap_or(v, default)` | safe extraction, never stresses; None/Err/plain all yield `default` |
| `unwrap(v)` | unsafe extraction, Some/Ok payload; otherwise Stress kind `unwrap` (the exceptional tier; rescue-catchable) |
| `e?!` (postfix) | propagation, Some/Ok unwrap to the payload; None/Err return FROM the enclosing gene with that variant; plain values pass through; never contained by rescue (SPEC §9) |
| `try_num` / `try_index` / `try_get` / `try_pop` | W06 stage 2 (v2.6.0): Result variants of the failure-prone core builtins (SPEC §9) |
| `try_first` / `try_last` / `try_char_at` / `try_env` / `try_json_parse` / `try_re_groups` | W06 stage 2 wave 2 (v2.7.0): extraction, environment, parsing families as Results; `Err` payloads are engine-neutral raw-input echoes or fixed strings (SPEC §9 compat note); capability denials and the ReDoS ceiling stay Stresses |

JSON view: `{"ok":1}` / `{"err":"x"}` / `{"some":1}` / `null` for None.

Discovery rules for agents and humans:
- module genes are documented by their one-line headers in each file;
- anything the module exports is importable, nothing is hidden;
- new modules MUST land with a row here and proof coverage (SPEC §17).
