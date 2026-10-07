// super-bench: 57-aspect dispatcher (Rust lane)
// Mirrors super.op / super_py.py / super_js.js with identical algorithms.
// Prints: OK <checksum> <elapsed_ms>
use std::collections::{HashMap, HashSet};
use std::env;
use std::time::Instant;

fn lcg_next(x: i64) -> i64 {
    (x * 48271) % 2147483647
}

// ---------------- NUM ----------------

fn a_num_int_add() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..2000000 {
        acc = acc + i;
    }
    acc
}

fn a_num_int_mixed() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..600000 {
        let t = (acc * 3 + i) % 1000003;
        acc = (t + t / 7 + i % 13) % 1000003;
    }
    acc
}

fn a_num_float_add() -> i64 {
    let mut f: f64 = 0.0;
    for i in 0..2000000 {
        f = f + i as f64 * 0.5 - f * 0.0000001;
    }
    f as i64
}

fn a_num_float_math() -> i64 {
    let mut x: f64 = 1.0;
    let mut c: i64 = 0;
    for _ in 0..300000 {
        x = (x * 1.7 + 0.3).sqrt();
        if x > 100.0 {
            x = 1.0;
        }
        if x > 2.0 {
            c += 1;
        }
    }
    c * 100000 + (x * 1000.0) as i64
}

fn a_num_trialdiv() -> i64 {
    let mut c: i64 = 0;
    for i in 2..40000 {
        let mut d: i64 = 2;
        let mut p = true;
        while d * d <= i {
            if i % d == 0 {
                p = false;
                break;
            }
            d += 1;
        }
        if p {
            c += 1;
        }
    }
    c
}

fn a_num_roundtrip() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..200000 {
        let s = (i % 100000).to_string();
        acc += s.parse::<i64>().unwrap();
    }
    acc
}

fn a_num_parse_float() -> i64 {
    let mut acc: i64 = 0;
    for _ in 0..200000 {
        let v: f64 = "1234.5678".parse().unwrap();
        acc += v as i64;
    }
    acc
}

fn a_num_divmod() -> i64 {
    let mut c3: i64 = 0;
    let mut c5: i64 = 0;
    let mut c15: i64 = 0;
    for i in 0..1000000 {
        if i % 3 == 0 {
            c3 += 1;
        }
        if i % 5 == 0 {
            c5 += 1;
        }
        if i % 15 == 0 {
            c15 += 1;
        }
    }
    c3 * 100000000 + c5 * 10000 + c15
}

// ---------------- CTRL ----------------

fn a_ctl_while() -> i64 {
    let mut i: i64 = 1200000;
    let mut acc: i64 = 0;
    while i > 0 {
        acc += i % 7;
        i -= 1;
    }
    acc
}

fn a_ctl_for() -> i64 {
    let mut acc: i64 = 0;
    for _ in 0..1200000 {
        acc += 1;
    }
    acc
}

fn a_ctl_nested() -> i64 {
    let mut s: i64 = 0;
    for i in 0..600 {
        for j in 0..600 {
            s += (i * 600 + j) % 1000;
        }
    }
    s
}

fn add1(a: i64) -> i64 {
    a + 1
}

fn a_ctl_call() -> i64 {
    let mut acc: i64 = 0;
    for _ in 0..600000 {
        acc = add1(acc);
    }
    acc
}

fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn a_ctl_fib() -> i64 {
    let mut t: i64 = 0;
    for _ in 0..5 {
        t += fib(22);
    }
    t
}

fn down(n: i64) -> i64 {
    if n == 0 {
        return 0;
    }
    1 + down(n - 1)
}

fn a_ctl_deep_rec() -> i64 {
    let mut t: i64 = 0;
    for _ in 0..6 {
        t += down(5000);
    }
    t
}

fn is_even(n: i64) -> bool {
    if n == 0 {
        return true;
    }
    is_odd(n - 1)
}

fn is_odd(n: i64) -> bool {
    if n == 0 {
        return false;
    }
    is_even(n - 1)
}

fn a_ctl_mutual() -> i64 {
    let mut c: i64 = 0;
    for _ in 0..2000 {
        if is_even(50) {
            c += 1;
        }
    }
    c
}

