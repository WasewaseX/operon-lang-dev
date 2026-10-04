#!/usr/bin/env python3
"""loop_mem_driver.py — P3 loop-memory audit driver (builder-E).

Spawns one process per leg (shape x size) for each engine. RSS instrument:
the driver polls the operon child's /proc/<pid>/status VmHWM (peak RSS of
the post-exec mm — wait4 ru_maxrss carries the spawner's fork floor and is
kept only as a cross-check column); the CPython and Rust mirrors self-report
VmHWM at exit. Unified output lines:

  LOOPRSS engine=<e> shape=<s> n=<N> outer=<O> time_ms=<t> hwm_kb=<hwm> ... <child extras>
"""
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OPERON = os.path.join(ROOT, "bin", "operon")
RS_BIN = "/tmp/loop_mem_rs"

SHAPES = ["acc_list", "transient_while", "transient_forrange", "transient_while_let", "nested"]
SIZES = [1_000_000, 2_000_000, 4_000_000]


def read_hwm(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM"):
                    return int(line.split()[1])
    except (FileNotFoundError, ProcessLookupError):
        pass
    return None


def run_operon(cmd, env):
    """Fork/exec operon; poll the child's VmHWM while it runs (post-exec mm,
    resets on exec — the clean instrument). Returns (stdout, hwm_kb, status)."""
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.dup2(w, 1)
        os.close(r)
        os.close(w)
        try:
            os.execvpe(cmd[0], cmd, env)
        finally:
            os._exit(127)
    os.close(w)
    last_hwm = 0
    while True:
        done, status, ru = os.wait4(pid, os.WNOHANG)
        if done == pid:
            return b"".join(read_fd(r)).decode(errors="replace"), last_hwm, status, ru.ru_maxrss
        h = read_hwm(pid)
        if h is not None:
            last_hwm = h
        time.sleep(0.004)


def read_fd(fd):
    chunks = []
    while True:
        b = os.read(fd, 65536)
        if not b:
            break
        chunks.append(b)
    os.close(fd)
    return chunks


def run_child(cmd, env):
    """Spawn (py/rs fixtures self-report VmHWM). Returns (stdout, status)."""
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.dup2(w, 1)
        os.close(r)
        os.close(w)
        try:
            os.execvpe(cmd[0], cmd, env)
        finally:
            os._exit(127)
    os.close(w)
    _, status, _ = os.wait4(pid, 0)
    out = b"".join(read_fd(r)).decode(errors="replace")
    return out, status


def main():
    sizes = SIZES
    if len(sys.argv) > 1:
        sizes = [int(x) for x in sys.argv[1].split(",")]

    for shape in SHAPES:
        for n in sizes:
            eff_n = n // 1000 if shape == "nested" else n  # nested: OUTER x (n/1000)
            outer = 1000 if shape == "nested" else 1
            env = dict(os.environ)
            env.update({"LOOP_MEM_SHAPE": shape, "LOOP_MEM_N": str(eff_n), "LOOP_MEM_OUTER": str(outer)})

            out, hwm, status, ru = run_operon([OPERON, "run",
                                               "--allow-env", "LOOP_MEM_SHAPE",
                                               "--allow-env", "LOOP_MEM_N",
                                               "--allow-env", "LOOP_MEM_OUTER",
                                               os.path.join(HERE, "loop_mem.op")], env)
            line = next((l for l in out.splitlines() if l.startswith("LOOP ")), None)
            if status != 0 or line is None:
                print(f"LOOPRSS engine=operon shape={shape} n={eff_n} outer={outer} ERROR status={status} out={out[:200]}")
            else:
                print(f"LOOPRSS hwm_kb={hwm} ru_maxrss_kb={ru} {line.split(' ', 1)[1]}")

            for engine, cmd in (
                ("cpython", [sys.executable, os.path.join(HERE, "loop_mem_py.py")]),
                ("rust", [RS_BIN, shape, str(eff_n), str(outer)]),
            ):
                out, status = run_child(cmd, env)
                line = next((l for l in out.splitlines() if l.startswith("LOOP ")), None)
                if status != 0 or line is None:
                    print(f"LOOPRSS engine={engine} shape={shape} n={eff_n} outer={outer} ERROR status={status}")
                else:
                    print(f"LOOPRSS {line.split(' ', 1)[1]}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
