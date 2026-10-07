// super-bench: 57-aspect dispatcher (Node.js lane)
// Mirrors super.op / super_py.py / super_rs.rs with identical algorithms.
// Prints: OK <checksum> <elapsed_ms>
const { performance } = require("perf_hooks");

function lcg_next(x) {
  return (x * 48271) % 2147483647;
}

function deep_eq(a, b) {
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (!deep_eq(a[i], b[i])) return false;
    }
    return true;
  }
  return a === b;
}

// ---------------- NUM ----------------

function a_num_int_add() {
  let acc = 0;
  for (let i = 0; i < 2000000; i++) acc = acc + i;
  return acc;
}

function a_num_int_mixed() {
  let acc = 0;
  for (let i = 0; i < 600000; i++) {
    const t = (acc * 3 + i) % 1000003;
    acc = (t + Math.floor(t / 7) + (i % 13)) % 1000003;
  }
  return acc;
}

function a_num_float_add() {
  let f = 0.0;
  for (let i = 0; i < 2000000; i++) f = f + i * 0.5 - f * 0.0000001;
  return Math.floor(f);
}

function a_num_float_math() {
  let x = 1.0;
  let c = 0;
  for (let i = 0; i < 300000; i++) {
    x = Math.sqrt(x * 1.7 + 0.3);
    if (x > 100.0) x = 1.0;
    if (x > 2.0) c = c + 1;
  }
  return c * 100000 + Math.floor(x * 1000.0);
}

function a_num_trialdiv() {
  let c = 0;
  for (let i = 2; i < 40000; i++) {
    let d = 2;
    let p = true;
    while (d * d <= i) {
      if (i % d === 0) {
        p = false;
        break;
      }
      d = d + 1;
    }
    if (p) c = c + 1;
  }
  return c;
}

function a_num_roundtrip() {
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    const s = String(i % 100000);
    acc = acc + parseInt(s, 10);
  }
  return acc;
}

function a_num_parse_float() {
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    const v = parseFloat("1234.5678");
    acc = acc + Math.floor(v);
  }
  return acc;
}

function a_num_divmod() {
  let c3 = 0;
  let c5 = 0;
  let c15 = 0;
  for (let i = 0; i < 1000000; i++) {
    if (i % 3 === 0) c3 = c3 + 1;
    if (i % 5 === 0) c5 = c5 + 1;
    if (i % 15 === 0) c15 = c15 + 1;
  }
  return c3 * 100000000 + c5 * 10000 + c15;
}

// ---------------- CTRL ----------------

function a_ctl_while() {
  let i = 1200000;
  let acc = 0;
  while (i > 0) {
    acc = acc + (i % 7);
    i = i - 1;
  }
  return acc;
}

function a_ctl_for() {
  let acc = 0;
  for (let i = 0; i < 1200000; i++) acc = acc + 1;
  return acc;
}

function a_ctl_nested() {
  let s = 0;
  for (let i = 0; i < 600; i++) {
    for (let j = 0; j < 600; j++) s = s + (i * 600 + j) % 1000;
  }
  return s;
}

function add1(a) {
  return a + 1;
}

function a_ctl_call() {
  let acc = 0;
  for (let i = 0; i < 600000; i++) acc = add1(acc);
  return acc;
}

function fib(n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}

function a_ctl_fib() {
  let t = 0;
  for (let i = 0; i < 5; i++) t = t + fib(22);
  return t;
}

function down(n) {
  if (n === 0) return 0;
  return 1 + down(n - 1);
}

function a_ctl_deep_rec() {
  let t = 0;
  for (let i = 0; i < 6; i++) t = t + down(5000);
  return t;
}

function is_even(n) {
  if (n === 0) return true;
  return is_odd(n - 1);
}

function is_odd(n) {
  if (n === 0) return false;
  return is_even(n - 1);
}

function a_ctl_mutual() {
  let c = 0;
  for (let i = 0; i < 2000; i++) {
    if (is_even(50)) c = c + 1;
  }
  return c;
}

function a_ctl_branch() {
  const c = new Array(16).fill(0);
  for (let i = 0; i < 1000000; i++) {
    const v = i % 16;
    if (v === 0) c[0] = c[0] + 1;
    else if (v === 1) c[1] = c[1] + 1;
    else if (v === 2) c[2] = c[2] + 1;
    else if (v === 3) c[3] = c[3] + 1;
    else if (v === 4) c[4] = c[4] + 1;
    else if (v === 5) c[5] = c[5] + 1;
    else if (v === 6) c[6] = c[6] + 1;
    else if (v === 7) c[7] = c[7] + 1;
    else if (v === 8) c[8] = c[8] + 1;
    else if (v === 9) c[9] = c[9] + 1;
    else if (v === 10) c[10] = c[10] + 1;
    else if (v === 11) c[11] = c[11] + 1;
    else if (v === 12) c[12] = c[12] + 1;
    else if (v === 13) c[13] = c[13] + 1;
    else if (v === 14) c[14] = c[14] + 1;
    else if (v === 15) c[15] = c[15] + 1;
    else c[0] = c[0] + 1;
  }
  let chk = 0;
  for (let k = 0; k < 16; k++) chk = chk + c[k] * (k + 1);
  return chk;
}

