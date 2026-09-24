/* ==========================================================================
   Operon playground — browser subset interpreter (plain JS build)
   --------------------------------------------------------------------------
   Implements the SPEC.md v2.0 expression core with the 4-rung Total Grammar:
     rung 1 canonical · rung 2 synonyms · rung 3 wobble repair (edit distance)
     rung 4 semantic fallback (unbound wobble, stray skip, auto-close)
   plus a Total-Grammar runtime: soft failures degrade to null + note.

   This is the file the page loads. app.ts is its faithful typed twin — keep
   the two logically identical.
   ========================================================================== */
"use strict";

/* ------------------------- language tables (SPEC §3, §4) ------------------ */

const KEYWORDS = [
  "gene", "let", "if", "elif", "else", "while", "loop", "for", "in", "return",
  "break", "continue", "match", "case", "use", "tad", "anchor", "export",
  "import", "enhance", "silence", "stress", "rescue", "raise", "fate", "state",
  "regulate", "activates", "inhibits", "strength", "toggle", "repressilator",
  "period", "frame", "proof", "guard", "splice", "variant", "edit", "replace",
  "apply", "true", "false", "null", "and", "or", "not", "collect", "ires",
];
const KEYWORD_SET = new Set(KEYWORDS);
const MARKS = ["@acetylate", "@methylate", "@m6a"];

// rung 2: word synonyms → canonical keyword (print/echo/say/show map to the
// promote builtin; the parser resolves the name, so the target is "promote").
const SYNONYMS = new Map([
  ["fn", "gene"], ["func", "gene"], ["def", "gene"], ["fun", "gene"],
  ["sub", "gene"], ["lambda", "gene"], ["proc", "gene"],
  ["print", "promote"], ["echo", "promote"], ["say", "promote"], ["show", "promote"],
  ["var", "let"], ["val", "let"], ["const", "let"],
  ["elseif", "elif"],
  ["foreach", "for"], ["each", "for"],
  ["import", "use"], ["include", "use"], ["require", "use"],
  ["ret", "return"],
  ["stop", "break"],
  ["next", "continue"], ["skip", "continue"],
  ["yes", "true"], ["on", "true"],
  ["no", "false"], ["off", "false"],
  ["nil", "null"], ["none", "null"], ["nothing", "null"],
]);

const STMT_KEYWORDS = KEYWORDS.filter(
  (k) => !["true", "false", "null", "and", "or", "not", "in", "collect"].includes(k)
);
const EXPR_KEYWORDS = ["true", "false", "null", "and", "or", "not", "in", "gene", "collect", "if", "for"];

const BUILTINS = ["promote", "len", "str", "num", "type", "range", "push", "pop",
  "keys", "values", "abs", "min", "max", "sum", "clock", "distance", "codon", "exit"];

const SUPPORTED_STMT = new Set([
  "let", "if", "while", "loop", "for", "return", "break", "continue", "gene",
  "elif", "else", "case", // the last three are parsed contextually; reaching
  // the dispatcher with them means they are stray — handled with a note.
]);

const ASSIGN_OPS = new Set(["=", "+=", "-=", "*=", "//="]);
const STEP_MAX = 2000000;
const DEPTH_MAX = 10000;

/* ------------------------------ helpers ----------------------------------- */

// Pure-JS edit distance (used by wobble repair AND the `distance` builtin).
function levenshtein(a, b) {
  if (a === b) return 0;
  const m = a.length, n = b.length;
  if (m === 0) return n;
  if (n === 0) return m;
  let prev = new Array(n + 1), cur = new Array(n + 1);
  for (let j = 0; j <= n; j++) prev[j] = j;
  for (let i = 1; i <= m; i++) {
    cur[0] = i;
    for (let j = 1; j <= n; j++) {
      const sub = prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1);
      cur[j] = Math.min(sub, prev[j] + 1, cur[j - 1] + 1);
    }
    const t = prev; prev = cur; cur = t;
  }
  return prev[n];
}

// Floats are boxed so the int/float distinction of SPEC §2 survives JS numbers.
class Flt {
  constructor(n) { this.n = n; }
}
function numVal(v) { return v instanceof Flt ? v.n : v; }
function flt(n) { return new Flt(n); }

function note(ctx, rung, message, line) {
  ctx.notes.push({ line: line || 0, rung, message });
}
function step(ctx) {
  if (++ctx.steps > STEP_MAX) throw new Sg("abort", null);
}

// Control-flow signals (the only things that ever "throw" in this interpreter).
class Sg {
  constructor(kind, value) { this.kind = kind; this.value = value; }
}

/* ------------------------------ the lexer --------------------------------- */

// Token: {t:"word"|"num"|"str"|"op"|"mark"|"nl"|"eof", v, line, f?, parts?}
// Word tokens carry canonical spellings where a safe context allows conversion
// (synonyms are applied except directly after ".", which is a member position).
function lex(src, ctx, lineOffset) {
  const toks = [];
  let i = 0, line = lineOffset || 1;
  const N = src.length;
  const push = (t, v, extra) => toks.push(Object.assign({ t, v, line }, extra || {}));

  while (i < N) {
    const c = src[i];
    if (c === "\n") {
      if (toks.length && toks[toks.length - 1].t === "nl") { line++; i++; continue; }
      push("nl", "\\n"); line++; i++; continue;
    }
    if (c === " " || c === "\t" || c === "\r") { i++; continue; }
    if (c === "#") { while (i < N && src[i] !== "\n") i++; continue; } // `#!` included
    if (c === ";") { i++; continue; } // allowed and ignored (also `;;`)

    if (c === '"' || c === "'") {
      if (c === "'") note(ctx, 3, "single quote treated as double quote (wobble)", line);
      const s = readString(src, i, ctx, line, c);
      push("str", s.text, { parts: s.parts });
      i = s.next;
      continue;
    }

    if (c >= "0" && c <= "9") {
      const m = /^\d+(\.\d+)?([eE][+-]?\d+)?/.exec(src.slice(i));
      const text = m[0];
      const isFloat = /[.eE]/.test(text);
      push("num", parseFloat(text), { f: isFloat });
      i += text.length;
      continue;
    }

    if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i));
      const w = m[0];
      const prev = toks[toks.length - 1];
      const afterDot = prev && prev.t === "op" && prev.v === ".";
      i += w.length;
      if (!afterDot && SYNONYMS.has(w)) {
        const can = SYNONYMS.get(w);
        note(ctx, 2, "'" + w + "' → '" + can + "' (synonym)", line);
        push("word", can);
        continue;
      }
      push("word", w);
      continue;
    }

    if (c === "@") {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i + 1));
      if (m) { push("mark", m[0]); i += 1 + m[0].length; continue; }
      note(ctx, 4, "skipped stray '@'", line); i++; continue;
    }

    // operators, longest first
    const three = src.substr(i, 3), two = src.substr(i, 2);
    if (three === "//=") { push("op", "//="); i += 3; continue; }
    const TWO = ["==", "!=", "<=", ">=", "&&", "||", "=>", "->", "+=", "-=", "*=", "//"];
    if (TWO.indexOf(two) >= 0) {
      if (two === "&&") { note(ctx, 2, "'&&' → 'and' (synonym)", line); push("word", "and"); }
      else if (two === "||") { note(ctx, 2, "'||' → 'or' (synonym)", line); push("word", "or"); }
      else push("op", two);
      i += 2; continue;
    }
    if (c === "!") { note(ctx, 2, "'!' → 'not' (synonym)", line); push("word", "not"); i++; continue; }
    if ("+-*/%<>=()[]{},:.?".indexOf(c) >= 0) { push("op", c); i++; continue; }

    note(ctx, 4, "skipped stray character '" + c + "'", line);
    i++;
  }
  toks.push({ t: "eof", v: "<eof>", line });
  return toks;
}

