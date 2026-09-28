//! W067 stage 2 — node-addressed `.rna` engine (`rna2`).
//!
//! Patches carrying a `syntax: v2` header dispatch to the node-addressed
//! engine: parse the CURRENT source fresh, address declarations by
//! name(+ordinal), mutate the AST, reprint via the canonical formatter.
//! Safety contract: all-or-nothing (any miss → nothing written), bare-name
//! ambiguity refuses (ordinal guide given), plain `#` comments refuse
//! (reprint drops them), body replacements parse FIRST. Patches WITHOUT the
//! header keep v1 text semantics byte-compatible.

use operon::rna2::{apply_rna_v2, is_v2_patch, parse_v2_patch, plain_comment_lines};
use std::process::Command;

const SRC: &str = r#"gene alpha(x) {
  return x + 1
}

gene alpha(x) {
  return x + 100
}

gene beta(y) {
  let r = alpha(y)
  return r * 2
}
"#;

// ------------------------------------------------------------ header + patch parser

#[test]
fn header_detection() {
    assert!(is_v2_patch("syntax: v2\nrename gene a -> b"));
    assert!(is_v2_patch("# lead comment\n\nsyntax: v2\n"));
    assert!(
        !is_v2_patch("edit f { replace \"a\" -> \"b\" }"),
        "v1 patch stays v1"
    );
    assert!(!is_v2_patch(""), "empty patch is v1 (not an error)");
    assert!(!is_v2_patch("syntax: v3\n"), "unknown version is not v2");
}

#[test]
fn patch_parser_rules_and_errors() {
    let rules = parse_v2_patch(
        "syntax: v2\nrename gene a#2 -> b\ndelete splice hom\nbody gene g {\nreturn 1\n}\n",
    )
    .expect("parses");
    assert_eq!(rules.len(), 3);

    // missing header
    assert!(parse_v2_patch("rename gene a -> b").is_err());
    // unknown verb
    assert!(parse_v2_patch("syntax: v2\neat gene a").is_err());
    // rename without arrow
    assert!(parse_v2_patch("syntax: v2\nrename gene a b").is_err());
    // regulate without ordinal
    assert!(parse_v2_patch("syntax: v2\ndelete regulate").is_err());
    // variant without dot
    assert!(parse_v2_patch("syntax: v2\ndelete variant homv1").is_err());
    // unclosed body
    assert!(parse_v2_patch("syntax: v2\nbody gene g {\nreturn 1\n").is_err());
    // zero ordinal
    assert!(parse_v2_patch("syntax: v2\nrename gene a#0 -> b").is_err());
}

// ------------------------------------------------------------ comment guard

#[test]
fn comment_guard_lines() {
    assert_eq!(plain_comment_lines("# plain\n"), vec![1]);
    assert!(
        plain_comment_lines("## doc\n").is_empty(),
        "## is a doc comment"
    );
    assert!(
        plain_comment_lines("let s = \"no # comment inside string\"\n").is_empty(),
        "# inside a string literal is not a comment"
    );
    assert_eq!(plain_comment_lines("let x = 1 # trailing\n"), vec![1]);
    assert!(
        plain_comment_lines("let s = \"\"\"multi\n# not a comment\n\"\"\"\n").is_empty(),
        "hash inside a multiline string is not a comment"
    );
}

#[test]
fn apply_refuses_plain_comments_without_flag() {
    let src = "# header comment\ngene f() {\n  return 1\n}\n";
    let patch = "syntax: v2\ndelete gene f\n";
    let err = apply_rna_v2(src, patch, false).expect_err("refused");
    assert!(
        err.contains("plain '#' comments"),
        "refusal names the hazard: {err}"
    );
    assert!(err.contains("lines [1]"), "refusal lists the lines: {err}");
    // with the flag the same apply proceeds
    let rep = apply_rna_v2(src, patch, true).expect("allowed under the flag");
    assert!(!rep.new_text.as_deref().unwrap_or("").contains("gene f()"));
}

// ------------------------------------------------------------ rename

