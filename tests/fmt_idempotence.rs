//! W47 (ROADMAP-100): the formatter byte-stability law, enforced over the
//! whole checked-in corpus.
//!
//! Law 1 — idempotence: fmt(fmt(x, cfg), cfg) == fmt(x, cfg) for EVERY
//! supported config. A formatter that oscillates is a diff machine, not a
//! formatter.
//!
//! Law 2 — formatting only: fmt output re-parses with no rung-3/rung-4 notes
//! (never burns, never falls back). Config changes may rearrange whitespace
//! and quote spelling; they may never alter the token stream's meaning.
//!
//! Runs over std/, tests/ (minus redteam — adversarial bytes are a different
//! lane), examples/, apps/. Default config plus the two extreme knobs
//! (indent 4 + single quotes) so both the law and the quote-decision logic
//! hold corpus-wide, on every platform (part of the windows-blocking gate).

use operon::parser;
use operon::tools::{format_program, format_program_with, FmtConfig, QuoteMode};

fn corpus() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // sandbox hygiene: only TRACKED .op files are the stability corpus —
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
                    // redteam payloads are adversarial BYTES (BOM/CRLF attacks):
                    // their file identity is the test, formatting them is meaningless
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

fn assert_stable(src: &str, cfg: &FmtConfig, path: &std::path::Path) {
    let once = format_program_with(&parser::parse(src), cfg);
    let twice = format_program_with(&parser::parse(&once), cfg);
    assert_eq!(
        once,
        twice,
        "formatter is not idempotent under {cfg:?} for {}",
        path.display()
    );
    // Law 2: the formatted file must parse without falling back (rung 4).
    // Under Single-quote mode the EXPECTED quote-repair note (rung 3) is
    // allowed by design: single-quoted re-emission is lossless but
    // non-canonical, and the lexer says so on every re-parse. Default mode
    // stays strict — nothing rung>=3 at all.
    let reparsed = parser::parse(&once);
    let severe: Vec<String> = reparsed
        .notes
        .iter()
        .filter(|n| n.rung >= 3)
        .map(|n| n.message.clone())
        .filter(|m| {
            cfg.quotes != QuoteMode::Single || m != "single-quoted string repaired to double quotes"
        })
        .collect();
    assert!(
        severe.is_empty(),
        "fmt output for {} produced rung>=3 notes: {severe:?}",
        path.display()
    );
}

#[test]
fn fmt_idempotence_corpus_default_config() {
    let cfg = FmtConfig::default();
    let files = corpus();
    assert!(
        files.len() > 100,
        "corpus unexpectedly small: {}",
        files.len()
    );
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        assert_stable(&src, &cfg, f);
    }
}

#[test]
fn fmt_idempotence_corpus_indent4_single_quotes() {
    let cfg = FmtConfig {
        indent: 4,
        quotes: QuoteMode::Single,
        ..FmtConfig::default()
    };
    for f in corpus() {
        let src = std::fs::read_to_string(&f).unwrap_or_default();
        assert_stable(&src, &cfg, &f);
    }
}

#[test]
fn fmt_quote_decision_is_lossless_and_idempotent() {
    // plain text -> single under Single mode
    let cfg_s = FmtConfig {
        indent: 2,
        quotes: QuoteMode::Single,
        ..FmtConfig::default()
    };
    let out = format_program_with(&parser::parse("show(\"plain text\")"), &cfg_s);
    assert!(
        out.contains("'plain text'"),
        "expected single quotes: {out}"
    );
    // content with a single-quote char must fall back to double (lossless rule)
    let out2 = format_program_with(&parser::parse("show(\"it's here\")"), &cfg_s);
    assert!(
        out2.contains("\"it's here\""),
        "expected double fallback: {out2}"
    );
    // interpolation always stays double (canonical form for interpolated strings)
    let out3 = format_program_with(&parser::parse("show(\"hi {name}\")"), &cfg_s);
    assert!(
        out3.contains("\"hi {name}\""),
        "expected interp to stay double: {out3}"
    );
    // and the single-quoted output re-parses to the SAME value (round-trip)
    let prog = parser::parse(&out);
    assert!(
        out.contains("'plain text'"),
        "single-quote round trip changed program: {prog:?}"
    );
}