// Reads a string starting at src[i] === quote. Handles \n \t \\ \" \{ escapes
// and {expr} interpolation (with nested braces and nested strings). A `"` that
// never closes consumes to EOF with a note; `'` (already a wobble) ends at EOL.
function readString(src, i, ctx, line, quote) {
  const parts = [];
  let buf = "", j = i + 1;
  const flush = () => { if (buf) { parts.push({ txt: buf }); buf = ""; } };
  const N = src.length;
  while (j < N) {
    const c = src[j];
    if (c === "\\") {
      const d = src[j + 1];
      if (d === "n") buf += "\n";
      else if (d === "t") buf += "\t";
      else if (d === "\\") buf += "\\";
      else if (d === quote) buf += quote;
      else if (d === "{") buf += "{";
      else buf += "\\" + (d === undefined ? "" : d);
      j += 2; continue;
    }
    if (c === quote) { j++; return { parts, text: textOf(parts, buf), next: j }; }
    if (c === "{") {
      flush();
      let k = j + 1, depth = 1, sub = "";
      while (k < N) {
        const s = src[k];
        if (s === "\\") { sub += s + (src[k + 1] || ""); k += 2; continue; }
        if (s === '"' || s === "'") {
          const q2 = s; k++;
          while (k < N && src[k] !== q2) { if (src[k] === "\\") k++; k++; }
          k++; continue;
        }
        if (s === "{") { depth++; sub += s; k++; continue; }
        if (s === "}") { depth--; if (depth === 0) { k++; break; } sub += s; k++; continue; }
        sub += s; k++;
      }
      if (depth !== 0) note(ctx, 4, "unterminated '{' in string — interpolation closed at string end", line);
      parts.push({ expr: sub.trim() });
      j = k; continue;
    }
    if (c === "\n" && quote === "'") break; // repair: ' ends at end of line
    buf += c; j++;
  }
  if (quote === '"') note(ctx, 4, "unterminated string — consumed to end of file", line);
  else note(ctx, 4, "unterminated string — closed at end of line", line);
  return { parts, text: textOf(parts, buf), next: j };
}
function textOf(parts, tail) {
  if (tail) parts.push({ txt: tail });
  if (!parts.some((p) => p.expr !== undefined)) {
    return parts.map((p) => p.txt || "").join("");
  }
  return "";
}

/* --------------------------- known-name pre-pass --------------------------- */
// Collects every name the file ever defines (lets, gene names + params, for
// binders, assignment targets) plus builtins. An identifier OUTSIDE this set is
// what rung 4 calls an "unbound wobble" — string literal of its own name.
function knownNames(toks) {
  const names = new Set(BUILTINS);
  const add = (w) => { if (w && /^[A-Za-z_][A-Za-z0-9_]*$/.test(w)) names.add(w); };
  const collectParams = (pi) => {
    let depth = 0;
    for (let k = pi; k < toks.length; k++) {
      const tk = toks[k];
      if (tk.t === "op" && tk.v === "(") depth++;
      else if (tk.t === "op" && tk.v === ")") { depth--; if (depth === 0) return; }
      else if (depth === 1 && tk.t === "word") add(tk.v);
    }
  };
  for (let j = 0; j < toks.length; j++) {
    const t = toks[j];
    if (t.t !== "word") continue;
    if ((t.v === "let" || t.v === "for") && toks[j + 1] && toks[j + 1].t === "word") add(toks[j + 1].v);
    if (t.v === "gene") {
      const n1 = toks[j + 1], n2 = toks[j + 2];
      if (n1 && n1.t === "word" && n2 && n2.t === "op" && n2.v === "(") { add(n1.v); collectParams(j + 2); }
      else if (n1 && n1.t === "op" && n1.v === "(") collectParams(j + 1);
    }
    const nt = toks[j + 1];
    if (nt && nt.t === "op" && ASSIGN_OPS.has(nt.v)) add(t.v);
  }
  return names;
}

/* ------------------------------- the parser -------------------------------- */

const LITERAL_NODES = new Set(["lit", "interp", "list", "map", "lambda"]);

class Parser {
  constructor(toks, ctx, names) {
    this.toks = toks; this.i = 0; this.ctx = ctx; this.names = names;
    this.noLitCall = 0; // >0 while parsing a lambda body: a `(` after a
    // literal does not become a call — it attaches to the lambda itself
    // (`gene (a) => a * 2 (21)` === call the lambda with 21 → 42).
  }
  peek(o) { return this.toks[Math.min(this.i + (o || 0), this.toks.length - 1)]; }
  next() { const t = this.toks[this.i]; if (t.t !== "eof") this.i++; return t; }
  atOp(v, o) { const t = this.peek(o); return t.t === "op" && t.v === v; }
  atWord(v, o) { const t = this.peek(o); return t.t === "word" && t.v === v; }
  line() { return this.peek().line; }
  note(rung, msg, line) { note(this.ctx, rung, msg, line || this.line()); }
  skipNl() { while (this.peek().t === "nl") this.next(); }

  repair(w, candidates) {
    const thr = w.length <= 4 ? 1 : 2;
    let best = null, bestD = thr + 1, ties = 0;
    for (const k of candidates) {
      const d = levenshtein(w, k);
      if (d <= thr) {
        if (d < bestD) { bestD = d; best = k; ties = 1; }
        else if (d === bestD) ties++;
      }
    }
    return ties === 1 ? best : null; // ambiguity → rung 4 for that token
  }

  /* ---- program & statements ---- */

  parseProgram() {
    const stmts = [];
    for (;;) {
      this.skipNl();
      if (this.peek().t === "eof") break;
      const st = this.parseStatement();
      if (st) stmts.push(st);
    }
    return stmts;
  }

  parseStatement() {
    const t = this.peek();
    if (t.t === "op" && t.v === "}") { this.next(); this.note(4, "skipped extra `}`"); return null; }
    if (t.t === "mark") { this.next(); this.note(4, "mark '@" + t.v + "' is outside the playground subset — ignored"); return null; }
    if (t.t === "op" && t.v === "{") {
      const n1 = this.peek(1), n2 = this.peek(2);
      const looksMap = n1.t === "op" && n1.v === "}" ||
        ((n1.t === "word" || n1.t === "str") && n2.t === "op" && n2.v === ":");
      if (looksMap) return this.parseExprStmt();
      this.next(); this.note(4, "skipped stray `}` opener `{`");
      return null;
    }
    if (t.t === "op") { this.next(); this.note(4, "skipped stray '" + t.v + "'"); return null; }
    if (t.t === "eof") return null;

    if (t.t === "word") {
      const w = t.v;
      if (SUPPORTED_STMT.has(w)) return this.parseKeywordStmt(w);
      if (KEYWORD_SET.has(w)) {
        this.next();
        this.note(4, "'" + w + "' is outside the playground subset — statement skipped");
        this.skipStatement();
        return null;
      }
      if (!this.names.has(w)) {
        const rep = this.repair(w, STMT_KEYWORDS);
        if (rep) { this.note(3, "repaired '" + w + "' → '" + rep + "' (wobble)"); return this.parseKeywordStmt(rep); }
      }
      return this.parseExprStmt();
    }
    // num/str in statement position: harmless expression statement
    return this.parseExprStmt();
  }

