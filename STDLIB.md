# STDLIB — the Operon standard library

The std layer is **written in Operon itself** (`.op` files under `std/`) — no Rust, no new
builtins. It runs identically on the Rust core and the Python oracle (the differential
harness executes every example below on both).

```operon
use std/math as m          # prefixed: m.clamp(...)
use std/strings            # unprefixed: capital(...) directly
```

**Proof coverage**: every function below is exercised by `operon test tests/` — the `Proven in`
column points at the proof file. The suite is differential: each file runs on the Rust core
AND the Python oracle, and the outputs must match byte-for-byte.

**Gotchas worth knowing before you write std-style code** (each one bit someone once):

- A bare `{` inside a string literal **starts interpolation** — even after an escaped quote.
  Build raw JSON / braces with `chr(123)` / `chr(125)`.
- `"\r"` is **not** an escape (it stays literal backslash-r). `\n` and `\t` are real. Use
  `chr(13)` for carriage returns.
- The words `on`/`off`/`yes`/`no`/`next`/`iter`/`type`/`each`... are synonyms or keywords
  (`off` repairs to `false`). Don't use them as variable names.
- `/` is float division; `//` is integer division; `%` is floored (`-7 % 3 == 2`).
- Single-quoted string literals work but emit a repair note — prefer double quotes.

---

## std/math — `use std/math as m`

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| clamp | `clamp(x, lo, hi)` | constrain x into [lo, hi] | `m.clamp(5, 0, 3) == 3` | stdlib_selfhost.op |
| lerp | `lerp(a, b, t)` | linear interpolation a→b by t | `m.lerp(0, 10, 0.5) == 5.0` | std_math_ext.op |
| mean | `mean(xs)` | arithmetic mean (0.0 for empty) | `m.mean([1, 2, 3]) == 2.0` | stdlib_selfhost.op |
| variance | `variance(xs)` | population variance | `m.variance([2, 4, 4, 4, 5, 5, 7, 9]) == 4.0` | std_math_ext.op |
| stddev | `stddev(xs)` | sqrt of variance | `m.stddev([2, 2]) == 0.0` | std_math_ext.op |
| median | `median(xs)` | middle value (average of two for even n) | `m.median([3, 1, 2]) == 2.0` | stdlib_selfhost.op |
| hill | `hill(x, k, n)` | saturation curve xⁿ/(xⁿ+kⁿ) | `m.hill(1.0, 1.0, 2) == 0.5` | stdlib_selfhost.op |
| sigmoid | `sigmoid(x)` | logistic 1/(1+e^-x) | `m.sigmoid(0.0) == 0.5` | stdlib_selfhost.op |
| digits | `digits(n)` | decimal digits, most significant first | `m.digits(402) == [4, 0, 2]` | stdlib_selfhost.op |
| round | `round(x)` | **half away from zero** (`round(2.5) == 3`, `round(-2.5) == -3`) | `m.round(2.4) == 2` | std_math_ext.op |
| round_to | `round_to(x, n)` | round to n decimals (returns a number; use fmt_fixed for text) | `m.round_to(3.14159, 3) == 3.142` | std_math_ext.op |
| sign | `sign(x)` | -1 / 0 / 1 | `m.sign(0 - 5) == -1` | std_math_ext.op |
| gcd | `gcd(a, b)` | greatest common divisor (abs-valued, always >= 0) | `m.gcd(12, 18) == 6` | std_math_ext.op |
| lcm | `lcm(a, b)` | least common multiple; 0 if either is 0 | `m.lcm(4, 6) == 12` | std_math_ext.op |
| factorial | `factorial(n)` | n! (n >= 0) | `m.factorial(5) == 120` | std_math_ext.op |
| isqrt | `isqrt(n)` | integer sqrt by binary search, float-error-free | `m.isqrt(17) == 4` | std_math_ext.op |
| divmod | `divmod(a, b)` | `[quotient, remainder]`, floored semantics | `m.divmod(17, 5) == [3, 2]` | std_math_ext.op |
| wrap | `wrap(x, lo, hi)` | cycle x into [lo, hi), negatives included | `m.wrap(370, 0, 360) == 10` | std_math_ext.op |