fn a_ctl_branch() -> i64 {
    let mut c = [0i64; 16];
    for i in 0..1000000 {
        let v = i % 16;
        if v == 0 {
            c[0] += 1;
        } else if v == 1 {
            c[1] += 1;
        } else if v == 2 {
            c[2] += 1;
        } else if v == 3 {
            c[3] += 1;
        } else if v == 4 {
            c[4] += 1;
        } else if v == 5 {
            c[5] += 1;
        } else if v == 6 {
            c[6] += 1;
        } else if v == 7 {
            c[7] += 1;
        } else if v == 8 {
            c[8] += 1;
        } else if v == 9 {
            c[9] += 1;
        } else if v == 10 {
            c[10] += 1;
        } else if v == 11 {
            c[11] += 1;
        } else if v == 12 {
            c[12] += 1;
        } else if v == 13 {
            c[13] += 1;
        } else if v == 14 {
            c[14] += 1;
        } else if v == 15 {
            c[15] += 1;
        } else {
            c[0] += 1;
        }
    }
    let mut chk: i64 = 0;
    for k in 0..16 {
        chk += c[k] * (k as i64 + 1);
    }
    chk
}

fn a_ctl_match() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..500000 {
        match i % 8 {
            0 => acc += 21,
            1 => acc += 28,
            2 => acc += 35,
            3 => acc += 42,
            4 => acc += 49,
            5 => acc += 56,
            6 => acc += 63,
            _ => acc += 70,
        }
    }
    acc
}

fn make_adder(k: i64) -> Box<dyn Fn(i64) -> i64> {
    Box::new(move |x| x + k)
}

fn a_ctl_closure() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..200000 {
        let f = make_adder(i % 10);
        acc += f(i % 1000);
    }
    acc
}

// ---------------- STR ----------------

fn a_str_cat() -> i64 {
    let mut s = String::new();
    for _ in 0..30000 {
        s = s + "ab";
    }
    s.len() as i64
}

fn a_str_join() -> i64 {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..200000 {
        parts.push((i % 1000).to_string());
    }
    let s = parts.join(",");
    s.len() as i64
}

fn a_str_slice() -> i64 {
    let base = "The quick brown fox jumps over the lazy dog";
    let mut acc: i64 = 0;
    for i in 0..200000 {
        let a = (i % 30) as usize;
        let p = &base[a..a + 10];
        acc += p.len() as i64;
    }
    acc
}

fn a_str_replace() -> i64 {
    let base = "The quick brown fox jumps over the lazy dog";
    let mut acc: i64 = 0;
    for _ in 0..100000 {
        let t = base.replace("o", "0").replace("e", "3");
        acc += t.len() as i64;
    }
    acc
}

fn a_str_split() -> i64 {
    let w10 = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
    let mut line = String::new();
    for _ in 0..10 {
        line = line + w10;
    }
    let mut acc: i64 = 0;
    for _ in 0..20000 {
        let ws = line.split(' ').count();
        acc += ws as i64;
    }
    acc
}

fn a_str_case() -> i64 {
    let base = "The quick brown fox jumps over the lazy dog";
    let mut acc: i64 = 0;
    for _ in 0..60000 {
        let u = base.to_uppercase();
        let l = u.to_lowercase();
        let t = format!("  {}  ", l);
        acc += t.trim().chars().count() as i64;
    }
    acc
}

fn a_str_compare() -> i64 {
    let base = "The quick brown fox jumps over the lazy dog";
    let s2 = "The quick brown fox jumps over the lazy dof";
    let mut c: i64 = 0;
    for _ in 0..300000 {
        if base == s2 {
            c += 1;
        }
        if base.starts_with("The quick") {
            c += 1;
        }
        if base.ends_with("dog") {
            c += 1;
        }
    }
    c
}

fn a_str_interp() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..100000 {
        let t = format!("v={},x={},y={}", i, i % 97, (i * 7) % 1000);
        acc += t.chars().count() as i64;
    }
    acc
}

fn a_str_contains() -> i64 {
    let base = "The quick brown fox jumps over the lazy dog";
    let mut c: i64 = 0;
    let mut d: i64 = 0;
    for _ in 0..200000 {
        if base.contains("quick") {
            c += 1;
        }
        if base.contains("zebra") {
            d += 1;
        }
    }
    c * 3 + d
}

fn a_str_build() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..60000 {
        let p = format!("é{}ü中", i % 100);
        let q = p.to_uppercase();
        acc += q.chars().count() as i64;
    }
    acc
}