  // Consume tokens of an unsupported statement: to end of line, or through a
  // balanced brace group (match/tad/stress/fate/regulate/… degrade cleanly).
  skipStatement() {
    let depth = 0;
    for (;;) {
      const t = this.peek();
      if (t.t === "eof") return;
      if (t.t === "op" && t.v === "{") { depth++; this.next(); continue; }
      if (t.t === "op" && t.v === "}") {
        this.next();
        if (depth === 0) return;
        depth--;
        if (depth === 0) return;
        continue;
      }
      if (depth === 0 && t.t === "nl") { this.next(); return; }
      this.next();
    }
  }

  expectWord(what) {
    const t = this.peek();
    if (t.t === "word") { this.next(); return t.v; }
    this.note(4, "expected " + (what || "a name") + " — null used");
    return null;
  }

  parseKeywordStmt(w) {
    this.next(); // consume the (possibly repaired) keyword
    const line = this.line();
    if (w === "let") {
      const name = this.expectWord("a name after `let`");
      let expr = { k: "lit", v: null, line };
      if (this.atOp("=")) { this.next(); expr = this.parseExpr(); }
      else this.note(4, "`let` without '=' — null used");
      return name ? { k: "let", name, expr, line } : null;
    }
    if (w === "if") return this.parseIf();
    if (w === "while") {
      const cond = this.parseExpr();
      const body = this.parseBlock();
      return { k: "while", cond, body, line };
    }
    if (w === "loop") {
      const body = this.parseBlock();
      return { k: "loop", body, line };
    }
    if (w === "for") return this.parseFor();
    if (w === "return") {
      const t = this.peek();
      const bare = t.t === "nl" || t.t === "eof" || (t.t === "op" && t.v === "}");
      return { k: "ret", expr: bare ? { k: "lit", v: null, line } : this.parseExpr(), line };
    }
    if (w === "break") return { k: "break", line };
    if (w === "continue") return { k: "cont", line };
    if (w === "gene") return this.parseGeneStmt();
    // elif / else / case arriving here are strays without their opener
    this.note(4, "stray '" + w + "' — skipped");
    this.skipStatement();
    return null;
  }

  parseIf() {
    const line = this.line();
    const clauses = [];
    const cond = this.parseExpr();
    const body = this.parseBlock();
    clauses.push({ cond, body });
    let elseBody = null;
    for (;;) {
      const save = this.i;
      this.skipNl();
      if (this.atWord("elif")) { this.next(); const c2 = this.parseExpr(); const b2 = this.parseBlock(); clauses.push({ cond: c2, body: b2 }); continue; }
      if (this.atWord("else")) { this.next(); this.skipNl(); elseBody = this.parseBlock(); break; }
      this.i = save;
      break;
    }
    return { k: "if", clauses, elseBody, line };
  }

  parseFor() {
    const line = this.line();
    let name = "_";
    if (this.peek().t === "word") name = this.next().v;
    else this.note(4, "expected loop variable name — '_' used");
    if (this.atWord("in")) this.next();
    else this.note(4, "expected `in` in for loop — assumed");
    const iter = this.parseExpr();
    const body = this.parseBlock();
    return { k: "for", name, iter, body, line };
  }

  parseGeneStmt() {
    const line = this.line();
    const t = this.peek(), t1 = this.peek(1);
    if (t.t === "word" && t1.t === "op" && t1.v === "(") {
      const name = this.next().v;
      const params = this.parseParams();
      this.skipGuardIfAny();
      const body = this.parseBlock();
      return { k: "genedef", fn: { k: "lambda", name, params, body, isExpr: false, line }, line };
    }
    // anonymous lambda in statement position — parse as an expression
    const fn = this.parseLambdaRest(null, line);
    return { k: "expr", expr: fn, line };
  }

  parseParams() {
    const params = [];
    if (!this.atOp("(")) { this.note(4, "expected '(' after gene name"); return params; }
    this.next();
    for (;;) {
      this.skipNl();
      const t = this.peek();
      if (t.t === "op" && t.v === ")") { this.next(); break; }
      if (t.t === "eof") { this.note(4, "auto-closed parameter list at end of file"); break; }
      if (t.t === "word") {
        this.next();
        const pname = t.v;
        let def = null;
        if (this.atOp("=")) { this.next(); def = this.parseExpr(); }
        params.push({ name: pname, def });
        this.skipNl();
        if (this.atOp(",")) { this.next(); continue; }
        if (this.atOp(")")) { this.next(); break; }
        if (this.peek().t === "eof") { this.note(4, "auto-closed parameter list at end of file"); break; }
        this.note(4, "skipped stray token in parameter list");
        this.next();
        continue;
      }
      this.note(4, "skipped stray '" + t.v + "' in parameter list");
      this.next();
    }
    return params;
  }

  // uORF guard: canonical in the full language, skipped honestly here.
  skipGuardIfAny() {
    if (!this.atWord("guard")) return;
    this.next();
    this.note(4, "guard clause is outside the playground subset — clause skipped");
    if (this.atOp("(")) {
      let depth = 0;
      for (;;) {
        const t = this.peek();
        if (t.t === "eof") break;
        this.next();
        if (t.t === "op" && t.v === "(") depth++;
        else if (t.t === "op" && t.v === ")") { depth--; if (depth === 0) break; }
      }
    }
    this.skipNl();
    if (this.atWord("else")) { this.next(); this.skipNl(); if (this.atOp("{")) this.parseBlock(); }
  }

  parseBlock() {
    this.skipNl();
    const stmts = [];
    if (!this.atOp("{")) {
      this.note(4, "expected '{' — using the rest of the line as the body");
      while (this.peek().t !== "nl" && this.peek().t !== "eof") {
        const st = this.parseStatement();
        if (st) stmts.push(st);
        if (this.peek().t === "nl" || this.peek().t === "eof") break;
      }
      return stmts;
    }
    this.next();
    for (;;) {
      this.skipNl();
      const t = this.peek();
      if (t.t === "op" && t.v === "}") { this.next(); break; }
      if (t.t === "eof") { this.note(4, "auto-closed block brace(s) at end of file"); break; }
      const st = this.parseStatement();
      if (st) stmts.push(st);
    }
    return stmts;
  }

  /* ---- expressions ---- */

  parseExpr() { return this.parseOr(); }

  binLoop(sub, ops, kw) {
    let l = sub.call(this);
    for (;;) {
      const t = this.peek();
      let op = null;
      if (t.t === "op" && ops.indexOf(t.v) >= 0) op = t.v;
      else if (kw && t.t === "word" && t.v === kw) op = kw;
      else if (kw && t.t === "word" && !this.names.has(t.v) && t.v !== kw) {
        const rep = this.repair(t.v, [kw]);
        if (rep) { this.note(3, "repaired '" + t.v + "' → '" + kw + "' (wobble)"); op = kw; }
      }
      if (!op) return l;
      this.next();
      const r = sub.call(this);
      l = { k: "bin", op, a: l, b: r, line: t.line };
    }
  }

  parseOr() { return this.binLoop(this.parseAnd, ["or"], "or"); }
  parseAnd() { return this.binLoop(this.parseNot, ["and"], "and"); }

  parseNot() {
    const t = this.peek();
    if (t.t === "word" && t.v === "not") { this.next(); return { k: "un", op: "not", a: this.parseNot(), line: t.line }; }
    if (t.t === "word" && !this.names.has(t.v) && t.v !== "not") {
      const rep = this.repair(t.v, ["not"]);
      if (rep) { this.note(3, "repaired '" + t.v + "' → 'not' (wobble)"); this.next(); return { k: "un", op: "not", a: this.parseNot(), line: t.line }; }
    }
    return this.parseCmp();
  }

