// W01-s2 (static type system stage 2): the compile-time type checker pins.
// The checker rides the check stream with its own stable codes (E02
// type-mismatch, E03 no-method, W13 unknown-type-name, W14 null-return,
// W15 non-exhaustive-match, W16 trait-bound-violation). Every finding
// here mirrors a runtime soft contract (the `unfolded` Stress family or a
// "has no method" note); the dynamic side must stay finding-free, so the
// negative tests matter as much as the positive ones.

use operon::lint::{rule_stream, Stream};
use operon::parser::parse;
use operon::types;

fn check_rules(src: &str) -> Vec<(String, String)> {
    let prog = parse(src);
    types::check(&prog)
        .into_iter()
        .map(|f| (f.rule, f.message))
        .collect()
}

#[test]
fn user_example_method_on_int_is_caught() {
    // THE owner directive, verbatim in spirit: `x = 10; x.name()` must be
    // caught BEFORE runtime, without any annotations anywhere.
    let src = "x = 10\nx.name()\n";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "no-method" && m.contains("int has no method 'name'")),
        "expected no-method on int, got {found:?}"
    );
    // the dynamic side is untouched: the program still RUNS (yielding null
    // at the runtime's own note), so this is a diagnostic, not a rejection
    assert!(rule_stream("no-method") == Stream::Check);
}

#[test]
fn call_argument_boundary_mismatch() {
    let src = "\
gene add(a: int, b: int) -> int {
    return a + b
}
print(add(1, 2))
print(add(\"x\", 2))
";
    let found = check_rules(src);
    assert!(
        found.iter().any(|(r, m)| r == "type-mismatch"
            && m.contains("argument 'a' for gene 'add' expects int, got str")),
        "expected the boundary mismatch, got {found:?}"
    );
    // the well-typed call produces nothing
    assert_eq!(
        found.iter().filter(|(r, _)| r == "type-mismatch").count(),
        1
    );
}

#[test]
fn return_boundary_mismatch() {
    let src = "\
gene bad() -> int {
    return \"nope\"
}
";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("return of gene 'bad'")),
        "got {found:?}"
    );
}

#[test]
fn annotated_let_and_assign_boundaries() {
    let src = "\
let n: int = 5
n = \"now a str\"
";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("assignment to 'n' expects int")),
        "got {found:?}"
    );

    let src2 = "let s: str = 3\n";
    let found2 = check_rules(src2);
    assert!(
        found2
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("declaration of 's' expects str")),
        "got {found2:?}"
    );
}

#[test]
fn numeric_widening_direction_is_the_runtime_rule() {
    // float accepts int (widening), int REFUSES float (no silent narrowing)
    let src = "\
gene widen(n: float) -> float {
    return n * 2.0
}
widen(3)
let f: float = 7
let i: int = 2.5
";
    let found = check_rules(src);
    assert!(
        !found
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("'f'")),
        "float accepts int, got {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("declaration of 'i' expects int")),
        "int must refuse float, got {found:?}"
    );
}

#[test]
fn unknown_type_name_warns() {
    let src = "gene f(x: Futr) { return x }\n";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "unknown-type-name" && m.contains("'Futr'")),
        "typo armor must warn, got {found:?}"
    );
    // capitalized generic heads are known
    let src2 = "gene g(v: Option<int>) -> int? { return v }\n";
    let found2 = check_rules(src2);
    assert!(
        !found2.iter().any(|(r, _)| r == "unknown-type-name"),
        "Option<int> is a known head, got {found2:?}"
    );
}

#[test]
fn implicit_null_return_warns() {
    let src = "\
gene maybe(flag) -> int {
    if flag {
        return 1
    }
}
";
    let found = check_rules(src);
    assert!(
        found.iter().any(|(r, _)| r == "null-return"),
        "fall-through with non-optional ret must warn, got {found:?}"
    );

    // an optional return annotation accepts the fall-through
    let src2 = "\
gene maybe(flag) -> int? {
    if flag {
        return 1
    }
}
";
    let found2 = check_rules(src2);
    assert!(
        !found2.iter().any(|(r, _)| r == "null-return"),
        "int? accepts null, got {found2:?}"
    );

    // a bare `return` repairs to `return null` (Total Grammar), so the
    // boundary it violates is the return annotation itself (E02)
    let src3 = "gene quit() -> int { return }\n";
    let found3 = check_rules(src3);
    assert!(
        found3
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("expects int, got null")),
        "got {found3:?}"
    );
}

