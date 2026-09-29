// W002 stage 2: unreachable-match-arm end-to-end pins, driven through the
// same public surface `operon lint` uses (lint_style is the lint stream the
// CLI prints). The unit pins in src/lint.rs cover the rule internals; this
// file owns the CONTRACT view: stable code, stream ownership, and the
// false-positive budget (every live-shape probe here must stay silent,
// because the corpus bar is zero new warnings on tests/ + std/ + apps/).

use operon::lint::{lint_style, rule_code, Sev, Stream};
use operon::parser::parse;

fn w06(src: &str) -> Vec<String> {
    lint_style(&parse(src))
        .iter()
        .filter(|f| f.rule == "unreachable-match-arm")
        .map(|f| f.message.clone())
        .collect()
}

#[test]
fn dead_arms_are_flagged_with_the_stable_code() {
    let findings = lint_style(&parse(
        "match some(1) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n}\n",
    ));
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "unreachable-match-arm")
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", findings);
    assert_eq!(hits[0].code, "W06", "stable code, never renumbered");
    assert_eq!(hits[0].sev, Sev::Warning, "advisory: Total Grammar");
    assert!(
        hits[0].message.contains("broader arm first"),
        "{:?}",
        hits[0]
    );
    // the message names the blocking arm so the fix is mechanical
    assert!(hits[0].message.contains("arm 1"), "{:?}", hits[0]);
    // W06 belongs to the lint stream; check stays correctness-only
    assert_eq!(
        operon::lint::rule_stream("unreachable-match-arm"),
        Stream::Lint
    );
    assert_eq!(rule_code("unreachable-match-arm"), "W06");
}

#[test]
fn strictly_broader_shapes_one_finding_each() {
    // variant payload wildcard, tag-only form, nested payload, list shape,
    // *rest prefix, map presence, legacy multi-run literal
    let cases = [
        ("Some(_)", "match some(1) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n}\n"),
        ("tag-only", "match some(9) {\n    case Some {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n}\n"),
        ("nested", "match some(some(1)) {\n    case Some(Some(_)) {\n        print(1)\n    }\n    case Some(Some(n)) {\n        print(2)\n    }\n}\n"),
        ("list", "match [1, 2] {\n    case [a, b] {\n        print(1)\n    }\n    case [1, 2] {\n        print(2)\n    }\n}\n"),
        ("rest", "match [1, 2] {\n    case [1, *t] {\n        print(1)\n    }\n    case [1, 2] {\n        print(2)\n    }\n}\n"),
        ("map", "match {x: 1} {\n    case {x} {\n        print(1)\n    }\n    case {x: 1} {\n        print(2)\n    }\n}\n"),
        ("multi", "match 2 {\n    case 1, 2 {\n        print(1)\n    }\n    case 2 {\n        print(2)\n    }\n}\n"),
    ];
    for (shape, src) in cases {
        let hits = w06(src);
        assert_eq!(hits.len(), 1, "{}: {:?}", shape, hits);
    }
}