  parseCmp() {
    let l = this.parseAdd();
    for (;;) {
      const t = this.peek();
      let op = null;
      if (t.t === "op" && ["==", "!=", "<", "<=", ">", ">="].indexOf(t.v) >= 0) op = t.v;
      else if (t.t === "word" && t.v === "in") op = "in";
      if (!op) return l;
      this.next();
      const r = this.parseAdd();
      l = { k: "bin", op, a: l, b: r, line: t.line };
    }
  }
  parseAdd() { return this.binLoop(this.parseMul, ["+", "-"], null); }
  parseMul() { return this.binLoop(this.parseUnary, ["*", "/", "//", "%"], null); }

  parseUnary() {
    const t = this.peek();
    if (t.t === "op" && t.v === "-") { this.next(); return { k: "un", op: "neg", a: this.parseUnary(), line: t.line }; }
    if (t.t === "op" && t.v === "+") { this.next(); return this.parseUnary(); }
    return this.parsePostfix();
  }

  parsePostfix() {
    let e = this.parsePrimary();
    for (;;) {
      const t = this.peek();
      if (t.t === "op" && t.v === "(") {
        if (this.noLitCall > 0 && LITERAL_NODES.has(e.k)) break;
        e = { k: "call", fn: e, args: this.parseArgs(), line: t.line };
        continue;
      }
      if (t.t === "op" && t.v === "[") {
        this.next();
        const idx = this.parseExpr();
        this.skipNl();
        if (this.atOp("]")) this.next();
        else if (this.peek().t === "eof") this.note(4, "auto-closed ']' at end of file");
        else this.note(4, "auto-closed ']' before '" + this.peek().v + "'");
        e = { k: "index", obj: e, idx, line: t.line };
        continue;
      }
      if (t.t === "op" && t.v === ".") {
        this.next();
        const nt = this.peek();
        if (nt.t === "word") {
          this.next();
          if (this.atOp("(")) e = { k: "method", obj: e, name: nt.v, args: this.parseArgs(), line: nt.line };
          else e = { k: "member", obj: e, name: nt.v, line: nt.line };
          continue;
        }
        this.note(4, "skipped stray token after '.'");
        this.next();
        continue;
      }
      return e;
    }
    return e;
  }

  parseArgs() {
    const args = [];
    this.next(); // "("
    this.skipNl();
    if (this.atOp(")")) { this.next(); return args; }
    for (;;) {
      args.push(this.parseExpr());
      this.skipNl();
      if (this.atOp(",")) { this.next(); this.skipNl(); continue; }
      if (this.atOp(")")) { this.next(); break; }
      if (this.peek().t === "eof") { this.note(4, "auto-closed call parens at end of file"); break; }
      this.note(4, "auto-closed call parens before '" + this.peek().v + "'");
      break;
    }
    return args;
  }

  parsePrimary() {
    const t = this.peek();
    if (t.t === "num") { this.next(); return { k: "lit", v: t.f ? flt(t.v) : t.v, line: t.line }; }
    if (t.t === "str") {
      this.next();
      if (t.parts && t.parts.some((p) => p.expr !== undefined)) return this.interpNode(t.parts, t.line);
      return { k: "lit", v: t.v, line: t.line };
    }
    if (t.t === "mark") { this.next(); this.note(4, "mark '@" + t.v + "' outside the playground subset — ignored"); return { k: "lit", v: null, line: t.line }; }
    if (t.t === "op") {
      if (t.v === "(") {
        this.next();
        const e = this.parseExpr();
        this.skipNl();
        if (this.atOp(")")) this.next();
        else if (this.peek().t === "eof") this.note(4, "auto-closed ')' at end of file");
        else this.note(4, "auto-closed ')' before '" + this.peek().v + "'");
        return e;
      }
      if (t.v === "[") return this.parseList();
      if (t.v === "{") return this.parseMap();
      this.next();
      this.note(4, "skipped stray '" + t.v + "' in expression");
      return { k: "lit", v: null, line: t.line };
    }
    if (t.t === "word") {
      const w = t.v;
      if (w === "true" || w === "false" || w === "null") { this.next(); return { k: "lit", v: w === "true" ? true : w === "false" ? false : null, line: t.line }; }
      if (w === "gene") { this.next(); return this.parseLambdaRest(null, t.line); }
      if (KEYWORD_SET.has(w)) {
        this.next();
        this.note(4, "'" + w + "' is outside the playground subset — skipped");
        return { k: "lit", v: null, line: t.line };
      }
      if (this.names.has(w)) { this.next(); return { k: "var", name: w, line: t.line }; }
      const rep = this.repair(w, EXPR_KEYWORDS);
      if (rep) {
        this.note(3, "repaired '" + w + "' → '" + rep + "' (wobble)");
        this.next();
        if (rep === "true" || rep === "false" || rep === "null") return { k: "lit", v: rep === "true" ? true : rep === "false" ? false : null, line: t.line };
        if (rep === "gene") return this.parseLambdaRest(null, t.line);
        if (rep === "not") return { k: "un", op: "not", a: this.parseNot(), line: t.line };
        return { k: "lit", v: null, line: t.line };
      }
      this.next();
      // frozen spec §4: unknown identifiers stay variable references; the
      // evaluator's unbound-variable path yields null with a note (no crash)
      return { k: "var", name: w, line: t.line };
    }
    this.next();
    this.note(4, "skipped stray '" + t.v + "' in expression");
    return { k: "lit", v: null, line: t.line };
  }

  parseList() {
    const line = this.line();
    this.next(); // "["
    const items = [];
    this.skipNl();
    if (this.atOp("]")) { this.next(); return { k: "list", items, line }; }
    for (;;) {
      items.push(this.parseExpr());
      this.skipNl();
      if (this.atOp(",")) { this.next(); this.skipNl(); if (this.atOp("]")) { this.next(); break; } continue; }
      if (this.atOp("]")) { this.next(); break; }
      if (this.peek().t === "eof") { this.note(4, "auto-closed list ']' at end of file"); break; }
      this.note(4, "auto-closed list ']' before '" + this.peek().v + "'");
      break;
    }
    return { k: "list", items, line };
  }

  parseMap() {
    const line = this.line();
    this.next(); // "{"
    const entries = [];
    this.skipNl();
    if (this.atOp("}")) { this.next(); return { k: "map", entries, line }; }
    for (;;) {
      const kt = this.peek();
      let key = null;
      if (kt.t === "word") { key = kt.v; this.next(); }
      else if (kt.t === "str") { key = kt.v; this.next(); }
      else { this.note(4, "expected map key — skipped"); this.next(); }
      this.skipNl();
      let val = { k: "lit", v: null, line };
      if (this.atOp(":")) { this.next(); val = this.parseExpr(); }
      else this.note(4, "expected ':' after map key — null used");
      if (key !== null) entries.push([key, val]);
      this.skipNl();
      if (this.atOp(",")) { this.next(); this.skipNl(); if (this.atOp("}")) { this.next(); break; } continue; }
      if (this.atOp("}")) { this.next(); break; }
      if (this.peek().t === "eof") { this.note(4, "auto-closed map '}' at end of file"); break; }
      this.note(4, "auto-closed map '}' before '" + this.peek().v + "'");
      break;
    }
    return { k: "map", entries, line };
  }

  parseLambdaRest(name, line) {
    this.skipNl();
    let params = [];
    if (this.atOp("(")) params = this.parseParams();
    else this.note(4, "expected '(' after gene — assumed empty parameter list");
    this.skipNl();
    if (this.atOp("=>")) {
      this.next();
      this.noLitCall++;
      const body = this.parseExpr();
      this.noLitCall--;
      return { k: "lambda", name, params, body, isExpr: true, line };
    }
    if (this.atOp("{")) {
      const body = this.parseBlock();
      return { k: "lambda", name, params, body, isExpr: false, line };
    }
    this.note(4, "lambda body missing — returns null");
    return { k: "lambda", name, params, body: { k: "lit", v: null, line }, isExpr: true, line };
  }

