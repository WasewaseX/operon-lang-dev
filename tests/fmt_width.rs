//! W47-v2 (ROADMAP-100): the `--width` soft line-width pass, enforced by law.
//!
//! The width pass is a POST-PRINT wrapper: it runs after the canonical render
//! and may only insert newlines at parser-proven-safe comma points (inside
//! `(`/`[` groups — the element loops that call `eat_newlines_inline()`.
//! This file proves the three laws that make that claim true, corpus-wide:
//!
//! Law A — AST identity: wrapping changes NOTHING. The AST parsed from the
//! width-formatted output equals the AST parsed from the canonical
//! (unwrapped) render, for every corpus file and every swept width.
//!
//! Law B — zero notes: the wrapped output re-parses with exactly the same
//! notes as the canonical render (by construction: none). A break the parser
//! did not tolerate would surface here as a rung-3/4 repair note.
//!
//! Law C — idempotence: fmt(w) ∘ fmt(w) == fmt(w), byte-exact, per config.
//!
//! Honest scope pinned by unit tests: openers never break, `{` groups never
//! break (block-vs-map ambiguity), strings/interpolations are opaque, depth-0
//! commas (multi-assign lists) never break, unbreakable lines stay long.

use operon::ast;
use operon::parser;
use operon::tools::{format_program_with, parse_fmt_config, FmtConfig};

fn contained_depth_pin(p: &std::path::Path) -> bool {
    // Z-SWEEP4-FIXCORPUS-CONTAIN (2026-10-09): the two parser depth pins are
    // pathological BY DESIGN - they sit exactly at the #111/#112 guard
    // boundary, and fmt's canonicalized output of guard-boundary content
    // re-parses as truncated (rung-3 note), violating the fmt corpus laws
    // for files that exist to trip the parser guard. Out of the fmt corpus
    // contract, loudly, until the fixer-lane depth discipline lands (option
    // a, coordinator incident post cfe5beb). Their PARSE-side contract is
    // asserted in fix_corpus law1 (big-stack thread).
    // Match on FILE NAME: Windows paths use backslashes, so path-suffix
    // matching would silently miss on the windows CI legs.
    const CONTAINED: &[&str] = &["parser_depth_leak_pin.op", "parser_depth_calls_pin.op"];
    match p.file_name().and_then(|n| n.to_str()) {
        Some(name) => CONTAINED.contains(&name),
        None => false,
    }
}

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    // Z-SWEEP4-FIXCORPUS-CONTAIN: corpus tests parse the two parser depth
    // pins (tests/differential/parser_depth_{calls,leak}_pin.op), whose
    // 4000-deep nesting sits exactly at the default 2MB test-thread stack
    // boundary - any compilation variance flips it (2026-10-09 S0 incident).
    // Their parse contract is real and stays asserted, on a dedicated
    // big-stack thread. Precedent: vm_parity rt_p4b containment.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack test thread")
        .join()
        .expect("big-stack test thread panicked")
}