## std/iter — `use std/iter as it` (list adapters)

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| take | `take(seq, n)` | first n items of a sequence or list | `it.take([9, 8, 7], 2) == [9, 8]` | stdlib_selfhost.op |
| drain | `drain(seq)` | collect everything left | `it.drain(seq_obj)` | stdlib_selfhost.op* |
| enumerate_pairs | `enumerate_pairs(xs)` | `[[0, x0], [1, x1], ...]` | `it.enumerate_pairs(["a"]) == [[0, "a"]]` | std_iter_ext.op |
| zip | `zip(a, b)` | pair up to the shorter length | `it.zip([1, 2], ["a"]) == [[1, "a"]]` | stdlib_selfhost.op |
| flatten | `flatten(xs)` | one level deep | `it.flatten([[1], [2, 3]]) == [1, 2, 3]` | stdlib_selfhost.op |
| unique | `unique(xs)` | dedupe preserving first occurrence | `it.unique([1, 2, 1]) == [1, 2]` | stdlib_selfhost.op |
| chunk | `chunk(xs, n)` | fixed-size groups, last may be short | `it.chunk([1, 2, 3, 4, 5], 2) == [[1, 2], [3, 4], [5]]` | stdlib_selfhost.op |
| range_step | `range_step(a, b, step)` | explicit-step range | `it.range_step(0, 10, 3) == [0, 3, 6, 9]` | std_iter_ext.op |
| map | `map(xs, f)` | transform each item | `it.map([1, 2], gene (x) => x * 2) == [2, 4]` | std_iter_ext.op |
| filter | `filter(xs, pred)` | keep matches | `it.filter(range(0, 4), gene (x) => x % 2 == 0) == [0, 2]` | std_iter_ext.op |
| fold | `fold(xs, acc, f)` | left fold with explicit init | `it.fold([1, 2, 3], 0, gene (a, x) => a + x) == 6` | std_iter_ext.op |
| scan | `scan(xs, acc, f)` | fold that remembers every step | `it.scan([1, 2], 10, add) == [11, 13]` | std_iter_ext.op |
| any | `any(xs, pred)` | true if at least one match | `it.any([1, 2], gt1) == true` | std_iter_ext.op |
| all | `all(xs, pred)` | true if every item matches | `it.all([2, 4], even) == true` | std_iter_ext.op |
| take_while | `take_while(xs, pred)` | longest matching prefix | `it.take_while([1, 2, 0], pos) == [1, 2]` | std_iter_ext.op |
| drop_while | `drop_while(xs, pred)` | skip the matching prefix | `it.drop_while([1, 2, 3, 0], lt3) == [3, 0]` | std_iter_ext.op |
| find_first | `find_first(xs, pred)` | first match or null | `it.find_first([5, 6], gt5) == 6` | std_iter_ext.op |
| index_of | `index_of(xs, v)` | position of first `==` or -1 | `it.index_of(["a", "b"], "b") == 1` | std_iter_ext.op |
| intersperse | `intersperse(xs, sep)` | weave sep between items | `it.intersperse([1, 2], 0) == [1, 0, 2]` | std_iter_ext.op |
| sliding | `sliding(xs, n)` | overlapping windows | `it.sliding([1, 2, 3], 2) == [[1, 2], [2, 3]]` | std_iter_ext.op |
| sort_by_key | `sort_by_key(xs, keyf)` | sort on a computed key | `it.sort_by_key(words, gene (s) => len(s))` | std_iter_ext.op |
| reversed | `reversed(xs)` | new reversed list (does not mutate) | `it.reversed([1, 2]) == [2, 1]` | std_iter_ext.op |
| concat_all | `concat_all(xss)` | concatenate a list of lists | `it.concat_all([[1], [2]]) == [1, 2]` | std_iter_ext.op |

(For generator-style composition over `sequence` genes, see std/seq below — `map_seq` etc.)

