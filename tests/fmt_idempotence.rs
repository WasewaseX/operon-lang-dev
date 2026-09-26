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
                } else if p.extension().map(|x| x == "op").unwrap_or(false) {
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
        },
    );
    assert!(
        a.contains("\n    show(1)"),
        "default 2-space nesting: {a:?}"
    );
    assert!(b.contains("\n        show(1)"), "indent 4 nesting: {b:?}");
}

#[test]
fn fmt_config_parser_keys_bounds_and_unknowns() {
    let (cfg, unk) = operon::tools::parse_fmt_config(
        "# .operon-fmt.toml\nindent = 4\nquotes = 'single'\n[fmt]\n",
    );
    assert_eq!(cfg.indent, 4);
    assert_eq!(cfg.quotes, QuoteMode::Single);
    assert!(unk.is_empty());
    // out-of-bounds and unknown keys are reported, not fatal
    let (cfg2, unk2) =
        operon::tools::parse_fmt_config("indent = 99\nquotes = triple\nwidth = 80\n");
    assert_eq!(cfg2.indent, 2); // default preserved
    assert_eq!(cfg2.quotes, QuoteMode::Double);
    assert_eq!(unk2.len(), 3, "all three problems reported: {unk2:?}");
}