function a_ctl_match() {
  let acc = 0;
  for (let i = 0; i < 500000; i++) {
    switch (i % 8) {
      case 0: acc = acc + 21; break;
      case 1: acc = acc + 28; break;
      case 2: acc = acc + 35; break;
      case 3: acc = acc + 42; break;
      case 4: acc = acc + 49; break;
      case 5: acc = acc + 56; break;
      case 6: acc = acc + 63; break;
      default: acc = acc + 70; break;
    }
  }
  return acc;
}

function make_adder(k) {
  return (x) => x + k;
}

function a_ctl_closure() {
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    const f = make_adder(i % 10);
    acc = acc + f(i % 1000);
  }
  return acc;
}

// ---------------- STR ----------------

function a_str_cat() {
  let s = "";
  for (let i = 0; i < 30000; i++) s = s + "ab";
  return s.length;
}

function a_str_join() {
  const parts = [];
  for (let i = 0; i < 200000; i++) parts.push(String(i % 1000));
  const s = parts.join(",");
  return s.length;
}

function a_str_slice() {
  const base = "The quick brown fox jumps over the lazy dog";
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    const p = base.slice(i % 30, (i % 30) + 10);
    acc = acc + p.length;
  }
  return acc;
}

function a_str_replace() {
  const base = "The quick brown fox jumps over the lazy dog";
  let acc = 0;
  for (let i = 0; i < 100000; i++) {
    const t = base.replaceAll("o", "0").replaceAll("e", "3");
    acc = acc + t.length;
  }
  return acc;
}

function a_str_split() {
  const w10 = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
  let line = "";
  for (let i = 0; i < 10; i++) line = line + w10;
  let acc = 0;
  for (let i = 0; i < 20000; i++) {
    const ws = line.split(" ");
    acc = acc + ws.length;
  }
  return acc;
}

function a_str_case() {
  const base = "The quick brown fox jumps over the lazy dog";
  let acc = 0;
  for (let i = 0; i < 60000; i++) {
    const u = base.toUpperCase();
    const l = u.toLowerCase();
    const t = ("  " + l + "  ").trim();
    acc = acc + t.length;
  }
  return acc;
}

function a_str_compare() {
  const base = "The quick brown fox jumps over the lazy dog";
  const s2 = "The quick brown fox jumps over the lazy dof";
  let c = 0;
  for (let i = 0; i < 300000; i++) {
    if (base === s2) c = c + 1;
    if (base.startsWith("The quick")) c = c + 1;
    if (base.endsWith("dog")) c = c + 1;
  }
  return c;
}

function a_str_interp() {
  let acc = 0;
  for (let i = 0; i < 100000; i++) {
    const t = `v=${i},x=${i % 97},y=${(i * 7) % 1000}`;
    acc = acc + t.length;
  }
  return acc;
}

function a_str_contains() {
  const base = "The quick brown fox jumps over the lazy dog";
  let c = 0;
  let d = 0;
  for (let i = 0; i < 200000; i++) {
    if (base.includes("quick")) c = c + 1;
    if (base.includes("zebra")) d = d + 1;
  }
  return c * 3 + d;
}

function a_str_build() {
  let acc = 0;
  for (let i = 0; i < 60000; i++) {
    const p = "é" + String(i % 100) + "ü中";
    const q = p.toUpperCase();
    acc = acc + q.length;
  }
  return acc;
}

// ---------------- LIST ----------------

function a_lst_push() {
  const xs = [];
  for (let i = 0; i < 300000; i++) xs.push(i);
  let acc = 0;
  for (const v of xs) acc = acc + v;
  return acc;
}

function a_lst_idx() {
  const xs = [];
  for (let i = 0; i < 100000; i++) xs.push(i % 100000);
  let acc = 0;
  for (let i = 0; i < 600000; i++) acc = acc + xs[(i * 7) % 100000];
  return acc;
}

function a_lst_iter() {
  const xs = [];
  for (let i = 0; i < 100000; i++) xs.push(i % 1000);
  let acc = 0;
  for (let p = 0; p < 6; p++) {
    for (const v of xs) acc = acc + v;
  }
  return acc;
}

