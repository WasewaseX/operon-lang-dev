#!/usr/bin/env python3
# csvstat.py — the Python reference implementation of apps/csvstat/csvstat.op
#
# BYTE-IDENTITY CONTRACT: every output byte of this program must equal the
# Operon implementation's output on the same inputs (the bench asserts it
# via sha256 over stdout). The rules that make that hold:
#   - integers render via str(int) on both sides;
#   - render_ratio() mirrors csvstat.op's float op sequence op-for-op:
#     x = num/den (one IEEE-754 division of exact ints), sc = 10^nd by
#     repeated *10.0, r = floor(x*sc + 0.5) — bit-identical on both
#     engines — then integer-only split and zero-pad;
#   - no float repr()/str() of floats is ever printed;
#   - CSV parsing via the stdlib csv module (RFC4180 subset: quoted
#     separators, doubled quotes) — the fixture domain where Operon's
#     std/csv and Python agree exactly; the e2e asserts the agreement.

import csv
import math
import sys


def render_ratio(num, den, nd):
    # MIRRORED FLOAT SEQUENCE — keep op-for-op identical with csvstat.op
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


def is_digits(s):
    t = s
    if len(t) == 0:
        return False
    if t[0] == "-":
        t = t[1:]
    if len(t) == 0:
        return False
    for ch in t:
        if ch < "0" or ch > "9":
            return False
    return True


def col_kind(values):
    seen = 0
    for v in values:
        if v != "":
            if not is_digits(v):
                return "str"
            seen += 1
    if seen == 0:
        return "str"
    return "int"


def value_counts(values):
    m = {}
    for v in values:
        if v != "":
            m[v] = m.get(v, 0) + 1
    return m


def top_pairs(values, n):
    m = value_counts(values)
    items = [[k, c] for k, c in m.items()]
    # count desc, then value ascending — insertion-ordered Python dicts
    # keep first-occurrence order for equal (count, value); the sort key
    # is total so ordering is fully determined
    items.sort(key=lambda p: (-p[1], p[0]))
    return items[:n]


def numeric_stats(values):
    n = 0
    mn = mx = 0
    mn_s = mx_s = ""
    total = 0
    first = True
    for v in values:
        if v != "":
            x = int(v)
            if first:
                mn = mx = x
                mn_s = mx_s = v
                first = False
            else:
                if x < mn:
                    mn = x
                    mn_s = v
                if x > mx:
                    mx = x
                    mx_s = v
            total += x
            n += 1
    return [n, mn_s, mx_s, total]


def null_count(values):
    return sum(1 for v in values if v == "")


def dashes(widths):
    return "+" + "".join("-" * (w + 2) + "+" for w in widths)


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


def parse(path):
    with open(path, newline="") as f:
        return [row for row in csv.reader(f)]


def cmd_info(text_rows, path):
    rows = text_rows
    if len(rows) == 0:
        print(f"csvstat info {path}")
        print("rows,0")
        print("columns,0")
        return
    head = rows[0]
    ncols = len(head)
    cols = []
    for i in range(ncols):
        col = []
        for j in range(1, len(rows)):
            col.append(rows[j][i] if i < len(rows[j]) else "")
        cols.append(col)
    print(f"csvstat info {path}")
    print(f"rows,{len(rows) - 1}")
    print(f"columns,{ncols}")
    print("column,kind,nulls")
    for i in range(ncols):
        print(f"{head[i]},{col_kind(cols[i])},{null_count(cols[i])}")


def cmd_stats(text_rows, path):
    rows = text_rows
    if len(rows) == 0:
        print(f"csvstat stats {path}")
        print("rows,0")
        return
    head = rows[0]
    ncols = len(head)
    print(f"csvstat stats {path}")
    print(f"rows,{len(rows) - 1}")
    print("column,count,nulls,min,max,sum,mean")
    for i in range(ncols):
        col = []
        for j in range(1, len(rows)):
            col.append(rows[j][i] if i < len(rows[j]) else "")
        if col_kind(col) == "int":
            st = numeric_stats(col)
            n, total = st[0], st[3]
            print(f"{head[i]},{n},{null_count(col)},{st[1]},{st[2]},{total},{render_ratio(total, n, 2)}")
        else:
            m = value_counts(col)
            print(f"{head[i]},{len(col) - null_count(col)},{null_count(col)},distinct={len(m)}")


def cmd_top(text_rows, path, col, n):
    rows = text_rows
    if len(rows) == 0:
        print(f"csvstat top {path} {col} {n}")
        print("rows,0")
        return
    head = rows[0]
    idx = -1
    for i, h in enumerate(head):
        if h == col:
            idx = i
    if idx < 0:
        print(f"csvstat: no column named {col}")
        return
    values = [rows[j][idx] if idx < len(rows[j]) else "" for j in range(1, len(rows))]
    nrows = len(rows) - 1
    print(f"csvstat top {path} {col} {n}")
    print("rank,value,count,share")
    for rank, p in enumerate(top_pairs(values, n), 1):
        share = render_ratio(p[1] * 100, nrows, 2)
        print(f"{rank},{p[0]},{p[1]},{share}")


def cmd_table(text_rows, path, n):
    rows = text_rows
    print(f"csvstat table {path} {n}")
    if len(rows) == 0:
        print("(empty)")
        return
    head = rows[0]
    shown = rows[1:n + 1]
    for line in render_table(head, shown):
        print(line)


def usage():
    print("usage: csvstat info FILE")
    print("       csvstat stats FILE")
    print("       csvstat top FILE COL N")
    print("       csvstat table FILE [N]")


def main():
    pos = [a for a in sys.argv[1:] if not a.startswith("-")]
    if len(pos) < 2:
        usage()
        return
    cmd, path = pos[0], pos[1]
    try:
        rows = parse(path)
    except OSError:
        print(f"csvstat: cannot read {path}")
        sys.exit(2)
        return
    if cmd == "info":
        cmd_info(rows, path)
    elif cmd == "stats":
        cmd_stats(rows, path)
    elif cmd == "top":
        if len(pos) < 4:
            usage()
            return
        cmd_top(rows, path, pos[2], int(pos[3]))
    elif cmd == "table":
        n = int(pos[2]) if len(pos) >= 3 else 10
        cmd_table(rows, path, n)
    else:
        usage()


if __name__ == "__main__":
    main()
