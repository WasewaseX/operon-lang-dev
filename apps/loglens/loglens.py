#!/usr/bin/env python3
# loglens.py — the Python reference implementation of apps/loglens/loglens.op
#
# BYTE-IDENTITY CONTRACT: every output byte of this program must equal the
# Operon implementation's output on the same inputs, and equally the Rust
# (loglens.rs) and Node (loglens.js) baselines — the bench asserts it via
# sha256 over stdout on every run of every workload. The rules that make
# that hold:
#   - the parser is SPLIT-BASED and identical in all four engines:
#       pieces = line.split('"')   -> exactly 3 pieces per the fixture
#                                     contract (malformed lines are
#                                     skipped, never counted)
#       host   = pieces[0].split(" ")[0]
#       req    = pieces[1].split(" ") -> [method, url, proto]
#       rest   = pieces[2].split(" ") -> ["", status, bytes]
#   - records iterate in file order; maps count keys in first-touch order
#     but every printed order is TOTAL (count desc then value asc, or
#     status ascending numerically), so map iteration order never leaks;
#   - integers render via str(int) on every side; bytes/status stay
#     integers end to end (i64-safe: 50k x 60k < 2^53);
#   - ratios render via render_ratio(), whose float op sequence
#     (x = num/den; sc = 10^nd by repeated *10; r = floor(x*sc + 0.5);
#     integer split) is mirrored EXACTLY by loglens.op, loglens.js and
#     loglens.rs — all are IEEE-754 doubles on exact integer inputs, so
#     r is bit-identical and every later step is integer arithmetic;
#   - no float repr()/str() of floats is ever printed;
#   - the fixture ends with exactly one trailing newline; a trailing
#     empty piece after the final "\n" split is dropped, and only \n is
#     used (no CR anywhere).

import math
import sys


def render_ratio(num, den, nd):
    # MIRRORED FLOAT SEQUENCE — keep op-for-op identical with loglens.op
    x = num / den
    sc = 1.0
    for _ in range(nd):
        sc = sc * 10.0
    r = math.floor(x * sc + 0.5)
    si = int(sc)
    whole = r // si
    frac = r % si
    if nd == 0:
        return f"{whole}"
    fd = str(frac)
    while len(fd) < nd:
        fd = "0" + fd
    return f"{whole}.{fd}"


def split_lines(text):
    # identical contract to loglens.op's split_log_lines:
    # split on \n, drop ONE trailing empty piece if the text ends with \n
    parts = text.split("\n")
    if len(parts) > 0 and parts[-1] == "":
        parts.pop()
    return parts


def parse_line(line):
    # returns [host, url, status_str, bytes_int] or None (malformed)
    pieces = line.split('"')
    if len(pieces) != 3:
        return None
    host_parts = pieces[0].split(" ")
    if len(host_parts) < 1 or host_parts[0] == "":
        return None
    host = host_parts[0]
    req = pieces[1].split(" ")
    if len(req) != 3:
        return None
    url = req[1]
    rest = pieces[2].split(" ")
    if len(rest) != 3 or rest[0] != "":
        return None
    status = rest[1]
    byts = rest[2]
    if not is_digits(status) or not is_digits(byts):
        return None
    return [host, url, status, int(byts)]


def is_digits(s):
    if len(s) == 0:
        return False
    for ch in s:
        if ch < "0" or ch > "9":
            return False
    return True


def parse_records(text):
    out = []
    for line in split_lines(text):
        rec = parse_line(line)
        if rec is not None:
            out.append(rec)
    return out


def count_map(values):
    m = {}
    for v in values:
        m[v] = m.get(v, 0) + 1
    return m


def top_pairs(m, n, numeric=False):
    # count desc, then value asc (numeric value asc for the status field);
    # the key is total so the stable sort leaves no ambiguity
    items = [[k, c] for k, c in m.items()]
    if numeric:
        items.sort(key=lambda p: (-p[1], int(p[0])))
    else:
        items.sort(key=lambda p: (-p[1], p[0]))
    return items[:n]


