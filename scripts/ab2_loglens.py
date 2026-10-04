import statistics, subprocess, sys, time
ROOT = "/home/z/my-project/operon-lang-dev"
OP = ROOT + "/target/release/operon"
CMD = [OP, "run", ROOT + "/apps/loglens/bench/ab2.op", "--cell",
       ROOT + "/apps/loglens/loglens.cell", "--allow-read", ROOT,
       "--fuel", "20000000000", "--"]
for stage in ["extract", "call", "v2", "v1"]:
    ts = []
    for _ in range(3):
        t0 = time.perf_counter()
        p = subprocess.run(CMD + [stage], capture_output=True, cwd=ROOT)
        ts.append((time.perf_counter() - t0) * 1000.0)
        if p.returncode != 0:
            print(stage, "ERR", p.stderr.decode()[:200]); sys.exit(1)
    print(f"{stage:8s} {statistics.median(ts):9.1f} ms")
