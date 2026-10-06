#!/usr/bin/env python3
# native.py — CPython mirrors of the operon benchmark fixtures, run as a
# PROCESS (so all four runners are timed the same end-to-end way).
# Workload functions are 1:1 with scripts/bench_compare.py native_* mirrors.
# Every workload prints the SAME canonical string as the operon fixture so
# run_xlang.py can cross-verify correctness across all four runners.


def w_fib25():
    def fib(n):
        return n if n < 2 else fib(n - 1) + fib(n - 2)
    return f"fib(25) = {fib(25)}"


def w_loops():
    acc = 0
    for i in range(200000):
        acc = acc + i % 7
    return f"acc = {acc}"


def w_strings():
    s = ""
    for i in range(4000):
        s = s + "x{t}y".replace("{t}", str(i % 10))
    return f"len = {len(s)}"


def w_collections():
    m = {}
    xs = []
    for i in range(20000):
        k = "k" + str(i % 2000)
        if k in m:
            m[k] = m[k] + 1
        else:
            m[k] = 1
        xs.append(i % 97)
    total = 0
    for v in xs:
        total = total + v
    distinct = 0
    for _ in m:
        distinct += 1
    return f"total = {total}, distinct = {distinct}, k1 = {m['k1']}"


def w_recursion():
    def pasc(n, k):
        return 1 if (k == 0 or k == n) else pasc(n - 1, k - 1) + pasc(n - 1, k)
    return f"C(20,10) = {pasc(20, 10)}"


def w_grn():
    # mirror of grn.op: per-call gate check, driver fired at 1.0 — all pass.
    level = {"driver": 0.0}

    def worker_a(n):
        if level.get("driver", 0.0) < 0.3 - 0.25:
            return None
        return n + 1

    def worker_b(n):
        if level.get("driver", 0.0) < 0.5:
            return None
        return n * 2

    def reporter(n):
        if level.get("driver", 0.0) < 0.7:
            return None
        return n - 1

    level["driver"] = 1.0  # grn_fire("driver")
    acc = 0
    for i in range(20000):
        acc = acc + worker_a(i) + worker_b(i) + reporter(i)
    return f"acc = {acc}"


def w_m_empty():
    return "ok"


def w_m_call():
    def nop(n):
        return n
    a = 0
    for i in range(100000):
        a = a + nop(i)
    return f"a = {a}"


def w_m_forrange():
    a = 0
    for i in range(200000):
        a = a + 1
    return f"a = {a}"


def w_m_while():
    i, a = 200000, 0
    while i > 0:
        a = a + 1
        i = i - 1
    return f"a = {a}"


def w_m_varread():
    x, a = 7, 0
    for i in range(200000):
        a = a + x
    return f"a = {a}"


def w_m_intadd():
    a, b = 0, 1
    for i in range(300000):
        a = a + b
    return f"a = {a}"


def w_m_listpush():
    xs = []
    for i in range(50000):
        xs.append(i)
    return f"len = {len(xs)}"


def w_m_listidx():
    xs = [i for i in range(2000)]
    a = 0
    for i in range(100000):
        a = a + xs[i % 2000]
    return f"a = {a}"


def w_m_mapset():
    m = {}
    for i in range(40000):
        m[str(i % 4000)] = i
    return f"len = {len(m)}"


def w_m_mapget():
    m = {}
    for i in range(4000):
        m[str(i)] = i
    a = 0
    for i in range(50000):
        a = a + m[str(i % 4000)]
    return f"a = {a}"


def w_m_strcat():
    s = ""
    for i in range(12000):
        s = s + "ab"
    return f"len = {len(s)}"


WORKLOADS = {
    "fib25": w_fib25, "loops": w_loops, "strings": w_strings,
    "collections": w_collections, "recursion": w_recursion, "grn": w_grn,
    "m_empty": w_m_empty, "m_call": w_m_call, "m_forrange": w_m_forrange,
    "m_while": w_m_while, "m_varread": w_m_varread, "m_intadd": w_m_intadd,
    "m_listpush": w_m_listpush, "m_listidx": w_m_listidx,
    "m_mapset": w_m_mapset, "m_mapget": w_m_mapget, "m_strcat": w_m_strcat,
}

if __name__ == "__main__":
    import sys
    name = sys.argv[1] if len(sys.argv) > 1 else ""
    fn = WORKLOADS.get(name)
    print(fn() if fn else f"unknown workload: {name}")
