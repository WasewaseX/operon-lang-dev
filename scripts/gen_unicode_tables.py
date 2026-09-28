#!/usr/bin/env python3
"""gen_unicode_tables.py, W028 stage 2: emit src/unicode_tables.rs FROM Python's
unicodedata. The Python stdlib module IS the reference implementation here: the
oracle (bootstrap/oracle.py) calls the very same module, so the two cores agree
by construction instead of by hand-copied tables (the W028 critique: hand-pulled
normalization tables are error prone, the generator is committed next to them).

What is emitted (data only; the runtime algorithms live in src/interp.rs):
  DECOMP_INDEX + DECOMP_POOL   canonical decompositions, fully expanded
                               (unicodedata.normalize('NFD', ch), so each
                               entry is already canonically ordered). Hangul
                               syllables AC00..D7A3 are NOT tabulated, the
                               cores expand them algorithmically.
  COMPOSE_PAIRS                (starter, combining) -> composite, derived
                               EMPIRICALLY: for every codepoint c whose
                               NFC(NFD(c)) == c with a 2+ char NFD, the pairs
                               its re-composition walk needs. Composition
                               exclusions (Full_Composition_Exclusion) are
                               honored by construction: a codepoint that fails
                               the round trip contributes no pair.
  CCC_RUNS                     combining classes, run-length encoded.
  FOLD_SINGLE / FOLD_MULTI     full case folding (C+F), chr(cp).casefold(),
                               identity mappings skipped.
  CAT_RUNS + CAT_NAMES         general category, run-length encoded.

Canonical ordering is NOT pre-flattened across codepoint boundaries: each
stored expansion is ordered, but the cores re-apply the ordering pass over the
concatenated string (stable ccc sort per combining run), the classic UAX #15
pipeline: decompose, stability-sort by combining class, then compose left to
right with the blocking rule.

Verification (printed; the script EXITS 1 if any check fails, no tables are
written on failure): the exact runtime algorithm below is run against
unicodedata for
  (1) every codepoint 0..0x110000: model NFD(ch) == unicodedata NFD(ch),
      model NFC(NFD(ch)) == unicodedata NFC(ch) (the round-trip bar),
      model NFC(ch) == unicodedata NFC(ch);
  (2) every combining-mark pair after a starter (canonical-order reordering
      and blocking), every composite followed by every combining mark, and
      Hangul jamo sweeps (L+V, LV+T, L+V+T);
  (3) casefold context-freedom: the per-char table applied over a deterministic
      string sample equals str.casefold() on the whole string (folding has NO
      context rules, unlike lowercase: final sigma folds unconditionally);
  (4) the category RLE reconstructs unicodedata.category for all codepoints.
Run: python3 scripts/gen_unicode_tables.py    (from the repo root)
"""
import random
import sys
import unicodedata

MAX_CP = 0x110000
UNIDATA_VERSION = unicodedata.unidata_version

# ---------------------------------------------------------------- model
# The exact algorithm the Rust core implements (src/interp.rs, W28 section).
# Kept here in one place so generator verification and runtime share semantics.


def ccc_of(cp):
    return unicodedata.combining(chr(cp))


def canon_order(chars):
    """Stable sort of each maximal run of nonzero-ccc chars (UAX #15)."""
    out = []
    run = []
    for cp in chars:
        if ccc_of(cp) == 0:
            run.sort(key=ccc_of)  # Python sort is stable
            out.extend(run)
            run = []
            out.append(cp)
        else:
            run.append(cp)
    run.sort(key=ccc_of)
    out.extend(run)
    return out


def hangul_decomp(cp):
    """Hangul syllable -> jamo list, or None. Algorithmic per UAX #15."""
    if 0xAC00 <= cp <= 0xD7A3:
        s = cp - 0xAC00
        l = 0x1100 + s // 588
        v = 0x1161 + (s % 588) // 28
        if s % 28 == 0:
            return [l, v]
        return [l, v, 0x11A7 + s % 28]
    return None


