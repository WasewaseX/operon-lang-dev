#!/usr/bin/env python3
"""Generate tests/redteam/rt_p14e_rho_megacistron.op (loop-10 F-9).

A 1500-cistron unit under rho.termination: the O(N) nakedness scan +
O(N) queue update per call must stay inside the fuel/note contract
(rt_p14b precedent). Silences are chosen to exercise every scan path:
  c3    strength 0.0  -> p_g = 0 fast skip (degenerate silence is neutral)
  c700  strength 0.5  -> stochastic nakedness; if naked, the catch-up
                         probability at d=798 is 1-(1-catch)^798 = 1.0
                         under catch 0.5, deterministic catch (one draw:
                         the q==1 fast path skips the catch-up draw)
  c1497 bare          -> deterministic nakedness at distance 2 (q = 0.75)
  c1498 strength 0.5  -> stochastic nakedness at distance 1 (q = 0.5)
queue_cap = 0.0 keeps every cistron unshielded so terminations can happen.
"""
N = 1500
out = []
out.append("# reg-bio-4 loop-10 (F-9): megacistron + Rho termination - adversarial")
out.append("# load shape. A 1500-cistron unit with scattered silences, called at the")
out.append("# tail under rho.termination (catch 0.5, queue_cap 0.0 = unshielded).")
out.append("# Containment: bounded runtime (O(N) scan + O(N) queue per call), no")
out.append("# hang, no OOM, notes stay under the cap. Cell: rt_p14e.cell.")
out.append("regulate {")
out.append("    c1498 translates ptail rate 0.1")
out.append("}")
out.append("operon mega {")
for i in range(N):
    out.append(f"    c{i} rbs 0.5;")
out.append("}")
for i in range(N):
    out.append(f'gene c{i}() {{ return "c{i}" }}')
out.append("silence c3 strength 0.0;")
out.append("silence c700 strength 0.5;")
out.append("silence c1497;")
out.append("silence c1498 strength 0.5;")
out.append("main {")
out.append("    let lost = 0")
out.append("    for i in range(300) {")
out.append("        if c1499() == null { lost = lost + 1 }")
out.append("    }")
out.append("    let mid = 0")
out.append("    for i in range(50) {")
out.append("        if c750() == null { mid = mid + 1 }")
out.append("    }")
out.append('    promote("tail lost: {lost} mid lost: {mid}")')
out.append("}")
with open("tests/redteam/rt_p14e_rho_megacistron.op", "w") as f:
    f.write("\n".join(out) + "\n")
print(f"wrote tests/redteam/rt_p14e_rho_megacistron.op ({len(out)} lines)")
