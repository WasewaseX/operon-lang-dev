// W066: `.cell` schema pins for the parse lane. The runtime loader and the
// check-time lint engine (`src/lint.rs`) tell ONE story about the same file:
// these tests pin the shared rule vocabulary (W10 `cell-unknown-key` /
// W11 `cell-type-mismatch`), the closest-key typo hint, the silent-parse
// contract for every declared key, and the frozen `parse_cell` map behavior
// (garbage in -> map out, never a rejection, never a panic).

use operon::genes::{
    parse_cell, parse_cell_checked, CellKind, CELL_SCHEMA, CELL_TYPE_MISMATCH, CELL_UNKNOWN_KEY,
};
use std::collections::HashMap;

/// Every exact schema key with a representative valid value (families are
/// covered by allow/ligand/variant entries below).
fn valid_cell() -> String {
    let mut lines = vec![
        "allow.read = /tmp".to_string(),
        "allow.write = /tmp, /var/out".to_string(),
        "ligand.atp = 0.4".to_string(),
        "variant.lacZ = B".to_string(),
    ];
    for spec in CELL_SCHEMA {
        if spec.name.ends_with('*') {
            continue; // families staged above
        }
        let v = match spec.kind {
            CellKind::Number => "0.5",
            CellKind::Integer => "10",
            CellKind::Bool => "true",
            CellKind::Str => "value",
        };
        lines.push(format!("{} = {}", spec.name, v));
    }
    lines.join("\n")
}

#[test]
fn every_declared_key_parses_silently() {
    let src = valid_cell();
    let (map, notes) = parse_cell_checked(&src);
    assert!(
        notes.is_empty(),
        "valid values for every schema key must stay silent: {:?}",
        notes
    );
    for spec in CELL_SCHEMA {
        if spec.name.ends_with('*') {
            continue;
        }
        assert!(map.contains_key(spec.name), "missing {}", spec.name);
    }
    // families land under their prefix
    assert_eq!(map.get("allow.read").unwrap(), "/tmp");
    assert_eq!(map.get("ligand.atp").unwrap(), "0.4");
    assert_eq!(map.get("variant.lacZ").unwrap(), "B");
}

#[test]
fn sections_compose_like_the_loader() {
    // [section] + key = value must form section.key (loader contract, pinned
    // against tests/granted/rho_prob.cell's shape)
    let (map, notes) = parse_cell_checked("[rho]\ntermination = true\ncatch = 1.0\n");
    assert!(notes.is_empty(), "{:?}", notes);
    assert_eq!(map.get("rho.termination").unwrap(), "true");
    assert_eq!(map.get("rho.catch").unwrap(), "1.0");
}

#[test]
fn unknown_key_note_names_key_and_closest_known_key() {
    let (map, notes) = parse_cell_checked("m6a.reader.decayy = 0.5\n");
    // Total Grammar: the key is still delivered to the map (ignored silently
    // by the engine), the note is advisory
    assert_eq!(map.get("m6a.reader.decayy").unwrap(), "0.5");
    assert_eq!(notes.len(), 1, "{:?}", notes);
    assert_eq!(notes[0].rule, CELL_UNKNOWN_KEY);
    assert_eq!(notes[0].line, 1);
    assert!(
        notes[0].message.contains("'m6a.reader.decayy'"),
        "names the key: {}",
        notes[0].message
    );
    assert!(
        notes[0]
            .message
            .contains("did you mean 'm6a.reader.decay'?"),
        "closest-key hint: {}",
        notes[0].message
    );
}

#[test]
fn section_dropped_key_gets_a_suffix_hint() {
    // the classic accident: header line mangled, the key lands top-level
    // ("decay" instead of "[m6a] decay"). Deterministic tie-break picks the
    // schema-order-first candidate among the "…decay" family.
    let (_, notes) = parse_cell_checked("decay = 0.5\n");
    assert_eq!(notes.len(), 1, "{:?}", notes);
    assert_eq!(notes[0].rule, CELL_UNKNOWN_KEY);
    assert!(
        notes[0].message.contains("did you mean 'grn.decay'?"),
        "suffix-affinity hint: {}",
        notes[0].message
    );
}