def model_nfd(cps):
    out = []
    for cp in cps:
        h = hangul_decomp(cp)
        if h is not None:
            out.extend(h)
        else:
            d = DECOMP.get(cp)
            if d is not None:
                out.extend(d)
            else:
                out.append(cp)
    return canon_order(out)


def compose_pair(a, b):
    """(starter, next) -> composite. Hangul L+V and LV+T are algorithmic."""
    if 0x1100 <= a <= 0x1112 and 0x1161 <= b <= 0x1175:
        return 0xAC00 + ((a - 0x1100) * 21 + (b - 0x1161)) * 28
    if 0xAC00 <= a <= 0xD7A3 and (a - 0xAC00) % 28 == 0 and 0x11A8 <= b <= 0x11C2:
        return a + (b - 0x11A7)
    return COMPOSE.get((a, b))


def model_nfc(cps):
    chars = canon_order(model_nfd(cps))
    if not chars:
        return chars
    out = []
    starter = None
    starter_pos = 0
    last_cc = 0
    for cp in chars:
        cc = ccc_of(cp)
        if starter is not None and (last_cc == 0 or last_cc < cc):
            comp = compose_pair(starter, cp)
            if comp is not None:
                out[starter_pos] = comp
                starter = comp
                continue
        if cc == 0:
            starter = cp
            starter_pos = len(out)
        last_cc = cc
        out.append(cp)
    return out


def model_casefold(cps):
    out = []
    for cp in cps:
        f = FOLD_MULTI.get(cp)
        if f is not None:
            out.extend(f)
        else:
            f = FOLD_SINGLE.get(cp)
            out.append(f if f is not None else cp)
    return out


def model_category(cp):
    lo, hi = 0, len(CAT_RUNS) - 1
    while lo <= hi:
        mid = (lo + hi) // 2
        start, end, idx = CAT_RUNS[mid]
        if cp < start:
            hi = mid - 1
        elif cp > end:
            lo = mid + 1
        else:
            return CAT_NAMES[idx]
    return "Cn"


# ---------------------------------------------------------------- collect

print(f"collecting unicode data (unicodedata {UNIDATA_VERSION}) ...")

DECOMP = {}        # cp -> tuple of cps, canonical, fully expanded, ordered
for cp in range(MAX_CP):
    ch = chr(cp)
    if 0xAC00 <= cp <= 0xD7A3:
        continue  # Hangul is algorithmic, never tabulated
    d = unicodedata.normalize("NFD", ch)
    if d != ch:
        DECOMP[cp] = tuple(ord(c) for c in d)

FIRSTLEVEL = {}    # cp -> one-level decomposition (UnicodeData text field)
for cp in range(MAX_CP):
    if 0xAC00 <= cp <= 0xD7A3:
        continue  # Hangul is algorithmic, never tabulated
    d = unicodedata.decomposition(chr(cp))
    if not d or d.startswith("<"):
        continue  # no decomposition, or a <compat>/<super>/<fraction> form
    FIRSTLEVEL[cp] = tuple(int(x, 16) for x in d.split())

COMPOSE = {}       # (a, b) -> composite
# Pairs come from ONE-LEVEL decompositions, not the fully expanded form: a
# character like U+01D5 (U with diaeresis and macron) decomposes to
# (U+00DC, U+0304) at the first level and the composition walk composes
# U+0055 + U+0308 -> U+00DC -> U+01D5 through exactly those intermediates.
# The NFC-stability gate IS the Full_Composition_Exclusion filter: a
# codepoint whose NFC(NFD(c)) != c contributes no pair (e.g. U+0344, the
# Tibetan double-vowel forms).
multi_pending = []  # (cp, seq) for 3+ char composable decompositions
for cp, seq in sorted(FIRSTLEVEL.items()):
    if len(seq) < 2:
        continue  # singletons never compose
    if unicodedata.normalize("NFC", chr(cp)) != chr(cp):
        continue  # not NFC-stable: contributes no pair (exclusions by construction)
    if len(seq) == 2:
        prev = COMPOSE.get((seq[0], seq[1]))
        if prev is not None and prev != cp:
            sys.exit(f"FATAL: pair ({seq[0]:04X}, {seq[1]:04X}) maps to both "
                     f"{prev:04X} and {cp:04X}")
        COMPOSE[(seq[0], seq[1])] = cp
    else:
        multi_pending.append((cp, seq))

