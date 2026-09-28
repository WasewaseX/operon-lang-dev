//! W65 (ROADMAP-100): the `operon fix` migrator, enforced by law.
//!
//! Law 1 — meaning preservation (corpus-wide): for EVERY checked-in .op file,
//! the canonical form of the fixed source equals the canonical form of the
//! original: canonical(fix(x)) == canonical(x). fix is a surface-syntax
//! migrator, never a semantic rewriter.
//!
//! Law 2 — idempotence: fix(fix(x)) == fix(x). A migrator that oscillates is
//! a diff machine.
//!
//! Law 3 — canonical output: fix output re-parses with zero rung-2+ repair
//! notes (the migration consumed the legacy surface it exists to consume).
//!
//! Targeted unit tests pin each v1 migration (s::→dot, synonym
//! canonicalization) including string/comment awareness. The const→let
//! migration is RETIRED (W05 red-main r5 hotfix): `const` is live
//! semantics — immutable binding + deep freeze — so a fixer rewrite to
//! `let` would be a MEANING change, forbidden by law 1. A dedicated test
//! pins the retirement.

use operon::parser;
use operon::tools::{fix_source, format_program};

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
                    // redteam payloads are adversarial BYTES: their identity
                    // is the test, migrating them is meaningless
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

#[test]
fn law1_fix_never_changes_canonical_meaning_corpus_wide() {
    for path in corpus() {
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue, // unreadable (binary-ish) payloads are another lane's contract
        };
        let (fixed, _rep) = fix_source(&src);
        let canon_before = format_program(&parser::parse(&src));
        let canon_after = format_program(&parser::parse(&fixed));
        assert_eq!(
            canon_before,
            canon_after,
            "fix changed program meaning for {}",
            path.display()
        );
    }
}

#[test]
fn law2_fix_is_idempotent() {
    let legacy = "const x = 10\ngene g() { var y = retrn_value::get() }";
    let (once, _) = fix_source(legacy);
    let (twice, rep2) = fix_source(&once);
    assert_eq!(once, twice, "second fix pass must be a no-op");
    assert_eq!(rep2.const_to_let, 0, "no const left to migrate");
    assert_eq!(rep2.s_dot, 0, "no :: left to migrate");
}

#[test]
fn law3_fix_output_reparses_canonical() {
    // W05 hotfix: the const→let migration is RETIRED — const is live
    // semantics (immutable binding + deep freeze); rewriting it would be a
    // meaning change (law 1). The s::→dot migration (dx-r3 legacy) remains.
    let legacy = "\
const limit = 3
gene scaled(x) { var base = config::limit return base + x }
";
    let (fixed, rep) = fix_source(legacy);
    assert_eq!(rep.const_to_let, 0, "const migration retired — always 0");
    assert_eq!(rep.s_dot, 1);
    assert!(rep.canonicalized >= 1, "var synonym must be counted");
    let prog = parser::parse(&fixed);
    let repairs: Vec<_> = prog.notes.iter().filter(|n| n.rung >= 2).collect();
    assert!(
        repairs.is_empty(),
        "fixed source must parse canonical, got: {:?}",
        repairs
    );
    assert!(
        fixed.contains("const limit"),
        "live const must survive fix: {}",
        fixed
    );
    assert!(!fixed.contains("::"), ":: must be gone");
    assert!(fixed.contains(".limit"), "{}", fixed);
}

#[test]
fn const_is_live_fix_never_rewrites_it() {
    // W05 red-main r5: const carries freeze semantics; a fixer rewrite to
    // let would unfreeze the binding — a MEANING change, forbidden by law 1.
    // This is the pin that failed CI on ecbff93 (const_freeze.op corpus law).
    let src = "const x = 10";
    let (fixed, rep) = fix_source(src);
    assert_eq!(rep.const_to_let, 0, "retired migration is always 0");
    assert!(
        fixed.contains("const x"),
        "const binding must survive: {}",
        fixed
    );
    let canon_before = format_program(&parser::parse(src));
    let canon_after = format_program(&parser::parse(&fixed));
    assert_eq!(canon_before, canon_after, "law 1 holds on const programs");
}