// ---------------- LIST ----------------

fn a_lst_push() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..300000 {
        xs.push(i);
    }
    let mut acc: i64 = 0;
    for v in &xs {
        acc += v;
    }
    acc
}

fn a_lst_idx() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..100000 {
        xs.push(i % 100000);
    }
    let mut acc: i64 = 0;
    for i in 0..600000 {
        acc += xs[(i * 7) as usize % 100000];
    }
    acc
}

fn a_lst_iter() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..100000 {
        xs.push(i % 1000);
    }
    let mut acc: i64 = 0;
    for _ in 0..6 {
        for v in &xs {
            acc += v;
        }
    }
    acc
}

fn a_lst_slice() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..2000 {
        xs.push(i);
    }
    let mut acc: i64 = 0;
    for i in 0..60000 {
        let a = (i % 1500) as usize;
        let p = &xs[a..a + 100];
        acc += p.len() as i64;
    }
    acc
}

fn a_lst_sort() -> i64 {
    let mut x: i64 = 123456789;
    let mut xs: Vec<i64> = Vec::new();
    for _ in 0..60000 {
        x = lcg_next(x);
        xs.push(x % 1000000);
    }
    xs.sort();
    xs[0] + xs[29999] + xs[59999]
}

fn qs(a: &mut Vec<i64>, lo: i64, hi: i64) {
    if lo >= hi {
        return;
    }
    let p = a[hi as usize];
    let mut i = lo - 1;
    for j in lo..hi {
        if a[j as usize] <= p {
            i += 1;
            let t = a[i as usize];
            a[i as usize] = a[j as usize];
            a[j as usize] = t;
        }
    }
    let t2 = a[(i + 1) as usize];
    a[(i + 1) as usize] = a[hi as usize];
    a[hi as usize] = t2;
    qs(a, lo, i);
    qs(a, i + 2, hi);
}

fn a_lst_sort_lang() -> i64 {
    let mut x: i64 = 987654321;
    let mut a: Vec<i64> = Vec::new();
    for _ in 0..3000 {
        x = lcg_next(x);
        a.push(x % 100000);
    }
    qs(&mut a, 0, 2999);
    a[0] + a[1500] + a[2999]
}

fn a_lst_comp() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..200000 {
        xs.push(i % 1000);
    }
    let a1: Vec<i64> = xs.iter().filter(|x| *x % 3 == 0).map(|x| x * 2).collect();
    let mut s1: i64 = 0;
    for v in &a1 {
        s1 += v;
    }
    let a2: Vec<i64> = a1.iter().filter(|x| *x % 6 == 0).cloned().collect();
    let mut s2: i64 = 0;
    for v in &a2 {
        s2 += v;
    }
    s1 + s2
}

fn a_lst_search() -> i64 {
    let mut x: i64 = 555555555;
    let mut xs: Vec<i64> = Vec::new();
    for _ in 0..1000 {
        x = lcg_next(x);
        xs.push(x % 100000);
    }
    let mut found: i64 = 0;
    let mut scans: i64 = 0;
    for i in 0..8000 {
        let t = (i * 37) % 150000;
        let mut k: usize = 0;
        while k < 1000 {
            scans += 1;
            if xs[k] == t {
                found += 1;
                break;
            }
            k += 1;
        }
    }
    found * 100000000 + scans
}

fn a_lst_reverse() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..1000 {
        xs.push(i);
    }
    let mut acc: i64 = 0;
    for _ in 0..30000 {
        let mut r = xs.clone();
        r.reverse();
        acc += r[0];
    }
    acc
}

fn a_lst_insert_del() -> i64 {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..3000 {
        xs.push(i);
    }
    for i in 0..10000 {
        xs.insert((i % 3000) as usize, i);
        xs.remove(((i * 7) % 3000) as usize);
    }
    xs[0] + xs[1500] + xs[2999]
}

// ---------------- MAP ----------------

fn a_map_set() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..300000 {
        m.insert((i % 100000).to_string(), i);
    }
    m["0"] + m["50000"]
}

fn a_map_get() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..100000 {
        m.insert(i.to_string(), i);
    }
    let mut acc: i64 = 0;
    for i in 0..600000 {
        acc += m[&((i * 7) % 100000).to_string()];
    }
    acc
}

