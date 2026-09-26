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
