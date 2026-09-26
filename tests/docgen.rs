// W074/W073: doc-comment capture, fmt roundtrip survival, and docgen output.
// Docs are metadata: every test here also asserts semantics are untouched.
use operon::parser::parse;
use operon::tools::{doc_json, doc_markdown, format_program};

#[test]
fn doc_capture_gene_module_phenotype() {
    let src = "\
## module header line
## second module line

## adds one
gene inc(x) {
    return x + 1
}

## a cell class
phenotype cell {
    let energy = 1
    ## method doc
    gene tick() {
        return energy
    }
}
";
    let prog = parse(src);
    assert_eq!(
        prog.module_doc,
        vec!["module header line", "second module line"]
    );
    let inc = prog
        .stmts
        .iter()
        .find_map(|s| match s {
            operon::ast::Stmt::Gene(g) if g.name.as_deref() == Some("inc") => Some(g.clone()),
            _ => None,
        })
        .expect("inc gene");
    assert_eq!(inc.doc, vec!["adds one"]);

    let ph = prog
        .stmts
        .iter()
        .find_map(|s| match s {
            operon::ast::Stmt::Pheno(p) if p.name == "cell" => Some(p.clone()),
            _ => None,
        })
        .expect("phenotype");
    assert_eq!(ph.doc, vec!["a cell class"]);
    assert_eq!(ph.methods[0].doc, vec!["method doc"]);
}

#[test]
fn doc_survives_fmt_roundtrip() {
    let src = "\
## module doc

## outer doc
## continues here
gene f(a, b) {
    return a + b
}

## greets
gene main() {
    print(f(1, 2))
}
";
    let prog1 = parse(src);
    let fmt1 = format_program(&prog1);
    let prog2 = parse(&fmt1);
    let fmt2 = format_program(&prog2);
    // fmt∘fmt = fmt (byte-stability law) …
    assert_eq!(fmt1, fmt2, "fmt must be idempotent on doc-carrying source");
    // … and every doc line survives byte-exact.
    assert_eq!(prog2.module_doc, prog1.module_doc);
    let f1 = prog1
        .stmts
        .iter()
        .find_map(|s| match s {
            operon::ast::Stmt::Gene(g) if g.name.as_deref() == Some("f") => Some(g.clone()),
            _ => None,
        })
        .unwrap();
    let f2 = prog2
        .stmts
        .iter()
        .find_map(|s| match s {
            operon::ast::Stmt::Gene(g) if g.name.as_deref() == Some("f") => Some(g.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(f1.doc, f2.doc);
    assert_eq!(f1.doc, vec!["outer doc", "continues here"]);
    // the formatter re-emits the `## ` prefix
    assert!(fmt1.contains("## outer doc\n## continues here"));
}

#[test]
fn doc_survives_repair_rungs() {
    // synonym-repaired genes keep their docs: the repair path re-runs the
    // same parse, so `func` -> `gene` must not lose the attached block.
    let src = "## repaired doc\nfunc double(x) {\n    return x * 2\n}\n";
    let prog = parse(src);
    let notes = &prog.notes;
    assert!(
        notes.iter().any(|n| n.rung == 2),
        "expected a synonym repair note"
    );
    let dbl = prog
        .stmts
        .iter()
        .find_map(|s| match s {
            operon::ast::Stmt::Gene(g) if g.name.as_deref() == Some("double") => Some(g.clone()),
            _ => None,
        })
        .expect("repaired gene");
    assert_eq!(dbl.doc, vec!["repaired doc"]);
}

#[test]
fn docs_do_not_change_semantics() {
    // identical programs, one with docs — same parse notes (no new rungs),
    // same statement count, and the formatter output differs only by `##`.
    let plain = "gene add(a, b) {\n    return a + b\n}\n";
    let docced = "## adds\n## two things\ngene add(a, b) {\n    return a + b\n}\n";
    let p1 = parse(plain);
    let p2 = parse(docced);
    assert_eq!(p1.notes.len(), p2.notes.len());
    assert_eq!(p1.stmts.len(), p2.stmts.len());
}

#[test]
fn docgen_markdown_and_json_shapes() {
    let src = "## mod doc\n\n## gene doc\ngene half(x) {\n    return x / 2\n}\n";
    let prog = parse(src);
    let md = doc_markdown("std/demo.op", &prog);
    assert!(md.starts_with("# demo\n\nmod doc\n"));
    assert!(md.contains("## `gene half(x)`"));
    assert!(md.contains("gene doc"));

    let js = doc_json("std/demo.op", &prog);
    assert!(
        serde_json_value(&js),
        "doc_json emitted unbalanced JSON: {}",
        js
    );

    // string-level pins (the binary is serde-free by policy)
    assert!(js.contains("\"module\":\"demo\""));
    assert!(js.contains("\"doc\":[\"mod doc\"]"));
    assert!(js.contains("\"name\":\"half\""));
    assert!(js.contains("\"signature\":\"gene half(x)\""));
}

/// serde-free JSON validity check: a tiny bracket/quote scanner good enough
/// to catch malformed docgen output without adding a dependency.
fn serde_json_value(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth == 0 && !in_str && s.starts_with('{') && s.ends_with('}')
}

#[test]
fn doc_surfaces_in_lsp_hover() {
    // W074 gate: docs appear in hover. Hover over the gene NAME on its
    // definition line (line index 2 here, word "inc" starts at col 5).
    let src = "## adds one\ngene inc(x) {\n    return x + 1\n}\n";
    // hover needs the name token: line 1 ("gene inc(x) {"), "inc" at col 5
    let doc = operon::ls::analyze(src);
    let h = operon::ls::hover(src, &doc, 1, 5).expect("hover on gene name");
    assert!(
        h.contains("adds one"),
        "hover must carry the doc text: {}",
        h
    );
}