fn a_map_miss() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..50000 {
        m.insert(i.to_string(), i);
    }
    let mut c: i64 = 0;
    for i in 0..300000 {
        if m.contains_key(&format!("nope{}", i % 5000)) {
            c += 1;
        }
    }
    300000 - c
}

fn a_map_iter() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..20000 {
        m.insert(i.to_string(), i);
    }
    let mut acc: i64 = 0;
    for _ in 0..10 {
        for k in m.keys() {
            acc += m[k];
        }
    }
    acc
}

fn a_map_incr() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for j in 0..5000 {
        m.insert(format!("w{}", j), 0);
    }
    for i in 0..200000 {
        let k = format!("w{}", i % 5000);
        let v = m[&k];
        m.insert(k, v + 1);
    }
    let mut s: i64 = 0;
    for j in 0..50 {
        s += m[&format!("w{}", j)];
    }
    s * 100000 + m.len() as i64
}

fn a_map_nested() -> i64 {
    let mut outer: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for i in 0..100 {
        let mut inner: HashMap<String, i64> = HashMap::new();
        for j in 0..50 {
            inner.insert(j.to_string(), i * j);
        }
        outer.insert(i.to_string(), inner);
    }
    let mut acc: i64 = 0;
    for i in 0..200000 {
        acc += outer[&(i % 100).to_string()][&(i % 50).to_string()];
    }
    acc
}

fn a_map_del() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..20000 {
        m.insert(i.to_string(), i);
    }
    for i in 0..50000 {
        let k = ((i * 7) % 20000).to_string();
        m.remove(&k);
        m.insert(k, i);
    }
    m.len() as i64 * 100000 + m["0"]
}

#[derive(PartialEq, Eq, Hash, Clone)]
enum Key {
    I(i64),
    S(String),
}

fn a_map_mixed() -> i64 {
    let mut m: HashMap<Key, String> = HashMap::new();
    let mut m2: HashMap<Key, i64> = HashMap::new();
    for i in 0..150000 {
        m.insert(Key::I(i % 50000), format!("v{}", i));
        m2.insert(Key::S(format!("s{}", i % 25000)), i);
    }
    (m.len() + m2.len()) as i64
}

// ---------------- SET ----------------

fn scan_in(xs: &Vec<i64>, t: i64) -> bool {
    for v in xs {
        if *v == t {
            return true;
        }
    }
    false
}

fn a_set_algebra() -> i64 {
    let mut x: i64 = 246813579;
    let mut xs: Vec<i64> = Vec::new();
    for _ in 0..16000 {
        x = lcg_next(x);
        xs.push(x % 8000);
    }
    let s: HashSet<i64> = xs.iter().cloned().collect();
    let mut c: i64 = 0;
    for i in 0..2000 {
        if s.contains(&(i % 9000)) {
            c += 1;
        }
    }
    s.len() as i64 * 100000 + c
}

// ---------------- ALGO ----------------

fn a_alg_sieve() -> i64 {
    let n: usize = 50000;
    let mut flags = vec![true; n];
    let mut c: i64 = 0;
    for i in 2..n {
        if flags[i] {
            c += 1;
            let mut j = i * i;
            while j < n {
                flags[j] = false;
                j += i;
            }
        }
    }
    c
}

fn a_alg_mandel() -> i64 {
    let mut total: i64 = 0;
    for row in 0..160 {
        for col in 0..240 {
            let x0 = col as f64 * 0.00875 - 2.1;
            let y0 = row as f64 * 0.00625 - 0.5;
            let mut zx = 0.0f64;
            let mut zy = 0.0f64;
            let mut it: i64 = 0;
            while it < 50 {
                let nx = zx * zx - zy * zy + x0;
                zy = 2.0 * zx * zy + y0;
                zx = nx;
                if zx * zx + zy * zy > 4.0 {
                    break;
                }
                it += 1;
            }
            total += it;
        }
    }
    total
}

struct Node {
    l: Option<Box<Node>>,
    r: Option<Box<Node>>,
}

fn tnode_build(d: i64) -> Option<Box<Node>> {
    if d == 0 {
        return None;
    }
    Some(Box::new(Node {
        l: tnode_build(d - 1),
        r: tnode_build(d - 1),
    }))
}

