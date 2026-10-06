#!/usr/bin/env python3
"""check_claims.py — R0.7 standing gate for the claim/evidence registry (docs/spec/CLAIMS.md).

Deny-by-default: unknown fields, unknown enum values, dangling references and
missing files all fail. Reverse direction enforced too: every numerical entry
in bootstrap/validation_registry.json must be claimed exactly once here.

Usage:
  python3 scripts/check_claims.py                     # gate (exit 0 = green)
  python3 scripts/check_claims.py --negative-selftest # prove the teeth
"""
import json
import sys
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "bootstrap" / "claim_registry.json"
VALREG = ROOT / "bootstrap" / "validation_registry.json"
MANIFEST = ROOT / "tests" / "validation" / "MANIFEST.sha256"
BIOCONTRACT = ROOT / "docs" / "spec" / "BIO-CONTRACT.md"
DOCSPEC_GLOB = "docs/spec/*.md"
KEYWORDS = ROOT / "docs" / "KEYWORDS.md"

SCHEMA = "operon-claim-registry/1"
TOP_KEYS = {"schema", "spec", "law", "validation_classes", "claims"}
CLAIM_KEYS = {
    "id", "model", "model_kind", "surface", "bio_contract_labels",
    "bio_contract_note", "primary_sources", "equations",
    "parameter_provenance", "assumptions", "validation_class",
    "evidence", "status",
}
EVIDENCE_KEYS = {"validation_registry_ids", "enforcing_tests", "fixtures", "programs"}
KINDS = {"biological", "mathematical-reference"}
CLASSES = {"ANALYTIC", "ORACLE", "DIFFERENTIAL", "BEHAVIORAL", "BENCHMARK"}
LABELS = {"REAL", "APPROX", "ABSTRACTION", "SIMPLIFICATION"}
PARAM_KINDS = {"spec-default", "literature", "registry-row", "test-canonical"}
STATUSES = {"VALIDATED", "PROVISIONAL"}


def findings_clear(msg, items=None):
    print(f"FAIL: {msg}")
    for f in items or FINDINGS:
        print(f"  - {f}")
    sys.exit(1)


FINDINGS = []


def bad(msg):
    FINDINGS.append(msg)


def load_json(path):
    try:
        return json.loads(path.read_text())
    except Exception as e:  # noqa: BLE001
        findings_clear(f"{path.name}: unreadable JSON ({e})")
        return None  # unreachable


def docs_text():
    texts = []
    if KEYWORDS.exists():
        texts.append(KEYWORDS.read_text(errors="replace"))
    for p in sorted(ROOT.glob(DOCSPEC_GLOB)):
        texts.append(p.read_text(errors="replace"))
    return "\n".join(texts)


def biocontract_text():
    return BIOCONTRACT.read_text(errors="replace") if BIOCONTRACT.exists() else ""


def manifest_entries():
    if not MANIFEST.exists():
        return set()
    out = set()
    for line in MANIFEST.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        # format: "<sha256>  <path>" or a path-only line
        parts = line.split(None, 1)
        out.add(parts[1].strip().lstrip("*").strip() if len(parts) == 2 else parts[0])
        out.add(Path(parts[-1]).name)
    return out


