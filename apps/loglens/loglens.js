#!/usr/bin/env node
// loglens.js — the Node.js baseline of apps/loglens/loglens.op
//
// BYTE-IDENTITY CONTRACT (see loglens.py header for the full rules):
// the parser is split-based and identical in all four engines; integers
// render via String(int); every printed order is TOTAL; render_ratio()
// mirrors the Operon float sequence op-for-op (IEEE-754 doubles on
// exact integer inputs => bit-identical, then integer-only arithmetic);
// no float is ever printed except through render_ratio().

function renderRatio(num, den, nd) {
  // MIRRORED FLOAT SEQUENCE — keep op-for-op identical with loglens.op
  const x = num / den;
  let sc = 1.0;
  for (let i = 0; i < nd; i++) {
    sc = sc * 10.0;
  }
  const r = Math.floor(x * sc + 0.5);
  const si = Math.floor(sc);
  const whole = Math.floor(r / si);
  const frac = r % si;
  if (nd === 0) return `${whole}`;
  let fd = `${frac}`;
  while (fd.length < nd) fd = "0" + fd;
  return `${whole}.${fd}`;
}

function isDigits(s) {
  if (s.length === 0) return false;
  for (const ch of s) {
    if (ch < "0" || ch > "9") return false;
  }
  return true;
}

function splitLines(text) {
  // split on \n, drop ONE trailing empty piece ("...\n" files)
  const parts = text.split("\n");
  if (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();
  return parts;
}

function parseLine(line) {
  const pieces = line.split('"');
  if (pieces.length !== 3) return null;
  const hostParts = pieces[0].split(" ");
  if (hostParts.length < 1 || hostParts[0] === "") return null;
  const host = hostParts[0];
  const req = pieces[1].split(" ");
  if (req.length !== 3) return null;
  const rest = pieces[2].split(" ");
  if (rest.length !== 3 || rest[0] !== "") return null;
  if (!isDigits(rest[1]) || !isDigits(rest[2])) return null;
  return [host, req[1], rest[1], parseInt(rest[2], 10)];
}

function parseRecords(text) {
  const out = [];
  for (const line of splitLines(text)) {
    const rec = parseLine(line);
    if (rec !== null) out.push(rec);
  }
  return out;
}

function countMap(values) {
  const m = new Map();
  for (const v of values) {
    m.set(v, (m.get(v) || 0) + 1);
  }
  return m;
}

function topPairs(m, n, numeric) {
  // count desc, then value asc (numeric value asc for status)
  const items = [...m.entries()].map(([k, c]) => [k, c]);
  items.sort((a, b) => {
    if (a[1] !== b[1]) return b[1] - a[1];
    if (numeric) {
      const av = parseInt(a[0], 10);
      const bv = parseInt(b[0], 10);
      return av - bv;
    }
    return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0;
  });
  return items.slice(0, n);
}

function sumBytes(recs) {
  let total = 0;
  for (const r of recs) total += r[3];
  return total;
}

function cmdStats(text, path) {
  const recs = parseRecords(text);
  const hosts = countMap(recs.map((r) => r[0]));
  const urls = countMap(recs.map((r) => r[1]));
  const status = countMap(recs.map((r) => r[2]));
  console.log(`loglens stats ${path}`);
  console.log(`records,${recs.length}`);
  console.log(`hosts,${hosts.size}`);
  console.log(`urls,${urls.size}`);
  console.log(`bytes,${sumBytes(recs)}`);
  console.log("status,count");
  const items = [...status.entries()].map(([k, c]) => [k, c]);
  items.sort((a, b) => parseInt(a[0], 10) - parseInt(b[0], 10));
  for (const [k, c] of items) console.log(`${k},${c}`);
}

function cmdTop(text, path, field, n) {
  if (field !== "host" && field !== "url" && field !== "status") {
    console.log(`loglens: unknown field ${field}`);
    process.exit(2);
  }
  const recs = parseRecords(text);
  const idx = field === "host" ? 0 : field === "url" ? 1 : 2;
  const m = countMap(recs.map((r) => r[idx]));
  console.log(`loglens top ${path} ${field} ${n}`);
  console.log("rank,value,count,share");
  const pairs = topPairs(m, n, field === "status");
  let rank = 1;
  for (const p of pairs) {
    const share = renderRatio(p[1] * 100, recs.length, 2);
    console.log(`${rank},${p[0]},${p[1]},${share}`);
    rank += 1;
  }
}

function cmdErrors(text, path, n) {
  const recs = parseRecords(text);
  const errs = recs.filter((r) => parseInt(r[2], 10) >= 400);
  const m = countMap(errs.map((r) => r[1]));
  console.log(`loglens errors ${path} ${n}`);
  console.log(`count,${errs.length}`);
  console.log(`bytes,${sumBytes(errs)}`);
  console.log("rank,url,count,share");
  const pairs = topPairs(m, n, false);
  let rank = 1;
  for (const p of pairs) {
    const share = renderRatio(p[1] * 100, errs.length, 2);
    console.log(`${rank},${p[0]},${p[1]},${share}`);
    rank += 1;
  }
}

function dashes(widths) {
  let out = "+";
  for (const w of widths) out += "-".repeat(w + 2) + "+";
  return out;
}

function tableRow(cells, widths) {
  let out = "|";
  for (let i = 0; i < cells.length; i++) {
    out += " " + cells[i] + " ".repeat(widths[i] - cells[i].length) + " |";
  }
  return out;
}

function renderTable(head, rows) {
  const widths = head.map((h) => h.length);
  for (const r of rows) {
    for (let i = 0; i < head.length; i++) {
      if (r[i].length > widths[i]) widths[i] = r[i].length;
    }
  }
  const out = [dashes(widths), tableRow(head, widths), dashes(widths)];
  for (const r of rows) out.push(tableRow(r, widths));
  out.push(dashes(widths));
  return out;
}

function cmdTable(text, path, n) {
  const recs = parseRecords(text);
  console.log(`loglens table ${path} ${n}`);
  const head = ["host", "url", "status", "bytes"];
  const shown = recs.slice(0, n).map((r) => [r[0], r[1], r[2], String(r[3])]);
  for (const l of renderTable(head, shown)) console.log(l);
}

function usage() {
  console.log("usage: loglens stats FILE");
  console.log("       loglens top FILE FIELD N");
  console.log("       loglens errors FILE [N]");
  console.log("       loglens table FILE [N]");
}

function main() {
  const pos = process.argv.slice(2);
  if (pos.length < 2) {
    usage();
    return;
  }
  const cmd = pos[0];
  const path = pos[1];
  let text;
  try {
    text = require("fs").readFileSync(path, "utf8");
  } catch (e) {
    console.log(`loglens: cannot read ${path}`);
    process.exit(2);
  }
  if (cmd === "stats") cmdStats(text, path);
  else if (cmd === "top") {
    if (pos.length < 4) {
      usage();
      return;
    }
    cmdTop(text, path, pos[2], parseInt(pos[3], 10));
  } else if (cmd === "errors") {
    const n = pos.length >= 3 ? parseInt(pos[2], 10) : 10;
    cmdErrors(text, path, n);
  } else if (cmd === "table") {
    const n = pos.length >= 3 ? parseInt(pos[2], 10) : 10;
    cmdTable(text, path, n);
  } else usage();
}

main();
