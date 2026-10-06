// native.js — Node.js mirrors of the operon benchmark fixtures (the
// "random mainstream language" runner). Every workload prints the SAME
// canonical string as the operon fixture so run_xlang.py can cross-verify
// correctness across all four runners. 1:1 with the native-py mirrors in
// scripts/bench_compare.py.

function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

function pasc(n, k) { return (k === 0 || k === n) ? 1 : pasc(n - 1, k - 1) + pasc(n - 1, k); }

function w_fib25() { return `fib(25) = ${fib(25)}`; }

function w_loops() {
  let acc = 0;
  for (let i = 0; i < 200000; i++) acc += i % 7;
  return `acc = ${acc}`;
}

function w_strings() {
  let s = "";
  for (let i = 0; i < 4000; i++) s = s + "x{t}y".replace("{t}", String(i % 10));
  return `len = ${s.length}`;
}

function w_collections() {
  const m = new Map();
  const xs = [];
  for (let i = 0; i < 20000; i++) {
    const k = "k" + String(i % 2000);
    m.set(k, (m.get(k) || 0) + 1);
    xs.push(i % 97);
  }
  let total = 0;
  for (const v of xs) total += v;
  return `total = ${total}, distinct = ${m.size}, k1 = ${m.get("k1")}`;
}

function w_recursion() { return `C(20,10) = ${pasc(20, 10)}`; }

function w_grn() {
  // mirror of grn.op: per-call gate check, driver fired at 1.0 — all pass.
  const level = 1.0;
  const worker_a = (n) => (level < 0.3 - 0.25 ? null : n + 1);
  const worker_b = (n) => (level < 0.5 ? null : n * 2);
  const reporter = (n) => (level < 0.7 ? null : n - 1);
  let acc = 0;
  for (let i = 0; i < 20000; i++) acc += worker_a(i) + worker_b(i) + reporter(i);
  return `acc = ${acc}`;
}

function w_m_empty() { return "ok"; }

function w_m_call() {
  const nop = (n) => n;
  let a = 0;
  for (let i = 0; i < 100000; i++) a += nop(i);
  return `a = ${a}`;
}

function w_m_forrange() {
  let a = 0;
  for (let i = 0; i < 200000; i++) a += 1;
  return `a = ${a}`;
}

function w_m_while() {
  let i = 200000, a = 0;
  while (i > 0) { a += 1; i -= 1; }
  return `a = ${a}`;
}

function w_m_varread() {
  const x = 7;
  let a = 0;
  for (let i = 0; i < 200000; i++) a += x;
  return `a = ${a}`;
}

function w_m_intadd() {
  let a = 0;
  const b = 1;
  for (let i = 0; i < 300000; i++) a += b;
  return `a = ${a}`;
}

function w_m_listpush() {
  const xs = [];
  for (let i = 0; i < 50000; i++) xs.push(i);
  return `len = ${xs.length}`;
}

function w_m_listidx() {
  const xs = [];
  for (let i = 0; i < 2000; i++) xs.push(i);
  let a = 0;
  for (let i = 0; i < 100000; i++) a += xs[i % 2000];
  return `a = ${a}`;
}

function w_m_mapset() {
  const m = new Map();
  for (let i = 0; i < 40000; i++) m.set(String(i % 4000), i);
  return `len = ${m.size}`;
}

function w_m_mapget() {
  const m = new Map();
  for (let i = 0; i < 4000; i++) m.set(String(i), i);
  let a = 0;
  for (let i = 0; i < 50000; i++) a += m.get(String(i % 4000));
  return `a = ${a}`;
}

function w_m_strcat() {
  let s = "";
  for (let i = 0; i < 12000; i++) s = s + "ab";
  return `len = ${s.length}`;
}

const name = process.argv[2] || "";
const table = {
  fib25: w_fib25, loops: w_loops, strings: w_strings, collections: w_collections,
  recursion: w_recursion, grn: w_grn, m_empty: w_m_empty, m_call: w_m_call,
  m_forrange: w_m_forrange, m_while: w_m_while, m_varread: w_m_varread,
  m_intadd: w_m_intadd, m_listpush: w_m_listpush, m_listidx: w_m_listidx,
  m_mapset: w_m_mapset, m_mapget: w_m_mapget, m_strcat: w_m_strcat,
};
const fn = table[name];
console.log(fn ? fn() : `unknown workload: ${name}`);