fn tnode_count(t: &Option<Box<Node>>) -> i64 {
    match t {
        None => 0,
        Some(n) => 1 + tnode_count(&n.l) + tnode_count(&n.r),
    }
}

fn a_alg_trees() -> i64 {
    let mut t: i64 = 0;
    for _ in 0..3 {
        let root = tnode_build(14);
        t += tnode_count(&root);
    }
    t
}

fn a_alg_matrix() -> i64 {
    let n: usize = 96;
    let mut a: Vec<Vec<i64>> = Vec::new();
    let mut b: Vec<Vec<i64>> = Vec::new();
    for i in 0..n {
        let mut ra: Vec<i64> = Vec::new();
        let mut rb: Vec<i64> = Vec::new();
        for j in 0..n {
            ra.push((i as i64 * 7 + j as i64) % 97);
            rb.push((i as i64 * 3 + j as i64 * 5) % 97);
        }
        a.push(ra);
        b.push(rb);
    }
    let mut acc: i64 = 0;
    for i in 0..n {
        for j in 0..n {
            let mut s: i64 = 0;
            for k in 0..n {
                s += a[i][k] * b[k][j];
            }
            acc += s % 1000003;
        }
    }
    acc
}

fn a_alg_wordfreq() -> i64 {
    let mut m: HashMap<String, i64> = HashMap::new();
    for j in 0..800 {
        m.insert(format!("w{}", j), 0);
    }
    for i in 0..2000 {
        let mut parts: Vec<String> = Vec::new();
        for j in 0..12 {
            parts.push(format!("w{}", (i * 13 + j * 7) % 800));
        }
        let line = parts.join(" ");
        for w in line.split(' ') {
            let v = m[w];
            m.insert(w.to_string(), v + 1);
        }
    }
    m["w0"] * 100000 + m.len() as i64
}

// minimal JSON (our doc shape: objects/arrays/strings/ints/bools)
enum J {
    N(i64),
    S(String),
    B(bool),
    A(Vec<J>),
    O(Vec<(String, J)>),
}

fn j_enc(v: &J, out: &mut String) {
    match v {
        J::N(n) => out.push_str(&n.to_string()),
        J::S(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        J::B(b) => out.push_str(if *b { "true" } else { "false" }),
        J::A(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                j_enc(x, out);
            }
            out.push(']');
        }
        J::O(kv) => {
            out.push('{');
            for (i, (k, x)) in kv.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                j_enc(x, out);
            }
            out.push('}');
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn val(&mut self) -> J {
        self.ws();
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut kv: Vec<(String, J)> = Vec::new();
                self.ws();
                if self.b[self.i] == b'}' {
                    self.i += 1;
                    return J::O(kv);
                }
                loop {
                    self.ws();
                    let s = self.string();
                    self.ws();
                    self.i += 1; // ':'
                    let v = self.val();
                    kv.push((s, v));
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    } else {
                        self.i += 1; // '}'
                        break;
                    }
                }
                J::O(kv)
            }
            b'[' => {
                self.i += 1;
                let mut xs: Vec<J> = Vec::new();
                self.ws();
                if self.b[self.i] == b']' {
                    self.i += 1;
                    return J::A(xs);
                }
                loop {
                    let v = self.val();
                    xs.push(v);
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    } else {
                        self.i += 1; // ']'
                        break;
                    }
                }
                J::A(xs)
            }
            b'"' => J::S(self.string()),
            b't' => {
                self.i += 4;
                J::B(true)
            }
            b'f' => {
                self.i += 5;
                J::B(false)
            }
            b'n' => {
                self.i += 4;
                J::N(0)
            }
            _ => {
                let st = self.i;
                while self.i < self.b.len()
                    && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'-')
                {
                    self.i += 1;
                }
                let s = std::str::from_utf8(&self.b[st..self.i]).unwrap();
                J::N(s.parse().unwrap())
            }
        }
    }
    fn string(&mut self) -> String {
        self.i += 1; // '"'
        let st = self.i;
        while self.b[self.i] != b'"' {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[st..self.i]).unwrap().to_string();
        self.i += 1;
        s
    }
}

fn j_get<'a>(v: &'a J, k: &str) -> &'a J {
    match v {
        J::O(kv) => {
            for (kk, vv) in kv {
                if kk == k {
                    return vv;
                }
            }
            panic!("missing key {}", k)
        }
        _ => panic!("not an object"),
    }
}