#[test]
fn live_shapes_stay_silent_false_positive_budget() {
    // every probe here is REACHABLE at runtime under first-match-wins; a
    // finding on any of them would be a corpus false positive
    let live = [
        // a guard can miss, so arms after a guarded arm stay live
        "match 1 {\n    case x if x > 5 {\n        print(1)\n    }\n    case 1 {\n        print(2)\n    }\n}\n",
        // Some(x) binds but matches every payload-carrying Some, so the arms
        // it leaves alive are the tag-only form (payload-less values) and
        // other families; Some(_) after it would be DEAD (same value set)
        "match some(1) {\n    case Some(x) {\n        print(1)\n    }\n    case Some {\n        print(2)\n    }\n    case None {\n        print(3)\n    }\n}\n",
        // different families never cover each other
        "match none() {\n    case Some(_) {\n        print(1)\n    }\n    case None {\n        print(2)\n    }\n}\n",
        // partial or-coverage: the None alternative keeps the arm alive
        "match none() {\n    case Some(_) {\n        print(1)\n    }\n    case Some(1) | None {\n        print(2)\n    }\n}\n",
        // exact-length earlier cannot cover a *rest later arm
        "match [1, 2] {\n    case [1, 2] {\n        print(1)\n    }\n    case [1, *t] {\n        print(2)\n    }\n}\n",
        // a constrained map sub-pattern cannot cover presence-only
        "match {x: 5} {\n    case {x: 1} {\n        print(1)\n    }\n    case {x} {\n        print(2)\n    }\n}\n",
        // a key the earlier arm never requires keeps the later arm live
        "match {x: 1} {\n    case {x, y} {\n        print(1)\n    }\n    case {x} {\n        print(2)\n    }\n}\n",
        // floats stay out of the algebra (NaN never deep_eq-equals itself)
        "match 1.5 {\n    case 1.5 {\n        print(1)\n    }\n    case 2.5 {\n        print(2)\n    }\n}\n",
        // distinct byte payloads whose from_utf8_lossy forms collide stay
        // live: deep_eq compares raw bytes, the key must be exact
        "match some(b\"\\xef\\xbf\\xbd\") {\n    case Some(b\"\\xff\") {\n        print(1)\n    }\n    case Some(b\"\\xef\\xbf\\xbd\") {\n        print(2)\n    }\n}\n",
    ];
    for (i, src) in live.iter().enumerate() {
        let hits = w06(src);
        assert!(hits.is_empty(), "probe {}: {:?}", i, hits);
    }
}

#[test]
fn identical_literal_arms_belong_to_w05() {
    // one symptom, one owner: same literal twice is duplicate-match-arm's
    // finding (with a first-seen arm number); W06 defers to it
    let findings = lint_style(&parse(
        "match 7 {\n    case 7 {\n        print(1)\n    }\n    case 7 {\n        print(2)\n    }\n}\n",
    ));
    assert!(
        !findings.iter().any(|f| f.rule == "unreachable-match-arm"),
        "{:?}",
        findings
    );
    assert!(findings.iter().any(|f| f.rule == "duplicate-match-arm"));
}

