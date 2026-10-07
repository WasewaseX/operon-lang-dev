#!/usr/bin/env python3
# super-bench: 57-aspect dispatcher (CPython lane)
# Mirrors super.op / super_js.js / super_rs.rs with identical algorithms.
# Prints: OK <checksum> <elapsed_ms>
import math
import sys
import time


def lcg_next(x):
    return (x * 48271) % 2147483647


# ---------------- NUM ----------------

def a_num_int_add():
    acc = 0
    for i in range(2000000):
        acc = acc + i
    return acc


def a_num_int_mixed():
    acc = 0
    for i in range(600000):
        t = (acc * 3 + i) % 1000003
        acc = (t + t // 7 + i % 13) % 1000003
    return acc


def a_num_float_add():
    f = 0.0
    for i in range(2000000):
        f = f + i * 0.5 - f * 0.0000001
    return int(f)


def a_num_float_math():
    x = 1.0
    c = 0
    for _ in range(300000):
        x = math.sqrt(x * 1.7 + 0.3)
        if x > 100.0:
            x = 1.0
        if x > 2.0:
            c += 1
    return c * 100000 + int(x * 1000.0)


def a_num_trialdiv():
    c = 0
    for i in range(2, 40000):
        d = 2
        p = True
        while d * d <= i:
            if i % d == 0:
                p = False
                break
            d = d + 1
        if p:
            c += 1
    return c


def a_num_roundtrip():
    acc = 0
    for i in range(200000):
        s = str(i % 100000)
        acc = acc + int(s)
    return acc


def a_num_parse_float():
    acc = 0
    for _ in range(200000):
        v = float("1234.5678")
        acc = acc + int(v)
    return acc


def a_num_divmod():
    c3 = 0
    c5 = 0
    c15 = 0
    for i in range(1000000):
        if i % 3 == 0:
            c3 += 1
        if i % 5 == 0:
            c5 += 1
        if i % 15 == 0:
            c15 += 1
    return c3 * 100000000 + c5 * 10000 + c15


# ---------------- CTRL ----------------

def a_ctl_while():
    i = 1200000
    acc = 0
    while i > 0:
        acc = acc + i % 7
        i = i - 1
    return acc


def a_ctl_for():
    acc = 0
    for _ in range(1200000):
        acc = acc + 1
    return acc


def a_ctl_nested():
    s = 0
    for i in range(600):
        for j in range(600):
            s = s + (i * 600 + j) % 1000
    return s


def add1(a):
    return a + 1


def a_ctl_call():
    acc = 0
    for _ in range(600000):
        acc = add1(acc)
    return acc


def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def a_ctl_fib():
    t = 0
    for _ in range(5):
        t = t + fib(22)
    return t


def down(n):
    if n == 0:
        return 0
    return 1 + down(n - 1)


def a_ctl_deep_rec():
    t = 0
    for _ in range(6):
        t = t + down(5000)
    return t


def is_even(n):
    if n == 0:
        return True
    return is_odd(n - 1)


def is_odd(n):
    if n == 0:
        return False
    return is_even(n - 1)


def a_ctl_mutual():
    c = 0
    for _ in range(2000):
        if is_even(50):
            c += 1
    return c


def a_ctl_branch():
    c = [0] * 16
    for i in range(1000000):
        v = i % 16
        if v == 0:
            c[0] += 1
        elif v == 1:
            c[1] += 1
        elif v == 2:
            c[2] += 1
        elif v == 3:
            c[3] += 1
        elif v == 4:
            c[4] += 1
        elif v == 5:
            c[5] += 1
        elif v == 6:
            c[6] += 1
        elif v == 7:
            c[7] += 1
        elif v == 8:
            c[8] += 1
        elif v == 9:
            c[9] += 1
        elif v == 10:
            c[10] += 1
        elif v == 11:
            c[11] += 1
        elif v == 12:
            c[12] += 1
        elif v == 13:
            c[13] += 1
        elif v == 14:
            c[14] += 1
        elif v == 15:
            c[15] += 1
        else:
            c[0] += 1
    return sum(c[k] * (k + 1) for k in range(16))


def a_ctl_match():
    acc = 0
    for i in range(500000):
        v = i % 8
        if v == 0:
            acc = acc + 21
        elif v == 1:
            acc = acc + 28
        elif v == 2:
            acc = acc + 35
        elif v == 3:
            acc = acc + 42
        elif v == 4:
            acc = acc + 49
        elif v == 5:
            acc = acc + 56
        elif v == 6:
            acc = acc + 63
        else:
            acc = acc + 70
    return acc


def make_adder(k):
    return lambda x: x + k


def a_ctl_closure():
    acc = 0
    for i in range(200000):
        f = make_adder(i % 10)
        acc = acc + f(i % 1000)
    return acc


# ---------------- STR ----------------

def a_str_cat():
    s = ""
    for _ in range(30000):
        s = s + "ab"
    return len(s)


def a_str_join():
    parts = []
    for i in range(200000):
        parts.append(str(i % 1000))
    s = ",".join(parts)
    return len(s)


def a_str_slice():
    base = "The quick brown fox jumps over the lazy dog"
    acc = 0
    for i in range(200000):
        p = base[i % 30:i % 30 + 10]
        acc = acc + len(p)
    return acc


def a_str_replace():
    base = "The quick brown fox jumps over the lazy dog"
    acc = 0
    for _ in range(100000):
        t = base.replace("o", "0").replace("e", "3")
        acc = acc + len(t)
    return acc


def a_str_split():
    w10 = "alpha beta gamma delta epsilon zeta eta theta iota kappa"
    line = ""
    for _ in range(10):
        line = line + w10
    acc = 0
    for _ in range(20000):
        ws = line.split(" ")
        acc = acc + len(ws)
    return acc


def a_str_case():
    base = "The quick brown fox jumps over the lazy dog"
    acc = 0
    for _ in range(60000):
        u = base.upper()
        l = u.lower()
        t = ("  " + l + "  ").strip()
        acc = acc + len(t)
    return acc


def a_str_compare():
    base = "The quick brown fox jumps over the lazy dog"
    s2 = "The quick brown fox jumps over the lazy dof"
    c = 0
    for _ in range(300000):
        if base == s2:
            c += 1
        if base.startswith("The quick"):
            c += 1
        if base.endswith("dog"):
            c += 1
    return c


def a_str_interp():
    acc = 0
    for i in range(100000):
        t = f"v={i},x={i % 97},y={(i * 7) % 1000}"
        acc = acc + len(t)
    return acc


def a_str_contains():
    base = "The quick brown fox jumps over the lazy dog"
    c = 0
    d = 0
    for _ in range(200000):
        if "quick" in base:
            c += 1
        if "zebra" in base:
            d += 1
    return c * 3 + d


def a_str_build():
    acc = 0
    for i in range(60000):
        p = "é" + str(i % 100) + "ü中"
        q = p.upper()
        acc = acc + len(q)
    return acc


# ---------------- LIST ----------------

def a_lst_push():
    xs = []
    for i in range(300000):
        xs.append(i)
    acc = 0
    for v in xs:
        acc = acc + v
    return acc


def a_lst_idx():
    xs = []
    for i in range(100000):
        xs.append(i % 100000)
    acc = 0
    for i in range(600000):
        acc = acc + xs[(i * 7) % 100000]
    return acc


def a_lst_iter():
    xs = []
    for i in range(100000):
        xs.append(i % 1000)
    acc = 0
    for _ in range(6):
        for v in xs:
            acc = acc + v
    return acc


def a_lst_slice():
    xs = []
    for i in range(2000):
        xs.append(i)
    acc = 0
    for i in range(60000):
        p = xs[i % 1500:i % 1500 + 100]
        acc = acc + len(p)
    return acc


def a_lst_sort():
    x = 123456789
    xs = []
    for _ in range(60000):
        x = lcg_next(x)
        xs.append(x % 1000000)
    xs.sort()
    return xs[0] + xs[29999] + xs[59999]


def qs(a, lo, hi):
    if lo >= hi:
        return 0
    p = a[hi]
    i = lo - 1
    for j in range(lo, hi):
        if a[j] <= p:
            i = i + 1
            t = a[i]
            a[i] = a[j]
            a[j] = t
    t2 = a[i + 1]
    a[i + 1] = a[hi]
    a[hi] = t2
    qs(a, lo, i)
    qs(a, i + 2, hi)
    return 0


def a_lst_sort_lang():
    x = 987654321
    a = []
    for _ in range(3000):
        x = lcg_next(x)
        a.append(x % 100000)
    qs(a, 0, 2999)
    return a[0] + a[1500] + a[2999]


def a_lst_comp():
    xs = []
    for i in range(200000):
        xs.append(i % 1000)
    a1 = [x * 2 for x in xs if x % 3 == 0]
    s1 = 0
    for v in a1:
        s1 = s1 + v
    a2 = [x for x in a1 if x % 6 == 0]
    s2 = 0
    for v in a2:
        s2 = s2 + v
    return s1 + s2


def a_lst_search():
    x = 555555555
    xs = []
    for _ in range(1000):
        x = lcg_next(x)
        xs.append(x % 100000)
    found = 0
    scans = 0
    for i in range(8000):
        t = (i * 37) % 150000
        k = 0
        while k < 1000:
            scans += 1
            if xs[k] == t:
                found += 1
                break
            k = k + 1
    return found * 100000000 + scans


def a_lst_reverse():
    xs = []
    for i in range(1000):
        xs.append(i)
    acc = 0
    for _ in range(30000):
        r = xs[::-1]
        acc = acc + r[0]
    return acc


def a_lst_insert_del():
    xs = []
    for i in range(3000):
        xs.append(i)
    for i in range(10000):
        xs.insert(i % 3000, i)
        xs.pop((i * 7) % 3000)
    return xs[0] + xs[1500] + xs[2999]


# ---------------- MAP ----------------

def a_map_set():
    m = {}
    for i in range(300000):
        m[str(i % 100000)] = i
    return m["0"] + m["50000"]


def a_map_get():
    m = {}
    for i in range(100000):
        m[str(i)] = i
    acc = 0
    for i in range(600000):
        acc = acc + m[str((i * 7) % 100000)]
    return acc


def a_map_miss():
    m = {}
    for i in range(50000):
        m[str(i)] = i
    c = 0
    for i in range(300000):
        if ("nope" + str(i % 5000)) in m:
            c += 1
    return 300000 - c


def a_map_iter():
    m = {}
    for i in range(20000):
        m[str(i)] = i
    acc = 0
    for _ in range(10):
        for k in m.keys():
            acc = acc + m[k]
    return acc


def a_map_incr():
    m = {}
    for j in range(5000):
        m["w" + str(j)] = 0
    for i in range(200000):
        k = "w" + str(i % 5000)
        m[k] = m[k] + 1
    s = 0
    for j in range(50):
        s = s + m["w" + str(j)]
    return s * 100000 + len(m)


def a_map_nested():
    outer = {}
    for i in range(100):
        inner = {}
        for j in range(50):
            inner[str(j)] = i * j
        outer[str(i)] = inner
    acc = 0
    for i in range(200000):
        acc = acc + outer[str(i % 100)][str(i % 50)]
    return acc


def a_map_del():
    m = {}
    for i in range(20000):
        m[str(i)] = i
    for i in range(50000):
        k = str((i * 7) % 20000)
        del m[k]
        m[k] = i
    return len(m) * 100000 + m["0"]


def a_map_mixed():
    m = {}
    for i in range(150000):
        m[i % 50000] = "v" + str(i)
        m["s" + str(i % 25000)] = i
    return len(m)


# ---------------- SET ----------------

def scan_in(xs, t):
    for v in xs:
        if v == t:
            return True
    return False


def a_set_algebra():
    x = 246813579
    xs = []
    for _ in range(16000):
        x = lcg_next(x)
        xs.append(x % 8000)
    s = set(xs)
    c = 0
    for i in range(2000):
        if (i % 9000) in s:
            c += 1
    return len(s) * 100000 + c


# ---------------- ALGO ----------------

def a_alg_sieve():
    n = 50000
    flags = [True] * n
    c = 0
    for i in range(2, n):
        if flags[i]:
            c += 1
            j = i * i
            while j < n:
                flags[j] = False
                j = j + i
    return c


def a_alg_mandel():
    total = 0
    for row in range(160):
        for col in range(240):
            x0 = (col * 0.00875) - 2.1
            y0 = (row * 0.00625) - 0.5
            zx = 0.0
            zy = 0.0
            it = 0
            while it < 50:
                nx = zx * zx - zy * zy + x0
                zy = 2.0 * zx * zy + y0
                zx = nx
                if zx * zx + zy * zy > 4.0:
                    break
                it = it + 1
            total = total + it
    return total


def tnode_build(d):
    if d == 0:
        return None
    return [1, tnode_build(d - 1), tnode_build(d - 1)]


def tnode_count(t):
    if t is None:
        return 0
    return 1 + tnode_count(t[1]) + tnode_count(t[2])


def a_alg_trees():
    t = 0
    for _ in range(3):
        root = tnode_build(14)
        t = t + tnode_count(root)
    return t


def a_alg_matrix():
    n = 96
    a = []
    b = []
    for i in range(n):
        ra = []
        rb = []
        for j in range(n):
            ra.append((i * 7 + j) % 97)
            rb.append((i * 3 + j * 5) % 97)
        a.append(ra)
        b.append(rb)
    acc = 0
    for i in range(n):
        for j in range(n):
            s = 0
            for k in range(n):
                s = s + a[i][k] * b[k][j]
            acc = acc + s % 1000003
    return acc


def a_alg_wordfreq():
    m = {}
    for j in range(800):
        m["w" + str(j)] = 0
    for i in range(2000):
        parts = []
        for j in range(12):
            parts.append("w" + str((i * 13 + j * 7) % 800))
        line = " ".join(parts)
        ws = line.split(" ")
        for w in ws:
            m[w] = m[w] + 1
    return m["w0"] * 100000 + len(m)


def a_alg_json_rt():
    import json as _json
    doc = {}
    for i in range(120):
        doc["row" + str(i)] = {"id": i, "name": "item-" + str(i), "tags": ["a", "b", "c"], "score": i * 3, "active": i % 2 == 0}
    acc = 0
    for i in range(150):
        s = _json.dumps(doc)
        back = _json.loads(s)
        acc = acc + back["row" + str(i % 120)]["id"] + len(back["row" + str((i + 7) % 120)]["tags"])
    return acc


def a_alg_json_big():
    import json as _json
    doc = {}
    for i in range(800):
        tags = []
        for j in range(5):
            tags.append("t" + str((i + j) % 32))
        doc["r" + str(i)] = {"id": i, "kind": "k" + str(i % 9), "tags": tags, "w": (i * 7) % 1000, "ok": i % 3 != 0, "note": "n" + str(i % 64)}
    s = _json.dumps(doc)
    acc = 0
    for _ in range(2):
        back = _json.loads(s)
        for i in range(800):
            r = back["r" + str(i)]
            acc = acc + r["id"] + len(r["tags"]) + r["w"]
    return acc


def a_alg_deep_eq():
    a = []
    for i in range(50):
        sub = [i, i + 1]
        a.append([i, "s" + str(i % 7), sub])
    b = []
    for i in range(50):
        sub = [i, i + 1]
        b.append([i, "s" + str(i % 7), sub])
    b2 = []
    for i in range(50):
        sub = [i, i + 1]
        if i == 25:
            b2.append([i, "DIFF", sub])
        else:
            b2.append([i, "s" + str(i % 7), sub])
    c = 0
    for _ in range(10000):
        if a == b:
            c += 1
        if a == b2:
            c += 1
    return c


def ocalc(i):
    if i % 3 == 0:
        return (False, 0)
    return (True, (i * 7) % 1000)


def a_alg_opt():
    acc = 0
    for i in range(200000):
        ok, v = ocalc(i)
        acc = acc + (v if ok else 1)
    return acc


def a_floor():
    return 0


ASPECTS = {
    "num_int_add": a_num_int_add,
    "num_int_mixed": a_num_int_mixed,
    "num_float_add": a_num_float_add,
    "num_float_math": a_num_float_math,
    "num_trialdiv": a_num_trialdiv,
    "num_roundtrip": a_num_roundtrip,
    "num_parse_float": a_num_parse_float,
    "num_divmod": a_num_divmod,
    "ctl_while": a_ctl_while,
    "ctl_for": a_ctl_for,
    "ctl_nested": a_ctl_nested,
    "ctl_call": a_ctl_call,
    "ctl_fib": a_ctl_fib,
    "ctl_deep_rec": a_ctl_deep_rec,
    "ctl_mutual": a_ctl_mutual,
    "ctl_branch": a_ctl_branch,
    "ctl_match": a_ctl_match,
    "ctl_closure": a_ctl_closure,
    "str_cat": a_str_cat,
    "str_join": a_str_join,
    "str_slice": a_str_slice,
    "str_replace": a_str_replace,
    "str_split": a_str_split,
    "str_case": a_str_case,
    "str_compare": a_str_compare,
    "str_interp": a_str_interp,
    "str_contains": a_str_contains,
    "str_build": a_str_build,
    "lst_push": a_lst_push,
    "lst_idx": a_lst_idx,
    "lst_iter": a_lst_iter,
    "lst_slice": a_lst_slice,
    "lst_sort": a_lst_sort,
    "lst_sort_lang": a_lst_sort_lang,
    "lst_comp": a_lst_comp,
    "lst_search": a_lst_search,
    "lst_reverse": a_lst_reverse,
    "lst_insert_del": a_lst_insert_del,
    "map_set": a_map_set,
    "map_get": a_map_get,
    "map_miss": a_map_miss,
    "map_iter": a_map_iter,
    "map_incr": a_map_incr,
    "map_nested": a_map_nested,
    "map_del": a_map_del,
    "map_mixed": a_map_mixed,
    "set_algebra": a_set_algebra,
    "alg_sieve": a_alg_sieve,
    "alg_mandel": a_alg_mandel,
    "alg_trees": a_alg_trees,
    "alg_matrix": a_alg_matrix,
    "alg_wordfreq": a_alg_wordfreq,
    "alg_json_rt": a_alg_json_rt,
    "alg_json_big": a_alg_json_big,
    "alg_deep_eq": a_alg_deep_eq,
    "alg_opt": a_alg_opt,
    "floor": a_floor,
}


def main():
    sys.setrecursionlimit(20000)
    aspect_id = sys.argv[1]
    fn = ASPECTS[aspect_id]
    t0 = time.perf_counter()
    chk = fn()
    dt = (time.perf_counter() - t0) * 1000.0
    print(f"OK {chk} {dt:.6f}")


if __name__ == "__main__":
    main()