#[test]
fn hopeless_keys_get_no_hint_but_still_note() {
    let (_, notes) = parse_cell_checked("zzz = 1\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].rule, CELL_UNKNOWN_KEY);
    assert!(!notes[0].message.contains("did you mean"), "{:?}", notes);
}

#[test]
fn type_mismatch_note_names_expected_and_got() {
    let (_, notes) = parse_cell_checked("methylate.threshold = abc\n");
    assert_eq!(notes.len(), 1, "{:?}", notes);
    assert_eq!(notes[0].rule, CELL_TYPE_MISMATCH);
    assert!(
        notes[0].message.contains("expects an integer, got 'abc'"),
        "{}",
        notes[0].message
    );

    let (_, notes) = parse_cell_checked("grn.decay = fast\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].rule, CELL_TYPE_MISMATCH);
    assert!(
        notes[0].message.contains("expects number, got 'fast'"),
        "{}",
        notes[0].message
    );

    let (_, notes) = parse_cell_checked("rho.termination = maybe\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].rule, CELL_TYPE_MISMATCH);
    assert!(
        notes[0]
            .message
            .contains("expects a bool ('true'/'false'), got 'maybe'"),
        "{}",
        notes[0].message
    );

    // allow.exit is the boolean member of the grant family
    let (_, notes) = parse_cell_checked("allow.exit = sort-of\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].rule, CELL_TYPE_MISMATCH);
}

#[test]
fn engine_bool_vocabulary_is_accepted() {
    // scope.cancel_on_error flips OFF on "off"; the rest arm on "true"
    let (_, notes) = parse_cell_checked(
        "scope.cancel_on_error = off\nrho.termination = true\nmethylate.quiet = false\n",
    );
    assert!(notes.is_empty(), "{:?}", notes);
}

#[test]
fn garbage_payload_never_rejects_and_map_is_frozen() {
    // tests/redteam/rt_p7g_cell_garbage.cell payload shape: brackets soup,
    // empty values, dangling quotes. Parse is total; notes are advisory;
    // the map is exactly what the (unchanged) parse_cell always produced.
    let garbage = "[[[[\nkey === ===\n[unclosed\n= = =\nallow.read = \n\"\"\"\n";
    let map = parse_cell(garbage);
    let (checked_map, _notes) = parse_cell_checked(garbage);
    assert_eq!(map.get("allow.read").unwrap(), "");
    assert_eq!(map.get("key").unwrap(), "== ===");
    assert_eq!(map.get("").unwrap(), "= =");
    assert_eq!(map, checked_map, "parse_cell == parse_cell_checked.0");
    assert_eq!(map.len(), 3, "total parse: only real key=value lines land");
}

#[test]
fn frozen_map_parity_on_real_fixtures() {
    // byte-for-byte legacy behavior on the shapes the repo ships (rho_prob,
    // pybridge, visibility_strict, timing cells), garbage included
    let cases: Vec<&str> = vec![
        "[rho]\ntermination = true\ncatch = 0.5\n[ribosome]\nqueue_cap = 0.0\n",
        "[allow]\npy = math, json, time, numpy\n\n[py]\ntimeout_ms = 5000\n",
        "modules.visibility = strict\n",
        "[allow]\nrun = \"sleep\"\n",
        "# only a comment\n\n",
        "",
        "allow.read = /etc\n",
        "6a]\ndecay = 0.5\n",
    ];
    let expected: fn(&str) -> HashMap<String, String> = legacy_parse_cell;
    for src in cases {
        assert_eq!(
            parse_cell(src),
            expected(src),
            "parse_cell drifted from legacy on {:?}",
            src
        );
        let (m, _) = parse_cell_checked(src);
        assert_eq!(m, expected(src));
    }
}

/// The legacy algorithm, verbatim from before the W066 schema work (the
/// reference this file pins parse_cell against).
fn legacy_parse_cell(src: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = t.find('=') {
            let k = t[..eq].trim();
            let v = t[eq + 1..].trim().trim_matches('"').to_string();
            let key = if section.is_empty() {
                k.to_string()
            } else {
                format!("{}.{}", section, k)
            };
            out.insert(key, v);
        }
    }
    out
}

#[test]
fn check_time_lint_and_parse_lane_agree_on_rule_names() {
    // one story: for any payload, the rule vocabulary emitted at check time
    // (src/lint.rs lint_cell) is the SAME set the parse lane emits
    let src = "methylate.threshold = abc\nm6a.reader.decayy = 1\nallow.read = /tmp\n";
    let bindings = operon::lint::lint_cell(src);
    let lint_rules: Vec<&str> = bindings.iter().map(|f| f.rule.as_str()).collect();
    let parse_rules: Vec<&str> = parse_cell_checked(src).1.iter().map(|n| n.rule).collect();
    assert_eq!(parse_rules.len(), 2, "{:?}", parse_rules);
    for r in &parse_rules {
        assert!(
            lint_rules.contains(r),
            "parse lane rule '{}' missing from check-time lint ({:?})",
            r,
            lint_rules
        );
    }
}

#[test]
fn schema_doc_lists_every_declared_key() {
    // docs/specs/CELL-SCHEMA.md is the human mirror of CELL_SCHEMA; the two
    // must move in the same PR (doc rule 4, now mechanically checked)
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/specs/CELL-SCHEMA.md"
    ))
    .expect("CELL-SCHEMA.md is checked in");
    for spec in CELL_SCHEMA {
        let needle = format!("`{}`", spec.name);
        assert!(
            doc.contains(&needle),
            "docs/specs/CELL-SCHEMA.md does not list `{}`",
            spec.name
        );
    }
}

#[test]
fn unicode_keys_do_not_confuse_the_hint() {
    // byte-level Levenshtein must not panic on multi-byte keys
    let (_, notes) = parse_cell_checked("그것 = 1\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].rule, CELL_UNKNOWN_KEY);
}

#[test]
fn allow_grants_are_redacted_from_methyl_reads() {
    // sec-r3 (re-audit #15): capability grants are the host's business, the
    // program must not read them back through the introspection door — the
    // Rust core redacts `allow.*` from methyl() (pinned through the CLI, the
    // surface the loader actually drives). Known oracle-lane gap: the Python
    // mirror does not redact yet (reported to the integrator), so this pin
    // is deliberately Rust-lane only.
    use std::fs;
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("op_cell_schema_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let op = dir.join("redact.op");
    let cell = dir.join("redact.cell");
    fs::write(
        &op,
        "main { print(methyl(\"allow.read\", \"redacted\")) }\n",
    )
    .unwrap();
    fs::write(&cell, "allow.read = /etc/hostname\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&op)
        .arg("--cell")
        .arg(&cell)
        .output()
        .expect("run operon");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    fs::remove_dir_all(&dir).ok();
    assert_eq!(
        stdout.trim(),
        "redacted",
        "allow.* must be redacted from methyl() reads, got: {stdout}"
    );
}
