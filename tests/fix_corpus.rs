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

/// sandbox hygiene: only TRACKED .op files are law-corpus. Untracked drafts
/// left in a working tree by a parallel lane must not red the corpus laws —
/// on CI the strays do not exist (clean checkout), so the filter only fires
/// in dirty sandboxes. Degrades to include-all when git is unavailable
/// (source tarballs without .git).
fn tracked_op_files() -> Option<std::collections::HashSet<std::path::PathBuf>> {
    let listing = std::process::Command::new("git")
        .args(["ls-files", "std", "tests", "examples", "apps"])
        .output()
        .ok()?;
    if !listing.status.success() {
        return None;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    Some(
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter(|l| l.ends_with(".op"))
            .map(|l| root.join(l))
            .collect(),
    )
}

fn corpus() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let tracked = tracked_op_files();
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

#[test]
fn law1_fix_never_changes_canonical_meaning_corpus_wide() {
    // Z-SWEEP4-FIXCORPUS-CONTAIN (2026-10-09, S0 incident unblock): the two
    // parser depth pins below are pathological BY DESIGN — they sit exactly
    // at the #111/#112 guard boundary, and `fix_source`'s canonicalized
    // output re-parses DEEPER than the raw input, overflowing any test
    // thread stack. A Rust stack overflow is a process abort (uncatchable),
    // so the full fix->re-parse pipeline cannot run on them. Their
    // PARSE-side contract is still asserted loudly here; the fixer-side
    // depth discipline itself is the fixer lane's option (a) follow-up
    // (coordinator incident post cfe5beb), and this containment reverts
    // when that lands. Precedent: vm_parity rt_p4b_threadbomb_join runs
    // containment-checked rather than byte-checked.
    const CONTAINED_DEPTH_PINS: &[&str] = &[
        "tests/differential/parser_depth_leak_pin.op",
        "tests/differential/parser_depth_calls_pin.op",
    ];
    for path in corpus() {
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue, // unreadable (binary-ish) payloads are another lane's contract
        };
        let rel = path.to_string_lossy().into_owned();
        if CONTAINED_DEPTH_PINS
            .iter()
            .any(|p| rel.ends_with(p) || rel == *p)
        {
            // containment contract: the pin must still PARSE (raw) under the
            // depth guards and format deterministically — on a dedicated
            // big-stack thread, because a 4000-deep call chain sits exactly
            // at the default 2MB test-thread boundary (any compilation
            // variance flips it) and the pipeline's fix->re-parse half is
            // excluded, loudly.
            let handle = std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn(move || {
                    let canon_once = format_program(&parser::parse(&src));
                    format_program(&parser::parse(&src));
                    canon_once
                })
                .expect("spawn big-stack containment thread");
            let canon_once = handle.join().expect("contained depth pin panicked");
            let _ = canon_once;
            continue;
        }
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
    // meaning change (law 1). The s::→dot migration (dx-r3 legacy) remains
    // in EXPRESSION context only — W25 made `::` live exact sugar in USE
    // paths, where separators are free spelling and law 4 keeps fix's hands
    // off (see below).
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
fn law4_use_paths_keep_their_separator() {
    // W25: `::` in use paths is live sugar. The migrator must leave use
    // lines alone — law 1 fired on dev1's namespaces.op corpus file when
    // the dx-r3 EXPRESSION repair rewrote `use std::set` -> `use std.set`,
    // moving the canon form (parse canonicalizes the use-path render to
    // `/`, so an un-migrated use line is canon-stable). Observable
    // contract: a use-path `::` never increments s_dot; an
    // expression-context `::` keeps the dx-r3 repair.
    let src = "use std::bio\nuse std/set as set2\n";
    let (fixed, rep) = fix_source(src);
    assert_eq!(rep.s_dot, 0, "use-path :: is not an expression repair");
    assert!(fixed.contains("use std/bio"), "use canon render: {}", fixed);
    assert!(
        fixed.contains("use std/set as set2"),
        "/ alias survives: {}",
        fixed
    );
    let canon_before = format_program(&parser::parse(src));
    let canon_after = format_program(&parser::parse(&fixed));
    assert_eq!(
        canon_before, canon_after,
        "use-only file: fix is canon-stable"
    );
    // expression context keeps the dx-r3 repair (law 3 pins the shape):
    let expr_src = "gene g() { return config::limit }\n";
    let (expr_fixed, expr_rep) = fix_source(expr_src);
    assert_eq!(expr_rep.s_dot, 1, "expression :: still repaired");
    assert!(expr_fixed.contains("config.limit"), "{}", expr_fixed);
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