  interpNode(parts, line) {
    const out = [];
    for (const p of parts) {
      if (p.expr === undefined) { out.push({ txt: p.txt || "" }); continue; }
      if (!p.expr) { out.push({ txt: "{" }); continue; }
      try {
        const sub = new Parser(lex(p.expr, this.ctx, line), this.ctx, this.names);
        const ast = sub.parseExpr();
        if (sub.peek().t !== "eof") sub.note(4, "trailing tokens in interpolation — ignored");
        out.push({ ast, line });
      } catch (e) {
        note(this.ctx, 4, "bad interpolation '{" + p.expr + "}' — kept literal", line);
        out.push({ txt: "{" + p.expr + "}" });
      }
    }
    return { k: "interp", parts: out, line };
  }

  parseExprStmt() {
    const line = this.line();
    const e = this.parseExpr();
    const t = this.peek();
    if (t.t === "op" && ASSIGN_OPS.has(t.v)) {
      this.next();
      const rhs = this.parseExpr();
      if (e.k === "var" || e.k === "index" || e.k === "member") {
        return { k: "assign", target: e, op: t.v, expr: rhs, line };
      }
      this.note(4, "invalid assignment target — statement skipped");
      return null;
    }
    return { k: "expr", expr: e, line };
  }
}

/* ------------------------------ the evaluator ------------------------------ */

class Env {
  constructor(parent) { this.vars = new Map(); this.parent = parent; }
  lookup(name) {
    let e = this;
    while (e) { if (e.vars.has(name)) return { env: e, val: e.vars.get(name) }; e = e.parent; }
    return null;
  }
  root() { let e = this; while (e.parent) e = e.parent; return e; }
}

function truthy(v) {
  if (v === null || v === undefined || v === false) return false;
  if (v === true) return true;
  if (v instanceof Flt) return v.n !== 0;
  if (typeof v === "number") return v !== 0;
  if (typeof v === "string") return v.length > 0;
  if (Array.isArray(v)) return v.length > 0;
  if (v instanceof Map) return v.size > 0;
  return true; // genes / natives
}

function typeOf(v) {
  if (v === null || v === undefined) return "null";
  if (typeof v === "boolean") return "bool";
  if (v instanceof Flt) return "float";
  if (typeof v === "number") return "int";
  if (typeof v === "string") return "str";
  if (Array.isArray(v)) return "list";
  if (v instanceof Map) return "map";
  if (v && v.__gene) return "gene";
  return "native";
}

function deepEq(a, b) {
  if (a === b) return true;
  if ((a instanceof Flt || typeof a === "number") && (b instanceof Flt || typeof b === "number")) {
    return numVal(a) === numVal(b); // Int/Float compare numerically (SPEC §2)
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEq(a[i], b[i])) return false;
    return true;
  }
  if (a instanceof Map && b instanceof Map) {
    if (a.size !== b.size) return false;
    for (const [k, v] of a) { if (!b.has(k) || !deepEq(v, b.get(k))) return false; }
    return true;
  }
  return false;
}

function formatFloat(n) {
  if (Number.isInteger(n)) return n.toFixed(1); // 6/3 → 2.0
  return String(n);
}

function str(v) {
  if (v === null || v === undefined) return "null";
  if (v === true) return "true";
  if (v === false) return "false";
  if (v instanceof Flt) return formatFloat(v.n);
  if (typeof v === "number") return String(v);
  if (typeof v === "string") return v;
  if (Array.isArray(v)) return "[" + v.map(str).join(", ") + "]";
  if (v instanceof Map) return "{" + Array.from(v.entries()).map((kv) => kv[0] + ": " + str(kv[1])).join(", ") + "}";
  if (v && v.__gene) return "<gene " + (v.name || "<anonymous>") + ">";
  return "<native " + (v.name || "fn") + ">";
}

function numCoerce(ctx, v, line) {
  if (typeof v === "number") return v;
  if (v instanceof Flt) return v;
  if (v === true) return 1;
  if (v === false) return 0;
  if (typeof v === "string") {
    const s = v.trim();
    if (/^-?\d+$/.test(s)) return parseInt(s, 10);
    const f = Number(s);
    if (s !== "" && isFinite(f)) return flt(f);
    note(ctx, 0, "num() failed — 0 used", line);
    return 0;
  }
  note(ctx, 0, "num() failed — 0 used", line);
  return 0;
}

function arith(ctx, op, a, b, line) {
  const na = a instanceof Flt || typeof a === "number";
  const nb = b instanceof Flt || typeof b === "number";
  if (!na || !nb) {
    if (op === "+" && typeof a === "string" && typeof b === "string") return a + b;
    if (op === "+" && Array.isArray(a) && Array.isArray(b)) return a.concat(b);
    note(ctx, 0, "unfolded: '" + op + "' on " + typeOf(a) + " and " + typeOf(b) + " — null returned", line);
    return null;
  }
  const x = numVal(a), y = numVal(b);
  const f = a instanceof Flt || b instanceof Flt;
  const wrap = (n) => (f ? flt(n) : n);
  const overflow = (n) => {
    if (!f && !Number.isSafeInteger(n)) {
      note(ctx, 0, "overflow: integer out of range — null returned", line);
      return true;
    }
    return false;
  };
  switch (op) {
    case "+": { const r = x + y; return overflow(r) ? null : wrap(r); }
    case "-": { const r = x - y; return overflow(r) ? null : wrap(r); }
    case "*": { const r = x * y; return overflow(r) ? null : wrap(r); }
    case "/":
      if (y === 0) { note(ctx, 0, "unfolded: division by zero — null returned", line); return null; }
      return flt(x / y); // true division, always Float
    case "//":
      if (y === 0) { note(ctx, 0, "unfolded: division by zero — null returned", line); return null; }
      return Math.floor(x / y); // floor division, Int
    case "%":
      if (y === 0) { note(ctx, 0, "unfolded: division by zero — null returned", line); return null; }
      return wrap(x - Math.floor(x / y) * y); // floored remainder, sign of divisor
  }
  note(ctx, 0, "unfolded: unknown operator '" + op + "'", line);
  return null;
}

function compare(ctx, op, a, b, line) {
  if (op === "==") return deepEq(a, b);
  if (op === "!=") return !deepEq(a, b);
  const na = a instanceof Flt || typeof a === "number";
  const nb = b instanceof Flt || typeof b === "number";
  const ok = (na && nb) || (typeof a === "string" && typeof b === "string");
  if (!ok) {
    note(ctx, 0, "unfolded: cannot order " + typeOf(a) + " and " + typeOf(b) + " — null returned", line);
    return null;
  }
  const x = numVal(a), y = numVal(b);
  switch (op) {
    case "<": return x < y;
    case "<=": return x <= y;
    case ">": return x > y;
    case ">=": return x >= y;
    case "in": return null;
  }
  return null;
}