# Failsafe: 3+ char one-level decompositions re-compose through intermediate
# pairs derived from two-char entries (none exist in Unicode 15.0 outside
# Hangul, which is algorithmic; any gap is a derivation bug, fail loudly).
changed = True
while multi_pending and changed:
    changed = False
    still = []
    for cp, seq in multi_pending:
        cur = seq[0]
        ok = True
        for nxt in seq[1:]:
            comp = compose_pair(cur, nxt)
            if comp is None:
                # derive the intermediate pair empirically: the unique
                # codepoint whose NFD is exactly (cur, nxt) and is NFC-stable
                target = None
                for cp2, seq2 in DECOMP.items():
                    if len(seq2) == 2 and seq2[0] == cur and seq2[1] == nxt:
                        if unicodedata.normalize("NFC", chr(cp2)) == chr(cp2):
                            if target is not None:
                                sys.exit(f"FATAL: ambiguous intermediate "
                                         f"composite for ({cur:04X}, {nxt:04X})")
                            target = cp2
                if target is None:
                    ok = False
                    break
                COMPOSE[(cur, nxt)] = target
                comp = target
                changed = True
            cur = comp
        if not ok:
            still.append((cp, seq))
        elif cur != cp:
            sys.exit(f"FATAL: recomposition walk of U+{cp:04X} landed on "
                     f"U+{cur:04X}")
    multi_pending = still
if multi_pending:
    sys.exit(f"FATAL: {len(multi_pending)} multi-char decompositions could not "
             "be recomposed from derived pairs")

FOLD_SINGLE = {}   # cp -> cp
FOLD_MULTI = {}    # cp -> tuple of cps
for cp in range(MAX_CP):
    ch = chr(cp)
    f = ch.casefold()
    if f == ch:
        continue
    cfs = [ord(c) for c in f]
    if len(cfs) == 1:
        FOLD_SINGLE[cp] = cfs[0]
    else:
        FOLD_MULTI[cp] = tuple(cfs)

CAT_NAMES = []     # two-letter category strings, indexed by CAT_RUNS
CAT_RUNS = []      # (start, end, name index)
prev_name = None
run_start = 0
for cp in range(MAX_CP):
    name = unicodedata.category(chr(cp))
    if name != prev_name:
        if prev_name is not None:
            if prev_name not in CAT_NAMES:
                CAT_NAMES.append(prev_name)
            CAT_RUNS.append((run_start, cp - 1, CAT_NAMES.index(prev_name)))
        prev_name = name
        run_start = cp
if prev_name not in CAT_NAMES:
    CAT_NAMES.append(prev_name)
CAT_RUNS.append((run_start, MAX_CP - 1, CAT_NAMES.index(prev_name)))

print(f"  decompositions: {len(DECOMP)} entries (Hangul excluded, algorithmic)")
print(f"  composition pairs: {len(COMPOSE)}")
print(f"  casefold: {len(FOLD_SINGLE)} single-char, {len(FOLD_MULTI)} multi-char")
print(f"  categories: {len(CAT_RUNS)} runs over {len(CAT_NAMES)} names")

# ---------------------------------------------------------------- verify

print("verification 1/4: per-codepoint NFC/NFD round trip over "
      f"{MAX_CP} codepoints ...")