#[test]
fn rename_gene_rewrites_decl_and_call_sites() {
    // Renaming alpha#2 -> big rewrites the decl AND every Ident of the old
    // name — beta's `alpha(y)` call follows the name to `big` (documented:
    // identifier-precise, not scope-aware; strictly safer than v1's
    // substring replace, same author responsibility).
    let patch = "syntax: v2\nrename gene alpha#2 -> big\n";
    let rep = apply_rna_v2(SRC, patch, false).expect("applies");
    let out = rep.new_text.expect("printed");
    assert!(out.contains("gene big(x)"), "decl renamed:\n{out}");
    assert!(
        out.contains("let r = big(y)"),
        "beta's call follows the name:\n{out}"
    );
    assert!(
        out.contains("gene alpha(x)"),
        "alpha#1 keeps its name:\n{out}"
    );
    let rule = &rep.rules[0];
    assert!(rule.target_found && rule.applied);
    assert!(
        rule.detail.contains("decl + 1 reference node(s)"),
        "{}",
        rule.detail
    );
}

#[test]
fn rename_moves_call_sites_of_the_old_name() {
    // rename BOTH alphas is refused (bare name, two decls)…
    let patch = "syntax: v2\nrename gene alpha -> solo\n";
    let rep = apply_rna_v2(SRC, patch, false).expect("report (refused)");
    assert!(rep.new_text.is_none(), "all-or-nothing: nothing written");
    assert!(!rep.rules[0].target_found);
    assert!(
        rep.rules[0].detail.contains("ambiguous"),
        "{}",
        rep.rules[0].detail
    );
    assert!(rep.rules[0].detail.contains("alpha#1") && rep.rules[0].detail.contains("alpha#2"));
    assert_eq!(rep.missed(), 1);

    // …then rename the first decl by ordinal: beta's call site (an Ident
    // named `alpha`) follows the name to `first`.
    let patch2 = "syntax: v2\nrename gene alpha#1 -> first\n";
    let rep2 = apply_rna_v2(SRC, patch2, false).expect("applies");
    let out2 = rep2.new_text.expect("printed");
    assert!(out2.contains("gene first(x)"), "decl renamed:\n{out2}");
    assert!(
        out2.contains("let r = first(y)"),
        "call site rewritten:\n{out2}"
    );
    assert!(rep2.rules[0].detail.contains("decl + 1 reference node(s)"));
}

#[test]
fn rename_phenotype_rewrites_constructors_and_annotations() {
    let src = "phenotype Cell {\n  let m = 1\n}\n\nphenotype Lab {\n  let c = 2\n}\n\ngene make(n) {\n  let x = new Cell()\n  return n\n}\n";
    let patch = "syntax: v2\nrename phenotype Cell -> Organelle\n";
    let rep = apply_rna_v2(src, patch, false).expect("applies");
    let out = rep.new_text.expect("printed");
    assert!(
        out.contains("phenotype Organelle {"),
        "decl renamed:\n{out}"
    );
    assert!(
        out.contains("new Organelle()"),
        "constructor rewritten:\n{out}"
    );
    assert!(out.contains("phenotype Lab {"), "sibling untouched:\n{out}");
}

#[test]
fn rename_fate_rewrites_call_idents() {
    // Fate call sites are plain Ident nodes (FateNew is fabricated at
    // runtime by call_value) — the ident rewrite moves them.
    let src = "fate Cycle {\n  state gap1 -> synthesis\n  enter gap1\n}\n\ngene probe() {\n  return Cycle() != null\n}\n";
    let patch = "syntax: v2\nrename fate Cycle -> Loop\n";
    let rep = apply_rna_v2(src, patch, false).expect("applies");
    let out = rep.new_text.expect("printed");
    assert!(out.contains("fate Loop {"), "decl renamed:\n{out}");
    assert!(
        out.contains("return Loop() != null"),
        "call-site ident rewritten:\n{out}"
    );
}

#[test]
fn rename_splice_moves_root_and_references() {
    let src = "splice hom {\n  variant v1 { return 11 }\n  variant v2 { return 22 }\n}\n\ngene use() {\n  return hom()\n}\n";
    let patch = "syntax: v2\nrename splice hom -> hum\nrename variant hum.v1 -> fast\n";
    let rep = apply_rna_v2(src, patch, false).expect("applies");
    let out = rep.new_text.expect("printed");
    assert!(out.contains("splice hum {"), "root renamed:\n{out}");
    assert!(out.contains("return hum()"), "reference rewritten:\n{out}");
    // variant rename composed on the already-renamed root (sequential
    // resolution against the edited AST)
    assert!(out.contains("variant fast {"), "variant renamed:\n{out}");
    assert!(
        !out.contains("variant v1 {"),
        "old variant name gone:\n{out}"
    );
}