function evalExpr(nd, env, ctx) {
  step(ctx);
  switch (nd.k) {
    case "lit": return nd.v;
    case "interp": {
      let s = "";
      for (const p of nd.parts) s += p.txt !== undefined ? p.txt : str(evalExpr(p.ast, env, ctx));
      return s;
    }
    case "var": {
      const hit = env.lookup(nd.name);
      if (!hit) { note(ctx, 0, "unbound variable '" + nd.name + "' — null returned", nd.line); return null; }
      return hit.val;
    }
    case "list": return nd.items.map((it) => evalExpr(it, env, ctx));
    case "map": {
      const m = new Map();
      for (const [k, ve] of nd.entries) m.set(k, evalExpr(ve, env, ctx));
      return m;
    }
    case "lambda": return { __gene: true, name: nd.name || null, params: nd.params, body: nd.body, isExpr: nd.isExpr, env };
    case "genedef": {
      const fnv = { __gene: true, name: nd.fn.name, params: nd.fn.params, body: nd.fn.body, isExpr: nd.fn.isExpr, env };
      if (env.vars.has(nd.fn.name)) note(ctx, 0, "rebinding '" + nd.fn.name + "'", nd.line);
      env.vars.set(nd.fn.name, fnv);
      return fnv;
    }
    case "un": {
      if (nd.op === "not") return !truthy(evalExpr(nd.a, env, ctx));
      const v = evalExpr(nd.a, env, ctx);
      if (!(v instanceof Flt || typeof v === "number")) {
        note(ctx, 0, "unfolded: unary '-' on " + typeOf(v) + " — null returned", nd.line);
        return null;
      }
      return v instanceof Flt ? flt(-v.n) : -v;
    }
    case "bin": {
      if (nd.op === "or") { const a = evalExpr(nd.a, env, ctx); return truthy(a) ? a : evalExpr(nd.b, env, ctx); }
      if (nd.op === "and") { const a = evalExpr(nd.a, env, ctx); return truthy(a) ? evalExpr(nd.b, env, ctx) : a; }
      if (nd.op === "in") {
        const item = evalExpr(nd.a, env, ctx);
        const box = evalExpr(nd.b, env, ctx);
        if (Array.isArray(box)) return box.some((x) => deepEq(x, item));
        if (typeof box === "string") return box.indexOf(typeof item === "string" ? item : str(item)) >= 0;
        if (box instanceof Map) return box.has(typeof item === "string" ? item : str(item));
        note(ctx, 0, "unfolded: 'in' over " + typeOf(box) + " — null returned", nd.line);
        return null;
      }
      const a = evalExpr(nd.a, env, ctx);
      const b = evalExpr(nd.b, env, ctx);
      if (["==", "!=", "<", "<=", ">", ">="].indexOf(nd.op) >= 0) return compare(ctx, nd.op, a, b, nd.line);
      return arith(ctx, nd.op, a, b, nd.line);
    }
    case "index": {
      const obj = evalExpr(nd.obj, env, ctx);
      const idx = evalExpr(nd.idx, env, ctx);
      if (Array.isArray(obj)) {
        if (typeof idx !== "number") { note(ctx, 0, "unfolded: list index must be int — null returned", nd.line); return null; }
        const i = idx < 0 ? obj.length + idx : idx;
        if (i < 0 || i >= obj.length) { note(ctx, 0, "missing: list index " + idx + " out of range — null returned", nd.line); return null; }
        return obj[i];
      }
      if (typeof obj === "string") {
        if (typeof idx !== "number") { note(ctx, 0, "unfolded: string index must be int — null returned", nd.line); return null; }
        const i = idx < 0 ? obj.length + idx : idx;
        if (i < 0 || i >= obj.length) { note(ctx, 0, "missing: string index out of range — null returned", nd.line); return null; }
        return obj[i];
      }
      if (obj instanceof Map) {
        const k = typeof idx === "string" ? idx : str(idx);
        if (!obj.has(k)) { note(ctx, 0, "missing key '" + k + "' — null returned", nd.line); return null; }
        return obj.get(k);
      }
      note(ctx, 0, "missing: cannot index " + typeOf(obj) + " — null returned", nd.line);
      return null;
    }
    case "member": {
      const obj = evalExpr(nd.obj, env, ctx);
      return memberGet(ctx, obj, nd.name, nd.line);
    }
    case "method": {
      const obj = evalExpr(nd.obj, env, ctx);
      const args = nd.args.map((a) => evalExpr(a, env, ctx));
      return callMethod(ctx, obj, nd.name, args, nd.line);
    }
    case "call": {
      const fnv = evalExpr(nd.fn, env, ctx);
      const args = nd.args.map((a) => evalExpr(a, env, ctx));
      return callValue(ctx, fnv, args, nd.line);
    }
  }
  note(ctx, 0, "internal: unknown node '" + nd.k + "'", nd.line || 0);
  return null;
}

function memberGet(ctx, obj, name, line) {
  if (obj instanceof Map) {
    if (obj.has(name)) return obj.get(name);
    note(ctx, 0, "missing key '" + name + "' — null returned", line);
    return null;
  }
  const table = typeof obj === "string" ? STR_METHODS : Array.isArray(obj) ? LIST_METHODS : obj instanceof Map ? MAP_METHODS : null;
  if (table && table.indexOf(name) >= 0) {
    return { __native: true, name: "method " + name, fn: (args) => callMethod(ctx, obj, name, args, line) };
  }
  note(ctx, 0, "missing member '" + name + "' on " + typeOf(obj) + " — null returned", line);
  return null;
}

const STR_METHODS = ["upper", "lower", "trim", "split", "replace", "contains", "starts", "ends", "repeat", "slice", "join", "len"];
const LIST_METHODS = ["map", "filter", "reduce", "each", "sort", "reverse", "contains", "index_of", "slice", "join", "len"];
const MAP_METHODS = ["keys", "values", "items", "has", "del", "len"];

function needStr(ctx, v, what, line) {
  if (typeof v === "string") return v;
  note(ctx, 0, "unfolded: " + what + " must be str, got " + typeOf(v) + " — null returned", line);
  return null;
}

function callMethod(ctx, obj, name, args, line) {
  step(ctx);
  const A = (i) => (args.length > i ? args[i] : null);
  if (typeof obj === "string") {
    switch (name) {
      case "upper": return obj.toUpperCase();
      case "lower": return obj.toLowerCase();
      case "trim": return obj.trim();
      case "split": {
        const sep = needStr(ctx, A(0), "split separator", line);
        if (sep === null) return null;
        return obj.split(sep);
      }
      case "replace": {
        const a = needStr(ctx, A(0), "replace target", line);
        const b = needStr(ctx, A(1), "replace source", line);
        if (a === null || b === null) return null;
        return obj.split(a).join(b); // all occurrences
      }
      case "contains": return obj.indexOf(str(A(0))) >= 0;
      case "starts": return obj.slice(0, str(A(0)).length) === str(A(0));
      case "ends": return obj.slice(-str(A(0)).length) === str(A(0)) && str(A(0)).length > 0 || str(A(0)).length === 0;
      case "repeat": {
        const n = A(0);
        if (typeof n !== "number" || n < 0) { note(ctx, 0, "unfolded: repeat needs a non-negative int — '' returned", line); return ""; }
        return obj.repeat(Math.floor(n));
      }
      case "slice": return sliceOf(obj, A(0), A(1), ctx, line);
      case "join": {
        const items = A(0);
        if (!Array.isArray(items)) { note(ctx, 0, "unfolded: str.join needs a list — null returned", line); return null; }
        return items.map(str).join(obj);
      }
      case "len": return obj.length;
    }
  }
  if (Array.isArray(obj)) {
    switch (name) {
      case "map": return obj.map((x) => callValue(ctx, A(0), [x], line));
      case "filter": return obj.filter((x) => truthy(callValue(ctx, A(0), [x], line)));
      case "reduce": {
        if (args.length < 2) { note(ctx, 0, "unfolded: reduce needs (f, init) — null returned", line); return null; }
        let acc = A(1);
        for (const x of obj) acc = callValue(ctx, A(0), [acc, x], line);
        return acc;
      }
      case "each": { for (const x of obj) callValue(ctx, A(0), [x], line); return null; }
      case "sort": {
        const copy = obj.slice();
        if (args.length >= 1) {
          const cmp = A(0);
          copy.sort((a, b) => (truthy(callValue(ctx, cmp, [a, b], line)) ? -1 : truthy(callValue(ctx, cmp, [b, a], line)) ? 1 : 0));
        } else {
          copy.sort((a, b) => {
            const na = a instanceof Flt || typeof a === "number", nb = b instanceof Flt || typeof b === "number";
            if (na && nb) return numVal(a) - numVal(b);
            if (typeof a === "string" && typeof b === "string") return a < b ? -1 : a > b ? 1 : 0;
            return str(a) < str(b) ? -1 : str(a) > str(b) ? 1 : 0;
          });
        }
        return copy;
      }
      case "reverse": return obj.slice().reverse();
      case "contains": return obj.some((x) => deepEq(x, A(0)));
      case "index_of": {
        for (let i = 0; i < obj.length; i++) if (deepEq(obj[i], A(0))) return i;
        return -1;
      }
      case "slice": return sliceOf(obj, A(0), A(1), ctx, line);
      case "join": return obj.map(str).join(str(A(0)));
      case "len": return obj.length;
    }
  }
  if (obj instanceof Map) {
    switch (name) {
      case "keys": return Array.from(obj.keys());
      case "values": return Array.from(obj.values());
      case "items": return Array.from(obj.entries()).map((kv) => [kv[0], kv[1]]);
      case "has": return obj.has(str(A(0)));
      case "del": { obj.delete(str(A(0))); return null; }
      case "len": return obj.size;
    }
  }
  note(ctx, 0, "unfolded: method '" + name + "' not available on " + typeOf(obj) + " — null returned", line);
  return null;
}

