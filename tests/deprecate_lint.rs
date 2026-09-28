// W064: deprecation mark pins. The mark is metadata: it must parse on any
// input (Total Grammar), survive the canonical formatter byte-exactly, and
// turn call sites into deprecated-use findings on the check stream with the
// migration text attached. The runtime never reads the mark, so no
// differential parity is claimed here (SPEC §3 marks table).

use operon::lint::{lint, Sev};
use operon::parser::parse;
use operon::tools::format_program;

#[test]
fn deprecated_use_fires_at_call_sites() {
    let src = "\
@deprecated(\"use new_style() instead\", since=\"2.4\")
gene old_style(n) { return n * 2 }

gene main() {
    print(old_style(21))
}
";
    let findings = lint(&parse(src));
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "deprecated-use")
        .collect();
    assert_eq!(hits.len(), 1, "one call site, one finding: {:?}", findings);
    let f = hits[0];
    assert_eq!(f.code, "W12", "stable code");
    assert_eq!(f.sev, Sev::Warning, "advisory warning, never an error");
    assert!(f.message.contains("old_style"), "names the gene: {:?}", f);
    assert!(
        f.message.contains("use new_style() instead"),
        "carries the migration text: {:?}",
        f
    );
    assert!(
        f.message.contains("since 2.4"),
        "carries the ladder gate: {:?}",
        f
    );
    assert_eq!(f.line, 5, "points at the call line");
}

#[test]
fn deprecated_without_since_omits_the_gate() {
    let src = "\
@deprecated(\"gone soon\")
gene old() { return 1 }

gene main() { let _x = old() }
";
    let findings = lint(&parse(src));
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "deprecated-use")
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", findings);
    assert!(hits[0].message.contains("gone soon"), "{:?}", hits[0]);
    assert!(
        !hits[0].message.contains("since"),
        "no gate, no since: {:?}",
        hits[0]
    );
}

#[test]
fn unmarked_genes_stay_clean() {
    let src = "gene f() { return 1 }\ngene main() { let _x = f() }\n";
    let findings = lint(&parse(src));
    assert!(
        !findings.iter().any(|f| f.rule == "deprecated-use"),
        "no mark, no finding: {:?}",
        findings
    );
}

#[test]
fn malformed_mark_degrades_to_a_note_not_a_rejection() {
    // Total Grammar: a broken payload never rejects the program, the mark
    // is dropped with a rung-4 note, the gene still parses and runs.
    let src = "@deprecated(42)\ngene f() { return 1 }\ngene main() { let _x = f() }\n";
    let prog = parse(src);
    let findings = lint(&prog);
    assert!(
        !findings.iter().any(|f| f.rule == "deprecated-use"),
        "dropped mark flags nothing: {:?}",
        findings
    );
}

#[test]
fn mark_survives_fmt_byte_exactly() {
    let src = "\
@deprecated(\"use new_style() instead\", since=\"2.4\")
gene old_style(n) {
  return n * 2
}

gene main() {
    print(old_style(21))
}
";
    let prog = parse(src);
    let out = format_program(&prog);
    assert!(
        out.contains("@deprecated(\"use new_style() instead\", since=\"2.4\")"),
        "canonical form re-emits the mark: {out}"
    );
    // fmt is a fixpoint: formatting the formatted output is byte-identical
    let out2 = format_program(&parse(&out));
    assert_eq!(out, out2, "fmt fixpoint");
}
