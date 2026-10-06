#!/usr/bin/env python3
"""bench_merge.py — stitch chunk JSONs from bench_chunk.py into one results file.

Usage: python3 scripts/bench/bench_merge.py out.json in1.json in2.json ...
Later files win on key collisions; "meta" is taken from the first input.
"""
import json
import sys

merged = {}
for i, path in enumerate(sys.argv[2:]):
    with open(path) as f:
        data = json.load(f)
    if i == 0:
        merged["meta"] = data.get("meta", {})
    for section in ("suites", "micros"):
        if section in data:
            merged.setdefault(section, {}).update(data[section])

with open(sys.argv[1], "w") as f:
    json.dump(merged, f, indent=2)
n_s = len(merged.get("suites", {}))
n_m = len(merged.get("micros", {}))
print(f"merged: {n_s} named, {n_m} micro -> {sys.argv[1]}")