fn corpus() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // sandbox hygiene: only TRACKED .op files are the width corpus —
    // untracked drafts from a parallel lane's working tree must not red the
    // law (CI sees clean checkouts; degrade to include-all without git).
    let tracked: Option<std::collections::HashSet<std::path::PathBuf>> =
        std::process::Command::new("git")
            .args(["ls-files", "std", "tests", "examples", "apps"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.ends_with(".op"))
                    .map(|l| root.join(l))
                    .collect()
            });
    let mut out = Vec::new();
    for dir in ["std", "tests", "examples", "apps"] {
        let base = root.join(dir);
        if !base.is_dir() {
            continue;
        }
        let mut stack = vec![base];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).expect("read_dir") {
                let p = e.expect("dir entry").path();
                if p.is_dir() {
                    // redteam payloads are adversarial BYTES (BOM/CRLF
                    // attacks): formatting them is meaningless (same policy
                    // as fmt_idempotence.rs)
                    if p.file_name().map(|n| n == "redteam").unwrap_or(false) {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().map(|x| x == "op").unwrap_or(false)
                    && tracked.as_ref().is_some_and(|t| t.contains(&p))
                {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

fn cfg_width(w: usize) -> FmtConfig {
    FmtConfig {
        width: Some(w),
        ..FmtConfig::default()
    }
}

fn canonical_of(src: &str) -> String {
    format_program_with(&parser::parse(src), &FmtConfig::default())
}

fn note_fingerprint(prog: &ast::Program) -> Vec<(u8, String)> {
    prog.notes
        .iter()
        .map(|n| (n.rung, n.message.clone()))
        .collect()
}

fn assert_width_laws(src: &str, cfg: &FmtConfig, path: &std::path::Path) {
    let canonical_prog = parser::parse(src);
    let canonical = format_program_with(&canonical_prog, &FmtConfig::default());
    let wrapped = format_program_with(&canonical_prog, cfg);

    // Law A — AST identity (render-invisible): the canonical render is a
    // pure function of the AST MINUS line metadata, so re-rendering the
    // wrapped output must reproduce the canonical text byte-exactly. (A raw
    // Debug-dump comparison would false-fail on shifted line numbers, which
    // are parse metadata, not meaning.)
    assert_eq!(
        canonical_of(&wrapped),
        canonical,
        "width pass changed the AST for {}",
        path.display()
    );

    // Law B — the wrapped output introduces NO new repair notes
    assert_eq!(
        note_fingerprint(&parser::parse(&wrapped)),
        note_fingerprint(&parser::parse(&canonical)),
        "width pass introduced repair notes for {}",
        path.display()
    );

    // Law C — idempotence under the SAME width config
    let twice = format_program_with(&parser::parse(&wrapped), cfg);
    assert_eq!(
        wrapped,
        twice,
        "width pass is not idempotent for {}",
        path.display()
    );
}

#[test]
fn fmt_width_corpus_laws() {
    with_big_stack(move || {
        let files = corpus();
        assert!(
            files.len() > 100,
            "corpus unexpectedly small: {}",
            files.len()
        );
        for w in [20usize, 40, 60, 80] {
            let cfg = cfg_width(w);
            for f in &files {
                if contained_depth_pin(f) {
                    continue;
                }
                let src = std::fs::read_to_string(f).unwrap_or_default();
                assert_width_laws(&src, &cfg, f);
            }
        }
    });
}

#[test]
fn fmt_width_wraps_call_args_with_hanging_indent() {
    let src = "show(first_argument, second_argument, third_argument)\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(20));
    assert_eq!(
        out,
        "show(first_argument,\n  second_argument,\n  third_argument)\n"
    );
}

#[test]
fn fmt_width_short_lines_are_byte_untouched() {
    let src = "show(a, b)\nlet x = 1\n";
    assert_eq!(
        format_program_with(&parser::parse(src), &cfg_width(80)),
        "show(a, b)\nlet x = 1\n"
    );
}

#[test]
fn fmt_width_default_config_is_off() {
    // the historical behavior: no width pass, long lines stay long
    let src = "show(first_argument, second_argument, third_argument)\n";
    assert_eq!(
        format_program_with(&parser::parse(src), &FmtConfig::default()),
        src
    );
}

#[test]
fn fmt_width_never_breaks_at_depth_zero() {
    // multi-assign target/value lists live at depth 0 — a newline there is
    // NOT parser-tolerated, so it must never be chosen
    let src = "let alpha, beta = 1111111111, 2222222222, 3333333333\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(10));
    assert_eq!(out, src, "depth-0 commas must never break");
}

#[test]
fn fmt_width_never_breaks_inside_braces() {
    // map-literal commas sit under `{` — poisoned stack — never chosen.
    // (bare keys canonicalize to Expr::Str, so fmt renders them quoted)
    let with_map = "show({alpha: 1, beta: 2}, 7777777)\n";
    let out = format_program_with(&parser::parse(with_map), &cfg_width(30));
    // the map's internal commas must stay on one line together; the call's
    // own comma (friendly paren stack) is the chosen break
    assert_eq!(
        out, "show({\"alpha\": 1, \"beta\": 2},\n  7777777)\n",
        "map literal was broken apart or break point wrong: {out:?}"
    );
    // AST identity (render-invisible) still holds — the laws test proves it
    // corpus-wide, this pins the crafted case explicitly
    assert_eq!(
        canonical_of(&out),
        canonical_of(with_map),
        "map-containing wrap changed the AST"
    );
}

#[test]
fn fmt_width_strings_and_interpolations_are_opaque() {
    // a comma inside a string is never a break point; the line has no
    // breakable comma at all, so it stays long (honest limitation)
    let src = "show(\"alpha, beta, gamma, delta, epsilon, zeta, eta, theta\")\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(15));
    assert_eq!(out, src, "string contents were touched");

    // interpolation carries REAL parens/commas inside the quotes — also
    // opaque; the break lands after the whole string, never inside it
    let src2 = "show(\"values {f(a, b)} and {g(c, d)}\", other_arg_here)\n";
    let out2 = format_program_with(&parser::parse(src2), &cfg_width(20));
    assert_eq!(
        canonical_of(&out2),
        canonical_of(src2),
        "interpolation wrap changed the AST"
    );
    assert!(
        out2.contains("\"values {f(a, b)} and {g(c, d)}\",\n"),
        "interpolation was broken apart: {out2:?}"
    );
}

#[test]
fn fmt_width_nested_string_inside_interpolation_is_opaque() {
    // REGRESSION (found by the corpus sweep on examples/cookbook/csv_report.op):
    // an interpolated expression containing a string literal whose CONTENT is
    // a comma — `{cv.csv_escape(tricky, ",")}`. A naive in-string flag closes
    // the outer string at the nested `"` and breaks inside the nested string,
    // changing its VALUE. The frame-stack scanner keeps the whole region
    // opaque: this line has zero break candidates and must not move.
    let src = "show(\"escaped: {cv.csv_escape(tricky, \",\")}\")\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(15));
    assert_eq!(
        out, src,
        "nested-string comma inside interpolation was broken: {out:?}"
    );

    // with a second argument the break lands after the WHOLE string
    let src2 = "show(\"escaped: {cv.csv_escape(tricky, \",\")}\", second_long_arg)\n";
    let out2 = format_program_with(&parser::parse(src2), &cfg_width(20));
    assert_eq!(
        canonical_of(&out2),
        canonical_of(src2),
        "interp+nested-string wrap changed the AST"
    );
    assert!(
        out2.contains("\",\")}\",\n"),
        "expected the break after the closing quote: {out2:?}"
    );
}

#[test]
fn fmt_width_gene_params_and_defaults_wrap() {
    let src = "gene render(width_x = 1111111111, height_y = 2222222222) {\n  show(1)\n}\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(24));
    assert_eq!(
        canonical_of(&out),
        canonical_of(src),
        "gene param wrap changed the AST"
    );
    assert!(
        out.contains("width_x = 1111111111,\n"),
        "expected a break after the first param: {out:?}"
    );
}

#[test]
fn fmt_width_nested_calls_wrap_innermost_first_after_outer() {
    // outer group breaks first (smallest depth); inner groups only if a
    // continuation segment still overflows (recursion)
    let src = "render(inner_call(alpha, beta), other(inner_two(x, y)))\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(30));
    assert_eq!(
        canonical_of(&out),
        canonical_of(src),
        "nested wrap changed the AST"
    );
    // outer commas broken → each argument starts its own line
    assert!(
        out.starts_with("render(inner_call(alpha, beta),\n"),
        "expected outer break: {out:?}"
    );
}

#[test]
fn fmt_width_unbreakable_lines_stay_long() {
    // no brackets, no commas, no candidates — honest no-op
    let src = "let very_long_identifier_name_here = some_other_long_identifier_value\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(20));
    assert_eq!(out, src, "unbreakable line must stay long");
}

#[test]
fn fmt_width_index_and_paren_wraps_stay_parser_safe() {
    // index brackets contain no top-level comma, but nested CALL commas may
    // break — the newline lands inside the call parens (tolerated), never
    // directly inside the index brackets (untolerated)
    let src = "matrix[render(first_argument, second_argument, third_argument)]\n";
    let out = format_program_with(&parser::parse(src), &cfg_width(20));
    assert_eq!(
        canonical_of(&out),
        canonical_of(src),
        "index+call wrap changed the AST"
    );

    // parenthesized single expressions never get a newline right after `(`:
    // a wrapped call inside keeps its breaks after commas only
    let src2 = "result = (render(first_argument, second_argument) + offset_value)\n";
    let out2 = format_program_with(&parser::parse(src2), &cfg_width(25));
    assert_eq!(
        canonical_of(&out2),
        canonical_of(src2),
        "paren-expr wrap changed the AST"
    );
}

#[test]
fn fmt_width_extreme_widths_terminate_and_stay_legal() {
    // width 1: every breakable comma chosen, recursion to max depth, and the
    // output STILL satisfies AST identity + zero notes + idempotence
    let src = "render(alpha, beta, gamma, delta, epsilon)\n";
    let cfg = cfg_width(1);
    let out = format_program_with(&parser::parse(src), &cfg);
    assert_eq!(
        canonical_of(&out),
        canonical_of(src),
        "width=1 wrap changed the AST"
    );
    assert_eq!(
        note_fingerprint(&parser::parse(&out)),
        Vec::<(u8, String)>::new(),
        "width=1 wrap produced notes"
    );
    let twice = format_program_with(&parser::parse(&out), &cfg);
    assert_eq!(out, twice, "width=1 wrap is not idempotent");
}

#[test]
fn fmt_width_config_file_key() {
    // width = N enables the pass
    let (cfg, unknown) = parse_fmt_config("width = 80\n");
    assert_eq!(cfg.width, Some(80));
    assert!(unknown.is_empty(), "unexpected unknown keys: {unknown:?}");
    // width = 0 is the explicit-off spelling
    let (cfg0, unknown0) = parse_fmt_config("width = 0\n");
    assert_eq!(cfg0.width, None);
    assert!(unknown0.is_empty(), "unexpected unknown keys: {unknown0:?}");
    // junk is reported, never swallowed
    let (cfgj, unknownj) = parse_fmt_config("width = banana\n");
    assert_eq!(cfgj.width, None);
    assert_eq!(unknownj.len(), 1, "junk width must be reported");
    // absent key leaves the default (off)
    let (cfgd, _) = parse_fmt_config("indent = 3\n");
    assert_eq!(cfgd.width, None);
}