#[test]
fn equivalent_set_arm_is_dead_too() {
    // Some(x) and Some(_) match the SAME set (any payload-carrying Some;
    // payload-less values miss both), so under first-match-wins the second
    // arm can never run. The algebra proves it: Bind subsumes Bind.
    let hits = w06(
        "match some(1) {\n    case Some(x) {\n        print(1)\n    }\n    case Some(_) {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
    assert!(hits[0].contains("broader arm first"), "{:?}", hits);
}

#[test]
fn suppression_by_comment_still_applies() {
    // the shared allow mechanism is line-local and applied at the CLI
    // surface: a comment on the finding's line (or the one above) drops it,
    // like every other lint rule. W06 anchors at the arm body's first
    // line-bearing statement, so the allow comment sits next to it
    let src = "match 1 {\n    case _ {\n        print(1)\n    }\n    case 1 {\n        # allow: unreachable-match-arm\n        print(2)\n    }\n}\n";
    let mut findings = operon::lint::lint_style(&parse(src));
    assert_eq!(findings.len(), 1, "{:?}", findings);
    operon::lint::apply_allows(&mut findings, src);
    assert!(findings.is_empty(), "allow comment must suppress");
}

#[test]
fn guard_on_later_arm_is_transparent_for_broader_blockers() {
    // Some(_) covers every payload-carrying Some; the later guard could only
    // shrink that already-covered set, and first-match-wins means it never
    // even evaluates. A guard on the DEAD side cannot resurrect the arm
    // (the catch-all case is pinned in src/lint.rs; this pins the broader
    // blocker path, where W05 is silent because the later arm is guarded)
    let hits = w06(
        "match some(1) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(x) if x > 0 {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
}

#[test]
fn guarded_narrower_arm_keeps_later_arms_live() {
    // the earlier arm can MISS (its guard), so it subsumes nothing even
    // though its shape alone would cover the later one: guards keep the
    // arms behind them reachable, always
    let hits = w06(
        "match some(1) {\n    case Some(x) if x > 5 {\n        print(1)\n    }\n    case Some(y) {\n        print(2)\n    }\n}\n",
    );
    assert!(hits.is_empty(), "{:?}", hits);
}

#[test]
fn identical_composite_arms_are_this_rules_story() {
    // W05 keys on plain literal arms only, so two identical ListPat arms
    // are flagged HERE, once: the second can never run under first-match-wins
    let hits = w06(
        "match [1, 2] {\n    case [1, 2] {\n        print(1)\n    }\n    case [1, 2] {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
}

#[test]
fn multi_overlap_partial_stays_live() {
    // the earlier run covers its 2 but not the later run's 3: a subject of 3
    // reaches the second arm, so the arm as a whole is reachable
    let hits = w06(
        "match 3 {\n    case 1, 2 {\n        print(1)\n    }\n    case 2, 3 {\n        print(2)\n    }\n}\n",
    );
    assert!(hits.is_empty(), "{:?}", hits);
}

#[test]
fn spanless_arm_body_anchors_at_line_one_and_suppresses_there() {
    // an arm whose body opens with a plain assignment carries no span, so
    // the finding anchors at the file's line 1; the allow comment must sit
    // ON line 1 (the line-above check saturates at 0, which never matches).
    // The match_v2.op corpus proof leans on exactly this mechanism for its
    // deliberate first-match-wins shadow probe
    let src = "# allow: unreachable-match-arm\nmatch 1 {\n    case _ {\n        r = 1\n    }\n    case 1 {\n        r = 2\n    }\n}\n";
    let mut findings = operon::lint::lint_style(&parse(src));
    assert_eq!(findings.len(), 1, "{:?}", findings);
    assert_eq!(findings[0].line, 1, "{:?}", findings);
    operon::lint::apply_allows(&mut findings, src);
    assert!(findings.is_empty(), "line-1 allow must suppress");
}

#[test]
fn fully_covered_or_alternatives_are_dead() {
    // the all-covered side of the or law: EVERY alternative of the later
    // or-pattern is covered by the earlier arm, so under first-match-wins
    // the whole arm can never run (the partial side stays live, pinned in
    // live_shapes_stay_silent_false_positive_budget)
    let hits = w06(
        "match some(1) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(1) | Some(y) {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
    assert!(hits[0].contains("broader arm first"), "{:?}", hits);
}

#[test]
fn earlier_or_alternative_covers_through_any_alt() {
    // an or-pattern matches through ANY alternative, so one covering
    // alternative (Some(_) here) is enough to block the payload arm behind
    // it; the None alternative only widens the earlier arm's own set
    let hits = w06(
        "match some(7) {\n    case None | Some(_) {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
    assert!(hits[0].contains("broader arm first"), "{:?}", hits);
}

#[test]
fn reordered_multi_run_is_dead() {
    // duplicate literal shapes stay this rule's story when they sit inside
    // composite arms (duplicate-match-arm only sees plain literal arms):
    // the earlier comma-run matches both values first, so the reordered run
    // can never win first-match-wins
    let hits = w06(
        "match 2 {\n    case 1, 2 {\n        print(1)\n    }\n    case 2, 1 {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
    assert!(hits[0].contains("broader arm first"), "{:?}", hits);
}

#[test]
fn unknown_tag_arm_is_a_runtime_true_catchall() {
    // the unknown-tag corner of the conservatism law: the parser degrades an
    // unknown capitalized tag to a whole-subject binding (runtime-true
    // catch-all, SPEC §5a), so arms behind it are genuinely dead and flagged
    // (pinned against the live interpreter in the W02-s2 session notes);
    // the ALGEBRA itself still never claims coverage for an unprovable
    // shape, floats and computed literals stay silent by design
    let hits = w06(
        "match some(1) {\n    case Blob(v) {\n        print(1)\n    }\n    case Some(y) {\n        print(2)\n    }\n}\n",
    );
    assert_eq!(hits.len(), 1, "{:?}", hits);
    assert!(hits[0].contains("unguarded catch-all"), "{:?}", hits);
}
