#!/usr/bin/env python3
"""rss_wrap.py — run CMD, forward its stdout, emit peak child RSS on stderr.

Used by scripts/bench_deep.py to measure the peak resident set of one
benchmark run. A FRESH wrapper process per measurement keeps
getrusage(RUSAGE_CHILDREN).ru_maxrss clean (it is a running max across
all waited children of THIS process, so reusing a wrapper would mix
measurements). Note: the max includes the app's own children (the mock
engines, ~2-3 MB bash) — identical noise for every implementation.

On Linux ru_maxrss is in kilobytes.
"""
import resource
import subprocess
import sys


def main():
    p = subprocess.run(sys.argv[1:], capture_output=True, text=True)
    sys.stdout.write(p.stdout)
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    sys.stderr.write(f"RSS_KB={peak}\n")
    sys.exit(p.returncode)


if __name__ == "__main__":
    main()
