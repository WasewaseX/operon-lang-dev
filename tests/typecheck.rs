//! TYPED-MODE: static type checker tests (src/typecheck.rs).
//!
//! Law: the checker is additive — every test here either (a) catches a
//! bad program statically, or (b) proves a good program checks clean.
//! The dynamic side is separately proven untouched by the differential
//! harness (bootstrap/harness.py) and the full cargo suite.

use operon::ast::Program;
use operon::lint::Sev;
use operon::parser;
use operon::typecheck::check_program;

fn check(src: &str) -> Vec<(usize, &'static str, Sev)> {
    let prog: Program = parser::parse(src);
    check_program(&prog)
        .into_iter()
        .map(|f| (f.line, f.code, f.sev))
        .collect()
}

fn errors(src: &str) -> Vec<(usize, &'static str)> {
    check(src)
        .into_iter()
        .filter(|(_, _, sev)| *sev == Sev::Error)
        .map(|(line, code, _)| (line, code))
        .collect()
}

fn clean(src: &str) {
    let finds = check(src);
    assert!(
        finds.is_empty(),
        "expected clean, got {:?}",
        finds.iter().map(|(l, c, _)| (*l, *c)).collect::<Vec<_>>()
    );
}

// ------------------------------------------------- the flagship catch

#[test]
fn t02_member_on_int_is_caught() {
    // THE canonical typed-mode catch: x = 10; x.name()
    let e = errors("x = 10\nx.name()\n");
    assert_eq!(e, vec![(2, "T02")]);
}

#[test]
fn t02_member_on_str_surface_miss() {
    let e = errors("let s = \"hi\"\ns.nonexistent()\n");
    assert_eq!(e, vec![(2, "T02")]);
}

#[test]
fn t02_real_line_numbers() {
    // the Method variant carries its own line (dx parity with Call/Index)
    let pad = "\n\n\n\n"; // push the call to line 5
    let e = errors(&format!("{}x = 3\nx.name()\n", pad));
    assert_eq!(e, vec![(6, "T02")]);
}

// ------------------------------------------------- annotations

#[test]
fn t01_annotated_let_mismatch() {
    // LetAnn statements carry no A13 stamp yet: line is best-effort (0)
    let e = errors("let n: int = \"str here\"\n");
    assert_eq!(e, vec![(0, "T01")]);
}

#[test]
fn annotated_let_ok() {
    clean("let n: int = 5\nlet s: str = \"a\"\nlet f: float = 1\nlet b: list[str] = [\"x\"]\n");
}

#[test]
fn capitalized_aliases_ok() {
    clean("let n: Int = 5\nlet s: Str = \"a\"\nlet f: Float = 1.5\nlet b: Bool = true\n");
}

#[test]
fn t04_unknown_annotation_name() {
    let finds = check("let n: wibble = 5\n");
    assert!(
        finds.iter().any(|(_, code, _)| *code == "T04"),
        "{:?}",
        finds
    );
}

#[test]
fn t01_gene_return_mismatch() {
    let e = errors("gene f() -> int {\n    return \"str\"\n}\n");
    assert_eq!(e, vec![(1, "T01")]);
}

#[test]
fn gene_return_inferred_ok() {
    clean("gene f() {\n    return 1 + 2\n}\n");
}

#[test]
fn t01_gene_param_mismatch_at_call() {
    let e =
        errors("gene add(a: int, b: int) -> int {\n    return a + b\n}\nlet r = add(1, \"x\")\n");
    assert_eq!(e, vec![(4, "T03")]);
}

#[test]
fn t10_return_annotation_with_fallthrough() {
    let finds = check("gene f() -> int {\n    if 1 > 0 {\n        return 1\n    }\n}\n");
    assert!(finds.iter().any(|(_, c, _)| *c == "T10"), "{:?}", finds);
}

// ------------------------------------------------- inference