nfd_hits = nfc_hits = 0
for cp in range(MAX_CP):
    ch = chr(cp)
    want_nfd = [ord(c) for c in unicodedata.normalize("NFD", ch)]
    want_nfc = [ord(c) for c in unicodedata.normalize("NFC", ch)]
    if model_nfd([cp]) != want_nfd:
        sys.exit(f"FAIL: model NFD of U+{cp:04X} diverges from unicodedata")
    nfd_hits += 1
    if model_nfc(model_nfd([cp])) != want_nfc:
        sys.exit(f"FAIL: NFC(NFD(U+{cp:04X})) != unicodedata NFC(U+{cp:04X})")
    nfc_hits += 1
    if model_nfc([cp]) != want_nfc:
        sys.exit(f"FAIL: model NFC of U+{cp:04X} diverges from unicodedata")
print(f"  OK: {nfd_hits} NFD and {nfc_hits} NFC round trips match unicodedata")

print("verification 2/4: combining-order and composition sweeps ...")
marks = sorted(cp for cp in range(MAX_CP) if ccc_of(cp) > 0)
sweep = 0
for m1 in marks:
    for m2 in marks:
        cps = [0x61, m1, m2]
        s = "".join(chr(c) for c in cps)
        want_nfc = [ord(c) for c in unicodedata.normalize("NFC", s)]
        want_nfd = [ord(c) for c in unicodedata.normalize("NFD", s)]
        if model_nfc(cps) != want_nfc or model_nfd(cps) != want_nfd:
            sys.exit("FAIL: mark-pair sweep diverges on "
                     + " ".join(f"U+{c:04X}" for c in cps))
        sweep += 1
composites = sorted(set(COMPOSE.values()))
for cp in composites:
    for m in marks:
        cps = [cp, m]
        s = "".join(chr(c) for c in cps)
        want = [ord(c) for c in unicodedata.normalize("NFC", s)]
        if model_nfc(cps) != want:
            sys.exit("FAIL: composite sweep diverges on "
                     + " ".join(f"U+{c:04X}" for c in cps))
        sweep += 1
for l in range(0x1100, 0x1113):
    for v in range(0x1161, 0x1176):
        for t in [None] + list(range(0x11A8, 0x11C3)):
            cps = [l, v] if t is None else [l, v, t]
            s = "".join(chr(c) for c in cps)
            want_nfc = [ord(c) for c in unicodedata.normalize("NFC", s)]
            want_nfd = [ord(c) for c in unicodedata.normalize("NFD", s)]
            if model_nfc(cps) != want_nfc or model_nfd(cps) != want_nfd:
                sys.exit("FAIL: Hangul sweep diverges on "
                         + " ".join(f"U+{c:04X}" for c in cps))
            sweep += 1
print(f"  OK: {sweep} sweep strings match unicodedata")

print("verification 3/4: casefold context-freedom on deterministic sample ...")
# Folding is context-free (unlike lowercasing, which owns the final-sigma
# rule), so per-char application must equal whole-string casefold. Prove it on
# a fixed-seed sample, then also assert the sigma forms fold identically in
# every surrounding context.
rng = random.Random(0x0F)
sample = 0
while sample < 20000:
    n = rng.randint(1, 6)
    cps = [rng.choice(range(MAX_CP)) for _ in range(n)]
    s = "".join(chr(c) for c in cps)
    if not s.isprintable():
        continue
    got = "".join(chr(c) for c in model_casefold(cps))
    if got != s.casefold():
        sys.exit(f"FAIL: casefold sample diverges on {s!r}")
    sample += 1
sigma = 0
for cp in range(MAX_CP):
    cps = [cp, 0x3C2, cp]   # final sigma flanked on both sides
    s = "".join(chr(c) for c in cps)
    got = "".join(chr(c) for c in model_casefold(cps))
    if got != s.casefold():
        sys.exit(f"FAIL: sigma context diverges around U+{cp:04X}")
    sigma += 1
print(f"  OK: {sample} sampled strings, {sigma} sigma-context strings match "
      "str.casefold")