## std/strings — `use std/strings as st`

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| words | `words(s)` | whitespace-split, empties removed | `st.words("a  b").len() == 2` | stdlib_selfhost.op |
| capital | `capital(s)` | uppercase the first character | `st.capital("gene") == "Gene"` | modules_std.op |
| pad_left | `pad_left(s, w, fill)` | left-pad to width | `st.pad_left("7", 3, "0") == "007"` | modules_std.op |
| pad_right | `pad_right(s, w, fill)` | right-pad to width | `st.pad_right("7", 3, ".") == "7.."` | modules_std.op |
| pad | `pad(s, w, fill)` | center-pad to width | `st.pad("a", 3, "-") == "-a-"` | std_strings_ext.op |
| starts_any | `starts_any(s, prefixes)` | any prefix matches | `st.starts_any("--v", ["-", "--"]) == true` | std_strings_ext.op |
| strip_prefix | `strip_prefix(s, p)` | remove p if present at the start | `st.strip_prefix("--flag", "--") == "flag"` | std_strings_ext.op |
| strip_suffix | `strip_suffix(s, x)` | remove x if present at the end | `st.strip_suffix("f.op", ".op") == "f"` | std_strings_ext.op |
| is_blank | `is_blank(s)` | empty or whitespace only | `st.is_blank("   ") == true` | std_strings_ext.op |
| chars | `chars(s)` | list of single-character strings | `st.chars("ab!") == ["a", "b", "!"]` | std_strings_ext.op |
| lines | `lines(s)` | split on \n, trailing empty dropped | `st.lines("a\nb\n") == ["a", "b"]` | std_strings_ext.op |
| title_case | `title_case(s)` | capitalize every word, whitespace-normalized | `st.title_case("hello world") == "Hello World"` | std_strings_ext.op |
| to_snake | `to_snake(s)` | camelCase / kebab / spaces → snake_case | `st.to_snake("myVarName") == "my_var_name"` | std_strings_ext.op |
| to_camel | `to_camel(s)` | words → lowerCamelCase | `st.to_camel("hello big world") == "helloBigWorld"` | std_strings_ext.op |
| to_kebab | `to_kebab(s)` | anything → kebab-case | `st.to_kebab("myVarName") == "my-var-name"` | std_strings_ext.op |
| ellipsis | `ellipsis(s, w)` | truncate to w with "..." tail | `st.ellipsis("abcdefgh", 6) == "abc..."` | std_strings_ext.op |
| unquote | `unquote(s)` | strip one matching quote pair | `st.unquote("\"x\"") == "x"` | std_strings_ext.op |

## std/collections — `use std/collections`

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| chunk | `chunk(xs, size)` | fixed-size groups | `collections.chunk([1, 2, 3, 4, 5], 2)` | modules_std.op |
| zip | `zip(a, b)` | pair to the shorter length | `collections.zip([1, 2], ["a", "b"])` | modules_std.op |
| group_by | `group_by(xs, keyf)` | map: key → list of members | `collections.group_by(words, first_letter)` | modules_std.op |
| take | `take(xs, n)` | slice(0, n) | `collections.take([9, 8, 7], 2) == [9, 8]` | modules_std.op |
| flatten | `flatten(xss)` | one-level concat | `collections.flatten([[1, 2], [3]])` | modules_std.op |
| count | `count(xs, pred)` | how many match | `collections.count([1, 2, 3, 4], even) == 2` | modules_std.op |

Note: collections.op and iter.op deliberately both define `chunk`/`zip`/`take`/`flatten` —
import one or the other (or use aliases) and behavior is identical.

## std/args — `use std/args as ar` (NEW in B2)

All helpers take a LIST of argv strings (pass `argv()` straight in). Convention:
`--flag` boolean · `--key value` next item · `--key=value` attached · `-k` single-dash is
boolean only (values need `=`) · everything else positional. A bare `--key` consumes the next
item even if that is a flag — use `=` form when in doubt.

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| args_norm | `args_norm(name)` | bare name → `--name`; dashed stays | `ar.args_norm("out") == "--out"` | std_args.op |
| args_positional | `args_positional(argv)` | non-flag, non-value items | `ar.args_positional(["tool", "--verbose", "x"]) == ["tool"]` | std_args.op |
| args_flag | `args_flag(argv, name)` | present? (truthy `=true/1/yes/on` honored) | `ar.args_flag(argv(), "verbose")` | std_args.op |
| args_has | `args_has(argv, name)` | key present in either form | `ar.args_has(["--level=0"], "level")` | std_args.op |
| args_value | `args_value(argv, name, default)` | value of --key / --key=value, else default | `ar.args_value(argv(), "out", "a.out")` | std_args.op |
| args_get | `args_get(argv, name)` | value or null | `ar.args_get(argv(), "tag")` | std_args.op |
| args_number | `args_number(argv, name, default)` | numeric option (int or float) | `ar.args_number(argv(), "jobs", 1)` | std_args.op |
| args_subcommand | `args_subcommand(argv)` | first positional or null | `ar.args_subcommand(argv()) == "build"` | std_args.op |

