# bench: call_args_py — CPython mirror of call_args.op (builder-E)
# Same genes as plain functions, same while-loop shape, same clock shape.
# The last rep's per-leg ns/call is printed in the ARGS line format.
import sys, time

def f0(): return 7
def f1(a): return a
def f2(a, b): return a + b
def f4(a, b, c, d): return a + b + c + d
def f8(a, b, c, d, e, f, g, h): return a + b + c + d + e + f + g + h

def leg0(n):
    s = 0; i = 0
    while i < n: s = s + f0(); i = i + 1
    return s

def leg1(n):
    s = 0; i = 0
    while i < n: s = s + f1(i); i = i + 1
    return s

def leg2(n):
    s = 0; i = 0
    while i < n: s = s + f2(i, i); i = i + 1
    return s

def leg4(n):
    s = 0; i = 0
    while i < n: s = s + f4(i, i, i, i); i = i + 1
    return s

def leg8(n):
    s = 0; i = 0
    while i < n: s = s + f8(i, i, i, i, i, i, i, i); i = i + 1
    return s

def main():
    n = int(sys.argv[1]); reps = int(sys.argv[2])
    f0(); f1(1); f2(1, 2); f4(1, 2, 3, 4); f8(1, 2, 3, 4, 5, 6, 7, 8)  # warm
    for r in range(reps):
        t0 = time.perf_counter(); s0 = leg0(n); t1 = time.perf_counter()
        s1 = leg1(n); t2 = time.perf_counter()
        s2 = leg2(n); t3 = time.perf_counter()
        s4 = leg4(n); t4 = time.perf_counter()
        s8 = leg8(n); t5 = time.perf_counter()
        n0 = (t1 - t0) * 1e9 / n; n1 = (t2 - t1) * 1e9 / n
        n2 = (t3 - t2) * 1e9 / n; n4 = (t4 - t3) * 1e9 / n
        n8 = (t5 - t4) * 1e9 / n
        if r == reps - 1:
            for ar, ns, ck in ((0, n0, s0), (1, n1, s1), (2, n2, s2),
                               (4, n4, s4), (8, n8, s8)):
                print(f"ARGS arity={ar} calls={n} ns_per_call={ns} ck={ck}")

if __name__ == "__main__":
    main()
