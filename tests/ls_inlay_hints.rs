//! W45-v3: LSP inlay hints — the checker's inferred types for un-annotated
//! bindings, precomputed per document version in the analyze pass.
//! Laws pinned here:
//! 1. Concrete literal/list/map bindings hint (`: int`, `: str`, `: list[int]`).
//! 2. Annotated bindings NEVER hint (the source already shows the type).
//! 3. `any`-typed bindings NEVER hint (the dynamic escape hatch carries no
//!    information — never a vacuous hint).
//! 4. The hint position is exactly past the binding name, on the binding's
//!    line (0-based coordinates, the file's convention).
//! 5. Hints ride `analyze`, the same pass as diagnostics — no second parse.

use operon::ls::analyze;

#[test]
fn hints_cover_concrete_bindings() {
    let src = "let n = 42\nlet s = \"hi\"\n";
    let doc = analyze(src);
    let dump: Vec<String> = doc
        .hints
        .iter()
        .map(|h| format!("{}:{}{}", h.line0, h.col0, h.label))
        .collect();
    assert!(
        dump.iter().any(|d| d.contains(": int")),
        "no int hint; got {:?}",
        dump
    );
    assert!(
        dump.iter().any(|d| d.contains(": str")),
        "no str hint; got {:?}",
        dump
    );
}

#[test]
fn hints_skip_annotated_and_any() {
    let src = "let ann: str = \"x\"\nlet dynv = mystery_call()\n";
    let doc = analyze(src);
    let dump: Vec<String> = doc
        .hints
        .iter()
        .map(|h| format!("{}:{}{}", h.line0, h.col0, h.label))
        .collect();
    // `ann` is annotated in source: no hint for it
    assert!(
        !dump.iter().any(|d| d.contains("ann")),
        "annotated binding must not hint; got {:?}",
        dump
    );
    // `dynv` is a phantom-called unknown: the checker's answer is `any`
    assert!(
        !dump.iter().any(|d| d.contains("dynv")),
        "any-typed binding must not hint; got {:?}",
        dump
    );
}

#[test]
fn hint_position_is_past_the_name() {
    let src = "let n = 42\n";
    let doc = analyze(src);
    assert_eq!(
        doc.hints.len(),
        1,
        "expected exactly one hint; got {:?}",
        doc.hints
    );
    let h = &doc.hints[0];
    assert_eq!(h.line0, 0, "hint on the binding's own line (0-based)");
    // `let n = 42`: the name ends at character 5 (l-e-t-space-n)
    assert_eq!(h.col0, 5, "hint renders right past the name");
    assert_eq!(h.label, ": int");
}

#[test]
fn hints_inside_gene_bodies_too() {
    let src = "gene make() {\n  let inner = 7\n  return inner\n}\n";
    let doc = analyze(src);
    let dump: Vec<String> = doc
        .hints
        .iter()
        .map(|h| format!("{}:{}{}", h.line0, h.col0, h.label))
        .collect();
    assert!(
        dump.iter().any(|d| d.contains("int")),
        "gene-body binding must hint; got {:?}",
        dump
    );
}
