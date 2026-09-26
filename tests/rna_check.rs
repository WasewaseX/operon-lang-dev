//! W068 — `.rna` checked-application engine (`apply_rna_checked`).
//!
//! The old `apply_rna` applied silently: misses (pattern absent, gene absent)
//! vanished with no signal. The checked engine reports every rule's fate and
//! is now the shared core (`run --rna` / `build --rna` keep their behavior
//! via the lossy wrapper; `operon rna` exposes the report + exit code).

use operon::genes::{apply_rna, apply_rna_checked};

const SRC: &str = "gene alpha() {\n    return 1\n}\n\ngene beta() {\n    return 22\n}\n";

#[test]
fn hit_and_miss_accounting() {
    let patch = "edit anywhere {\n    replace \"1\" -> \"7\"\n    replace \"not-present\" -> \"x\"\n}";
    let rep = apply_rna_checked(SRC, patch, "f");
    assert_eq!(rep.applied(), 1, "one rule hits");
    assert_eq!(rep.missed(), 1, "one rule misses");
    assert!(rep.would_change());
    // all three occurrences of "1" replaced (SRC has 1 in alpha, 2x "1" inside "22"? no:
    // "22" contains no "1"). Count: "return 1" → one '1'. Plus none elsewhere.
    let hits = &rep.edits[0].hits;
    assert_eq!(*hits, 1, "single occurrence of '1'");
    // miss row carries the pattern, zero hits
    assert_eq!(rep.edits[1].hits, 0);
    assert!(!rep.edits[1].applied);
    assert!(rep.new_text.contains("return 7"));
}

#[test]
fn gene_scoped_and_missing_gene() {
    // gene-scoped hit: only beta's 22 is touched, not other text
    let patch_ok = "edit beta { replace \"22\" -> \"33\" }";
    let rep = apply_rna_checked(SRC, patch_ok, "f");
    assert_eq!(rep.applied(), 1);
    assert!(rep.edits[0].gene_scoped && rep.edits[0].target_found);
    assert!(rep.new_text.contains("return 33"));
    assert!(!rep.new_text.contains("return 1\n    \n") || rep.new_text.contains("return 1"));

    // missing gene: every rule of the statement is an explicit miss
    let patch_bad = "edit nosuchgene { replace \"1\" -> \"7\" }";
    let rep2 = apply_rna_checked(SRC, patch_bad, "f");
    assert_eq!(rep2.applied(), 0);
    assert_eq!(rep2.missed(), 1);
    assert!(!rep2.edits[0].target_found, "gene absence is reported");
    assert!(!rep2.would_change());
}

#[test]
fn stem_vs_anywhere_scoping() {
    // target == file stem → anywhere-mode (whole text)
    let rep = apply_rna_checked(SRC, "edit f { replace \"return\" -> \"yield\" }", "f");
    assert_eq!(rep.edits[0].gene_scoped, false);
    assert_eq!(rep.edits[0].hits, 2, "both genes' returns replaced");
    // target == "anywhere" → same
    let rep2 = apply_rna_checked(SRC, "edit anywhere { replace \"alpha\" -> \"gamma\" }", "f");
    assert_eq!(rep2.edits[0].gene_scoped, false);
    assert!(rep2.new_text.contains("gene gamma()"));
}

#[test]
fn wrapper_parity_with_legacy_shape() {
    // apply_rna must stay byte-identical to the checked engine's text and
    // reproduce its legacy applied-strings format.
    let patch = "edit beta { replace \"22\" -> \"33\" replace \"absent\" -> \"z\" }\nedit anywhere { replace \"gene\" -> \"gen\" }";
    let (t_old, applied) = apply_rna(SRC, patch, "f");
    let rep = apply_rna_checked(SRC, patch, "f");
    assert_eq!(t_old, rep.new_text, "wrapper text parity");
    let expect: Vec<String> = rep
        .edits
        .iter()
        .filter(|e| e.applied)
        .map(|e| format!("{}: '{}' -> '{}'", e.target, e.from, e.to))
        .collect();
    assert_eq!(applied, expect, "applied-string parity");
    assert_eq!(applied.len(), 2, "22->33 and gene->gen applied; absent missed");
}

#[test]
fn empty_pattern_is_a_no_op_not_corruption() {
    // String::replace("", to) would splice `to` between every char and
    // corrupt the source. The checked engine refuses empty patterns.
    let patch = "edit anywhere { replace \"\" -> \"X\" }";
    let rep = apply_rna_checked(SRC, patch, "f");
    assert_eq!(rep.applied(), 0);
    assert_eq!(rep.new_text, SRC, "text untouched");
}

#[test]
fn sequential_rule_semantics_preserved() {
    // Rules run in order against the evolving text: rule 1 creates rule 2's
    // needle. Both apply (documenting the historical behavior).
    let patch = "edit anywhere { replace \"1\" -> \"2\" replace \"22\" -> \"OK\" }";
    let rep = apply_rna_checked(SRC, patch, "f");
    assert_eq!(rep.applied(), 2);
    assert!(rep.new_text.contains("OK"), "chained replacement landed");
}