function a_lst_slice() {
  const xs = [];
  for (let i = 0; i < 2000; i++) xs.push(i);
  let acc = 0;
  for (let i = 0; i < 60000; i++) {
    const p = xs.slice(i % 1500, (i % 1500) + 100);
    acc = acc + p.length;
  }
  return acc;
}

function a_lst_sort() {
  let x = 123456789;
  const xs = [];
  for (let i = 0; i < 60000; i++) {
    x = lcg_next(x);
    xs.push(x % 1000000);
  }
  xs.sort((a, b) => a - b);
  return xs[0] + xs[29999] + xs[59999];
}

function qs(a, lo, hi) {
  if (lo >= hi) return 0;
  const p = a[hi];
  let i = lo - 1;
  for (let j = lo; j < hi; j++) {
    if (a[j] <= p) {
      i = i + 1;
      const t = a[i];
      a[i] = a[j];
      a[j] = t;
    }
  }
  const t2 = a[i + 1];
  a[i + 1] = a[hi];
  a[hi] = t2;
  qs(a, lo, i);
  qs(a, i + 2, hi);
  return 0;
}

function a_lst_sort_lang() {
  let x = 987654321;
  const a = [];
  for (let i = 0; i < 3000; i++) {
    x = lcg_next(x);
    a.push(x % 100000);
  }
  qs(a, 0, 2999);
  return a[0] + a[1500] + a[2999];
}

function a_lst_comp() {
  const xs = [];
  for (let i = 0; i < 200000; i++) xs.push(i % 1000);
  const a1 = [];
  for (const x of xs) {
    if (x % 3 === 0) a1.push(x * 2);
  }
  let s1 = 0;
  for (const v of a1) s1 = s1 + v;
  const a2 = [];
  for (const x of a1) {
    if (x % 6 === 0) a2.push(x);
  }
  let s2 = 0;
  for (const v of a2) s2 = s2 + v;
  return s1 + s2;
}

function a_lst_search() {
  let x = 555555555;
  const xs = [];
  for (let i = 0; i < 1000; i++) {
    x = lcg_next(x);
    xs.push(x % 100000);
  }
  let found = 0;
  let scans = 0;
  for (let i = 0; i < 8000; i++) {
    const t = (i * 37) % 150000;
    let k = 0;
    while (k < 1000) {
      scans = scans + 1;
      if (xs[k] === t) {
        found = found + 1;
        break;
      }
      k = k + 1;
    }
  }
  return found * 100000000 + scans;
}

function a_lst_reverse() {
  const xs = [];
  for (let i = 0; i < 1000; i++) xs.push(i);
  let acc = 0;
  for (let i = 0; i < 30000; i++) {
    const r = xs.slice().reverse();
    acc = acc + r[0];
  }
  return acc;
}

function a_lst_insert_del() {
  const xs = [];
  for (let i = 0; i < 3000; i++) xs.push(i);
  for (let i = 0; i < 10000; i++) {
    xs.splice(i % 3000, 0, i);
    xs.splice((i * 7) % 3000, 1);
  }
  return xs[0] + xs[1500] + xs[2999];
}

// ---------------- MAP ----------------

function a_map_set() {
  const m = new Map();
  for (let i = 0; i < 300000; i++) m.set(String(i % 100000), i);
  return m.get("0") + m.get("50000");
}

function a_map_get() {
  const m = new Map();
  for (let i = 0; i < 100000; i++) m.set(String(i), i);
  let acc = 0;
  for (let i = 0; i < 600000; i++) acc = acc + m.get(String((i * 7) % 100000));
  return acc;
}

function a_map_miss() {
  const m = new Map();
  for (let i = 0; i < 50000; i++) m.set(String(i), i);
  let c = 0;
  for (let i = 0; i < 300000; i++) {
    if (m.has("nope" + String(i % 5000))) c = c + 1;
  }
  return 300000 - c;
}

function a_map_iter() {
  const m = new Map();
  for (let i = 0; i < 20000; i++) m.set(String(i), i);
  let acc = 0;
  for (let p = 0; p < 10; p++) {
    for (const k of m.keys()) acc = acc + m.get(k);
  }
  return acc;
}

function a_map_incr() {
  const m = new Map();
  for (let j = 0; j < 5000; j++) m.set("w" + String(j), 0);
  for (let i = 0; i < 200000; i++) {
    const k = "w" + String(i % 5000);
    m.set(k, m.get(k) + 1);
  }
  let s = 0;
  for (let j = 0; j < 50; j++) s = s + m.get("w" + String(j));
  return s * 100000 + m.size;
}