def cmd_stats(text, path):
    recs = parse_records(text)
    hosts = count_map([r[0] for r in recs])
    urls = count_map([r[1] for r in recs])
    status = count_map([r[2] for r in recs])
    total_bytes = 0
    for r in recs:
        total_bytes += r[3]
    print(f"loglens stats {path}")
    print(f"records,{len(recs)}")
    print(f"hosts,{len(hosts)}")
    print(f"urls,{len(urls)}")
    print(f"bytes,{total_bytes}")
    print("status,count")
    for k, c in sorted(status.items(), key=lambda p: int(p[0])):
        print(f"{k},{c}")


def cmd_top(text, path, field, n):
    if field not in ("host", "url", "status"):
        print(f"loglens: unknown field {field}")
        sys.exit(2)
    recs = parse_records(text)
    idx = 0 if field == "host" else (1 if field == "url" else 2)
    m = count_map([r[idx] for r in recs])
    print(f"loglens top {path} {field} {n}")
    print("rank,value,count,share")
    pairs = top_pairs(m, n, numeric=(field == "status"))
    rank = 1
    for p in pairs:
        share = render_ratio(p[1] * 100, len(recs), 2)
        print(f"{rank},{p[0]},{p[1]},{share}")
        rank += 1


def cmd_errors(text, path, n):
    recs = parse_records(text)
    errs = [r for r in recs if int(r[2]) >= 400]
    m = count_map([r[1] for r in errs])
    total_bytes = 0
    for r in errs:
        total_bytes += r[3]
    print(f"loglens errors {path} {n}")
    print(f"count,{len(errs)}")
    print(f"bytes,{total_bytes}")
    print("rank,url,count,share")
    pairs = top_pairs(m, n, numeric=False)
    rank = 1
    for p in pairs:
        share = render_ratio(p[1] * 100, len(errs), 2) if len(errs) > 0 else render_ratio(0, 1, 2)
        print(f"{rank},{p[0]},{p[1]},{share}")
        rank += 1


def cmd_table(text, path, n):
    recs = parse_records(text)
    print(f"loglens table {path} {n}")
    head = ["host", "url", "status", "bytes"]
    shown = [[r[0], r[1], r[2], str(r[3])] for r in recs[:n]]
    lines = render_table(head, shown)
    for l in lines:
        print(l)


def dashes(widths):
    out = "+"
    for w in widths:
        out += "-" * (w + 2) + "+"
    return out


def table_row(cells, widths):
    out = "|"
    for i in range(len(cells)):
        out += " " + cells[i] + " " * (widths[i] - len(cells[i])) + " |"
    return out


def render_table(head, rows):
    ncols = len(head)
    widths = [len(h) for h in head]
    for r in rows:
        for i in range(ncols):
            if len(r[i]) > widths[i]:
                widths[i] = len(r[i])
    out = [dashes(widths), table_row(head, widths), dashes(widths)]
    for r in rows:
        out.append(table_row(r, widths))
    out.append(dashes(widths))
    return out


def usage():
    print("usage: loglens stats FILE")
    print("       loglens top FILE FIELD N")
    print("       loglens errors FILE [N]")
    print("       loglens table FILE [N]")


def main():
    pos = sys.argv[1:]
    if len(pos) < 2:
        usage()
        return
    cmd = pos[0]
    path = pos[1]
    try:
        with open(path, "r") as f:
            text = f.read()
    except OSError:
        print(f"loglens: cannot read {path}")
        sys.exit(2)
    if cmd == "stats":
        cmd_stats(text, path)
    elif cmd == "top":
        if len(pos) < 4:
            usage()
            return
        cmd_top(text, path, pos[2], int(pos[3]))
    elif cmd == "errors":
        n = 10
        if len(pos) >= 3:
            n = int(pos[2])
        cmd_errors(text, path, n)
    elif cmd == "table":
        n = 10
        if len(pos) >= 3:
            n = int(pos[2])
        cmd_table(text, path, n)
    else:
        usage()


if __name__ == "__main__":
    main()
