# bench: call_fib_py — CPython anchor for the P4 call ladder (builder-E)
# Same recursive shape as call_fib.op; times ONLY the fib(n) call (in-process
# perf_counter, min of REPS in one process). Call counts are engine-
# independent: C(n) = 2*F(n+1) - 1.
import sys, time

def fib(n):
    if n < 2: return n
    return fib(n - 1) + fib(n - 2)

def main():
    n = int(sys.argv[1]); reps = int(sys.argv[2])
    fib(min(n, 20))  # warm
    best = None
    for _ in range(reps):
        t0 = time.perf_counter()
        r = fib(n)
        t1 = time.perf_counter()
        ms = (t1 - t0) * 1000.0
        if best is None or ms < best: best = ms
    print(f"CALLFIB n={n} r={r} ms={best}")

if __name__ == "__main__":
    main()