#[test]
fn fix_respects_strings_and_comments() {
    let src = r#"
gene g() {
  let s = "const x = 1 and a::b stay verbatim"
  let t = "escaped quote \" inside"
  let r = r"raw const and a::b untouched"
  # const and x::y in a comment stay
  return s + t + r
}
"#;
    let (fixed, rep) = fix_source(src);
    assert_eq!(rep.const_to_let, 0);
    assert_eq!(rep.s_dot, 0);
    // string CONTENTS survive (raw strings re-emit as canonically escaped
    // plain strings — the formatter's job — so the r-prefix itself is gone)
    assert!(
        fixed.contains("const x = 1 and a::b stay verbatim"),
        "{}",
        fixed
    );
    assert!(fixed.contains("raw const and a::b untouched"), "{}", fixed);
    // comment TEXT is never migrated (counts stay 0 above) — though the
    // canonical formatter re-emission drops plain `#` comments entirely,
    // exactly as `operon fmt` does (doc comments ride PR #20's W074 table)
    assert_eq!(
        format_program(&parser::parse(src)),
        format_program(&parser::parse(&fixed))
    );
}

#[test]
fn fix_triple_quoted_strings_survive() {
    let src = "gene g() { let t = \"\"\"const and x::y inside a triple\"\"\" return t }";
    let (fixed, rep) = fix_source(src);
    assert_eq!(rep.const_to_let, 0);
    assert_eq!(rep.s_dot, 0);
    assert!(
        fixed.contains("const and x::y inside a triple"),
        "{}",
        fixed
    );
}

#[test]
fn fix_dry_run_semantics_via_report_counts() {
    // nothing legacy → counts zero; layout may still canonicalize (that is
    // the fmt pass doing its job), so identity is pinned on the CANONICAL
    // form, not the byte form
    let canonical = "gene g() { return 1 }\n";
    let (fixed, rep) = fix_source(canonical);
    assert_eq!(rep.const_to_let, 0);
    assert_eq!(rep.s_dot, 0);
    assert_eq!(rep.canonicalized, 0);
    assert_eq!(
        fixed,
        format_program(&parser::parse(canonical)),
        "fix output IS the canonical form of the input"
    );
}

#[test]
fn fix_never_migrates_use_paths() {
    // W025 made `::` exact sugar in use paths; the legacy `expr::field`
    // migration is for method-call syntax only. Rewriting a use path to
    // dots changes the canonical form (law 1) even though the meaning is
    // the same. Pin: use lines stay verbatim, call sites still migrate.
    let src = "use a::b::c\nuse std/set as s2\nuse std/path\nlet v = x::y()";
    let (fixed, rep) = fix_source(src);
    // fmt prints use paths in its canonical spelling (:: and / render alike),
    // so the pin is on the DOTS never appearing: the buggy pass rewrote
    // `use a::b::c` to `use a.b.c`, which changed the canonical form (law 1)
    // even though the meaning was the same.
    assert!(
        !fixed.contains("use a.b.c") && !fixed.contains("use std.path"),
        "use path must not be migrated: {}",
        fixed
    );
    assert!(
        fixed.contains("x.y()"),
        "method-call :: still migrates: {}",
        fixed
    );
    assert_eq!(rep.s_dot, 1, "only the call-site migration counts");
    // law 3: the output reparses with zero rung-2+ repairs (the legacy
    // surface was consumed). Law 1 does NOT apply to this input by design:
    // `x::y()` parses as two repaired statements, `x.y()` as one method
    // call, and replacing the legacy spelling with the supported one is
    // exactly the migration's job. Use PATHS are different: there `::` is
    // current sugar, which is why they must stay verbatim.
    let prog = parser::parse(&fixed);
    let repairs: Vec<_> = prog.notes.iter().filter(|n| n.rung >= 2).collect();
    assert!(
        repairs.is_empty(),
        "fixed output must parse canonical: {:?}",
        repairs
    );
}