#[test]
fn option_result_match_exhaustiveness() {
    // missing the None arm
    let src = "\
gene f(r) -> int? {
    return some(1)
}
match f(1) {
    case Some(v) { print(v) }
}
";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "non-exhaustive-match" && m.contains("missing None")),
        "got {found:?}"
    );

    // Ok + Err both covered
    let src2 = "\
gene g() {
    return ok(1)
}
match g() {
    case Ok(v) { print(v) }
    case Err(e) { print(e) }
}
";
    let found2 = check_rules(src2);
    assert!(
        !found2.iter().any(|(r, _)| r == "non-exhaustive-match"),
        "got {found2:?}"
    );

    // a catch-all satisfies the contract
    let src3 = "\
gene h() -> int? {
    return none()
}
match h() {
    case Some(v) { print(v) }
    case _ { print(\"nothing\") }
}
";
    let found3 = check_rules(src3);
    assert!(
        !found3.iter().any(|(r, _)| r == "non-exhaustive-match"),
        "catch-all covers, got {found3:?}"
    );

    // a guarded arm does NOT count as coverage (the Rust rule)
    let src4 = "\
gene k() -> int? {
    return none()
}
match k() {
    case Some(v) if v > 0 { print(v) }
    case None { print(\"none\") }
}
";
    let found4 = check_rules(src4);
    assert!(
        found4
            .iter()
            .any(|(r, m)| r == "non-exhaustive-match" && m.contains("missing Some")),
        "guarded Some arm cannot prove coverage, got {found4:?}"
    );
}

#[test]
fn generic_gene_instantiates_per_call_site() {
    // the SAME generic gene takes an int and a str at different sites
    let src = "\
gene id<T>(x: T) -> T {
    return x
}
id(3)
id(\"s\")
id([1, 2])
";
    let found = check_rules(src);
    assert!(
        found.is_empty(),
        "a TypeVar accepts any argument, got {found:?}"
    );

    // a concrete container param still checks payloads
    let src2 = "\
gene ints(xs: List<int>) -> int {
    return len(xs)
}
ints([1, 2])
ints([\"a\"])
";
    let found2 = check_rules(src2);
    assert!(
        found2
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("expects List<int>, got list<str>")),
        "got {found2:?}"
    );
}

#[test]
fn trait_bound_violation_and_unknown_trait() {
    let src = "\
trait Show {
    gene display()
}
gene render<T: Show>(x: T) -> str {
    return x.display()
}
phenotype Plain {
    gene display() { return \"p\" }
}
phenotype Shown implements Show {
    gene display() { return \"s\" }
}
render(new Plain())
render(new Shown())
";
    let found = check_rules(src);
    let violated = found.iter().any(|(r, m)| {
        r == "trait-bound-violation" && m.contains("phenotype 'Plain' does not implement it")
    });
    assert!(violated, "Plain must violate the Show bound, got {found:?}");
    // Shown implements Show: no finding for that site
    assert!(
        !found
            .iter()
            .any(|(r, m)| r == "trait-bound-violation" && m.contains("Shown")),
        "got {found:?}"
    );

    let src2 = "gene r2<T: Missing>(x: T) { return x }\n";
    let found2 = check_rules(src2);
    assert!(
        found2
            .iter()
            .any(|(r, m)| r == "unknown-type-name" && m.contains("'Missing'")),
        "unknown trait bound warns, got {found2:?}"
    );
}

#[test]
fn dynamic_idioms_stay_finding_free() {
    // the corpus contract: the checker must NOT flood dynamic code
    let src = "\
# everything here is dynamic by design
x = 10
y = \"str\"
z = [1, 2, 3]
m = {\"k\": 1}
gene no_anns(a, b) {
    return a + b
}
no_anns(x, y)
if x > 5 {
    z.push(4)
}
for v in z {
    print(v)
}
let w = m[\"k\"] + 1
gene dyn_ret(u) {
    if u {
        return 1
    }
    return \"two\"
}
print(dyn_ret(true), len(z), m.has(\"k\"))
";
    let found = check_rules(src);
    assert!(
        found.is_empty(),
        "dynamic code must stay finding-free, got {found:?}"
    );
}

#[test]
fn aliases_resolve_and_compose() {
    let src = "\
type UserId = int
type Box = List<int>
gene load(u: UserId) -> int {
    return u
}
load(21)
load(\"x\")
let b: Box = [1, 2]
let bad: Box = [\"a\"]
";
    let found = check_rules(src);
    assert!(
        found.iter().any(|(r, m)| r == "type-mismatch"
            && m.contains("argument 'u' for gene 'load' expects UserId, got str")),
        "alias boundary checks the target (message quotes the alias), got {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "type-mismatch" && m.contains("declaration of 'bad' expects Box")),
        "got {found:?}"
    );
}

#[test]
fn sig_table_reports_declared_and_inferred() {
    let src = "\
gene add(a: int, b: int) -> int {
    return a + b
}
gene implicit() {
    return 1 + 2
}
";
    let prog = parse(src);
    let sigs = types::signatures(&prog);
    assert_eq!(sigs.len(), 2);
    assert_eq!(sigs[0].text, "gene add(a: int, b: int) -> int");
    assert!(sigs[0].declared);
    assert_eq!(sigs[1].text, "gene implicit() -> int");
    assert!(!sigs[1].declared, "inferred return marks the row");
}

#[test]
fn fmt_roundtrip_of_the_new_surface() {
    use operon::tools::format_program;
    let src = "\
type UserId = int
gene id<T>(x: T) -> T {
    return x
}
gene load(u: UserId) -> List<int>? {
    return [1]
}
";
    let once = format_program(&parse(src));
    let twice = format_program(&parse(&once));
    assert_eq!(once, twice, "fmt must be a fixpoint on the new surface");
    assert!(once.contains("type UserId = int"));
    assert!(once.contains("gene id<T>(x: T) -> T"));
    assert!(once.contains("gene load(u: UserId) -> List<int>?"));
}

#[test]
fn inference_uses_method_tables_for_known_families() {
    // str/list/map methods infer return types; a method on a KNOWN family
    // that does not exist is E03
    let src = "let s = \"hi\"\nprint(s.upper())\nlet n = 5\nprint(n.upper())\n";
    let found = check_rules(src);
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "no-method" && m.contains("int has no method 'upper'")),
        "got {found:?}"
    );
    assert!(
        !found
            .iter()
            .any(|(r, m)| r == "no-method" && m.contains("str")),
        "str.upper() exists, got {found:?}"
    );
}
