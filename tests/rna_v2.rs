//! W067 stage 2 — node-addressed `.rna` engine (`rna2`).
//!
//! Patches carrying a `syntax: v2` header dispatch to the node-addressed
//! engine: parse the CURRENT source fresh, address declarations by
//! name(+ordinal), mutate the AST, reprint via the canonical formatter.
//! Safety contract: all-or-nothing (any miss → nothing written), bare-name
//! ambiguity refuses (ordinal guide given), plain `#` comments refuse
//! (reprint drops them), body replacements parse FIRST. Patches WITHOUT the
//! header keep v1 text semantics byte-compatible.
//!
//! W68 section: `operon rna --check` validation mode (library `check_rna_v2`
//! and the CLI): engine detection, per-rule span/node fate, ambiguity, the
//! comment preflight, would-apply verdict, JSON shapes, exit codes, no writes.

use operon::rna2::{apply_rna_v2, check_rna_v2, is_v2_patch, parse_v2_patch, plain_comment_lines};
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

// ------------------------------------------------------------ W68: check mode (validate, never apply)

#[test]
fn check_reports_ambiguity_and_clean_apply() {
    // bare name shared by two decls: check must surface the same ambiguity
    // refusal (with the ordinal guide) a real apply would raise
    let rep = check_rna_v2(SRC, "syntax: v2\nrename gene alpha -> solo\n", false);
    assert!(!rep.would_apply, "ambiguous patch does not apply cleanly");
    assert!(rep
        .reason
        .as_deref()
        .unwrap_or("")
        .contains("all-or-nothing"));
    assert_eq!(rep.rules.len(), 1);
    assert!(
        !rep.rules[0].target_found,
        "ambiguity is a resolution failure"
    );
    assert!(rep.rules[0].detail.contains("ambiguous"));
    assert!(rep.rules[0].detail.contains("alpha#1") && rep.rules[0].detail.contains("alpha#2"));
    assert!(rep.parse_error.is_none(), "the patch itself parses");

    // the ordinal-addressed variant resolves and WOULD apply
    let ok = check_rna_v2(SRC, "syntax: v2\nrename gene alpha#2 -> big\n", false);
    assert!(ok.would_apply);
    assert!(
        ok.reason.is_none(),
        "a clean check carries no refusal reason"
    );
    assert_eq!(ok.rules.len(), 1);
    assert!(ok.rules[0].target_found && ok.rules[0].applied);
    assert!(
        ok.comment_lines.is_empty() && ok.comment_refusal.is_none(),
        "no comments in SRC, preflight clean"
    );
}

#[test]
fn check_parse_error_refuses_before_rules() {
    let rep = check_rna_v2(SRC, "syntax: v2\neat gene alpha\n", false);
    assert!(!rep.would_apply);
    assert!(rep.parse_error.is_some(), "the parse error is first-class");
    assert!(
        rep.rules.is_empty(),
        "rules never ran (apply order: parse -> preflight -> rules)"
    );
    assert!(
        rep.reason
            .as_deref()
            .unwrap_or("")
            .contains("patch parse error"),
        "{}",
        rep.reason.as_deref().unwrap_or("")
    );
}

#[test]
fn check_comment_preflight_mirrors_apply() {
    let src = "# header comment\ngene f() {\n  return 1\n}\n";
    // refused: same gate order as apply, no rule fates, refusal verbatim
    let rep = check_rna_v2(src, "syntax: v2\ndelete gene f\n", false);
    assert!(!rep.would_apply);
    assert_eq!(rep.comment_lines, vec![1], "target comment lines reported");
    assert!(rep.comment_refusal.is_some());
    assert!(
        rep.rules.is_empty(),
        "preflight refuses before rule resolution, like apply"
    );
    assert_eq!(
        rep.comment_refusal.as_deref(),
        Some(operon::rna2::comment_refusal_msg(&[1]).as_str()),
        "check carries the apply refusal VERBATIM (shared helper)"
    );
    // allowed: the same check under --allow-comment-drop would apply
    let ok = check_rna_v2(src, "syntax: v2\ndelete gene f\n", true);
    assert!(ok.would_apply);
    assert!(ok.comment_refusal.is_none());
    assert_eq!(
        ok.comment_lines,
        vec![1],
        "lines stay visible under the flag"
    );
    assert_eq!(ok.rules.len(), 1, "rules ran and resolve");
}

