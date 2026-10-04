# bench: call_bridged_py — CPython mirror of call_bridged.op (builder-E)
# abs() in CPython is a C builtin too — the same bridge analogue. The wrapped
# leg wraps abs in a Python def; the pure leg is a pure-Python negation.
import sys, time

def wrap(x): return abs(x)
def idneg(x): return -x

def leg_direct(n):
    s = 0; i = 0
    while i < n: s = s + abs(-i); i = i + 1
    return s

def leg_wrapped(n):
    s = 0; i = 0
    while i < n: s = s + wrap(i); i = i + 1
    return s

def leg_pure(n):
    s = 0; i = 0
    while i < n: s = s + idneg(i); i = i + 1
    return s

def main():
    n = int(sys.argv[1]); reps = int(sys.argv[2])
    abs(-1); wrap(1); idneg(1)  # warm
    for r in range(reps):
        t0 = time.perf_counter(); sd = leg_direct(n); t1 = time.perf_counter()
        sw = leg_wrapped(n); t2 = time.perf_counter()
        sp = leg_pure(n); t3 = time.perf_counter()
        nd = (t1 - t0) * 1e9 / n; nw = (t2 - t1) * 1e9 / n; np = (t3 - t2) * 1e9 / n
        if r == reps - 1:
            print(f"BRIDGE leg=direct calls={n} ns_per_call={nd} ck={sd}")
            print(f"BRIDGE leg=wrapped calls={n} ns_per_call={nw} ck={sw}")
            print(f"BRIDGE leg=pure calls={n} ns_per_call={np} ck={sp}")

if __name__ == "__main__":
    main()