// ------------------------------------------------------------ delete

#[test]
fn delete_is_all_or_nothing_and_honest() {
    // deleting a nonexistent decl refuses the whole patch
    let patch = "syntax: v2\ndelete gene beta\ndelete gene ghost\n";
    let rep = apply_rna_v2(SRC, patch, false).expect("report");
    assert!(rep.new_text.is_none(), "refused: nothing written");
    assert!(
        rep.rules[0].target_found,
        "resolvable rule still reports ok"
    );
    assert!(!rep.rules[1].target_found);
    assert_eq!(rep.missed(), 1);

    // ordinal out of range
    let patch2 = "syntax: v2\ndelete gene alpha#5\n";
    let rep2 = apply_rna_v2(SRC, patch2, false).expect("report");
    assert!(
        rep2.rules[0].detail.contains("out of range"),
        "{}",
        rep2.rules[0].detail
    );
}

#[test]
fn delete_variant_and_regulate_ordinal() {
    let src = "splice hom {\n  variant v1 { return 11 }\n  variant v2 { return 22 }\n}\n\nregulate {\n  a activates b\n}\n\nregulate {\n  c inhibits d strength 0.5\n}\n";
    // variant present
    let patch = "syntax: v2\ndelete variant hom.v2\n";
    let rep = apply_rna_v2(src, patch, false).expect("applies");
    let out = rep.new_text.expect("printed");
    assert!(out.contains("variant v1 {"), "survivor stays:\n{out}");
    assert!(!out.contains("variant v2 {"), "target gone:\n{out}");
    // variant absent → miss
    let rep2 = apply_rna_v2(src, "syntax: v2\ndelete variant hom.vX\n", false).expect("report");
    assert!(
        rep2.rules[0].detail.contains("no variant"),
        "{}",
        rep2.rules[0].detail
    );

    // regulate #2 removes the SECOND regulate statement
    let patch3 = "syntax: v2\ndelete regulate #2\n";
    let rep3 = apply_rna_v2(src, patch3, false).expect("applies");
    let out3 = rep3.new_text.expect("printed");
    assert!(
        out3.contains("activates"),
        "first regulate survives:\n{out3}"
    );
    assert!(
        !out3.contains("inhibits"),
        "second regulate removed:\n{out3}"
    );
    // bare/zero ordinal already rejected by the parser; out-of-range:
    let rep4 = apply_rna_v2(src, "syntax: v2\ndelete regulate #9\n", false).expect("report");
    assert!(
        rep4.rules[0].detail.contains("does not exist"),
        "{}",
        rep4.rules[0].detail
    );
}

// ------------------------------------------------------------ body replacement

#[test]
fn body_replacement_parses_first_and_runs() {
    // the replacement is parsed BEFORE any target lookup — a parse error
    // refuses the whole apply even when the target exists
    let patch = "syntax: v2\nbody gene beta {\n  let s = )))((\n}\n";
    let rep = apply_rna_v2(SRC, patch, false).expect("report");
    assert!(rep.new_text.is_none(), "parse error refuses everything");
    assert!(
        rep.rules[0].detail.contains("does not parse cleanly"),
        "{}",
        rep.rules[0].detail
    );

    // a clean replacement lands: beta sums 1..=y
    let patch2 = "syntax: v2\nbody gene beta {\n  let s = 0\n  for i in range(1, y + 1) {\n    s = s + i\n  }\n  return s\n}\n";
    let rep2 = apply_rna_v2(SRC, patch2, false).expect("applies");
    let out2 = rep2.new_text.expect("printed");
    assert!(out2.contains("s = s + i"), "new body present:\n{out2}");
    assert!(
        !out2.contains("let r = alpha(y)"),
        "beta's OLD body is gone (whole-body swap):\n{out2}"
    );
    assert!(
        out2.contains("return x + 1"),
        "other genes untouched:\n{out2}"
    );
    assert!(
        out2.contains("return x + 100"),
        "other genes untouched:\n{out2}"
    );
}

// ------------------------------------------------------------ reprint properties