## std/csv — `use std/csv as cv` (NEW in B2)

RFC-4180-flavored, separator-parameterized ("csv" also parses TSV/PSV). Quoted fields,
doubled-quote escapes, quoted separators and newlines survive. Pure string layer — no fs,
no caps. Real CRLF is swallowed (built with `chr(13)`; note `"\r"` is not an escape).

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| csv_escape | `csv_escape(field, sep)` | quote when needed, double inner quotes | `cv.csv_escape("a,b", ",") == "\"a,b\""` | std_csv.op |
| csv_row | `csv_row(fields, sep)` | serialize one row | `cv.csv_row(["a", "b,c"], ",") == "a,\"b,c\""` | std_csv.op |
| csv_parse | `csv_parse(text, sep)` | whole document → rows (list of field lists) | `cv.csv_parse(doc, ",")` | std_csv.op |
| csv_parse_line | `csv_parse_line(line, sep)` | one line → fields | `cv.csv_parse_line("1,\"x\",3", ",")` | std_csv.op |
| csv_records | `csv_records(text, sep)` | header + rows → list of maps | `cv.csv_records(doc, ",")[0]["name"]` | std_csv.op |
| csv_column | `csv_column(text, sep, name)` | one named column's values | `cv.csv_column(doc, ",", "age")` | std_csv.op |
| csv_count_fields | `csv_count_fields(line, sep)` | quote-aware field count | `cv.csv_count_fields("1,\"a,b\"", ",") == 2` | std_csv.op |

## std/json — `use std/json as js` (NEW in B2)

Navigation and reshaping over the `json_parse`/`json_str` builtins. Paths are dot-separated;
object steps go by key, array steps by canonical decimal index.

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| json_type | `json_type(v)` | JSON vocabulary: object/array/number/boolean/null | `js.json_type(v) == "object"` | std_json.op |
| json_is_object | `json_is_object(v)` | is a map | `js.json_is_object(v)` | std_json.op |
| json_is_array | `json_is_array(v)` | is a list | `js.json_is_array(v)` | std_json.op |
| json_get | `json_get(v, path)` | walk "user.tags.0", null on miss | `js.json_get(v, "user.name") == "Ada"` | std_json.op |
| json_get_or | `json_get_or(v, path, fallback)` | walk with explicit fallback | `js.json_get_or(v, "a.b", "FB")` | std_json.op |
| json_merge | `json_merge(a, b)` | shallow merge, right wins | `js.json_merge(cfg, overrides)` | std_json.op |
| json_pick | `json_pick(v, keys)` | keep only listed keys | `js.json_pick(v, ["id", "name"])` | std_json.op |
| json_omit | `json_omit(v, keys)` | drop listed keys | `js.json_omit(v, ["secret"])` | std_json.op |
| json_flatten | `json_flatten(v)` | dot-path → scalar map | `js.json_flatten(v)["a.b.0"]` | std_json.op |
| json_compact | `json_compact(text)` | parse + re-serialize (drops formatting) | `js.json_compact("{ "a" : 1 }")` | std_json.op |

(`json_flatten_into(v, prefix, out)` is the internal accumulator behind json_flatten — public
because std has no privacy, but you normally want json_flatten.)

## std/fmt — `use std/fmt as fm` (NEW in B2)

Pure presentation helpers. Rounding is **half away from zero**; documented per function.

| function | signature | what it does | example | proven in |
|---|---|---|---|---|
| fmt_fixed | `fmt_fixed(x, n)` | fixed decimals with zero padding | `fm.fmt_fixed(3.14159, 2) == "3.14"` | std_fmt.op |
| fmt_thousands | `fmt_thousands(n)` | digit grouping | `fm.fmt_thousands(1234567) == "1,234,567"` | std_fmt.op |
| fmt_pct | `fmt_pct(x, n)` | fraction → percent string | `fm.fmt_pct(0.125, 1) == "12.5%"` | std_fmt.op |
| fmt_bytes | `fmt_bytes(n)` | binary-unit human sizes | `fm.fmt_bytes(1536) == "1.5 KB"` | std_fmt.op |
| fmt_cell | `fmt_cell(row, headers, i)` | one table cell (map or list row) | used by fmt_table | std_fmt.op |
| fmt_pad_cell | `fmt_pad_cell(s, w)` | left-justified padding | used by fmt_table | std_fmt.op |
| fmt_table | `fmt_table(rows, headers)` | ASCII table (map rows keyed by header, or positional) | `fm.fmt_table(recs, ["name", "age"])` | std_fmt.op |
| fmt_bool | `fmt_bool(b)` | stable "true"/"false" | `fm.fmt_bool(1 == 1) == "true"` | std_fmt.op |

