#!/usr/bin/env python3
"""R0.6 — independent analytic validation harness (roadmap §27, Agent C).

The validation mechanism needed BEFORE stochastic biology grows: any future
stochastic claim (R1-R7 rows, all currently DEFERRED) is checked against a
closed-form analytic reference with pre-registered statistics — never against
a hand-picked threshold or a fuzzy "looks fine".

CAPTURE CONTRACT (fixture = the contract, the harness carries no tunables):
  * A fixture program is a TEMPLATE containing the literal token SEED inside
    its randomize(SEED) call. The harness substitutes the integer seed and
    writes the concrete program under target/r06/ before every run — the
    committed source never carries a concrete stream state.
  * The program prints WHITESPACE-SEPARATED NUMBERS on stdout, exactly:
      - reduce "ratio": token[0]/token[1] is the observed statistic and
        token[1] is the effective sample size n (e.g. "hits n",
        "on_count n", "total_draws m");
      - reduce "pmf": the tokens are a full count vector over k=0..K-1
        (e.g. the Binomial cell counts), compared by total-variation.
THREE GATES, all parameters pre-registered in the fixture file:
  1. DETERMINISM PIN   same seed twice -> byte-identical stdout + rc
                       (the W089/R0.5 mirrored-xorshift discipline).
  2. STATISTICAL GATE  observed statistic vs the analytic mean, z-scored by
                       the ANALYTIC variance (the reference is the truth
                       source — never the sample variance), two-sided, alpha
                       declared in the fixture.
  3. SUPPORT GATE      for pmf references: total-variation distance between
                       empirical and analytic pmf <= declared eps.
Minimized failure output (the #56 iter-3 lesson): fixture id, seed, gate,
the decision arithmetic shown, and the exact replay command.

Exit codes (read from a FILE when piping — WORKER-BEHAVIOR rule 1):
  0 all requested fixtures green
  1 at least one gate failed (failure blocks on stdout)
  2 refusal: schema violation, unknown reference, missing engine — a refusal
    is NOT a fail; nothing was tested.

Usage:
  python3 scripts/validation/harness.py                    # all fixtures
  python3 scripts/validation/harness.py --fixture V-R06-02-bernoulli
  python3 scripts/validation/harness.py --fixture V-R06-02-bernoulli --seed 23 --replay
  python3 scripts/validation/harness.py --pin-manifest
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import subprocess
import sys
from pathlib import Path

SCHEMA = "r06-fixture-v1"
REPO = Path(__file__).resolve().parent.parent.parent
FIXTURE_DIR = REPO / "tests" / "validation" / "fixtures"
PROGRAM_DIR = REPO / "tests" / "validation" / "programs"
TMP_DIR = REPO / "target" / "r06"
MANIFEST = REPO / "tests" / "validation" / "MANIFEST.sha256"


def two_sided_crit(alpha: float) -> float:
    """z with P(|Z| > z) == alpha; bisection on erf, exact to 1e-12, no deps."""
    lo, hi = 0.0, 20.0
    for _ in range(200):
        mid = (lo + hi) / 2.0
        if 2.0 * (1.0 - 0.5 * (1.0 + math.erf(mid / math.sqrt(2.0)))) > alpha:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2.0


# ---------------------------------------------------------------------------
# Analytic reference table: (mean, var_of_observed, pmf_or_None) for the
# statistic the fixture reports, given its effective n. References live HERE,
# never in the fixture programs — the program under test must not carry its
# own answer. Adding a reference = one entry + the fixture citing it.
# ---------------------------------------------------------------------------

def ref_uniform_mean(p: dict, n: int) -> tuple:
    # mean of n iid uniform(0,1) draws: mean 1/2, var 1/(12n)
    return 0.5, 1.0 / (12.0 * n), None


def ref_bernoulli_mean(p: dict, n: int) -> tuple:
    prob = float(p["p"])
    # sample mean of n iid bernoulli(prob)
    return prob, prob * (1.0 - prob) / n, None


def ref_binomial_pmf(p: dict, n: int) -> tuple:
    trials, prob = int(p["trials"]), float(p["p"])
    # exact Binomial(trials, prob) pmf; multinomial cell noise makes the
    # z-gate ill-defined here, so this reference is TV-gate-only by contract
    pmf = {k: math.comb(trials, k) * prob**k * (1.0 - prob) ** (trials - k)
           for k in range(trials + 1)}
    return trials * prob, trials * prob * (1.0 - prob), pmf


def ref_geometric_mean(p: dict, n: int) -> tuple:
    prob = float(p["p"])
    # mean of m iid geometric(prob) waits (support 1,2,...): mean 1/prob,
    # var (1-prob)/prob^2; the fixture reports the mean of m waits
    return 1.0 / prob, (1.0 - prob) / (prob * prob * n), None


def ref_markov2_stationary(p: dict, n: int) -> tuple:
    p01, q10 = float(p["p01"]), float(p["q10"])
    # time-in-state-1 fraction: pi1 = p01/(p01+q10). HONEST autocorrelation:
    # draws are NOT iid. For the 2-state chain the lag-k autocovariance of
    # the indicator is pi1(1-pi1)*lam^k (lam = 1-p01-q10, the second
    # eigenvalue), so the asymptotic variance of the time average is
    # pi1(1-pi1)*(1+lam)/(1-lam)/n — the exact closed form, declared here,
    # conservative by construction, never tuned per run.
    pi1 = p01 / (p01 + q10)
    lam = abs(1.0 - p01 - q10)
    if lam >= 1.0:
        raise ValueError("markov2: p01+q10 outside (0,2) breaks aperiodicity")
    return pi1, pi1 * (1.0 - pi1) * (1.0 + lam) / ((1.0 - lam) * n), None


REFERENCES = {
    "uniform-mean": ref_uniform_mean,
    "bernoulli-mean": ref_bernoulli_mean,
    "binomial-pmf": ref_binomial_pmf,
    "geometric-mean": ref_geometric_mean,
    "markov2-stationary": ref_markov2_stationary,
}


def resolve_engine() -> Path:
    for name in ("bin/operon", "bin/operon.exe"):
        cand = REPO / name
        if cand.exists():
            return cand
    sys.stderr.write(
        "[r06] REFUSAL (rc=2): no engine binary at bin/operon (or .exe).\n"
        "[r06] Nothing was tested. Build first: cargo build --release && "
        "cp target/release/operon bin/operon\n"
        "[r06] Stale-binary trap: re-copy after every rebuild (HANDOFF "
        "session-28).\n")
    sys.exit(2)


def engine_cmd(fx: dict, prog: Path, engine: Path) -> list:
    """Engine invocation: run + named frame (house convention: proof frames
    are walker-counted and vacuous frames fail) + any fixture extras."""
    cmd = [str(engine), "run", str(prog)]
    if fx.get("frame", "proof"):
        cmd += ["--frame", fx.get("frame", "proof")]
    cmd += fx.get("engine_args", [])
    return cmd


def run_program(engine: Path, fx: dict, prog: Path) -> tuple:
    proc = subprocess.run(engine_cmd(fx, prog, engine), capture_output=True)
    return proc.returncode, proc.stdout, proc.stderr


def concrete_program(fx: dict, seed: int) -> Path:
    """Substitute SEED -> the integer seed; write under target/r06/."""
    src = (REPO / fx["program"]).read_text()
    if "SEED" not in src:
        _refuse(f"{fx['id']}: program template lacks the SEED token")
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    out = TMP_DIR / f"{fx['id']}-{seed}.op"
    out.write_text(src.replace("SEED", str(seed)))
    return out


def load_fixtures(only: str | None) -> list:
    if not FIXTURE_DIR.is_dir():
        sys.stderr.write(f"[r06] REFUSAL (rc=2): missing {FIXTURE_DIR}\n")
        sys.exit(2)
    out = []
    for path in sorted(FIXTURE_DIR.glob("*.json")):
        fx = json.loads(path.read_text())
        for key in ("schema", "id", "program", "seeds", "reference",
                    "reduce", "gates"):
            if key not in fx:
                _refuse(f"{path.name}: missing fixture key '{key}'")
        if fx["schema"] != SCHEMA:
            _refuse(f"{path.name}: schema {fx['schema']!r} != {SCHEMA!r}")
        if not (REPO / fx["program"]).exists():
            _refuse(f"{path.name}: program {fx['program']!r} does not exist")
        if fx["reference"]["kind"] not in REFERENCES:
            _refuse(f"{path.name}: unknown reference "
                    f"{fx['reference']['kind']!r}")
        if fx["reduce"]["op"] not in ("ratio", "pmf"):
            _refuse(f"{path.name}: reduce.op must be 'ratio' or 'pmf'")
        if (fx["reduce"]["op"] == "pmf") != ("pmf" in fx["reference"]["kind"]):
            _refuse(f"{path.name}: reduce.op and reference kind disagree "
                    f"(pmf reduce requires a -pmf reference and vice versa)")
        if only and fx["id"] != only:
            continue
        out.append(fx)
    if only and not out:
        _refuse(f"no fixture with id {only!r}")
    return out


def _refuse(msg: str) -> None:
    sys.stderr.write(f"[r06] REFUSAL (rc=2): {msg}\n")
    sys.exit(2)


# ---------------------------------------------------------------------------
# Gates: each returns None (pass) or a minimized failure dict.
# ---------------------------------------------------------------------------

def gate_determinism(engine, fx, seed) -> dict | None:
    prog = concrete_program(fx, seed)
    rc1, out1, _ = run_program(engine, fx, prog)
    rc2, out2, _ = run_program(engine, fx, prog)
    if (rc1, out1) == (rc2, out2):
        return None
    i = 0
    while i < min(len(out1), len(out2)) and out1[i] == out2[i]:
        i += 1
    line = out1[:i].count(b"\n") + 1
    return {"gate": "determinism-pin",
            "detail": f"same seed {seed}: run1 rc={rc1} {len(out1)}B vs "
                      f"run2 rc={rc2} {len(out2)}B; first diff at byte {i} "
                      f"(line {line}) — the mirrored stream moved"}


def _reduce(fx, numbers: list) -> tuple:
    if fx["reduce"]["op"] == "ratio":
        if len(numbers) != 2 or numbers[1] <= 0:
            raise ValueError(
                f"reduce=ratio needs [numerator, n>0], got {numbers[:4]}")
        return numbers[0] / numbers[1], numbers[1]
    # pmf: the tokens ARE the count vector; effective n is their sum
    return numbers, sum(numbers)


def gate_statistical(fx, seed, numbers: list) -> dict | None:
    ref = fx["reference"]
    obs, n = _reduce(fx, numbers)
    mean, var, _ = REFERENCES[ref["kind"]](ref.get("params", {}), int(n))
    z = (obs - mean) / math.sqrt(var)
    alpha = float(fx["gates"]["z_alpha"])
    crit = two_sided_crit(alpha)
    if abs(z) <= crit:
        return None
    return {"gate": f"statistical-z(alpha={alpha}, crit={crit:.4f})",
            "detail": f"observed={obs:.6f} analytic mean={mean:.6f} "
                      f"analytic sd={math.sqrt(var):.6g} z={z:+.3f} "
                      f"(|z| > crit) — pre-registered in the fixture"}


def gate_support(fx, seed, numbers: list) -> dict | None:
    ref = fx["reference"]
    _, _, pmf = REFERENCES[ref["kind"]](ref.get("params", {}), 1)
    eps = float(fx["gates"]["tv_eps"])
    vec, total = _reduce(fx, numbers)
    if total <= 0:
        return {"gate": "support-tv", "detail": "empty sample reported"}
    emp = {k: c / total for k, c in enumerate(vec) if c}
    tv = 0.5 * sum(abs(emp.get(k, 0.0) - pk) for k, pk in pmf.items())
    if tv <= eps:
        return None
    worst = max(pmf, key=lambda k: abs(emp.get(k, 0.0) - pmf[k]))
    return {"gate": f"support-tv(eps={eps})",
            "detail": f"total variation={tv:.6f} > eps; worst cell k={worst}: "
                      f"empirical={emp.get(worst, 0.0):.6f} vs "
                      f"analytic={pmf[worst]:.6f}"}


def failure_block(fx, seed, fail: dict) -> str:
    return "\n".join([
        f"FAIL {fx['id']} seed={seed} gate={fail['gate']}",
        f"  program : {fx['program']}",
        f"  evidence: {fail['detail']}",
        f"  replay  : python3 scripts/validation/harness.py "
        f"--fixture {fx['id']} --seed {seed} --replay",
    ])


def parse_numbers(fx, stdout: bytes) -> list:
    text = stdout.decode("utf-8", errors="replace").strip()
    if not text:
        _fail_parse(fx, "program produced no stdout to parse")
    try:
        return [float(tok) for tok in text.split()]
    except ValueError:
        _fail_parse(fx, f"non-numeric token in stdout: {text[:80]!r}")


def _fail_parse(fx, why: str):
    print(f"FAIL {fx['id']} seed=? gate=parse\n  evidence: {why}\n"
          f"  replay  : python3 scripts/validation/harness.py "
          f"--fixture {fx['id']} --replay")
    sys.exit(1)


def main() -> int:
    ap = argparse.ArgumentParser(description="R0.6 analytic validation harness")
    ap.add_argument("--fixture", help="one fixture by id (default: all)")
    ap.add_argument("--seed", type=int, help="with --replay: the failing seed")
    ap.add_argument("--replay", action="store_true",
                    help="rerun one (fixture, seed), dump raw stdout/err/rc")
    ap.add_argument("--pin-manifest", action="store_true",
                    help="regenerate tests/validation/MANIFEST.sha256")
    ap.add_argument("--json", metavar="PATH", help="also write a JSON report")
    args = ap.parse_args()

    if args.pin_manifest:
        return _pin_manifest()

    fixtures = load_fixtures(args.fixture)

    if args.replay:
        if not args.fixture:
            _refuse("--replay needs --fixture")
        engine = resolve_engine()
        fx = fixtures[0]
        seed = args.seed if args.seed is not None else fx["seeds"][0]
        rc, out, err = run_program(engine, fx, concrete_program(fx, seed))
        sys.stdout.write(out.decode("utf-8", errors="replace"))
        sys.stderr.write(err.decode("utf-8", errors="replace"))
        print(f"[r06] replay {fx['id']} seed={seed}: rc={rc}, {len(out)}B "
              f"stdout, {len(err)}B stderr")
        return 0 if rc == 0 else 1

    engine = resolve_engine()
    rows, failures = [], 0
    for fx in fixtures:
        fx_fail = 0
        for seed in fx["seeds"]:
            prog = concrete_program(fx, seed)
            rc, out, _ = run_program(engine, fx, prog)
            if rc != 0:
                failures += 1
                fx_fail += 1
                print(failure_block(fx, seed, {
                    "gate": "engine-rc", "detail": f"engine exited rc={rc}"}))
                continue
            numbers = parse_numbers(fx, out)
            fail = gate_determinism(engine, fx, seed)
            if fail is None:
                fail = (gate_support(fx, seed, numbers)
                        if fx["reduce"]["op"] == "pmf"
                        else gate_statistical(fx, seed, numbers))
            if fail is not None:
                failures += 1
                fx_fail += 1
                print(failure_block(fx, seed, fail))
        verdict = "GREEN" if fx_fail == 0 else f"FAIL({fx_fail})"
        rows.append({"id": fx["id"], "seeds": len(fx["seeds"]),
                     "verdict": verdict})
        print(f"[r06] {fx['id']}: {verdict} ({len(fx['seeds'])} seeds, "
              f"ref={fx['reference']['kind']})")

    print(f"[r06] total: {len(fixtures)} fixtures, {failures} gate failure(s)")
    if args.json:
        Path(args.json).write_text(json.dumps(
            {"fixtures": rows, "failures": failures}, indent=2,
            sort_keys=True) + "\n")
    return 1 if failures else 0


def _pin_manifest() -> int:
    """Byte-pinned manifest: sha256 of every fixture + program, sorted paths,
    deterministic bytes (LF, trailing newline) — the house MANIFEST class."""
    files = sorted(list(FIXTURE_DIR.glob("*.json"))
                   + list(PROGRAM_DIR.glob("*.op")))
    lines = [f"{hashlib.sha256(f.read_bytes()).hexdigest()}  "
             f"{f.relative_to(REPO)}" for f in files]
    MANIFEST.write_text("\n".join(lines) + "\n")
    print(f"[r06] pinned {len(lines)} entries -> "
          f"{MANIFEST.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