#[test]
fn fmt_indent_option_changes_nesting_only() {
    // depth-2 block: default 2-space -> 4 spaces; indent 4 -> 8 spaces
    let src = "gene f() {\n  if true {\n    show(1)\n  }\n}\n";
    let a = format_program(&parser::parse(src));
    let b = format_program_with(
        &parser::parse(src),
        &FmtConfig {
            indent: 4,
            quotes: QuoteMode::Double,
            ..FmtConfig::default()
        },
    );
    assert!(
        a.contains("\n    show(1)"),
        "default 2-space nesting: {a:?}"
    );
    assert!(b.contains("\n        show(1)"), "indent 4 nesting: {b:?}");
}

#[test]
fn fmt_multistmt_lambda_body_survives() {
    // W47-v3 P0 fmt repair regression (2026-10-04): a braced multi-statement
    // lambda used to render as `gene(..) => null` — the body's first
    // statement was kept only if it was a return, and every statement after
    // a first return was silently dropped. apps/loglens' sort comparator was
    // destroyed live by `fmt --write`. The repair law: the braced form is
    // the ONLY rendering for a body that is not exactly one `return e`.
    let src = "gene main(){\n  let f = gene(a, b) {\n    if a < b {\n      return -1\n    }\n    return a - b\n  }\n  show(f(1, 2))\n}\n";
    let once = format_program(&parser::parse(src));
    assert!(!once.contains("=> null"), "body destroyed: {once:?}");
    assert!(
        once.contains("return -1"),
        "if-branch return lost: {once:?}"
    );
    assert!(once.contains("return a - b"), "final return lost: {once:?}");
    assert!(
        once.contains("gene(a, b) {"),
        "braced form expected: {once:?}"
    );
    // Law 1: idempotent on the repaired shape.
    let twice = format_program(&parser::parse(&once));
    assert_eq!(once, twice, "not idempotent: {once:?} vs {twice:?}");
    // Law 2: clean re-parse.
    let reparsed = parser::parse(&once);
    let severe: Vec<String> = reparsed
        .notes
        .iter()
        .filter(|n| n.rung >= 3)
        .map(|n| n.message.clone())
        .collect();
    assert!(severe.is_empty(), "rung>=3 notes: {severe:?}");
    // Zero drift: the single-`return` arrow form is untouched.
    let arrow = format_program(&parser::parse(
        "gene main(){\n  show([3, 1, 2].sort(gene(a, b) { return a < b }))\n}\n",
    ));
    assert!(
        arrow.contains("gene(a, b) => a < b"),
        "arrow form drifted: {arrow:?}"
    );
}

#[test]
fn fmt_config_parser_keys_bounds_and_unknowns() {
    let (cfg, unk) = operon::tools::parse_fmt_config(
        "# .operon-fmt.toml\nindent = 4\nquotes = 'single'\n[fmt]\n",
    );
    assert_eq!(cfg.indent, 4);
    assert_eq!(cfg.quotes, QuoteMode::Single);
    assert!(unk.is_empty());
    // out-of-bounds and unknown keys are reported, not fatal.
    // (W47-v2: `width` is now a KNOWN key — 80 parses; the unknown-report
    // coverage moved to a genuinely unknown key `wrap`.)
    let (cfg2, unk2) =
        operon::tools::parse_fmt_config("indent = 99\nquotes = triple\nwidth = 80\nwrap = yes\n");
    assert_eq!(cfg2.indent, 2); // default preserved
    assert_eq!(cfg2.quotes, QuoteMode::Double);
    assert_eq!(cfg2.width, Some(80)); // W47-v2: width is a real key now
    assert_eq!(unk2.len(), 3, "all three problems reported: {unk2:?}");
}
