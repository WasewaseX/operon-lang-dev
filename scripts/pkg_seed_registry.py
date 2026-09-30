#!/usr/bin/env python3
"""Seed a local file registry with the official Operon packages.

Builds publish envelopes for every package under packaging/packages/<name>
with the SAME deterministic encoder the Rust client uses (sorted files,
base64 payloads, trailing newline), computes sha256, and writes:

    <out>/index/<name>.json
    <out>/artifacts/<name>/<version>.opkg

The result is a registry directory the operon CLI consumes directly:
    OPERON_REGISTRY=<out> operon add http

Usage:
    python3 scripts/pkg_seed_registry.py [out-dir]     # default: ./registry
"""

import base64
import hashlib
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG_SRC = os.path.join(ROOT, "packaging", "packages")


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def parse_manifest(text: str) -> dict:
    # minimal reader for the fields we need (same subset the Rust side pins)
    out = {"deps": {}}
    section = None
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("["):
            section = line.strip("[]")
            continue
        if "=" not in line:
            continue
        k, v = line.split("=", 1)
        k, v = k.strip(), v.strip().strip('"')
        if section == "package":
            out[k] = v
        elif section == "dependencies":
            out["deps"][k] = v
    return out


def build_envelope(pkg_dir: str) -> bytes:
    manifest_path = os.path.join(pkg_dir, "operon.toml")
    with open(manifest_path, encoding="utf-8") as f:
        manifest_text = f.read()
    m = parse_manifest(manifest_text)
    files = []
    for rel in sorted(os.listdir(pkg_dir)):
        p = os.path.join(pkg_dir, rel)
        if os.path.isfile(p):
            if rel.endswith(".op") or rel.endswith(".toml") or rel.endswith(".md"):
                with open(p, "rb") as f:
                    files.append({"path": rel, "b64": b64(f.read())})
        elif rel == "py":
            # packages may ship a python helper tree (postgres)
            for sub in sorted(os.listdir(p)):
                if sub.endswith(".py"):
                    with open(os.path.join(p, sub), "rb") as f:
                        files.append(
                            {"path": f"py/{sub}", "b64": b64(f.read())}
                        )
    env = {
        "envelope": 1,
        "name": m["name"],
        "version": m["version"],
        "description": m.get("description", ""),
        "deps": m["deps"],
        "manifest": manifest_text,
        "files": files,
    }
    return (json.dumps(env, ensure_ascii=False, sort_keys=False) + "\n").encode("utf-8")


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "registry")
    os.makedirs(os.path.join(out, "index"), exist_ok=True)
    os.makedirs(os.path.join(out, "artifacts"), exist_ok=True)
    seeded = []
    for name in sorted(os.listdir(PKG_SRC)):
        pkg_dir = os.path.join(PKG_SRC, name)
        if not os.path.isdir(pkg_dir):
            continue
        artifact = build_envelope(pkg_dir)
        env = json.loads(artifact.decode("utf-8"))
        sha = hashlib.sha256(artifact).hexdigest()
        art_dir = os.path.join(out, "artifacts", name)
        os.makedirs(art_dir, exist_ok=True)
        art_path = os.path.join(art_dir, f"{env['version']}.opkg")
        with open(art_path, "wb") as f:
            f.write(artifact)
        index = {
            "name": env["name"],
            "description": env["description"],
            "versions": [
                {
                    "version": env["version"],
                    "yanked": False,
                    "sha256": sha,
                    "deps": env["deps"],
                    "description": env["description"],
                }
            ],
        }
        with open(os.path.join(out, "index", f"{name}.json"), "w", encoding="utf-8") as f:
            f.write(json.dumps(index, ensure_ascii=False) + "\n")
        seeded.append(f"{name} {env['version']} sha256={sha[:12]}…")
    print(f"seeded {len(seeded)} package(s) into {out}:")
    for s in seeded:
        print(f"  {s}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