#[test]
fn check_cli_clean_patch_writes_nothing() {
    let dir = unique_dir("chk_ok");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n\ngene beta(y) {\n  return alpha(y)\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "syntax: v2\nrename gene alpha -> gamma\n").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--check"]);
    assert_eq!(code, 0, "clean patch checks green: {}", stdout);
    assert!(stdout.contains("engine v2"), "engine detected: {}", stdout);
    assert!(stdout.contains("would apply cleanly"), "{}", stdout);
    assert!(stdout.contains("nothing written"), "{}", stdout);
    assert!(
        stdout.contains("ok   [gene alpha]"),
        "per-rule fate: {}",
        stdout
    );
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        before,
        "check NEVER writes"
    );
    // the guarantee is absolute: --check --write is a usage error (exit 2)
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("rna")
        .arg(&src)
        .arg(&patch)
        .args(["--check", "--write"])
        .output()
        .expect("run operon rna --check --write");
    assert_eq!(
        out.status.code(),
        Some(2),
        "--check + --write refuses at the CLI layer"
    );
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        before,
        "still untouched after the refused combo"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_cli_missing_span_and_ambiguous_exit_1() {
    let dir = unique_dir("chk_miss");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n\ngene beta(y) {\n  return alpha(y)\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "syntax: v2\ndelete gene ghost\ndelete gene beta\n").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--check"]);
    assert_eq!(code, 1, "a missed target fails the check: {}", stdout);
    assert!(stdout.contains("REFUSED"), "{}", stdout);
    assert!(stdout.contains("MISS [gene ghost]"), "{}", stdout);
    assert!(
        stdout.contains("ok   [gene beta]"),
        "resolvable rules still report ok: {}",
        stdout
    );
    assert_eq!(std::fs::read_to_string(&src).unwrap(), before);

    // ambiguity: bare name over two decls, ordinal guide in the detail
    let dup = dir.join("dup.op");
    let patch2 = dir.join("p2.rna");
    std::fs::write(
        &dup,
        "gene alpha(x) {\n  return x + 1\n}\n\ngene alpha(x) {\n  return x + 100\n}\n",
    )
    .unwrap();
    std::fs::write(&patch2, "syntax: v2\nrename gene alpha -> solo\n").unwrap();
    let (code2, stdout2, _stderr2) = run_rna_cli(&dir, "dup.op", "p2.rna", &["--check"]);
    assert_eq!(code2, 1, "ambiguity fails the check: {}", stdout2);
    assert!(
        stdout2.contains("ambiguous") && stdout2.contains("alpha#1") && stdout2.contains("alpha#2"),
        "ordinal guide present: {}",
        stdout2
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_cli_comment_preflight_and_flag() {
    let dir = unique_dir("chk_cmt");
    let src = dir.join("c.op");
    let patch = dir.join("p.rna");
    let before = "# header\ngene f() {\n  return 1\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "syntax: v2\ndelete gene f\n").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "c.op", "p.rna", &["--check"]);
    assert_eq!(code, 1, "comment preflight fails the check: {}", stdout);
    assert!(
        stdout.contains("plain '#' comments at lines [1]"),
        "refusal carries the apply wording verbatim: {}",
        stdout
    );
    assert_eq!(std::fs::read_to_string(&src).unwrap(), before);
    // with the flag the same check goes green (and states the drop cost)
    let (code2, stdout2, _stderr2) =
        run_rna_cli(&dir, "c.op", "p.rna", &["--check", "--allow-comment-drop"]);
    assert_eq!(code2, 0, "flag lifts the preflight: {}", stdout2);
    assert!(stdout2.contains("will be dropped"), "{}", stdout2);
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        before,
        "even a green check writes nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_cli_v2_json_shapes() {
    let dir = unique_dir("chk_json");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "syntax: v2\nrename gene alpha -> gamma\n").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--check", "--json"]);
    assert_eq!(code, 0, "{}", stdout);
    for key in [
        "\"engine\":\"v2\"",
        "\"check\":true",
        "\"would_apply\":true",
        "\"would_write\":false",
        "\"would_change\":true",
        "\"applied\":1",
        "\"missed\":0",
        "\"comment_preflight\":\"ok\"",
        "\"comment_lines\":[]",
        "\"verb\":\"rename\"",
        "\"target_found\":true",
    ] {
        assert!(stdout.contains(key), "clean JSON carries {key}: {}", stdout);
    }
    assert!(
        !stdout.contains("\"refused\""),
        "clean check is not refused: {}",
        stdout
    );

    // miss -> refused:true + reason, would_apply:false
    let patch2 = dir.join("miss.rna");
    std::fs::write(&patch2, "syntax: v2\ndelete gene ghost\n").unwrap();
    let (code2, stdout2, _stderr2) =
        run_rna_cli(&dir, "app.op", "miss.rna", &["--check", "--json"]);
    assert_eq!(code2, 1, "{}", stdout2);
    for key in [
        "\"would_apply\":false",
        "\"would_write\":false",
        "\"missed\":1",
        "\"refused\":true",
        "\"target_found\":false",
    ] {
        assert!(
            stdout2.contains(key),
            "missed JSON carries {key}: {}",
            stdout2
        );
    }
    assert!(
        stdout2.contains("\"reason\":\"all-or-nothing"),
        "{}",
        stdout2
    );

    // parse error -> refused with the patch parse error as reason
    let patch3 = dir.join("bad.rna");
    std::fs::write(&patch3, "syntax: v2\neat gene alpha\n").unwrap();
    let (code3, stdout3, _stderr3) = run_rna_cli(&dir, "app.op", "bad.rna", &["--check", "--json"]);
    assert_eq!(code3, 1, "{}", stdout3);
    assert!(
        stdout3.contains("\"reason\":\"patch parse error"),
        "{}",
        stdout3
    );
    assert!(
        stdout3.contains("\"rules\":[]"),
        "rules never ran: {}",
        stdout3
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_cli_v1_deprecation_interaction() {
    let dir = unique_dir("chk_v1");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "edit alpha { replace \"x + 1\" -> \"x + 7\" }").unwrap();
    let (code, stdout, stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--check", "--json"]);
    assert_eq!(code, 0, "v1 clean check exits 0: {}", stdout);
    for key in [
        "\"engine\":\"v1\"",
        "\"deprecated\":true",
        "\"check\":true",
        "\"would_apply\":true",
        "\"would_write\":false",
        "\"applied\":1",
        "\"hits\":1",
    ] {
        assert!(
            stdout.contains(key),
            "v1 check JSON carries {key}: {}",
            stdout
        );
    }
    assert!(
        stderr.contains("deprecated (info, W63 step 1)"),
        "the v1 deprecation note survives in check mode: {}",
        stderr
    );
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        before,
        "v1 check writes nothing"
    );

    // v1 miss: exit 1, miss row, still deprecated metadata
    let patch2 = dir.join("miss.rna");
    std::fs::write(&patch2, "edit ghost { replace \"x\" -> \"y\" }").unwrap();
    let (code2, stdout2, _stderr2) =
        run_rna_cli(&dir, "app.op", "miss.rna", &["--check", "--json"]);
    assert_eq!(code2, 1, "{}", stdout2);
    assert!(stdout2.contains("\"would_apply\":false"), "{}", stdout2);
    assert!(stdout2.contains("\"missed\":1"), "{}", stdout2);

    // v1 multi-hit: hit counts stay in the row (board: hit/miss/multi-hit)
    let two = dir.join("two.op");
    let patch3 = dir.join("multi.rna");
    std::fs::write(
        &two,
        "gene a() {\n  return 1\n}\n\ngene b() {\n  return 2\n}\n",
    )
    .unwrap();
    std::fs::write(&patch3, "edit anywhere { replace \"return\" -> \"yield\" }").unwrap();
    let (code3, stdout3, _stderr3) =
        run_rna_cli(&dir, "two.op", "multi.rna", &["--check", "--json"]);
    assert_eq!(code3, 0, "{}", stdout3);
    assert!(
        stdout3.contains("\"hits\":2"),
        "multi-hit counted: {}",
        stdout3
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_cli_apply_path_untouched_by_check_mode() {
    // the differential invariant: check mode must not perturb the apply
    // path. Same patch, same files: apply (dry-run default, then --write)
    // behaves exactly as before W68 landed.
    let dir = unique_dir("chk_apply");
    let src = dir.join("app.op");
    let patch = dir.join("p.rna");
    let before = "gene alpha(x) {\n  return x + 1\n}\n";
    std::fs::write(&src, before).unwrap();
    std::fs::write(&patch, "syntax: v2\nrename gene alpha -> gamma\n").unwrap();
    let (code, stdout, _stderr) = run_rna_cli(&dir, "app.op", "p.rna", &["--write"]);
    assert_eq!(code, 0, "{}", stdout);
    assert!(
        stdout.contains("1 applied, 0 missed"),
        "apply report wording unchanged: {}",
        stdout
    );
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        "gene gamma(x){\n  return x + 1\n}\n\n",
        "apply still reprints through the canonical formatter"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