#[test]
fn inference_local_types() {
    clean(
        "let a = 1\nlet b = 2.5\nlet c = a + 1\nlet s = \"x\" + \"y\"\nlet l = [1, 2, 3]\nlet m = {\"k\": 1}\n",
    );
}

#[test]
fn t03_int_plus_str() {
    let e = errors("let a = 1\nlet b = \"x\"\nlet c = a + b\n");
    assert_eq!(e, vec![(3, "T03")]);
}

#[test]
fn t03_impossible_ordering() {
    let e = errors("let m = {\"a\": 1}\nlet n = {\"b\": 2}\nlet c = m < n\n");
    assert_eq!(e, vec![(3, "T03")]);
}

#[test]
fn any_escapes_everything() {
    // dynamic values (args, builtins) never flag
    clean("x = arg(1)\ny = x + 1\nz = y.name()\n");
}

#[test]
fn nullish_unwraps_option() {
    clean("let o = none()\nlet v = o ?? 5\n");
}

// ------------------------------------------------- generics

#[test]
fn generics_resolve_and_check() {
    clean(
        "gene first<T>(items: list[T]) -> T? {\n    return items.get(0)\n}\nlet a = first([1, 2])\nlet b = first([\"x\"])\n",
    );
}

#[test]
fn t08_bound_violation() {
    // int does not satisfy a trait bound
    let e = errors(
        "trait Shape {\n    gene area()\n}\nphenotype Circle implements Shape {\n    gene area() {\n        return 1.0\n    }\n}\ngene describe<S: Shape>(s: S) -> str {\n    return str(s)\n}\nlet x = describe(5)\n",
    );
    assert_eq!(e, vec![(12, "T08")]);
}

#[test]
fn bound_satisfied_by_implementor() {
    clean(
        "trait Shape {\n    gene area()\n}\nphenotype Circle implements Shape {\n    gene area() {\n        return 1.0\n    }\n}\ngene describe<S: Shape>(s: S) -> str {\n    return \"ok\"\n}\nlet c = new Circle()\nlet x = describe(c)\n",
    );
}

#[test]
fn t08_numeric_bound_refuses_str() {
    let e = errors(
        "gene double<N: numeric>(n: N) -> N {\n    return n + n\n}\nlet x = double(\"str\")\n",
    );
    assert_eq!(e, vec![(4, "T08")]);
}

#[test]
fn numeric_bound_accepts_numbers() {
    clean(
        "gene double<N: numeric>(n: N) -> N {\n    return n + n\n}\nlet a = double(4)\nlet b = double(2.5)\n",
    );
}

// ------------------------------------------------- traits

#[test]
fn t07_missing_trait_method() {
    let e = errors(
        "trait T1 {\n    gene must()\n}\nphenotype Bad implements T1 {\n    gene other() {\n        return 1\n    }\n}\n",
    );
    assert_eq!(e, vec![(4, "T07")]);
}

#[test]
fn trait_contract_satisfied() {
    clean(
        "trait T1 {\n    gene must()\n}\nphenotype Good implements T1 {\n    gene must() {\n        return 1\n    }\n}\n",
    );
}

#[test]
fn t07_inherited_method_satisfies() {
    clean(
        "trait T1 {\n    gene must()\n}\nphenotype Base {\n    gene must() {\n        return 1\n    }\n}\nphenotype Child from Base implements T1 {\n}\n",
    );
}

// ------------------------------------------------- option/result

#[test]
fn option_result_annotations_ok() {
    clean(
        "let o: int? = none()\nlet p: int? = some(1)\nlet r: result[int, str] = ok(1)\nlet e: result[int, str] = err(\"x\")\n",
    );
}

#[test]
fn t06_propagate_on_non_variant() {
    // T06 is a WARNING by design (TYPED-MODE.md §10): propagation on a
    // non-variant is suspicious, not fatal (the dynamic side stresses)
    let finds = check("let n = 5\nlet m = n?!\n");
    assert_eq!(
        finds.iter().map(|(l, c, _)| (*l, *c)).collect::<Vec<_>>(),
        vec![(2, "T06")]
    );
}