// Python-style slice with negatives, clamped.
function sliceOf(seq, a, b, ctx, line) {
  const n = seq.length;
  const norm = (x, dflt) => {
    if (typeof x !== "number") return dflt;
    const i = Math.floor(x < 0 ? n + x : x);
    return Math.max(0, Math.min(n, i));
  };
  const s = norm(a, 0), e = norm(b, n);
  return s <= e ? seq.slice(s, e) : seq.slice(s, s);
}

function callValue(ctx, fnv, args, line) {
  step(ctx);
  if (fnv && fnv.__native) return fnv.fn(args, ctx, line);
  if (fnv && fnv.__gene) {
    ctx.depth++;
    if (ctx.depth > DEPTH_MAX) { ctx.depth--; note(ctx, 0, "overflow: recursion depth limit reached — null returned", line); return null; }
    try {
      const env = new Env(fnv.env);
      const ps = fnv.params;
      for (let i = 0; i < ps.length; i++) {
        if (i < args.length) env.vars.set(ps[i].name, args[i]);
        else if (ps[i].def) env.vars.set(ps[i].name, evalExpr(ps[i].def, env, ctx));
        else { env.vars.set(ps[i].name, null); note(ctx, 0, "missing argument '" + ps[i].name + "' — null used", line); }
      }
      if (args.length > ps.length) note(ctx, 0, "extra arguments ignored (" + (args.length - ps.length) + ")", line);
      if (fnv.isExpr) return evalExpr(fnv.body, env, ctx);
      try {
        execBlock(fnv.body, env, ctx);
        return null;
      } catch (e) {
        if (e instanceof Sg && e.kind === "return") return e.value;
        if (e instanceof Sg && (e.kind === "break" || e.kind === "continue")) {
          note(ctx, 0, "'" + e.kind + "' outside loop — ignored", line);
          return null;
        }
        throw e;
      }
    } finally {
      ctx.depth--;
    }
  }
  if (fnv === null || fnv === undefined) note(ctx, 0, "phantom call: calling null — null returned", line);
  else note(ctx, 0, "calling a non-gene (" + typeOf(fnv) + ") — null returned", line);
  return null;
}

function execBlock(stmts, env, ctx) {
  for (const st of stmts) {
    step(ctx);
    execStmt(st, env, ctx);
  }
}

function execStmt(st, env, ctx) {
  switch (st.k) {
    case "let": {
      const v = evalExpr(st.expr, env, ctx);
      if (env.vars.has(st.name)) note(ctx, 0, "rebinding '" + st.name + "'", st.line);
      env.vars.set(st.name, v);
      return;
    }
    case "assign": {
      const v = evalExpr(st.expr, env, ctx);
      const tgt = st.target;
      if (tgt.k === "var") {
        const hit = env.lookup(tgt.name);
        if (!hit) {
          if (st.op === "=") {
            env.root().vars.set(tgt.name, v); // auto-let at top scope (SPEC §5)
            note(ctx, 0, "auto-let '" + tgt.name + "' (assignment without let)", st.line);
          } else {
            note(ctx, 0, "unbound variable '" + tgt.name + "' — null returned", st.line);
          }
          return;
        }
        hit.env.vars.set(tgt.name, st.op === "=" ? v : arith(ctx, st.op.slice(0, -1), hit.val, v, st.line));
        return;
      }
      if (tgt.k === "index") {
        const obj = evalExpr(tgt.obj, env, ctx);
        const idx = evalExpr(tgt.idx, env, ctx);
        if (Array.isArray(obj)) {
          if (typeof idx !== "number") { note(ctx, 0, "unfolded: list index must be int — assignment skipped", st.line); return; }
          const i = idx < 0 ? obj.length + idx : idx;
          if (i < 0 || i >= obj.length) { note(ctx, 0, "missing: index " + idx + " out of range — assignment skipped", st.line); return; }
          obj[i] = st.op === "=" ? v : arith(ctx, st.op.slice(0, -1), obj[i], v, st.line);
          return;
        }
        if (obj instanceof Map) {
          const k = typeof idx === "string" ? idx : str(idx);
          obj.set(k, st.op === "=" ? v : arith(ctx, st.op.slice(0, -1), obj.get(k) === undefined ? null : obj.get(k), v, st.line));
          return;
        }
        note(ctx, 0, "missing: cannot assign into " + typeOf(obj) + " — skipped", st.line);
        return;
      }
      if (tgt.k === "member") {
        const obj = evalExpr(tgt.obj, env, ctx);
        if (obj instanceof Map) {
          obj.set(tgt.name, st.op === "=" ? v : arith(ctx, st.op.slice(0, -1), obj.has(tgt.name) ? obj.get(tgt.name) : null, v, st.line));
          return;
        }
        note(ctx, 0, "missing: cannot assign member on " + typeOf(obj) + " — skipped", st.line);
        return;
      }
      return;
    }
    case "if": {
      for (const c of st.clauses) {
        if (truthy(evalExpr(c.cond, env, ctx))) { execBlock(c.body, env, ctx); return; }
      }
      if (st.elseBody) execBlock(st.elseBody, env, ctx);
      return;
    }
    case "while": {
      for (;;) {
        step(ctx);
        if (!truthy(evalExpr(st.cond, env, ctx))) break;
        try { execBlock(st.body, env, ctx); } catch (e) {
          if (e instanceof Sg && e.kind === "break") break;
          if (e instanceof Sg && e.kind === "continue") continue;
          throw e;
        }
      }
      return;
    }
    case "loop": {
      for (;;) {
        step(ctx);
        try { execBlock(st.body, env, ctx); } catch (e) {
          if (e instanceof Sg && e.kind === "break") break;
          if (e instanceof Sg && e.kind === "continue") continue;
          throw e;
        }
      }
    }
    case "for": {
      const box = evalExpr(st.iter, env, ctx);
      let items = null;
      if (Array.isArray(box)) items = box;
      else if (typeof box === "string") items = Array.from(box);
      else if (box instanceof Map) items = Array.from(box.keys());
      else note(ctx, 0, "unfolded: for over " + typeOf(box) + " — nothing to iterate", st.line);
      if (!items) return;
      for (const item of items) {
        step(ctx);
        env.vars.set(st.name, item);
        try { execBlock(st.body, env, ctx); } catch (e) {
          if (e instanceof Sg && e.kind === "break") break;
          if (e instanceof Sg && e.kind === "continue") continue;
          throw e;
        }
      }
      return;
    }
    case "ret": throw new Sg("return", evalExpr(st.expr, env, ctx));
    case "break": throw new Sg("break", null);
    case "cont": throw new Sg("continue", null);
    case "expr": evalExpr(st.expr, env, ctx); return;
    case "genedef": evalExpr(st, env, ctx); return;
  }
}