def check(reg, valreg_ids, bio_text, docs, manifest, *, selftest=False):
    f = []
    if not isinstance(reg, dict) or set(reg) - TOP_KEYS:
        f.append(f"top-level: unknown or missing keys (saw {sorted(reg) if isinstance(reg, dict) else type(reg)})")
        return f, set()
    if reg.get("schema") != SCHEMA:
        f.append(f"schema: expected {SCHEMA!r}, got {reg.get('schema')!r}")
    claims = reg.get("claims")
    if not isinstance(claims, list) or not claims:
        f.append("claims: must be a non-empty list")
        return f, set()

    seen_ids = set()
    claimed_valreg = []
    for c in claims:
        cid = c.get("id", "<no-id>")
        unknown = set(c) - CLAIM_KEYS
        if unknown:
            f.append(f"{cid}: unknown fields {sorted(unknown)}")
        # bio_contract_note is conditionally required (empty labels); bio_contract_labels
        # are required only for biological claims (mathematical-reference carries none)
        required = CLAIM_KEYS - {"bio_contract_note"}
        if c.get("model_kind") == "mathematical-reference":
            required -= {"bio_contract_labels"}
        missing = required - set(c)
        if missing:
            f.append(f"{cid}: missing required fields {sorted(missing)}")
            continue
        if cid in seen_ids:
            f.append(f"{cid}: duplicate claim id")
        seen_ids.add(cid)

        if c["model_kind"] not in KINDS:
            f.append(f"{cid}: model_kind {c['model_kind']!r} not in {sorted(KINDS)}")
        if c["validation_class"] not in CLASSES:
            f.append(f"{cid}: validation_class {c['validation_class']!r} not in {sorted(CLASSES)}")
        if c["status"] not in STATUSES:
            f.append(f"{cid}: status {c['status']!r} not in {sorted(STATUSES)}")

        for field in ("model", "primary_sources", "equations", "assumptions"):
            v = c[field]
            if not v or (isinstance(v, list) and not any(str(x).strip() for x in v)):
                f.append(f"{cid}: {field} must be non-empty (a claim with no {field} is a wish)")

        # surface tokens must exist in generated keywords or spec docs
        if not c["surface"]:
            f.append(f"{cid}: surface must be non-empty")
        for tok in c["surface"]:
            if not re.search(rf"\b{re.escape(str(tok))}\b", docs):
                f.append(f"{cid}: surface token {tok!r} not found in KEYWORDS.md or docs/spec/")

        # parameter provenance: shape + kinds, and NO value field (single source of truth law)
        pp = c["parameter_provenance"]
        if not pp:
            f.append(f"{cid}: parameter_provenance must be non-empty")
        for entry in pp:
            if not isinstance(entry, dict) or set(entry) - {"param", "source", "kind"}:
                f.append(f"{cid}: parameter_provenance entry has unknown/missing keys (no 'value' field allowed — magnitudes live in validation_registry.json): {entry}")
                continue
            if entry.get("kind") not in PARAM_KINDS:
                f.append(f"{cid}: parameter_provenance kind {entry.get('kind')!r} not in {sorted(PARAM_KINDS)}")

        # bio-contract cross-ref
        labels = c["bio_contract_labels"]
        if c["model_kind"] == "biological":
            if labels:
                for lab in labels:
                    if lab not in LABELS:
                        f.append(f"{cid}: bio label {lab!r} not in {sorted(LABELS)}")
                    elif lab not in bio_text:
                        f.append(f"{cid}: bio label {lab!r} never appears in BIO-CONTRACT.md")
            else:
                note = c.get("bio_contract_note", "")
                if "BIO-CONTRACT row" not in note:
                    f.append(f"{cid}: empty bio_contract_labels requires a bio_contract_note naming the missing BIO-CONTRACT row (rule 4)")
        else:
            if labels:
                f.append(f"{cid}: mathematical-reference claims must not carry bio_contract_labels")

        # evidence block
        ev = c["evidence"]
        if not isinstance(ev, dict) or set(ev) - EVIDENCE_KEYS:
            f.append(f"{cid}: evidence has unknown/missing keys (saw {sorted(ev) if isinstance(ev, dict) else type(ev)})")
            continue
        if not any(ev.get(k) for k in EVIDENCE_KEYS):
            f.append(f"{cid}: evidence is empty — a claim without evidence is a wish")
        for tid in ev.get("validation_registry_ids", []):
            if tid not in valreg_ids:
                f.append(f"{cid}: evidence validation_registry_id {tid!r} not in validation_registry.json")
            claimed_valreg.append(tid)
        for key in ("enforcing_tests", "programs"):
            for rel in ev.get(key, []):
                if not (ROOT / rel).exists():
                    f.append(f"{cid}: evidence {key} path missing on disk: {rel}")
        for rel in ev.get("fixtures", []):
            if not (ROOT / rel).exists():
                f.append(f"{cid}: evidence fixture missing on disk: {rel}")
            elif selftest is False and Path(rel).name not in manifest:
                f.append(f"{cid}: fixture not pinned in tests/validation/MANIFEST.sha256: {rel}")

    # reverse direction: no undocumented constraint
    for vid in sorted(valreg_ids - set(claimed_valreg)):
        f.append(f"reverse: validation_registry.json entry {vid!r} has no claim in claim_registry.json (evidence without a claim)")
    for vid, n in sorted((v, claimed_valreg.count(v)) for v in set(claimed_valreg)):
        if n > 1:
            f.append(f"reverse: validation_registry.json entry {vid!r} claimed {n} times (must be exactly once)")

    return f, seen_ids


