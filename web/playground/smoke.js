// smoke.js — playground interpreter verification (run: node smoke.js)
const { runProgram } = require("./app.js");
const fs = require("fs");

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

// B4: gene main() is the entry point, same as the native core
check("main() is called as the entry gene",
    'gene main() {\n    promote("from main")\n}', "from main", 90);

// B4: quoted literals inside interpolations (parity with the native cores)
check('quoted literal inside interpolation', 'promote("{len("ab")}")', "2", 90);
check('bracket index with quotes inside interpolation',
    'let m = {"k": 7}\npromote("{m["k"]}")', "7", 90);

// B4: builtins are not first-class values (native-core parity)
{
    const r = runProgram('let f = len\npromote("f={f}")');
    const ok = r.output.join("\n").includes("f=null");
    if (ok) console.log("ok:   builtin read as value is null");
    else { failures++; console.log("FAIL: builtin-as-value", JSON.stringify(r.output), JSON.stringify(r.notes)); }
}

// B4: everyday builtins the cookbook relies on
check("has() membership", 'let m = {"a": 1}\npromote("{has(m, "a")} {has(m, "b")}")', "true false", 90);
check("remove(list, i) returns the element", 'let xs = ["a", "b"]\nlet v = remove(xs, 0)\npromote("{v} {xs}")', "a [\"b\"]", 90);
check("del(map, key)", 'let m = {"a": 1, "b": 2}\ndel(m, "a")\npromote("{keys(m)}")', "[\"b\"]", 90);
check("chr builds code points", 'promote(chr(123) + "x" + chr(125))', "{x}", 90);
check("json round trip", 'let v = json_parse(chr(123) + "\\"a\\": 2" + chr(125))\npromote(json_str(v))', '{"a":2}', 90);

// B4: display parity — strings nested in lists/maps render quoted
check("nested strings are quoted in display", 'promote(["a", "b"])', '["a", "b"]', 90);
check("top-level strings stay raw", 'promote("plain")', "plain", 90);

// stress/rescue is outside the labeled playground subset — it must be skipped
// with a note (not crash), matching the page's honesty label.
const r = runProgram('stress { promote("nope") }\npromote("still running")');
if (r.output.join("\n").includes("still running") && r.notes.some(n => n.message.includes("playground subset"))) {
    console.log("ok:   subset skip is graceful and labeled");
} else {
    failures++;
    console.log("FAIL: subset skip behavior");
}

// B4: the generated manifest must exist, carry the core version, and only
// mark entries verified when they REALLY match the frozen expected output.
try {
    const manifestSrc = fs.readFileSync(__dirname + "/examples.data.js", "utf8");
    global.window = {};
    eval(manifestSrc);
    const examples = global.window.PLAYGROUND_EXAMPLES || [];
    const version = global.window.PLAYGROUND_VERSION || "";
    if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
        failures++; console.log("FAIL: manifest version missing/malformed:", version);
    } else console.log(`ok:   manifest version ${version} matches the core scheme`);
    if (examples.length < 10) {
        failures++; console.log("FAIL: manifest too small:", examples.length);
    } else console.log(`ok:   manifest carries ${examples.length} examples`);
    // spot-check: every verified:true entry's output matches its expected text
    let bad = 0;
    for (const ex of examples) {
        if (ex.verified !== true || !ex.expected) continue;
        const got = runProgram(ex.code).output.join("\n").trim();
        if (got !== ex.expected.trim()) bad++;
    }
    if (bad) { failures++; console.log(`FAIL: ${bad} verified entries diverge from their frozen output`); }
    else console.log("ok:   every verified entry reproduces its frozen output");
} catch (e) {
    failures++;
    console.log("FAIL: examples.data.js —", e.message);
}

if (failures) {
    console.log(`SMOKE: ${failures} failure(s)`);
    process.exit(1);
}
console.log("SMOKE: all playground checks passed");