#[test]
fn propagate_on_option_ok() {
    clean("gene f() -> int {\n    let o = some(1)\n    return o?!\n}\n");
}

#[test]
fn t05_option_match_exhaustive() {
    clean(
        "let o: int? = some(1)\nmatch o {\n    case Some(v) { print(v) }\n    case None { print(\"none\") }\n}\n",
    );
}

#[test]
fn t05_option_match_missing_none() {
    let e = errors("let o: int? = some(1)\nmatch o {\n    case Some(v) { print(v) }\n}\n");
    // the match statement stamps its own line
    assert_eq!(e, vec![(2, "T05")]);
}

#[test]
fn t05_result_match_missing_err() {
    let e = errors("let r: result[int, str] = ok(1)\nmatch r {\n    case Ok(v) { print(v) }\n}\n");
    assert_eq!(e, vec![(2, "T05")]);
}

#[test]
fn t05_bool_match_missing_false() {
    let e = errors("let b = true\nmatch b {\n    case true { print(1) }\n}\n");
    assert_eq!(e, vec![(2, "T05")]);
}

#[test]
fn bool_match_exhaustive() {
    clean("let b = true\nmatch b {\n    case true { print(1) }\n    case false { print(0) }\n}\n");
}

#[test]
fn guarded_arms_cover_nothing() {
    let e = errors("let o: int? = some(1)\nmatch o {\n    case Some(v) if v > 0 { print(v) }\n}\n");
    // a guard can be false: BOTH variants stay uncovered
    assert_eq!(e, vec![(2, "T05")]);
}

#[test]
fn bind_arm_covers_everything() {
    clean("let o: int? = some(1)\nmatch o {\n    case other { print(other) }\n}\n");
}

#[test]
fn wildcard_covers_everything() {
    clean("let o: int? = some(1)\nmatch o {\n    case _ { print(\"any\") }\n}\n");
}

// ------------------------------------------------- phenotype members

#[test]
fn t02_phenotype_missing_method() {
    let e = errors(
        "phenotype P {\n    gene a() {\n        return 1\n    }\n}\nlet p = new P()\np.missing()\n",
    );
    assert_eq!(e, vec![(7, "T02")]);
}

#[test]
fn phenotype_method_ok() {
    clean("phenotype P {\n    gene a() {\n        return 1\n    }\n}\nlet p = new P()\np.a()\n");
}

#[test]
fn builtin_method_surfaces() {
    clean(
        "let s = \"ab\"\ns.upper()\ns.len()\nlet l = [1]\nl.push(2)\nl.len()\nlet m = {\"k\": 1}\nm.keys()\nm.len()\n",
    );
}

// ------------------------------------------------- assignment typing

#[test]
fn t09_assign_type_change() {
    // T09 is a WARNING by design: re-typing is legal dynamically
    // Assign statements carry no A13 stamp yet: line is best-effort (0)
    let finds = check("let n = 1\nn = \"str\"\n");
    assert_eq!(
        finds.iter().map(|(l, c, _)| (*l, *c)).collect::<Vec<_>>(),
        vec![(0, "T09")]
    );
}

#[test]
fn float_binding_accepts_int_widening() {
    // W01 numeric law mirrored: a float-typed binding accepts int
    // (safe widening); an int-typed binding REFUSES float (no silent
    // narrowing) -- that direction is T09.
    clean("let n = 1.0\nn = 2\n");
    let finds = check("let n = 1\nn = 2.5\n");
    assert!(finds.iter().any(|(_, c, _)| *c == "T09"), "{:?}", finds);
}

// ------------------------------------------------- soft-law guard

#[test]
fn unannotated_programs_check_clean() {
    // a fully dynamic program opts into nothing: no T findings
    clean("gene f(x) {\n    return x + 1\n}\nlet v = f(1)\nlet w = f(\"s\")\nprint(v)\n");
}