/* ------------------------------- builtins ---------------------------------- */

function makeGlobalEnv(ctx) {
  const env = new Env(null);
  const def = (name, fn) => env.vars.set(name, { __native: true, name, fn });
  const one = (args) => (args.length >= 1 ? args[0] : null);

  def("promote", (args, c) => { c.out.push(args.map(str).join(" ")); return null; });
  def("len", (args, c, ln) => {
    const v = one(args);
    if (typeof v === "string") return v.length;
    if (Array.isArray(v)) return v.length;
    if (v instanceof Map) return v.size;
    note(c, 0, "unfolded: len on " + typeOf(v) + " — null returned", ln);
    return null;
  });
  def("str", (args) => str(one(args)));
  def("num", (args, c, ln) => numCoerce(c, one(args), ln));
  def("type", (args) => typeOf(one(args)));
  def("range", (args, c, ln) => {
    const nums = args.map((a) => (a instanceof Flt || typeof a === "number" ? numVal(a) : NaN));
    if (args.length === 0 || nums.some((n) => isNaN(n))) {
      note(c, 0, "unfolded: range needs numbers — [] returned", ln);
      return [];
    }
    let a = 0, b = 0, s = 1;
    if (args.length === 1) { b = Math.floor(nums[0]); }
    else { a = Math.floor(nums[0]); b = Math.floor(nums[1]); if (args.length >= 3) s = Math.floor(nums[2]) || 1; }
    const out = [];
    if (s > 0) for (let i = a; i < b; i += s) out.push(i);
    else if (s < 0) for (let i = a; i > b; i += s) out.push(i);
    return out;
  });
  def("push", (args, c, ln) => {
    const l = one(args);
    if (!Array.isArray(l)) { note(c, 0, "unfolded: push needs a list — null returned", ln); return null; }
    l.push(args.length >= 2 ? args[1] : null);
    return l;
  });
  def("pop", (args, c, ln) => {
    const l = one(args);
    if (!Array.isArray(l) || l.length === 0) { note(c, 0, "missing: pop from empty or non-list — null returned", ln); return null; }
    return l.pop();
  });
  def("keys", (args, c, ln) => {
    const m = one(args);
    if (!(m instanceof Map)) { note(c, 0, "unfolded: keys needs a map — null returned", ln); return null; }
    return Array.from(m.keys());
  });
  def("values", (args, c, ln) => {
    const m = one(args);
    if (!(m instanceof Map)) { note(c, 0, "unfolded: values needs a map — null returned", ln); return null; }
    return Array.from(m.values());
  });
  def("abs", (args, c, ln) => {
    const v = one(args);
    if (!(v instanceof Flt || typeof v === "number")) { note(c, 0, "unfolded: abs on " + typeOf(v) + " — null returned", ln); return null; }
    return v instanceof Flt ? flt(Math.abs(v.n)) : Math.abs(v);
  });
  const pick = (args, c, ln, better) => {
    let list = args;
    if (args.length === 1 && Array.isArray(args[0])) list = args[0];
    let best = null;
    for (const v of list) {
      if (!(v instanceof Flt || typeof v === "number")) { note(c, 0, "unfolded: " + better.name + " on " + typeOf(v) + " — null returned", ln); return null; }
      if (best === null || better(numVal(v), numVal(best))) best = v;
    }
    if (best === null) note(c, 0, "missing: " + better.name + " of nothing — null returned", ln);
    return best;
  };
  def("min", (args, c, ln) => pick(args, c, ln, { name: "min", apply: (a, b) => a < b }.apply ? (a, b) => a < b : null));
  def("max", (args, c, ln) => pick(args, c, ln, (a, b) => a > b));
  def("sum", (args, c, ln) => {
    const list = args.length === 1 && Array.isArray(args[0]) ? args[0] : args;
    let acc = 0, isF = false;
    for (const v of list) {
      if (!(v instanceof Flt || typeof v === "number")) { note(c, 0, "unfolded: sum on " + typeOf(v) + " — null returned", ln); return null; }
      if (v instanceof Flt) isF = true;
      acc += numVal(v);
    }
    return isF ? flt(acc) : acc;
  });
  def("clock", () => {
    const t = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    return flt(t / 1000);
  });
  def("distance", (args) => levenshtein(str(one(args)), str(args.length >= 2 ? args[1] : "")));
  def("codon", (args, c, ln) => {
    const s = str(one(args));
    let score = 100;
    if (s.length > 24) score -= 4 * (s.length - 24);
    for (const ch of s) {
      if (ch >= "0" && ch <= "9") score -= 3;
      else if (!(ch >= "a" && ch <= "z") && ch !== "_") score -= 2;
    }
    return Math.max(0, score);
  });
  def("exit", () => { throw new Sg("exit", null); });
  return env;
}

/* ------------------------------ entry point -------------------------------- */

// runProgram(src) → { output: string[], notes: [{line, rung, message}], score, letter }
// Grading per SPEC §15: start 100; wobble (rung 3) −2; fallback (rung 4) −3;
// floor 50. Letter: A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60, else F.
function runProgram(src) {
  const ctx = { out: [], notes: [], steps: 0, depth: 0 };
  try {
    const toks = lex(String(src), ctx);
    const names = knownNames(toks);
    const parser = new Parser(toks, ctx, names);
    const program = parser.parseProgram();
    const genv = makeGlobalEnv(ctx);
    try {
      execBlock(program, genv, ctx);
    } catch (e) {
      if (e instanceof Sg) {
        if (e.kind === "abort") note(ctx, 0, "step budget exhausted — run stopped", 0);
        else if (e.kind === "break" || e.kind === "continue") note(ctx, 0, "'" + e.kind + "' outside loop — ignored", 0);
        // "return" and "exit" end the run (exit is the SPEC-sanctioned hard stop)
      } else if (e instanceof RangeError) {
        note(ctx, 0, "overflow: recursion depth exceeded — run stopped", 0);
      } else {
        note(ctx, 0, "internal: " + (e && e.message ? e.message : e), 0);
      }
    }
  } catch (e) {
    note(ctx, 0, "internal: " + (e && e.message ? e.message : e), 0);
  }
  let wob = 0, fb = 0;
  for (const n of ctx.notes) {
    if (n.rung === 3) wob++;
    else if (n.rung === 4) fb++;
  }
  const score = Math.max(50, 100 - 2 * wob - 3 * fb);
  const letter = score >= 90 ? "A" : score >= 80 ? "B" : score >= 70 ? "C" : score >= 60 ? "D" : "F";
  return { output: ctx.out, notes: ctx.notes, score, letter };
}

// Node smoke-test hook (the browser simply uses the global runProgram).
if (typeof module !== "undefined") { module.exports = { runProgram }; }
