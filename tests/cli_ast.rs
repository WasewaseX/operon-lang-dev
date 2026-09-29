// W039: `operon ast` dump pins. The dump is a Total Grammar window: any file
// parses, so the dump must work on canonical and repaired programs alike and
// must be byte-stable across runs. Docs are metadata, semantics untouched:
// every test here only reads the parse tree.

use operon::parser::parse;
use operon::tools::{ast_dump, ast_dump_json};

#[test]
fn ast_dump_is_deterministic_and_structural() {
    let src = "\
gene add(a, b) { return a + b }
let x: int = add(1, 2)
if x > 2 { print(x) }
";
    let prog = parse(src);
    let a = ast_dump(&prog, true);
    let b = ast_dump(&prog, true);
    assert_eq!(a, b, "same input, same dump, always");
    // structural anchors, not a full golden (the exact shapes may be refined
    // as the tool matures; determinism is the contract, layout is detail)
    assert!(a.contains("(Program"), "root node: {a}");
    assert!(a.contains("(Gene \"add\""), "gene def: {a}");
    assert!(a.contains("(Param \"a\")"), "params: {a}");
    assert!(a.contains("(LetAnn \"x\""), "typed let: {a}");
    assert!(a.contains("(Ann \"int\")"), "annotation node: {a}");
    assert!(a.contains("(Call"), "call: {a}");
    assert!(a.contains("(If"), "if: {a}");
}

#[test]
fn ast_dump_json_is_valid_and_named() {
    let src = "gene f() { return 1 }";
    let prog = parse(src);
    let j = ast_dump_json(&prog);
    // the JSON form is a nested ["Kind", ...] array; zero external crates, so
    // validity is checked structurally: root kind first, balanced brackets,
    // and the embedded payload strings survive escaping
    assert!(j.starts_with("[\"Program\""), "root kind: {j}");
    assert!(j.ends_with(']'), "closed root: {j}");
    assert_eq!(
        j.matches('[').count(),
        j.matches(']').count(),
        "balanced arrays: {j}"
    );
    assert_eq!(
        j.matches('"').count() % 2,
        0,
        "balanced quotes, escapes intact: {j}"
    );
}

#[test]
fn ast_dump_survives_total_grammar_repair() {
    // this source is grammatically broken on purpose; Total Grammar repairs
    // it instead of rejecting, and the dump must still render something
    let src = "gene f( { return 1 }";
    let prog = parse(src);
    let a = ast_dump(&prog, true);
    assert!(a.contains("(Program"), "repaired source still dumps: {a}");
}