print("verification 4/4: category run-length reconstruction ...")
cat_checked = 0
for cp in range(MAX_CP):
    if model_category(cp) != unicodedata.category(chr(cp)):
        sys.exit(f"FAIL: category of U+{cp:04X} diverges")
    cat_checked += 1
print(f"  OK: {cat_checked} categories reconstructed from RLE")

# ---------------------------------------------------------------- emit

HEADER = f"""//! unicode_tables.rs, GENERATED by scripts/gen_unicode_tables.py. DO NOT EDIT.
//!
//! W028 stage 2: source of truth is Python's unicodedata {UNIDATA_VERSION}
//! (the oracle imports the same module, both cores agree by construction).
//! Regenerate: python3 scripts/gen_unicode_tables.py
//!
//! Data plus tiny lookup accessors; the normalization algorithms (canonical
//! ordering, left-to-right composition with blocking, Hangul) live in
//! src/interp.rs. Shapes:
//!   DECOMP_INDEX/DECOMP_POOL  canonical decompositions, fully expanded,
//!                             sorted by codepoint, binary search; Hangul
//!                             syllables AC00..D7A3 are algorithmic (interp)
//!   COMPOSE_PAIRS             (starter, next) -> composite, sorted, binary
//!                             search; Hangul L+V / LV+T are algorithmic
//!   CCC_RUNS                  combining classes, RLE, binary search
//!   FOLD_SINGLE/FOLD_MULTI    full case folding (C+F), sorted, binary search
//!   CAT_RUNS/CAT_NAMES        general category, RLE, binary search
//! All `end` bounds are INCLUSIVE. Surrogate codepoints appear only inside
//! category runs (Rust `str` cannot hold them, the oracle cannot feed them
//! through `str` either).

pub const UNIDATA_VERSION: &str = "{UNIDATA_VERSION}";
"""

ACCESSORS = '''
/// Canonical decomposition of one codepoint, fully expanded (empty when the
/// codepoint has none). Hangul syllables are handled by the caller (interp).
pub fn canon_decomp(cp: u32) -> &'static [u32] {
    let i = DECOMP_INDEX.binary_search_by(|(c, _, _)| c.cmp(&cp));
    match i {
        Ok(i) => {
            let (_, off, len) = DECOMP_INDEX[i];
            let off = off as usize;
            &DECOMP_POOL[off..off + len as usize]
        }
        Err(_) => &[],
    }
}

/// Table lookup for one composition pair (Hangul is algorithmic in interp).
pub fn compose_table(a: u32, b: u32) -> Option<u32> {
    let i = COMPOSE_PAIRS.binary_search_by(|(a2, b2, _)| (*a2, *b2).cmp(&(a, b)));
    i.ok().map(|i| COMPOSE_PAIRS[i].2)
}

/// Combining class (0 when the codepoint has none or is unlisted).
pub fn ccc(cp: u32) -> u8 {
    let i = CCC_RUNS.binary_search_by(|(s, e, _)| {
        if cp < *s {
            std::cmp::Ordering::Greater
        } else if cp > *e {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });
    i.ok().map(|i| CCC_RUNS[i].2).unwrap_or(0)
}

/// Full case folding of one codepoint (None = identity).
pub fn fold_char(cp: u32) -> Option<&'static [u32]> {
    let i = FOLD_SINGLE.binary_search_by(|(c, _)| c.cmp(&cp));
    if let Ok(i) = i {
        return Some(std::slice::from_ref(&FOLD_SINGLE[i].1));
    }
    let i = FOLD_MULTI.binary_search_by(|(c, _)| c.cmp(&cp));
    i.ok().map(|i| FOLD_MULTI[i].1)
}

/// General category of one codepoint (two-letter abbreviation, "Cn" when the
/// binary search somehow misses).
pub fn category(cp: u32) -> &'static str {
    let i = CAT_RUNS.binary_search_by(|(s, e, _)| {
        if cp < *s {
            std::cmp::Ordering::Greater
        } else if cp > *e {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });
    i.ok()
        .map(|i| CAT_NAMES[CAT_RUNS[i].2 as usize])
        .unwrap_or("Cn")
}
'''