fn j_arr<'a>(v: &'a J) -> &'a Vec<J> {
    match v {
        J::A(xs) => xs,
        _ => panic!("not an array"),
    }
}

fn j_n(v: &J) -> i64 {
    match v {
        J::N(n) => *n,
        _ => panic!("not a number"),
    }
}

fn a_alg_json_rt() -> i64 {
    let mut rows: Vec<(String, J)> = Vec::new();
    for i in 0..120 {
        rows.push((
            format!("row{}", i),
            J::O(vec![
                ("id".to_string(), J::N(i)),
                ("name".to_string(), J::S(format!("item-{}", i))),
                ("tags".to_string(), J::A(vec![J::S("a".into()), J::S("b".into()), J::S("c".into())])),
                ("score".to_string(), J::N(i * 3)),
                ("active".to_string(), J::B(i % 2 == 0)),
            ]),
        ));
    }
    let doc = J::O(rows);
    let mut buf = String::new();
    j_enc(&doc, &mut buf);
    let mut acc: i64 = 0;
    for i in 0..150 {
        let mut p = P { b: buf.as_bytes(), i: 0 };
        let back = p.val();
        let r1 = j_get(&back, &format!("row{}", i % 120));
        acc += j_n(j_get(r1, "id"));
        let r2 = j_get(&back, &format!("row{}", (i + 7) % 120));
        acc += j_arr(j_get(r2, "tags")).len() as i64;
    }
    acc
}

fn a_alg_json_big() -> i64 {
    let mut rows: Vec<(String, J)> = Vec::new();
    for i in 0..800 {
        let mut tags: Vec<J> = Vec::new();
        for j in 0..5 {
            tags.push(J::S(format!("t{}", (i + j) % 32)));
        }
        rows.push((
            format!("r{}", i),
            J::O(vec![
                ("id".to_string(), J::N(i)),
                ("kind".to_string(), J::S(format!("k{}", i % 9))),
                ("tags".to_string(), J::A(tags)),
                ("w".to_string(), J::N((i * 7) % 1000)),
                ("ok".to_string(), J::B(i % 3 != 0)),
                ("note".to_string(), J::S(format!("n{}", i % 64))),
            ]),
        ));
    }
    let doc = J::O(rows);
    let mut buf = String::new();
    j_enc(&doc, &mut buf);
    let mut acc: i64 = 0;
    for _ in 0..2 {
        let mut p = P { b: buf.as_bytes(), i: 0 };
        let back = p.val();
        for i in 0..800 {
            let r = j_get(&back, &format!("r{}", i));
            acc += j_n(j_get(r, "id")) + j_arr(j_get(r, "tags")).len() as i64 + j_n(j_get(r, "w"));
        }
    }
    acc
}

// deep equality over our nested structure
#[derive(Clone)]
enum Val {
    I(i64),
    S(String),
    L(Vec<Val>),
}