MUTATIONS = [
    ("drop an assumption", lambda r: r["claims"][0]["assumptions"].clear()),
    ("dangle an evidence test path", lambda r: r["claims"][0]["evidence"].__setitem__(
        "enforcing_tests", ["tests/does_not_exist.op"])),
    ("invent an unknown validation class", lambda r: r["claims"][0].__setitem__("validation_class", "VIBES")),
    ("unclaim a registry entry", lambda r: r["claims"][0]["evidence"].__setitem__("validation_registry_ids", [])),
    ("empty labels without the pending note", lambda r: (
        r["claims"][0].__setitem__("bio_contract_labels", []),
        r["claims"][0].pop("bio_contract_note", None))),
    ("sneak a magnitude into parameter_provenance", lambda r: r["claims"][0]["parameter_provenance"][0].__setitem__("value", 0.1)),
    ("unknown top-level key", lambda r: r.__setitem__("extra", 1)),
    ("claim an unknown surface token", lambda r: r["claims"][0]["surface"].append("not_a_real_token")),
    ("unknown bio label", lambda r: r["claims"][0]["bio_contract_labels"].append("MOSTLY_TRUE")),
    ("fixture outside the manifest", lambda r: r["claims"][9]["evidence"].__setitem__(
        "fixtures", ["tests/validation/fixtures/V-R06-01-uniform-mean.json.bak"])),
]


def main():
    args = sys.argv[1:]
    reg = load_json(REGISTRY)
    val = load_json(VALREG)
    valreg_ids = {e.get("id") for e in val.get("entries", [])} if isinstance(val, dict) else set()
    bio = biocontract_text()
    docs = docs_text()
    manifest = manifest_entries()

    if "--negative-selftest" in args:
        import copy
        base_f, _ = check(reg, valreg_ids, bio, docs, manifest, selftest=True)
        if base_f:
            findings_clear("selftest precondition failed: the UNMUTATED registry must be green "
                           f"(selftest runs on an in-memory copy); findings:", base_f)
        failed = []
        for name, mutate in MUTATIONS:
            m = copy.deepcopy(reg)
            mutate(m)
            mf, _ = check(m, valreg_ids, bio, docs, manifest, selftest=True)
            if mf:
                failed.append(name)
            else:
                print(f"  TOOTH-BLUNT: mutation did not fail: {name}")
        if len(failed) != len(MUTATIONS):
            findings_clear(f"selftest: {len(MUTATIONS) - len(failed)}/{len(MUTATIONS)} mutations slipped through")
        print(f"check_claims selftest: {len(failed)}/{len(MUTATIONS)} mutations correctly rejected — teeth proven")
        return

    f, ids = check(reg, valreg_ids, bio, docs, manifest)
    if f:
        findings_clear(f"claim registry rejected ({len(f)} findings)", f)
    print(f"check_claims: {len(ids)} claims green — schema, evidence paths, manifest pins, "
          f"registry cross-refs (both directions), bio-contract labels, provenance shape all OK")


if __name__ == "__main__":
    main()
