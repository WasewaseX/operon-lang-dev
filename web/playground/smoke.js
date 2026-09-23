// smoke.js — playground interpreter verification (run: node smoke.js)
const { runProgram } = require("./app.js");

let failures = 0;
function check(name, src, expectOut, minScore) {
    const r = runProgram(src);
    const out = r.output.join("\n");
    const ok = out.includes(expectOut) && r.score >= (minScore ?? 0);
    if (ok) {
        console.log(`ok:   ${name}`);
    } else {
        failures++;
        console.log(`FAIL: ${name}`);
        console.log(`  output: ${JSON.stringify(out)}`);
        console.log(`  notes : ${JSON.stringify((r.notes || []).slice(0, 3))}`);
        console.log(`  score : ${r.score}`);
    }
}

check("arithmetic + interpolation", 'let x = 6\npromote("answer {x * 7}")', "answer 42", 90);
check("wobble repair runs normally", 'les x = 2\npromote("v={x}")', "v=2", 0);
check("lambda call", 'promote(gene (a) => a * 2 (21))', "42", 90);
check("list methods", 'promote([3, 1, 2].sort())', "[1, 2, 3]", 90);
check("unbound is null, not crash", 'promote(ghost)', "null", 0);
// stress/rescue is outside the labeled playground subset — it must be skipped
// with a note (not crash), matching the page's honesty label.
const r = runProgram('stress { promote("nope") }\npromote("still running")');
if (r.output.join("\n").includes("still running") && r.notes.some(n => n.message.includes("playground subset"))) {
    console.log("ok:   subset skip is graceful and labeled");
} else {
    failures++;
    console.log("FAIL: subset skip behavior");
}

if (failures) {
    console.log(`SMOKE: ${failures} failure(s)`);
    process.exit(1);
}
console.log("SMOKE: all playground checks passed");