fn veq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::I(x), Val::I(y)) => x == y,
        (Val::S(x), Val::S(y)) => x == y,
        (Val::L(x), Val::L(y)) => {
            if x.len() != y.len() {
                return false;
            }
            for i in 0..x.len() {
                if !veq(&x[i], &y[i]) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

fn a_alg_deep_eq() -> i64 {
    let mut a: Vec<Val> = Vec::new();
    for i in 0..50 {
        let sub = Val::L(vec![Val::I(i), Val::I(i + 1)]);
        a.push(Val::L(vec![
            Val::I(i),
            Val::S(format!("s{}", i % 7)),
            sub,
        ]));
    }
    let mut b: Vec<Val> = Vec::new();
    for i in 0..50 {
        let sub = Val::L(vec![Val::I(i), Val::I(i + 1)]);
        b.push(Val::L(vec![
            Val::I(i),
            Val::S(format!("s{}", i % 7)),
            sub,
        ]));
    }
    let mut b2: Vec<Val> = Vec::new();
    for i in 0..50 {
        let sub = Val::L(vec![Val::I(i), Val::I(i + 1)]);
        if i == 25 {
            b2.push(Val::L(vec![Val::I(i), Val::S("DIFF".into()), sub]));
        } else {
            b2.push(Val::L(vec![
                Val::I(i),
                Val::S(format!("s{}", i % 7)),
                sub,
            ]));
        }
    }
    let mut c: i64 = 0;
    for _ in 0..10000 {
        if veq(&Val::L(a.clone()), &Val::L(b.clone())) {
            c += 1;
        }
        if veq(&Val::L(a.clone()), &Val::L(b2.clone())) {
            c += 1;
        }
    }
    c
}

fn ocalc(i: i64) -> Result<i64, ()> {
    if i % 3 == 0 {
        Err(())
    } else {
        Ok((i * 7) % 1000)
    }
}

fn a_alg_opt() -> i64 {
    let mut acc: i64 = 0;
    for i in 0..200000 {
        acc += ocalc(i).unwrap_or(1);
    }
    acc
}

fn a_floor() -> i64 {
    0
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let aspect_id = &args[1];
    let (chk, dt) = match aspect_id.as_str() {
        "num_int_add" => time_it(a_num_int_add),
        "num_int_mixed" => time_it(a_num_int_mixed),
        "num_float_add" => time_it(a_num_float_add),
        "num_float_math" => time_it(a_num_float_math),
        "num_trialdiv" => time_it(a_num_trialdiv),
        "num_roundtrip" => time_it(a_num_roundtrip),
        "num_parse_float" => time_it(a_num_parse_float),
        "num_divmod" => time_it(a_num_divmod),
        "ctl_while" => time_it(a_ctl_while),
        "ctl_for" => time_it(a_ctl_for),
        "ctl_nested" => time_it(a_ctl_nested),
        "ctl_call" => time_it(a_ctl_call),
        "ctl_fib" => time_it(a_ctl_fib),
        "ctl_deep_rec" => time_it(a_ctl_deep_rec),
        "ctl_mutual" => time_it(a_ctl_mutual),
        "ctl_branch" => time_it(a_ctl_branch),
        "ctl_match" => time_it(a_ctl_match),
        "ctl_closure" => time_it(a_ctl_closure),
        "str_cat" => time_it(a_str_cat),
        "str_join" => time_it(a_str_join),
        "str_slice" => time_it(a_str_slice),
        "str_replace" => time_it(a_str_replace),
        "str_split" => time_it(a_str_split),
        "str_case" => time_it(a_str_case),
        "str_compare" => time_it(a_str_compare),
        "str_interp" => time_it(a_str_interp),
        "str_contains" => time_it(a_str_contains),
        "str_build" => time_it(a_str_build),
        "lst_push" => time_it(a_lst_push),
        "lst_idx" => time_it(a_lst_idx),
        "lst_iter" => time_it(a_lst_iter),
        "lst_slice" => time_it(a_lst_slice),
        "lst_sort" => time_it(a_lst_sort),
        "lst_sort_lang" => time_it(a_lst_sort_lang),
        "lst_comp" => time_it(a_lst_comp),
        "lst_search" => time_it(a_lst_search),
        "lst_reverse" => time_it(a_lst_reverse),
        "lst_insert_del" => time_it(a_lst_insert_del),
        "map_set" => time_it(a_map_set),
        "map_get" => time_it(a_map_get),
        "map_miss" => time_it(a_map_miss),
        "map_iter" => time_it(a_map_iter),
        "map_incr" => time_it(a_map_incr),
        "map_nested" => time_it(a_map_nested),
        "map_del" => time_it(a_map_del),
        "map_mixed" => time_it(a_map_mixed),
        "set_algebra" => time_it(a_set_algebra),
        "alg_sieve" => time_it(a_alg_sieve),
        "alg_mandel" => time_it(a_alg_mandel),
        "alg_trees" => time_it(a_alg_trees),
        "alg_matrix" => time_it(a_alg_matrix),
        "alg_wordfreq" => time_it(a_alg_wordfreq),
        "alg_json_rt" => time_it(a_alg_json_rt),
        "alg_json_big" => time_it(a_alg_json_big),
        "alg_deep_eq" => time_it(a_alg_deep_eq),
        "alg_opt" => time_it(a_alg_opt),
        "floor" => time_it(a_floor),
        _ => panic!("unknown aspect {}", aspect_id),
    };
    println!("OK {} {:.6}", chk, dt);
}

fn time_it(f: fn() -> i64) -> (i64, f64) {
    let t0 = Instant::now();
    let chk = f();
    let dt = t0.elapsed().as_secs_f64() * 1000.0;
    (chk, dt)
}