#[test]
fn reprint_is_fmt_stable() {
    // apply output goes through format_program, so fmt∘apply == apply.
    let patch = "syntax: v2\nrename gene alpha#2 -> big\n";
    let rep = apply_rna_v2(SRC, patch, false).expect("applies");
    let once = rep.new_text.expect("printed");
    let twice = operon::tools::format_program(&operon::parser::parse(&once));
    assert_eq!(once, twice, "apply output is a fmt fixpoint");
}

#[test]
fn v1_semantics_untouched_without_header() {
    // no `syntax: v2` header → the v1 checked engine, byte-compatible
    let patch = "edit alpha { replace \"x + 1\" -> \"x + 7\" }";
    // (v1 dispatch happens in the CLI; here assert is_v2_patch routes NO)
    assert!(!is_v2_patch(patch));
}

// ------------------------------------------------------------ stage 3 step 1: v1 deprecation marker (W63 policy, info)

fn run_rna_cli(
    dir: &std::path::Path,
    src_name: &str,
    patch_name: &str,
    extra: &[&str],
) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("rna")
        .arg(dir.join(src_name))
        .arg(dir.join(patch_name))
        .args(extra)
        .output()
        .expect("run operon rna");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    // per-CALL unique dir (the 2026-09-27 Windows-parallel lesson): nanos+pid
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir().join(format!(
        "operon_rna3_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn v1_cli_json_carries_deprecation_marker() {
    let dir = unique_dir("json");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    std::fs::write(&src, "gene alpha(x) {\n  return x + 1\n}\n").unwrap();
    std::fs::write(&patch, "edit alpha { replace \"x + 1\" -> \"x + 7\" }").unwrap();
    let (code, stdout, stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--json"]);
    assert_eq!(code, 0, "v1 apply must succeed: stdout={}", stdout);
    assert!(
        stdout.contains("\"engine\":\"v1\""),
        "v1 JSON must self-identify: {}",
        stdout
    );
    assert!(
        stdout.contains("\"deprecated\":true"),
        "v1 JSON must carry the deprecation flag: {}",
        stdout
    );
    assert!(
        stdout.contains("\"applied\":1"),
        "the edit itself still applies: {}",
        stdout
    );
    assert!(
        stderr.contains("syntax: v2"),
        "stderr note must point at the v2 migration: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v1_cli_human_note_goes_to_stderr_only() {
    let dir = unique_dir("human");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    std::fs::write(&src, "gene alpha(x) {\n  return x + 1\n}\n").unwrap();
    std::fs::write(&patch, "edit alpha { replace \"x + 1\" -> \"x + 7\" }").unwrap();
    let (code, stdout, stderr) = run_rna_cli(&dir, "app.op", "p.rna", &[]);
    assert_eq!(code, 0);
    // stdout stays the report; the deprecation note is stderr-only
    assert!(
        stdout.starts_with("rna: "),
        "stdout report unchanged: {}",
        stdout
    );
    assert!(
        stdout.contains("1 applied"),
        "report shows the apply: {}",
        stdout
    );
    assert!(
        stderr.contains("deprecated (info, W63 step 1)"),
        "note present: {}",
        stderr
    );
    assert!(
        !stdout.contains("deprecated"),
        "no deprecation text leaks into stdout: {}",
        stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v2_cli_json_is_not_marked_deprecated() {
    let dir = unique_dir("v2");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    std::fs::write(&src, "gene alpha(x) {\n  return x + 1\n}\n").unwrap();
    std::fs::write(&patch, "syntax: v2\nrename gene alpha -> alpha2").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--json"]);
    assert_eq!(code, 0, "v2 rename must succeed: {}", stdout);
    assert!(
        stdout.contains("\"engine\":\"v2\""),
        "v2 JSON self-identifies: {}",
        stdout
    );
    assert!(
        !stdout.contains("\"deprecated\":true"),
        "v2 is NOT deprecated: {}",
        stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v1_write_output_bytes_unchanged_by_deprecation() {
    let dir = unique_dir("bytes");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "edit alpha { replace \"x + 1\" -> \"x + 7\" }").unwrap();
    let (code, _stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--write"]);
    assert_eq!(code, 0);
    let after = std::fs::read_to_string(&src).unwrap();
    // the deprecation is METADATA ONLY: the file contract is byte-identical to
    // the pre-deprecation v1 engine (notes go to stderr, flags to --json)
    assert_eq!(after, "gene alpha(x) {\n  return x + 7\n}\n");
    let _ = std::fs::remove_dir_all(&dir);
}