## std/fs — `use std/fs as fsys` (NEW in B2)

**Safe wrappers over the capability-gated builtins.** The capability model stays default-deny
and untouched — wrappers carry no grants. On a missing file OR a denied capability they return
your fallback / null / false instead of throwing. "Safe" means never crashes the program;
denial still denies. If you must distinguish missing-vs-denied, call the raw builtins under
`stress / rescue` yourself.

| function | signature | what it does | proven in |
|---|---|---|---|
| fs_read_or | `fs_read_or(path, fallback)` | whole file as text, fallback on any failure | std_fs.op |
| fs_read | `fs_read(path)` | whole file as text or null | std_fs.op |
| fs_lines_or | `fs_lines_or(path, fallback)` | file → lines, fallback on failure | std_fs.op |
| fs_lines | `fs_lines(path)` | file → lines or null | std_fs.op |
| strings_lines | `strings_lines(text)` | pure: split on \n, trailing empty dropped | std_fs.op |
| fs_write_text | `fs_write_text(path, text)` | write text; true/false | std_fs.op |
| fs_write_lines | `fs_write_lines(path, lines)` | write list as terminated lines; true/false | std_fs.op |
| fs_append_line | `fs_append_line(path, line)` | append one line; true/false | std_fs.op |
| fs_read_json_or | `fs_read_json_or(path, fallback)` | read + parse with fallback | std_fs.op |
| fs_write_json | `fs_write_json(path, v)` | json_str + write; true/false | std_fs.op |
| fs_list_dir_or | `fs_list_dir_or(path, fallback)` | directory listing with fallback | std_fs.op |
| fs_size_or | `fs_size_or(path, fallback)` | file size with fallback | std_fs.op |

(The deny path is what CI proves — the proof suite runs without grants. Grant-path behavior is
the raw builtins' own contract; exercise it manually with `--allow-read/--allow-write`.)

## std/seq — `use std/seq as sq` (generator combinators)

Sequences are worker cells; values cross through a rendezvous channel. Combinators take the
SOURCE as the NAME of a sequence gene; transformation genes travel freely.

| function | signature | what it does | proven in |
|---|---|---|---|
| map_seq | `map_seq(src, sargs, f)` | lazy transform of a sequence | stdlib_selfhost.op |
| filter_seq | `filter_seq(src, sargs, pred)` | lazy filter | stdlib_selfhost.op |
| take_seq | `take_seq(src, sargs, n)` | first n values | stdlib_selfhost.op |
| concat_seq | `concat_seq(src_a, args_a, src_b, args_b)` | two sources back to back | stdlib_selfhost.op |
| fib_seq | `fib_seq(limit = 1e15)` | Fibonacci stream, bounded | stdlib_selfhost.op |
| range_seq | `range_seq(a, b, step)` | lazy range | stdlib_selfhost.op |

## std/bio — `use std/bio` (flavor domain, per D-008 gene vocabulary is intuition only)

| function | signature | what it does | proven in |
|---|---|---|---|
| codon_usage | `codon_usage(seq)` | 3-letter group frequency map | modules_std.op |
| is_palindromic_site | `is_palindromic_site(dna)` | reverse-complement palindrome check | modules_std.op |
| melting_point | `melting_point(dna)` | simple wallace-rule style estimate | modules_std.op |
| gc_skew | `gc_skew(dna)` | (G−C)/(G+C) | modules_std.op |

---

## Adding to the stdlib (for contributors)

1. Write the gene in the right module (pure `.op`, no new Rust builtins — that needs a
   DECISIONS entry).
2. Every function gets asserts in a `tests/std_*.op` proof file (or stdlib_selfhost.op) —
   the differential harness runs them on BOTH implementations, so stick to idioms both
   cores support.
3. Add a row to the table above in the same PR. An undocumented std function is a bug.
4. Naming: avoid synonym words (`off`, `on`, `next`, `iter`, `each`, `type`...) as variables —
   the Total Grammar repairs them (rung 2).