function a_map_nested() {
  const outer = new Map();
  for (let i = 0; i < 100; i++) {
    const inner = new Map();
    for (let j = 0; j < 50; j++) inner.set(String(j), i * j);
    outer.set(String(i), inner);
  }
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    acc = acc + outer.get(String(i % 100)).get(String(i % 50));
  }
  return acc;
}

function a_map_del() {
  const m = new Map();
  for (let i = 0; i < 20000; i++) m.set(String(i), i);
  for (let i = 0; i < 50000; i++) {
    const k = String((i * 7) % 20000);
    m.delete(k);
    m.set(k, i);
  }
  return m.size * 100000 + m.get("0");
}

function a_map_mixed() {
  const m = new Map();
  for (let i = 0; i < 150000; i++) {
    m.set(i % 50000, "v" + String(i));
    m.set("s" + String(i % 25000), i);
  }
  return m.size;
}

// ---------------- SET ----------------

function scan_in(xs, t) {
  for (const v of xs) {
    if (v === t) return true;
  }
  return false;
}

function a_set_algebra() {
  let x = 246813579;
  const xs = [];
  for (let i = 0; i < 16000; i++) {
    x = lcg_next(x);
    xs.push(x % 8000);
  }
  const s = new Set(xs);
  let c = 0;
  for (let i = 0; i < 2000; i++) {
    if (s.has(i % 9000)) c = c + 1;
  }
  return s.size * 100000 + c;
}

// ---------------- ALGO ----------------

function a_alg_sieve() {
  const n = 50000;
  const flags = new Array(n).fill(true);
  let c = 0;
  for (let i = 2; i < n; i++) {
    if (flags[i]) {
      c = c + 1;
      let j = i * i;
      while (j < n) {
        flags[j] = false;
        j = j + i;
      }
    }
  }
  return c;
}

function a_alg_mandel() {
  let total = 0;
  for (let row = 0; row < 160; row++) {
    for (let col = 0; col < 240; col++) {
      const x0 = col * 0.00875 - 2.1;
      const y0 = row * 0.00625 - 0.5;
      let zx = 0.0;
      let zy = 0.0;
      let it = 0;
      while (it < 50) {
        const nx = zx * zx - zy * zy + x0;
        zy = 2.0 * zx * zy + y0;
        zx = nx;
        if (zx * zx + zy * zy > 4.0) break;
        it = it + 1;
      }
      total = total + it;
    }
  }
  return total;
}

function tnode_build(d) {
  if (d === 0) return null;
  return [1, tnode_build(d - 1), tnode_build(d - 1)];
}

function tnode_count(t) {
  if (t === null) return 0;
  return 1 + tnode_count(t[1]) + tnode_count(t[2]);
}

function a_alg_trees() {
  let t = 0;
  for (let i = 0; i < 3; i++) {
    const root = tnode_build(14);
    t = t + tnode_count(root);
  }
  return t;
}

function a_alg_matrix() {
  const n = 96;
  const a = [];
  const b = [];
  for (let i = 0; i < n; i++) {
    const ra = [];
    const rb = [];
    for (let j = 0; j < n; j++) {
      ra.push((i * 7 + j) % 97);
      rb.push((i * 3 + j * 5) % 97);
    }
    a.push(ra);
    b.push(rb);
  }
  let acc = 0;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      let s = 0;
      for (let k = 0; k < n; k++) s = s + a[i][k] * b[k][j];
      acc = acc + s % 1000003;
    }
  }
  return acc;
}

function a_alg_wordfreq() {
  const m = new Map();
  for (let j = 0; j < 800; j++) m.set("w" + String(j), 0);
  for (let i = 0; i < 2000; i++) {
    const parts = [];
    for (let j = 0; j < 12; j++) parts.push("w" + String((i * 13 + j * 7) % 800));
    const line = parts.join(" ");
    const ws = line.split(" ");
    for (const w of ws) m.set(w, m.get(w) + 1);
  }
  return m.get("w0") * 100000 + m.size;
}

function a_alg_json_rt() {
  const doc = {};
  for (let i = 0; i < 120; i++) {
    doc["row" + String(i)] = { id: i, name: "item-" + String(i), tags: ["a", "b", "c"], score: i * 3, active: i % 2 === 0 };
  }
  let acc = 0;
  for (let i = 0; i < 150; i++) {
    const s = JSON.stringify(doc);
    const back = JSON.parse(s);
    acc = acc + back["row" + String(i % 120)].id + back["row" + String((i + 7) % 120)].tags.length;
  }
  return acc;
}