def u32s(cps):
    return ", ".join(f"0x{c:X}" for c in cps)


lines = [HEADER]

lines.append("\n// canonical decompositions (codepoint, pool offset, pool length)\n")
lines.append("pub static DECOMP_INDEX: &[(u32, u32, u32)] = &[\n")
pool = []
index = []
for cp in sorted(DECOMP):
    seq = DECOMP[cp]
    index.append((cp, len(pool), len(seq)))
    pool.extend(seq)
for cp, off, ln in index:
    lines.append(f"    (0x{cp:X}, {off}, {ln}),\n")
lines.append("];\n")
lines.append("\n// expanded decompositions, one flat codepoint pool\n")
lines.append("pub static DECOMP_POOL: &[u32] = &[\n")
for i in range(0, len(pool), 10):
    lines.append("    " + u32s(pool[i:i + 10]) + ",\n")
lines.append("];\n")

lines.append("\n// composition pairs (starter, next, composite), sorted by (starter, next)\n")
lines.append("pub static COMPOSE_PAIRS: &[(u32, u32, u32)] = &[\n")
for (a, b), c in sorted(COMPOSE.items()):
    lines.append(f"    (0x{a:X}, 0x{b:X}, 0x{c:X}),\n")
lines.append("];\n")

lines.append("\n// combining classes (start, end inclusive, class), sorted\n")
lines.append("pub static CCC_RUNS: &[(u32, u32, u8)] = &[\n")
runs = []
prev = 0
run_start = 0
for cp in range(MAX_CP):
    k = unicodedata.combining(chr(cp))
    if k != prev:
        if prev != 0:
            runs.append((run_start, cp - 1, prev))
        prev = k
        run_start = cp
if prev != 0:
    runs.append((run_start, MAX_CP - 1, prev))
for s, e, k in runs:
    lines.append(f"    (0x{s:X}, 0x{e:X}, {k}),\n")
lines.append("];\n")

lines.append("\n// full case folding, single-char mappings (codepoint, folded)\n")
lines.append("pub static FOLD_SINGLE: &[(u32, u32)] = &[\n")
for cp, t in sorted(FOLD_SINGLE.items()):
    lines.append(f"    (0x{cp:X}, 0x{t:X}),\n")
lines.append("];\n")
lines.append("\n// full case folding, multi-char mappings (codepoint, folded slice)\n")
lines.append("pub static FOLD_MULTI: &[(u32, &[u32])] = &[\n")
for cp, t in sorted(FOLD_MULTI.items()):
    lines.append(f"    (0x{cp:X}, &[{u32s(t)}]),\n")
lines.append("];\n")

lines.append("\n// general category names, indexed by CAT_RUNS third field\n")
lines.append("pub static CAT_NAMES: &[&str] = &[\n")
for n in CAT_NAMES:
    lines.append(f'    "{n}",\n')
lines.append("];\n")
lines.append("\n// general category runs (start, end inclusive, CAT_NAMES index)\n")
lines.append("pub static CAT_RUNS: &[(u32, u32, u8)] = &[\n")
for s, e, idx in CAT_RUNS:
    lines.append(f"    (0x{s:X}, 0x{e:X}, {idx}),\n")
lines.append("];\n")

lines.append(ACCESSORS)

out = "".join(lines)
with open("src/unicode_tables.rs", "w", encoding="utf-8") as f:
    f.write(out)
kb = len(out.encode("utf-8")) // 1024
print(f"wrote src/unicode_tables.rs ({kb} KiB, unicodedata {UNIDATA_VERSION})")
print(f"VERIFICATION PASSED: tables and runtime model agree with unicodedata "
      f"{UNIDATA_VERSION}")