function a_alg_json_big() {
  const doc = {};
  for (let i = 0; i < 800; i++) {
    const tags = [];
    for (let j = 0; j < 5; j++) tags.push("t" + String((i + j) % 32));
    doc["r" + String(i)] = { id: i, kind: "k" + String(i % 9), tags: tags, w: (i * 7) % 1000, ok: i % 3 !== 0, note: "n" + String(i % 64) };
  }
  const s = JSON.stringify(doc);
  let acc = 0;
  for (let p = 0; p < 2; p++) {
    const back = JSON.parse(s);
    for (let i = 0; i < 800; i++) {
      const r = back["r" + String(i)];
      acc = acc + r.id + r.tags.length + r.w;
    }
  }
  return acc;
}

function a_alg_deep_eq() {
  const a = [];
  for (let i = 0; i < 50; i++) {
    const sub = [i, i + 1];
    a.push([i, "s" + String(i % 7), sub]);
  }
  const b = [];
  for (let i = 0; i < 50; i++) {
    const sub = [i, i + 1];
    b.push([i, "s" + String(i % 7), sub]);
  }
  const b2 = [];
  for (let i = 0; i < 50; i++) {
    const sub = [i, i + 1];
    if (i === 25) {
      b2.push([i, "DIFF", sub]);
    } else {
      b2.push([i, "s" + String(i % 7), sub]);
    }
  }
  let c = 0;
  for (let i = 0; i < 10000; i++) {
    if (deep_eq(a, b)) c = c + 1;
    if (deep_eq(a, b2)) c = c + 1;
  }
  return c;
}

function ocalc(i) {
  if (i % 3 === 0) return [false, 0];
  return [true, (i * 7) % 1000];
}

function a_alg_opt() {
  let acc = 0;
  for (let i = 0; i < 200000; i++) {
    const [ok, v] = ocalc(i);
    acc = acc + (ok ? v : 1);
  }
  return acc;
}

function a_floor() {
  return 0;
}



const ASPECTS = {
  num_int_add: a_num_int_add,
  num_int_mixed: a_num_int_mixed,
  num_float_add: a_num_float_add,
  num_float_math: a_num_float_math,
  num_trialdiv: a_num_trialdiv,
  num_roundtrip: a_num_roundtrip,
  num_parse_float: a_num_parse_float,
  num_divmod: a_num_divmod,
  ctl_while: a_ctl_while,
  ctl_for: a_ctl_for,
  ctl_nested: a_ctl_nested,
  ctl_call: a_ctl_call,
  ctl_fib: a_ctl_fib,
  ctl_deep_rec: a_ctl_deep_rec,
  ctl_mutual: a_ctl_mutual,
  ctl_branch: a_ctl_branch,
  ctl_match: a_ctl_match,
  ctl_closure: a_ctl_closure,
  str_cat: a_str_cat,
  str_join: a_str_join,
  str_slice: a_str_slice,
  str_replace: a_str_replace,
  str_split: a_str_split,
  str_case: a_str_case,
  str_compare: a_str_compare,
  str_interp: a_str_interp,
  str_contains: a_str_contains,
  str_build: a_str_build,
};

const ASPECTS2 = {
  lst_push: a_lst_push,
  lst_idx: a_lst_idx,
  lst_iter: a_lst_iter,
  lst_slice: a_lst_slice,
  lst_sort: a_lst_sort,
  lst_sort_lang: a_lst_sort_lang,
  lst_comp: a_lst_comp,
  lst_search: a_lst_search,
  lst_reverse: a_lst_reverse,
  lst_insert_del: a_lst_insert_del,
  map_set: a_map_set,
  map_get: a_map_get,
  map_miss: a_map_miss,
  map_iter: a_map_iter,
  map_incr: a_map_incr,
  map_nested: a_map_nested,
  map_del: a_map_del,
  map_mixed: a_map_mixed,
  set_algebra: a_set_algebra,
  alg_sieve: a_alg_sieve,
  alg_mandel: a_alg_mandel,
  alg_trees: a_alg_trees,
  alg_matrix: a_alg_matrix,
  alg_wordfreq: a_alg_wordfreq,
  alg_json_rt: a_alg_json_rt,
  alg_json_big: a_alg_json_big,
  alg_deep_eq: a_alg_deep_eq,
  alg_opt: a_alg_opt,
  floor: a_floor,
};
Object.assign(ASPECTS, ASPECTS2);

const aspect_id = process.argv[2];
const fn = ASPECTS[aspect_id];
const t0 = performance.now();
const chk = fn();
const dt = performance.now() - t0;
console.log(`OK ${chk} ${dt.toFixed(6)}`);
