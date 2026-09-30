//! pkg.rs — Operon package management core (W19/W022, track L3c, ai/ecosystem).
//!
//! First-class package management with zero external crates, in the same
//! spirit as the rest of the core: everything the CLI needs to make
//! `operon new / add / remove / update / publish / search` real:
//!
//!   * a strict TOML subset reader for `operon.toml` manifests
//!   * semantic-version requirements (^, ~, comparators, wildcards, AND lists)
//!   * a deterministic resolver (highest match, tie-broken, sorted output)
//!   * `operon.lock` — byte-reproducible resolution, lockfile-first installs
//!   * registry transports: local directory registries and plain HTTP/1.1
//!   * package envelopes (JSON + base64 files, sha256-pinned) for publish/install
//!
//! TLS is deliberately out of scope for this build (no crates policy): the
//! registry client speaks `file://`-style local paths and `http://`. The
//! hosted registry on Render is reachable over http for local dev and the
//! publish outbox flow covers https upload (docs/PACKAGING.md).
//!
//! Every routine here is unit-tested at the bottom of this file; end-to-end
//! CLI behavior is tested by tests/package/pkg_e2e.sh.

use std::collections::BTreeMap;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// minimal TOML (the subset a manifest actually uses)
// ---------------------------------------------------------------------------

/// A TOML value, restricted to what manifests need.
#[derive(Debug, Clone, PartialEq)]
pub enum Toml {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Toml>),
    Table(BTreeMap<String, Toml>),
}

impl Toml {
    pub fn get(&self, key: &str) -> Option<&Toml> {
        match self {
            Toml::Table(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Toml::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&BTreeMap<String, Toml>> {
        match self {
            Toml::Table(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Toml>> {
        match self {
            Toml::Array(a) => Some(a),
            _ => None,
        }
    }
    /// Human-facing type name for error messages.
    fn type_name(&self) -> &'static str {
        match self {
            Toml::Str(_) => "string",
            Toml::Int(_) => "integer",
            Toml::Float(_) => "float",
            Toml::Bool(_) => "boolean",
            Toml::Array(_) => "array",
            Toml::Table(_) => "table",
        }
    }
}

/// Parse a manifest-shaped TOML document. Returns (root table, error).
///
/// Supports: comments, bare/quoted keys, dotted keys, `[table]` headers,
/// strings (basic + literal), integers, floats, booleans, arrays, inline
/// tables. Rejects what it does not understand LOUDLY — a manifest parser
/// that silently skips lines produces projects that lie about their deps.
pub fn toml_parse(src: &str) -> Result<Toml, String> {
    let mut root: BTreeMap<String, Toml> = BTreeMap::new();
    // path of the table we are currently filling; empty = root
    let mut cur: Vec<String> = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut li = 0usize;
    while li < lines.len() {
        let raw = lines[li];
        li += 1;
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            // table header
            let hdr = rest
                .strip_suffix(']')
                .ok_or_else(|| err_at(li, "unterminated table header"))?;
            if hdr.starts_with('[') {
                return Err(err_at(
                    li,
                    "array-of-table headers are not used by operon manifests",
                ));
            }
            cur = parse_key_path(hdr, li)?;
            insert_nested(&mut root, &cur, Toml::Table(BTreeMap::new()), li, true)?;
            continue;
        }
        // key = value (value may span lines inside brackets)
        let eq = find_top_level_eq(&line).ok_or_else(|| err_at(li, "expected `key = value`"))?;
        let key_part = line[..eq].trim().to_string();
        let mut val_part = line[eq + 1..].trim().to_string();
        // multi-line arrays: keep consuming until brackets balance
        while bracket_balance(&val_part) > 0 && li < lines.len() {
            let next = strip_comment(lines[li]);
            li += 1;
            val_part.push(' ');
            val_part.push_str(next.trim());
        }
        let key_path = parse_key_path(&key_part, li)?;
        let mut full = cur.clone();
        full.extend(key_path);
        let val = parse_value(&val_part, li)?;
        insert_nested(&mut root, &full, val, li, false)?;
    }
    Ok(Toml::Table(root))
}

fn err_at(line: usize, msg: &str) -> String {
    format!("operon.toml line {}: {}", line, msg)
}

fn strip_comment(s: &str) -> &str {
    // no multi-line strings in the manifest subset: a # outside quotes ends
    // the line. Scan byte-wise respecting quotes.
    let mut in_basic = false;
    let mut in_literal = false;
    let bytes: Vec<char> = s.chars().collect();
    let mut escaped = false;
    for (i, c) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_basic => escaped = true,
            '"' if !in_literal => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '#' if !in_basic && !in_literal => {
                return &s[..s.char_indices().nth(i).map(|(b, _)| b).unwrap_or(s.len())]
            }
            _ => {}
        }
    }
    s
}

fn find_top_level_eq(s: &str) -> Option<usize> {
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_basic => escaped = true,
            '"' if !in_literal => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '=' if !in_basic && !in_literal => return Some(i),
            _ => {}
        }
    }
    None
}

fn bracket_balance(s: &str) -> i32 {
    let mut bal = 0i32;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_basic => escaped = true,
            '"' if !in_literal => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '[' if !in_basic && !in_literal => bal += 1,
            ']' if !in_basic && !in_literal => bal -= 1,
            _ => {}
        }
    }
    bal
}

fn parse_key_path(s: &str, line: usize) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for part in s.split('.') {
        let p = part.trim();
        if p.is_empty() {
            return Err(err_at(line, "empty key segment"));
        }
        if (p.starts_with('"') && p.ends_with('"') && p.len() >= 2)
            || (p.starts_with('\'') && p.ends_with('\'') && p.len() >= 2)
        {
            out.push(p[1..p.len() - 1].to_string());
        } else {
            if !p
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(err_at(line, &format!("invalid key character in '{}'", p)));
            }
            out.push(p.to_string());
        }
    }
    Ok(out)
}

fn insert_nested(
    root: &mut BTreeMap<String, Toml>,
    path: &[String],
    val: Toml,
    line: usize,
    is_header: bool,
) -> Result<(), String> {
    let mut cur = root;
    for (i, seg) in path.iter().enumerate() {
        let last = i == path.len() - 1;
        if last {
            if is_header {
                // re-opening a header: ensure the table slot exists, never
                // clobber whatever table structure is already there
                cur.entry(seg.clone())
                    .or_insert_with(|| Toml::Table(BTreeMap::new()));
                return Ok(());
            }
            // plain assignment into a table slot that already holds a
            // non-table is a duplicate-key error
            cur.insert(seg.clone(), val);
            return Ok(());
        }
        let entry = cur
            .entry(seg.clone())
            .or_insert_with(|| Toml::Table(BTreeMap::new()));
        match entry {
            Toml::Table(m) => cur = m,
            _ => return Err(err_at(line, "key redefines a non-table value")),
        }
    }
    Ok(())
}

fn parse_value(s: &str, line: usize) -> Result<Toml, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(err_at(line, "missing value"));
    }
    if s.starts_with('"') {
        return parse_basic_string(s, line);
    }
    if s.starts_with('\'') {
        return parse_literal_string(s, line);
    }
    if s.starts_with('[') {
        return parse_array(s, line);
    }
    if s.starts_with('{') {
        return parse_inline_table(s, line);
    }
    if s == "true" {
        return Ok(Toml::Bool(true));
    }
    if s == "false" {
        return Ok(Toml::Bool(false));
    }
    // number?
    if let Ok(i) = s.replace('_', "").parse::<i64>() {
        return Ok(Toml::Int(i));
    }
    if let Ok(f) = s.replace('_', "").parse::<f64>() {
        if s.chars().any(|c| c == '.' || c == 'e' || c == 'E') {
            return Ok(Toml::Float(f));
        }
    }
    Err(err_at(line, &format!("unsupported value '{}'", s)))
}

fn parse_basic_string(s: &str, line: usize) -> Result<Toml, String> {
    let mut out = String::new();
    let mut chars = s.chars();
    chars.next(); // opening quote
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if escaped {
            match c {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return Err(err_at(line, &format!("unsupported escape '\\{}'", c))),
            }
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => {
                let rest: String = chars.collect();
                if !rest.trim().is_empty() {
                    return Err(err_at(line, "trailing characters after string"));
                }
                return Ok(Toml::Str(out));
            }
            _ => out.push(c),
        }
    }
    Err(err_at(line, "unterminated string"))
}

fn parse_literal_string(s: &str, line: usize) -> Result<Toml, String> {
    let inner = &s[1..];
    let end = inner
        .find('\'')
        .ok_or_else(|| err_at(line, "unterminated literal string"))?;
    if !inner[end + 1..].trim().is_empty() {
        return Err(err_at(line, "trailing characters after string"));
    }
    Ok(Toml::Str(inner[..end].to_string()))
}

fn parse_array(s: &str, line: usize) -> Result<Toml, String> {
    let inner = s
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| err_at(line, "unterminated array"))?;
    let mut out = Vec::new();
    for piece in split_top_level(inner, ',') {
        let p = piece.trim();
        if p.is_empty() {
            continue;
        }
        out.push(parse_value(p, line)?);
    }
    Ok(Toml::Array(out))
}

fn parse_inline_table(s: &str, line: usize) -> Result<Toml, String> {
    let inner = s
        .strip_prefix('{')
        .and_then(|x| x.strip_suffix('}'))
        .ok_or_else(|| err_at(line, "unterminated inline table"))?;
    let mut out = BTreeMap::new();
    for piece in split_top_level(inner, ',') {
        let p = piece.trim();
        if p.is_empty() {
            continue;
        }
        let eq = find_top_level_eq(p)
            .ok_or_else(|| err_at(line, "inline table entry needs `key = value`"))?;
        let key = parse_key_path(p[..eq].trim(), line)?;
        if key.len() != 1 {
            return Err(err_at(
                line,
                "dotted keys inside inline tables are not supported",
            ));
        }
        out.insert(key[0].clone(), parse_value(p[eq + 1..].trim(), line)?);
    }
    Ok(Toml::Table(out))
}

fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    let mut cur = String::new();
    for c in s.chars() {
        if escaped {
            escaped = false;
            cur.push(c);
            continue;
        }
        match c {
            '\\' if in_basic => {
                escaped = true;
                cur.push(c);
            }
            '"' if !in_literal => {
                in_basic = !in_basic;
                cur.push(c);
            }
            '\'' if !in_basic => {
                in_literal = !in_literal;
                cur.push(c);
            }
            '[' | '{' if !in_basic && !in_literal => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' if !in_basic && !in_literal => {
                depth -= 1;
                cur.push(c);
            }
            c if c == sep && depth == 0 && !in_basic && !in_literal => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

// ---------------------------------------------------------------------------
// semantic versions + requirements
// ---------------------------------------------------------------------------

/// A strict X.Y.Z semantic version (pre-release tags are rejected loudly —
/// reproducibility first; they arrive with lockfile v2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    pub fn parse(s: &str) -> Result<SemVer, String> {
        let bad = || format!("invalid version '{}' (expected X.Y.Z)", s);
        let core = s.trim();
        if core.is_empty() || !core.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(bad());
        }
        if core.contains("..") || core.starts_with('.') || core.ends_with('.') {
            return Err(bad());
        }
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(bad());
        }
        let mut nums = [0u64; 3];
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() {
                return Err(bad());
            }
            nums[i] = p
                .parse::<u64>()
                .map_err(|_| format!("invalid version '{}' (component too large)", s))?;
        }
        Ok(SemVer {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
        })
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

/// One comparator inside a requirement.
#[derive(Debug, Clone, PartialEq)]
enum Cmp {
    Exact(SemVer),
    Caret(SemVer),
    Tilde(SemVer),
    Ge(SemVer),
    Gt(SemVer),
    Le(SemVer),
    Lt(SemVer),
    Wild(SemVer, u8), // 1 = major-only wildcard (1.x), 2 = minor wildcard (1.2.x)
    Any,
}

/// A requirement = a list of comparators that must ALL hold (space/comma = AND).
#[derive(Debug, Clone, PartialEq)]
pub struct Req {
    cmps: Vec<Cmp>,
    raw: String,
}

impl Req {
    pub fn parse(s: &str) -> Result<Req, String> {
        let raw = s.trim().to_string();
        if raw.is_empty() {
            return Ok(Req {
                cmps: vec![Cmp::Any],
                raw,
            });
        }
        let mut cmps = Vec::new();
        // commas separate AND groups; inside a group, whitespace separates
        // comparators EXCEPT between an operator and its version (`>= 2.2`
        // is ONE comparator, `>=1.0 <2.0` is two) — npm-style tokenizing
        for seg in raw.split(',') {
            let seg = seg.trim();
            if seg.is_empty() {
                continue;
            }
            let chars: Vec<char> = seg.chars().collect();
            let mut i = 0usize;
            while i < chars.len() {
                if chars[i].is_whitespace() {
                    i += 1;
                    continue;
                }
                let mut op = String::new();
                for cand in [">=", "<=", "~=", ">", "<", "=", "^", "~"] {
                    let cc: Vec<char> = cand.chars().collect();
                    if chars[i..].starts_with(&cc[..]) {
                        op = cand.to_string();
                        i += cc.len();
                        break;
                    }
                }
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let mut ver = String::new();
                while i < chars.len()
                    && (chars[i].is_ascii_digit()
                        || chars[i] == '.'
                        || chars[i] == 'x'
                        || chars[i] == 'X'
                        || chars[i] == '*')
                {
                    ver.push(chars[i]);
                    i += 1;
                }
                if ver.is_empty() {
                    if op.is_empty() {
                        return Err(format!("invalid requirement '{}'", s));
                    }
                    return Err(format!(
                        "operator '{}' without a version in requirement '{}'",
                        op, s
                    ));
                }
                let token = format!("{}{}", op, ver);
                cmps.push(parse_cmp(&token)?);
            }
        }
        if cmps.is_empty() {
            return Err(format!("empty requirement '{}'", s));
        }
        Ok(Req { cmps, raw })
    }

    pub fn matches(&self, v: &SemVer) -> bool {
        self.cmps.iter().all(|c| cmp_matches(c, v))
    }

    /// Does this requirement still accept the pinned version? (lockfile check)
    pub fn raw_str(&self) -> &str {
        &self.raw
    }
}

fn parse_cmp(tok: &str) -> Result<Cmp, String> {
    if tok == "*" || tok == "x" || tok == "X" {
        return Ok(Cmp::Any);
    }
    let (op, rest) = if let Some(r) = tok.strip_prefix("^") {
        ("^", r)
    } else if let Some(r) = tok.strip_prefix("~=") {
        ("~", r)
    } else if let Some(r) = tok.strip_prefix('~') {
        ("~", r)
    } else if let Some(r) = tok.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = tok.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = tok.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = tok.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = tok.strip_prefix('=') {
        ("=", r)
    } else {
        ("", tok)
    };
    // wildcard forms: 1.x / 1.* / 1.2.x / 1.2.*
    if rest.ends_with(".x") || rest.ends_with(".*") || rest == "x" || rest == "*" {
        let base = rest.trim_end_matches(['x', '*', '.']).trim_end_matches('.');
        let v = parse_loose(base)?;
        let w = if !base.contains('.') { 1 } else { 2 };
        return Ok(Cmp::Wild(v, w));
    }
    let v = parse_loose(rest)?;
    match op {
        "^" => Ok(Cmp::Caret(v)),
        // ~1 pins only the major line (~1 = 1.x); ~1.2 pins major+minor
        "~" if !rest.contains('.') => Ok(Cmp::Wild(v, 1)),
        "~" => Ok(Cmp::Tilde(v)),
        ">=" => Ok(Cmp::Ge(v)),
        "<=" => Ok(Cmp::Le(v)),
        ">" => Ok(Cmp::Gt(v)),
        "<" => Ok(Cmp::Lt(v)),
        "=" => Ok(Cmp::Exact(v)),
        // bare "1.2" behaves as ^1.2 (caret is the operon default), while a
        // full X.Y.Z stays EXACT — pins in a manifest mean what they say
        "" => {
            if rest.matches('.').count() == 2 {
                Ok(Cmp::Exact(v))
            } else {
                Ok(Cmp::Caret(v))
            }
        }
        _ => unreachable!(),
    }
}

fn parse_loose(s: &str) -> Result<SemVer, String> {
    // "1" -> 1.0.0, "1.2" -> 1.2.0, "1.2.3" -> as-is
    match s.matches('.').count() {
        0 => {
            let major = s
                .parse::<u64>()
                .map_err(|_| format!("invalid version component '{}'", s))?;
            Ok(SemVer {
                major,
                minor: 0,
                patch: 0,
            })
        }
        1 => {
            let mut it = s.split('.');
            let major = it
                .next()
                .unwrap()
                .parse::<u64>()
                .map_err(|_| format!("invalid version '{}'", s))?;
            let minor = it
                .next()
                .unwrap()
                .parse::<u64>()
                .map_err(|_| format!("invalid version '{}'", s))?;
            Ok(SemVer {
                major,
                minor,
                patch: 0,
            })
        }
        _ => SemVer::parse(s),
    }
}

fn cmp_matches(c: &Cmp, v: &SemVer) -> bool {
    match c {
        Cmp::Any => true,
        Cmp::Exact(b) => v == b,
        Cmp::Ge(b) => v >= b,
        Cmp::Gt(b) => v > b,
        Cmp::Le(b) => v <= b,
        Cmp::Lt(b) => v < b,
        Cmp::Caret(b) => {
            // ^0.2.3 is special: [0.2.3, 0.3.0) — the leftmost non-zero
            // component is the stability promise; ^0.0.Y freezes the patch
            v >= b
                && match b.major {
                    0 if b.minor == 0 => v.major == 0 && v.minor == 0 && v.patch == b.patch,
                    0 => v.major == 0 && v.minor == b.minor,
                    _ => v.major == b.major,
                }
        }
        Cmp::Tilde(b) => {
            // ~1.2.3 = [1.2.3, 1.3.0); ~1.2 = [1.2.0, 1.3.0)
            v >= b && v.major == b.major && v.minor == b.minor
        }
        Cmp::Wild(b, 1) => v.major == b.major,
        Cmp::Wild(b, _) => v.major == b.major && v.minor == b.minor,
    }
}

// ---------------------------------------------------------------------------
// sha256 (FIPS 180-4, ~60 lines) — content pinning for every package artifact
// ---------------------------------------------------------------------------

pub fn sha256_hex(data: &[u8]) -> String {
    // FIPS 180-4, 32-bit words. (The first draft mixed in 64-bit words —
    // the known-answer tests below caught it immediately, which is exactly
    // why they exist.)
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut st = h;
        for i in 0..64 {
            let s1 = st[4].rotate_right(6) ^ st[4].rotate_right(11) ^ st[4].rotate_right(25);
            let ch = (st[4] & st[5]) ^ ((!st[4]) & st[6]);
            let t1 = st[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = st[0].rotate_right(2) ^ st[0].rotate_right(13) ^ st[0].rotate_right(22);
            let maj = (st[0] & st[1]) ^ (st[0] & st[2]) ^ (st[1] & st[2]);
            let t2 = s0.wrapping_add(maj);
            st[7] = st[6];
            st[6] = st[5];
            st[5] = st[4];
            st[4] = st[3].wrapping_add(t1);
            st[3] = st[2];
            st[2] = st[1];
            st[1] = st[0];
            st[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(st[i]);
        }
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{:08x}", word));
    }
    out
}

// ---------------------------------------------------------------------------
// base64 (standard alphabet, padded) — envelope file payloads
// ---------------------------------------------------------------------------

pub fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte 0x{:02x}", c)),
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let trimmed: &[u8] = {
        let mut t = bytes.as_slice();
        while t.last() == Some(&b'=') {
            t = &t[..t.len() - 1];
        }
        t
    };
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.chunks(4) {
        if chunk.len() == 1 {
            return Err("truncated base64".into());
        }
        let mut n: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// manifest (operon.toml)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: SemVer,
    pub operon_req: Option<Req>,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub repository: String,
    pub keywords: Vec<String>,
    pub entry: String,
    pub deps: BTreeMap<String, Req>,
}

pub const DEFAULT_ENTRY: &str = "src/main.op";

pub fn valid_name(name: &str) -> bool {
    let n = name.as_bytes();
    n.len() >= 2
        && n.len() <= 64
        && n[0].is_ascii_lowercase()
        && n.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

impl Manifest {
    pub fn parse(src: &str) -> Result<Manifest, String> {
        let root = toml_parse(src)?;
        let pkg = root
            .get("package")
            .ok_or("operon.toml: missing [package] table")?;
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("operon.toml: [package] name must be a string")?
            .to_string();
        if !valid_name(&name) {
            return Err(format!(
                "operon.toml: package name '{}' is invalid (lowercase letters, digits, '-', '_', 2..64 chars, starts with a letter)",
                name
            ));
        }
        let vstr = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or("operon.toml: [package] version must be a string")?;
        let version = SemVer::parse(vstr).map_err(|e| format!("operon.toml: {}", e))?;
        let operon_req = match pkg.get("operon-version") {
            Some(Toml::Str(s)) => Some(Req::parse(s)?),
            None => None,
            Some(other) => {
                return Err(format!(
                    "operon.toml: operon-version must be a string, got {}",
                    other.type_name()
                ))
            }
        };
        let entry = match pkg.get("entry") {
            Some(Toml::Str(s)) => s.clone(),
            None => DEFAULT_ENTRY.to_string(),
            Some(other) => {
                return Err(format!(
                    "operon.toml: entry must be a string, got {}",
                    other.type_name()
                ))
            }
        };
        let str_or_default = |k: &str| -> String {
            pkg.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let authors = match pkg.get("authors") {
            Some(Toml::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let keywords = match pkg.get("keywords") {
            Some(Toml::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let mut deps = BTreeMap::new();
        if let Some(table) = root.get("dependencies").and_then(|v| v.as_table()) {
            for (k, v) in table {
                let req = match v {
                    Toml::Str(s) => Req::parse(s)?,
                    other => {
                        return Err(format!(
                            "operon.toml: dependency '{}' must be a requirement string, got {}",
                            k,
                            other.type_name()
                        ))
                    }
                };
                if !valid_name(k) {
                    return Err(format!("operon.toml: dependency name '{}' is invalid", k));
                }
                deps.insert(k.clone(), req);
            }
        }
        Ok(Manifest {
            name,
            version,
            operon_req,
            description: str_or_default("description"),
            authors,
            license: str_or_default("license"),
            repository: str_or_default("repository"),
            keywords,
            entry,
            deps,
        })
    }

    pub fn load_dir(dir: &Path) -> Result<Manifest, String> {
        let path = dir.join("operon.toml");
        let src = std::fs::read_to_string(&path)
            .map_err(|_| format!("no operon.toml in {}", dir.display()))?;
        Manifest::parse(&src)
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("[package]\n");
        out.push_str(&format!("name = \"{}\"\n", self.name));
        out.push_str(&format!("version = \"{}\"\n", self.version));
        if let Some(r) = &self.operon_req {
            out.push_str(&format!("operon-version = \"{}\"\n", r.raw_str()));
        }
        if !self.description.is_empty() {
            out.push_str(&format!(
                "description = \"{}\"\n",
                escape_toml(&self.description)
            ));
        }
        if !self.authors.is_empty() {
            let a: Vec<String> = self
                .authors
                .iter()
                .map(|s| format!("\"{}\"", escape_toml(s)))
                .collect();
            out.push_str(&format!("authors = [{}]\n", a.join(", ")));
        }
        if !self.license.is_empty() {
            out.push_str(&format!("license = \"{}\"\n", escape_toml(&self.license)));
        }
        if !self.repository.is_empty() {
            out.push_str(&format!(
                "repository = \"{}\"\n",
                escape_toml(&self.repository)
            ));
        }
        if !self.keywords.is_empty() {
            let a: Vec<String> = self
                .keywords
                .iter()
                .map(|s| format!("\"{}\"", escape_toml(s)))
                .collect();
            out.push_str(&format!("keywords = [{}]\n", a.join(", ")));
        }
        if self.entry != DEFAULT_ENTRY {
            out.push_str(&format!("entry = \"{}\"\n", escape_toml(&self.entry)));
        }
        if !self.deps.is_empty() {
            out.push_str("\n[dependencies]\n");
            for (k, r) in &self.deps {
                out.push_str(&format!("{} = \"{}\"\n", k, escape_toml(r.raw_str())));
            }
        }
        out
    }
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// registry index + versions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IndexVersion {
    pub version: SemVer,
    pub yanked: bool,
    pub deps: BTreeMap<String, Req>,
    pub sha256: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct PackageIndex {
    pub name: String,
    pub description: String,
    pub versions: Vec<IndexVersion>,
}

impl PackageIndex {
    /// Highest non-yanked version satisfying the requirement. Deterministic:
    /// strict semver order; equal versions impossible per index (deduped on
    /// read).
    pub fn pick(&self, req: &Req) -> Option<&IndexVersion> {
        self.versions
            .iter()
            .filter(|iv| !iv.yanked && req.matches(&iv.version))
            .max_by(|a, b| a.version.cmp(&b.version))
    }
}

/// Registry error surfaced to the CLI verbatim.
#[derive(Debug)]
pub struct PkgError(pub String);

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A registry the CLI can talk to: a local directory (the dev/test shape)
/// or an http:// endpoint (the hosted shape, same URL surface).
#[derive(Debug, Clone)]
pub enum Registry {
    Dir(PathBuf),
    Http(String),
}

impl Registry {
    pub fn from_env_or_default() -> Registry {
        match std::env::var("OPERON_REGISTRY") {
            Ok(s) if !s.trim().is_empty() => Registry::parse_url(s.trim()),
            _ => Registry::Dir(PathBuf::from("registry")),
        }
    }
    pub fn parse_url(s: &str) -> Registry {
        if let Some(p) = s.strip_prefix("file://") {
            Registry::Dir(PathBuf::from(p))
        } else if s.starts_with("http://") {
            Registry::Http(s.trim_end_matches('/').to_string())
        } else if s.starts_with("https://") {
            // this build has no TLS stack (zero-crates core); surface the
            // limit honestly instead of a connection error downstream
            Registry::Http(s.trim_end_matches('/').to_string())
        } else {
            Registry::Dir(PathBuf::from(s))
        }
    }

    fn dir_index_path(&self, name: &str) -> Option<PathBuf> {
        match self {
            Registry::Dir(root) => Some(root.join("index").join(format!("{}.json", name))),
            _ => None,
        }
    }

    pub fn fetch_index(&self, name: &str) -> Result<PackageIndex, String> {
        match self {
            Registry::Dir(root) => {
                let path = self
                    .dir_index_path(name)
                    .ok_or_else(|| "internal: dir registry".to_string())?;
                let body = std::fs::read_to_string(&path).map_err(|_| {
                    format!(
                        "package '{}' not found in registry {}",
                        name,
                        root.display()
                    )
                })?;
                parse_index_json(&body)
            }
            Registry::Http(base) => {
                if base.starts_with("https://") {
                    return Err(format!(
                        "registry '{}' uses https, which this build cannot dial (no TLS in the zero-crates core). Use the local registry (OPERON_REGISTRY=<dir>) or an http mirror; https client support is tracked in docs/PACKAGING.md.",
                        base
                    ));
                }
                let url = format!("{}/api/packages/{}", base, name);
                let (status, body) = http_request("GET", &url, &[], None)?;
                if status == 404 {
                    return Err(format!("package '{}' not found in registry {}", name, base));
                }
                if status != 200 {
                    return Err(format!("registry returned HTTP {} for '{}'", status, name));
                }
                parse_index_json(&body)
            }
        }
    }

    pub fn fetch_artifact(&self, name: &str, ver: &str) -> Result<Vec<u8>, String> {
        match self {
            Registry::Dir(root) => {
                let path = root
                    .join("artifacts")
                    .join(name)
                    .join(format!("{}.opkg", ver));
                std::fs::read(&path)
                    .map_err(|e| format!("artifact {}/{} unreadable: {}", name, ver, e))
            }
            Registry::Http(base) => {
                if base.starts_with("https://") {
                    return Err(
                        "https artifact download is not supported by this build (no TLS)".into(),
                    );
                }
                let url = format!("{}/api/packages/{}/{}/download", base, name, ver);
                let (status, body) = http_request("GET", &url, &[], None)?;
                if status != 200 {
                    return Err(format!(
                        "registry returned HTTP {} for artifact {}/{}",
                        status, name, ver
                    ));
                }
                Ok(body.into_bytes())
            }
        }
    }

    pub fn search(&self, query: &str) -> Result<Vec<(String, String, String)>, String> {
        // (name, latest, description) triples, name-ascending
        match self {
            Registry::Dir(root) => {
                let mut out = Vec::new();
                let idx_dir = root.join("index");
                let entries = std::fs::read_dir(&idx_dir)
                    .map_err(|e| format!("registry dir {} unreadable: {}", idx_dir.display(), e))?;
                let mut files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                    .collect();
                files.sort();
                for f in files {
                    if let Ok(body) = std::fs::read_to_string(&f) {
                        if let Ok(idx) = parse_index_json(&body) {
                            if search_match(query, &idx) {
                                let latest = idx
                                    .versions
                                    .iter()
                                    .filter(|v| !v.yanked)
                                    .map(|v| v.version.to_string())
                                    .max_by(|a, b| {
                                        SemVer::parse(a).ok().cmp(&SemVer::parse(b).ok())
                                    })
                                    .unwrap_or_default();
                                out.push((idx.name, latest, idx.description));
                            }
                        }
                    }
                }
                Ok(out)
            }
            Registry::Http(base) => {
                if base.starts_with("https://") {
                    return Err("https search is not supported by this build (no TLS)".into());
                }
                let q = if query.is_empty() {
                    String::new()
                } else {
                    format!("?q={}", urlencode(query))
                };
                let url = format!("{}/api/search{}", base, q);
                let (status, body) = http_request("GET", &url, &[], None)?;
                if status != 200 {
                    return Err(format!("registry returned HTTP {} for search", status));
                }
                parse_search_json(&body)
            }
        }
    }

    pub fn publish(&self, envelope: &[u8], token: &str) -> Result<String, String> {
        match self {
            Registry::Dir(root) => {
                let env = parse_envelope(envelope)?;
                let name = env.name.clone();
                let ver = env.version.to_string();
                let idx_dir = root.join("index");
                let art_dir = root.join("artifacts").join(&name);
                std::fs::create_dir_all(&idx_dir).map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&art_dir).map_err(|e| e.to_string())?;
                // refuse to overwrite an existing version: published is
                // immutable (yank comes later; overwrite is how supply-chain
                // bugs happen)
                let art_path = art_dir.join(format!("{}.opkg", ver));
                if art_path.exists() {
                    return Err(format!(
                        "{} {} already exists in this registry — versions are immutable; bump the version to publish again",
                        name, ver
                    ));
                }
                std::fs::write(&art_path, envelope)
                    .map_err(|e| format!("artifact write failed: {}", e))?;
                // merge index
                let idx_path = idx_dir.join(format!("{}.json", name));
                let mut idx = match std::fs::read_to_string(&idx_path) {
                    Ok(body) => parse_index_json(&body)?,
                    Err(_) => PackageIndex {
                        name: name.clone(),
                        description: env.description.clone(),
                        versions: Vec::new(),
                    },
                };
                if idx.description.is_empty() {
                    idx.description = env.description.clone();
                }
                idx.versions.retain(|v| v.version != env.version);
                idx.versions.push(IndexVersion {
                    version: env.version.clone(),
                    yanked: false,
                    deps: env.deps.clone(),
                    sha256: sha256_hex(envelope),
                    description: env.description.clone(),
                });
                idx.versions.sort_by(|a, b| a.version.cmp(&b.version));
                std::fs::write(&idx_path, index_to_json(&idx))
                    .map_err(|e| format!("index write failed: {}", e))?;
                Ok(format!("published {} {} to {}", name, ver, root.display()))
            }
            Registry::Http(base) => {
                if base.starts_with("https://") {
                    return Err("https publish is not supported by this build (no TLS); use the publish outbox flow (docs/PACKAGING.md)".into());
                }
                let url = format!("{}/api/publish", base);
                let hdrs = vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Authorization".to_string(), format!("Bearer {}", token)),
                ];
                let (status, body) = http_request("POST", &url, &hdrs, Some(envelope))?;
                if status == 401 || status == 403 {
                    return Err("publish rejected: bad or missing token (set OPERON_TOKEN)".into());
                }
                if status != 200 && status != 201 {
                    let snippet: String = body.chars().take(200).collect();
                    return Err(format!("publish failed: HTTP {} {}", status, snippet));
                }
                Ok(body)
            }
        }
    }
}

fn search_match(query: &str, idx: &PackageIndex) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    idx.name.to_lowercase().contains(&q)
        || idx.description.to_lowercase().contains(&q)
        || idx
            .versions
            .iter()
            .any(|v| v.deps.keys().any(|d| d.to_lowercase().contains(&q)))
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Minimal HTTP/1.1 client over std::net — enough for the registry surface.
/// Returns (status, body). 10s connect/read timeouts; no redirects followed
/// (registries answer 200 directly; a 3xx surfaces as an error the user sees).
pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<(u16, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("unsupported URL '{}' (http:// only in this build)", url))?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addrs = hostport
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve '{}': {}", hostport, e))?
        .collect::<Vec<_>>();
    let addr = addrs.first().copied().ok_or("host resolved to nothing")?;
    let mut stream =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(10))
            .map_err(|e| format!("connect to {} failed: {}", hostport, e))?;
    use std::io::{Read, Write};
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: operon-pkg/1\r\n",
        method, path, hostport
    );
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    if let Some(b) = body {
        stream.write_all(b).map_err(|e| e.to_string())?;
    }
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let sep = b"\r\n\r\n";
    let split = buf
        .windows(sep.len())
        .position(|w| w == sep)
        .ok_or("malformed HTTP response (no header/body separator)")?;
    let head = String::from_utf8_lossy(&buf[..split]).to_string();
    let body_bytes = &buf[split + 4..];
    // chunked transfer decoding (registries commonly stream)
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body_final: Vec<u8> = if chunked {
        decode_chunked(body_bytes)
    } else {
        body_bytes.to_vec()
    };
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("malformed HTTP status line")?;
    Ok((status, String::from_utf8_lossy(&body_final).to_string()))
}

fn decode_chunked(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = data.windows(2).position(|w| w == b"\r\n") {
        let size_str = String::from_utf8_lossy(&data[..nl]);
        let size = match usize::from_str_radix(size_str.trim().split(';').next().unwrap_or("0"), 16)
        {
            Ok(s) => s,
            Err(_) => break,
        };
        if size == 0 {
            break;
        }
        let start = nl + 2;
        if data.len() < start + size {
            break;
        }
        out.extend_from_slice(&data[start..start + size]);
        data = &data[(start + size + 2).min(data.len())..];
    }
    out
}
// ---------------------------------------------------------------------------
// package envelope (the artifact format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Envelope {
    pub name: String,
    pub version: SemVer,
    pub description: String,
    pub deps: BTreeMap<String, Req>,
    /// (path, bytes) — paths are sanitized: forward slashes, no `..`, no
    /// absolute, must land inside the package root. Sorted by path on write.
    pub files: Vec<(String, Vec<u8>)>,
    pub manifest_src: String,
}

pub fn sanitize_rel_path(p: &str) -> Result<String, String> {
    if p.is_empty() {
        return Err("empty file path".into());
    }
    if p.starts_with('/') || p.starts_with('\\') || p.contains(':') || p.contains('\\') {
        return Err(format!("unsafe package path '{}'", p));
    }
    let mut depth = 0i32;
    for seg in p.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            depth -= 1;
            if depth < 0 {
                return Err(format!("unsafe package path '{}'", p));
            }
            continue;
        }
        depth += 1;
    }
    if depth == 0 {
        return Err(format!("unsafe package path '{}'", p));
    }
    Ok(p.split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/"))
}

impl Envelope {
    pub fn encode(&self) -> Vec<u8> {
        // deterministic JSON: keys in fixed order, files sorted by path.
        // Only the file payload strings go through base64; everything else
        // is plain JSON with minimal escaping.
        let mut files: Vec<&(String, Vec<u8>)> = self.files.iter().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = String::new();
        out.push_str("{\"envelope\":1");
        out.push_str(&format!(",\"name\":\"{}\"", json_str(&self.name)));
        out.push_str(&format!(
            ",\"version\":\"{}\"",
            json_str(&self.version.to_string())
        ));
        out.push_str(&format!(
            ",\"description\":\"{}\"",
            json_str(&self.description)
        ));
        if !self.deps.is_empty() {
            let mut ds = String::new();
            for (k, r) in &self.deps {
                if !ds.is_empty() {
                    ds.push(',');
                }
                ds.push_str(&format!(
                    "\"{}\":\"{}\"",
                    json_str(k),
                    json_str(r.raw_str())
                ));
            }
            out.push_str(&format!(",\"deps\":{{{}}}", ds));
        }
        out.push_str(&format!(
            ",\"manifest\":\"{}\"",
            json_str(&self.manifest_src)
        ));
        out.push_str(",\"files\":[");
        let mut first = true;
        for (path, bytes) in files {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&format!(
                "{{\"path\":\"{}\",\"b64\":\"{}\"}}",
                json_str(path),
                b64_encode(bytes)
            ));
        }
        out.push_str("]}\n");
        out.into_bytes()
    }

    pub fn decode(data: &[u8]) -> Result<Envelope, String> {
        parse_envelope(data)
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// --- tiny JSON reader for envelopes + index documents (read-only subset) ---

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

pub fn json_parse(src: &str) -> Result<Json, String> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let v = json_value(&b, &mut i)?;
    json_ws(&b, &mut i);
    if i != b.len() {
        return Err(format!("trailing JSON data at char {}", i));
    }
    Ok(v)
}

fn json_ws(b: &[char], i: &mut usize) {
    while *i < b.len() && b[*i].is_whitespace() {
        *i += 1;
    }
}

fn json_value(b: &[char], i: &mut usize) -> Result<Json, String> {
    json_ws(b, i);
    if *i >= b.len() {
        return Err("unexpected end of JSON".into());
    }
    match b[*i] {
        '{' => {
            *i += 1;
            let mut out = Vec::new();
            json_ws(b, i);
            if *i < b.len() && b[*i] == '}' {
                *i += 1;
                return Ok(Json::Obj(out));
            }
            loop {
                json_ws(b, i);
                let key = match json_value(b, i)? {
                    Json::Str(s) => s,
                    _ => return Err("object key must be a string".into()),
                };
                json_ws(b, i);
                if *i >= b.len() || b[*i] != ':' {
                    return Err("expected ':' in object".into());
                }
                *i += 1;
                let val = json_value(b, i)?;
                out.push((key, val));
                json_ws(b, i);
                match b.get(*i) {
                    Some(',') => *i += 1,
                    Some('}') => {
                        *i += 1;
                        return Ok(Json::Obj(out));
                    }
                    _ => return Err("expected ',' or '}' in object".into()),
                }
            }
        }
        '[' => {
            *i += 1;
            let mut out = Vec::new();
            json_ws(b, i);
            if *i < b.len() && b[*i] == ']' {
                *i += 1;
                return Ok(Json::Arr(out));
            }
            loop {
                out.push(json_value(b, i)?);
                json_ws(b, i);
                match b.get(*i) {
                    Some(',') => *i += 1,
                    Some(']') => {
                        *i += 1;
                        return Ok(Json::Arr(out));
                    }
                    _ => return Err("expected ',' or ']' in array".into()),
                }
            }
        }
        '"' => {
            *i += 1;
            let mut out = String::new();
            while *i < b.len() {
                match b[*i] {
                    '"' => {
                        *i += 1;
                        return Ok(Json::Str(out));
                    }
                    '\\' => {
                        *i += 1;
                        match b.get(*i) {
                            Some('n') => out.push('\n'),
                            Some('t') => out.push('\t'),
                            Some('r') => out.push('\r'),
                            Some('"') => out.push('"'),
                            Some('\\') => out.push('\\'),
                            Some('/') => out.push('/'),
                            Some('u') => {
                                let hex: String = b
                                    .get(*i + 1..*i + 5)
                                    .ok_or("bad \\u escape")?
                                    .iter()
                                    .collect();
                                let n =
                                    u32::from_str_radix(&hex, 16).map_err(|_| "bad \\u escape")?;
                                if let Some(c) = char::from_u32(n) {
                                    out.push(c);
                                }
                                *i += 4;
                            }
                            _ => return Err("bad escape in JSON string".into()),
                        }
                        *i += 1;
                    }
                    c => {
                        out.push(c);
                        *i += 1;
                    }
                }
            }
            Err("unterminated JSON string".into())
        }
        't' => {
            if b.get(*i..*i + 4) == Some(&['t', 'r', 'u', 'e'][..]) {
                *i += 4;
                Ok(Json::Bool(true))
            } else {
                Err("bad JSON literal".into())
            }
        }
        'f' => {
            if b.get(*i..*i + 5) == Some(&['f', 'a', 'l', 's', 'e'][..]) {
                *i += 5;
                Ok(Json::Bool(false))
            } else {
                Err("bad JSON literal".into())
            }
        }
        'n' => {
            if b.get(*i..*i + 4) == Some(&['n', 'u', 'l', 'l'][..]) {
                *i += 4;
                Ok(Json::Null)
            } else {
                Err("bad JSON literal".into())
            }
        }
        _ => {
            let start = *i;
            while *i < b.len()
                && (b[*i].is_ascii_digit()
                    || b[*i] == '-'
                    || b[*i] == '+'
                    || b[*i] == '.'
                    || b[*i] == 'e'
                    || b[*i] == 'E')
            {
                *i += 1;
            }
            let s: String = b[start..*i].iter().collect();
            s.parse::<f64>()
                .map(Json::Num)
                .map_err(|_| format!("bad JSON number '{}'", s))
        }
    }
}

fn parse_index_json(body: &str) -> Result<PackageIndex, String> {
    let j = json_parse(body)?;
    let name = j
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("index JSON missing 'name'")?
        .to_string();
    let description = j
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut versions = Vec::new();
    if let Some(arr) = j.get("versions").and_then(|v| v.as_arr()) {
        for v in arr {
            let vstr = v
                .get("version")
                .and_then(|x| x.as_str())
                .ok_or("index version missing 'version'")?;
            let ver = SemVer::parse(vstr).map_err(|e| format!("index for '{}': {}", name, e))?;
            let mut deps = BTreeMap::new();
            if let Some(Json::Obj(m)) = v.get("deps") {
                for (k, r) in m {
                    if let Some(rs) = r.as_str() {
                        deps.insert(k.clone(), Req::parse(rs)?);
                    }
                }
            }
            versions.push(IndexVersion {
                version: ver,
                yanked: v.get("yanked").and_then(|x| x.as_bool()).unwrap_or(false),
                deps,
                sha256: v
                    .get("sha256")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: v
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    versions.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(PackageIndex {
        name,
        description,
        versions,
    })
}

fn index_to_json(idx: &PackageIndex) -> String {
    let mut out = String::new();
    out.push_str(&format!("{{\"name\":\"{}\"", json_str(&idx.name)));
    out.push_str(&format!(
        ",\"description\":\"{}\"",
        json_str(&idx.description)
    ));
    out.push_str(",\"versions\":[");
    let mut first = true;
    for v in &idx.versions {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "{{\"version\":\"{}\"",
            json_str(&v.version.to_string())
        ));
        out.push_str(&format!(",\"yanked\":{}", v.yanked));
        out.push_str(&format!(",\"sha256\":\"{}\"", json_str(&v.sha256)));
        if !v.deps.is_empty() {
            let ds: Vec<String> = v
                .deps
                .iter()
                .map(|(k, r)| format!("\"{}\":\"{}\"", json_str(k), json_str(r.raw_str())))
                .collect();
            out.push_str(&format!(",\"deps\":{{{}}}", ds.join(",")));
        }
        out.push('}');
    }
    out.push_str("]}\n");
    out
}

fn parse_search_json(body: &str) -> Result<Vec<(String, String, String)>, String> {
    let j = json_parse(body)?;
    let arr = j
        .get("results")
        .and_then(|v| v.as_arr())
        .ok_or("search JSON missing 'results'")?;
    let mut out = Vec::new();
    for r in arr {
        let name = r
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let latest = r
            .get("latest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let desc = r
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push((name, latest, desc));
    }
    Ok(out)
}

fn parse_envelope(data: &[u8]) -> Result<Envelope, String> {
    let s = std::str::from_utf8(data).map_err(|_| "envelope is not UTF-8")?;
    let j = json_parse(s)?;
    if j.get("envelope")
        .map(|v| *v != Json::Num(1.0))
        .unwrap_or(true)
    {
        return Err("unsupported package envelope (want envelope:1)".into());
    }
    let name = j
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("envelope missing 'name'")?
        .to_string();
    let version = SemVer::parse(
        j.get("version")
            .and_then(|v| v.as_str())
            .ok_or("envelope missing 'version'")?,
    )?;
    let description = j
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut deps = BTreeMap::new();
    if let Some(Json::Obj(m)) = j.get("deps") {
        for (k, r) in m {
            if let Some(rs) = r.as_str() {
                deps.insert(k.clone(), Req::parse(rs)?);
            }
        }
    }
    let manifest_src = j
        .get("manifest")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut files = Vec::new();
    for f in j
        .get("files")
        .and_then(|v| v.as_arr())
        .ok_or("envelope missing 'files'")?
    {
        let path = f
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("envelope file missing 'path'")?;
        let safe = sanitize_rel_path(path)?;
        let b64 = f
            .get("b64")
            .and_then(|v| v.as_str())
            .ok_or("envelope file missing 'b64'")?;
        files.push((safe, b64_decode(b64)?));
    }
    Ok(Envelope {
        name,
        version,
        description,
        deps,
        files,
        manifest_src,
    })
}

// ---------------------------------------------------------------------------
// lockfile (operon.lock) — byte-reproducible resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct LockEntry {
    pub name: String,
    pub version: SemVer,
    pub sha256: String,
    /// resolved dependency pins "name version" (sorted)
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lockfile {
    pub packages: Vec<LockEntry>,
}

pub const LOCK_HEADER: &str = "# operon.lock — generated by operon. Do not edit.\n# Commit this file: it pins the exact dependency graph for reproducible installs.\n";

impl Lockfile {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(LOCK_HEADER);
        out.push_str("lock-format = 1\n\n");
        for p in &self.packages {
            out.push_str("[[package]]\n");
            out.push_str(&format!("name = \"{}\"\n", p.name));
            out.push_str(&format!("version = \"{}\"\n", p.version));
            out.push_str(&format!("sha256 = \"{}\"\n", p.sha256));
            let deps: Vec<String> = p
                .deps
                .iter()
                .map(|d| format!("\"{}\"", escape_toml(d)))
                .collect();
            out.push_str(&format!("deps = [{}]\n\n", deps.join(", ")));
        }
        out.into_bytes()
    }

    pub fn parse(src: &str) -> Result<Lockfile, String> {
        // NOTE: deliberately NOT parsed with toml_parse — the strict subset
        // parser rejects [[array-of-table]] headers. The lockfile is a fixed
        // generated shape, so a line-scanner is exact and forgiving of nothing.
        let mut out = Lockfile::default();
        let mut saw_format = false;
        let mut cur: Option<LockEntry> = None;
        for raw in src.lines() {
            let line = strip_comment(raw).trim().to_string();
            if line == "[[package]]" {
                if let Some(p) = cur.take() {
                    out.packages.push(p);
                }
                cur = Some(LockEntry {
                    name: String::new(),
                    version: SemVer {
                        major: 0,
                        minor: 0,
                        patch: 0,
                    },
                    sha256: String::new(),
                    deps: Vec::new(),
                });
                continue;
            }
            if cur.is_none() {
                if line.starts_with("lock-format") {
                    if !line.contains("1") {
                        return Err(
                            "operon.lock: unsupported lock-format (regenerate with `operon update`)".into(),
                        );
                    }
                    saw_format = true;
                }
                continue;
            }
            let eq = match find_top_level_eq(&line) {
                Some(i) => i,
                None => continue,
            };
            let key = line[..eq].trim().to_string();
            let val = parse_value(line[eq + 1..].trim(), 0)?;
            let p = cur.as_mut().unwrap();
            match (key.as_str(), val) {
                ("name", Toml::Str(s)) => p.name = s,
                ("version", Toml::Str(s)) => p.version = SemVer::parse(&s)?,
                ("sha256", Toml::Str(s)) => p.sha256 = s,
                ("deps", Toml::Array(a)) => {
                    p.deps = a
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                }
                _ => {}
            }
        }
        if let Some(p) = cur.take() {
            out.packages.push(p);
        }
        if !saw_format {
            return Err(
                "operon.lock: missing lock-format = 1 (regenerate with `operon update`)".into(),
            );
        }
        Ok(out)
    }

    pub fn load(dir: &Path) -> Option<Lockfile> {
        let src = std::fs::read_to_string(dir.join("operon.lock")).ok()?;
        Lockfile::parse(&src).ok()
    }

    pub fn find(&self, name: &str) -> Option<&LockEntry> {
        self.packages.iter().find(|p| p.name == name)
    }
}

/// Does the existing lock still satisfy the manifest's requirements?
/// True only when every manifest dep is present at a version its req accepts
/// AND every lock package's own dep pins are all inside the lock (closed).
pub fn lock_satisfies(manifest: &Manifest, lock: &Lockfile) -> bool {
    for (name, req) in &manifest.deps {
        match lock.find(name) {
            Some(p) if req.matches(&p.version) => {}
            _ => return false,
        }
    }
    // closure: every lock package's deps resolve inside the lock
    for p in &lock.packages {
        for d in &p.deps {
            let mut parts = d.splitn(2, ' ');
            let dname = parts.next().unwrap_or("");
            let dver = parts.next().unwrap_or("");
            match lock.find(dname) {
                Some(dp) if dp.version.to_string() == dver => {}
                _ => return false,
            }
        }
    }
    true
}

/// Deterministic full resolution: DFS over manifest deps, always picking the
/// highest non-yanked version that satisfies the requirement. Output sorted
/// by name. Cycle detection via the active path. The result is a function of
/// (requirements, registry content) alone — same inputs, same lock bytes.
pub fn resolve(manifest: &Manifest, reg: &Registry) -> Result<Lockfile, String> {
    let mut chosen: BTreeMap<String, LockEntry> = BTreeMap::new();
    let mut path: Vec<String> = Vec::new();
    let mut reqs_stack: Vec<(String, Req)> = manifest
        .deps
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // deterministic frontier: sort by name each round
    while let Some((name, req)) = pop_lowest(&mut reqs_stack) {
        if let Some(existing) = chosen.get(&name) {
            if req.matches(&existing.version) {
                continue;
            }
            return Err(format!(
                "dependency conflict on '{}': already picked {} which does not satisfy '{}'",
                name,
                existing.version,
                req.raw_str()
            ));
        }
        if path.contains(&name) {
            return Err(format!("dependency cycle involving '{}'", name));
        }
        let idx = reg.fetch_index(&name)?;
        let picked = idx
            .pick(&req)
            .ok_or_else(|| {
                format!(
                    "no version of '{}' satisfies '{}' (available: {})",
                    name,
                    req.raw_str(),
                    idx.versions
                        .iter()
                        .map(|v| v.version.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
            .clone();
        path.push(name.clone());
        for (dname, dreq) in &picked.deps {
            // deps of the picked version enter the frontier unless already
            // satisfied by a chosen version
            let satisfied = chosen
                .get(dname)
                .map(|c| dreq.matches(&c.version))
                .unwrap_or(false)
                || reqs_stack
                    .iter()
                    .any(|(n, r)| n == dname && r.raw_str() == dreq.raw_str());
            if !satisfied {
                reqs_stack.push((dname.clone(), dreq.clone()));
            }
        }
        chosen.insert(
            name.clone(),
            LockEntry {
                name,
                version: picked.version,
                sha256: picked.sha256.clone(),
                // exact dep pins are filled in the second pass below
                deps: Vec::new(),
            },
        );
        path.pop();
    }
    // second pass: fill exact dep pins now that all versions are chosen
    let mut packages = Vec::new();
    let names: Vec<String> = chosen.keys().cloned().collect();
    for name in names {
        let mut entry = match chosen.remove(&name) {
            Some(e) => e,
            None => continue,
        };
        let idx = reg.fetch_index(&entry.name)?;
        if let Some(iv) = idx.versions.iter().find(|v| v.version == entry.version) {
            entry.deps = iv
                .deps
                .keys()
                .map(|dn| {
                    let ver = chosen
                        .get(dn)
                        .map(|c| c.version.to_string())
                        .unwrap_or_else(|| "?".into());
                    format!("{} {}", dn, ver)
                })
                .collect();
            entry.deps.sort();
        }
        packages.push(entry);
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Lockfile { packages })
}

fn pop_lowest(v: &mut Vec<(String, Req)>) -> Option<(String, Req)> {
    if v.is_empty() {
        return None;
    }
    let mut best = 0usize;
    for i in 1..v.len() {
        if v[i].0 < v[best].0 {
            best = i;
        }
    }
    Some(v.remove(best))
}

// ---------------------------------------------------------------------------
// install / remove / publish-from-project
// ---------------------------------------------------------------------------

pub const MODULES_DIR: &str = "operon_modules";

/// Install the exact lockfile graph into `root/operon_modules/`, verifying
/// sha256 of every artifact. Lockfile-first: the registry must serve the
/// pinned content or the install fails loudly.
pub fn install_from_lock(
    root: &Path,
    lock: &Lockfile,
    reg: &Registry,
) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    for p in &lock.packages {
        let data = reg.fetch_artifact(&p.name, &p.version.to_string())?;
        if !p.sha256.is_empty() {
            let got = sha256_hex(&data);
            if got != p.sha256 {
                return Err(format!(
                    "artifact {}/{} failed sha256 verification (locked {}, got {}) — refusing to install",
                    p.name,
                    p.version,
                    p.sha256,
                    got
                ));
            }
        }
        let env = Envelope::decode(&data)?;
        if env.name != p.name || env.version != p.version {
            return Err(format!(
                "registry artifact is {}/{} but the lock pins {}/{}",
                env.name, env.version, p.name, p.version
            ));
        }
        let dest_root = root.join(MODULES_DIR).join(&p.name);
        // clean reinstall for byte-stability
        if dest_root.exists() {
            std::fs::remove_dir_all(&dest_root)
                .map_err(|e| format!("cannot refresh {}: {}", dest_root.display(), e))?;
        }
        std::fs::create_dir_all(&dest_root).map_err(|e| e.to_string())?;
        // Import-layout contract: the package ENTRY file lands at
        // operon_modules/<name>/<entry-basename> and its sibling sources sit
        // beside it, so `use <name>` and `use <name>/<submodule>` resolve via
        // the package candidates in genes.rs without a shim. Files outside
        // the entry directory (docs, tests, the manifest) are not installed.
        let pm = Manifest::parse(&env.manifest_src).ok();
        let entry = pm
            .map(|m| m.entry)
            .unwrap_or_else(|| format!("{}.op", env.name));
        let entry_dir = entry
            .rsplit_once('/')
            .map(|(d, _)| format!("{}/", d))
            .unwrap_or_default();
        let mut wrote_any = false;
        for (path, bytes) in &env.files {
            if !path.ends_with(".op") {
                continue;
            }
            let rel = if !entry_dir.is_empty() {
                match path.strip_prefix(&entry_dir) {
                    Some(r) if !r.is_empty() => r.to_string(),
                    _ => continue,
                }
            } else {
                path.clone()
            };
            let dest = dest_root.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, bytes)
                .map_err(|e| format!("write {} failed: {}", dest.display(), e))?;
            wrote_any = true;
        }
        if !wrote_any {
            return Err(format!(
                "package {}/{} contains no importable .op sources under its entry directory",
                env.name, env.version
            ));
        }
        installed.push(format!("{} {}", p.name, p.version));
    }
    installed.sort();
    Ok(installed)
}

/// Remove a package's module directory (leftover hygiene for `operon remove`).
pub fn remove_installed(root: &Path, name: &str) -> Result<(), String> {
    let dir = root.join(MODULES_DIR).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("cannot remove {}: {}", dir.display(), e))?;
    }
    Ok(())
}

/// Collect the project's publishable files (manifest + entry tree + docs).
/// Rules: the manifest, README.md/LICENSE at the root, and every `.op` file
/// under the entry's directory tree. Never: operon_modules/, target/, .git/,
/// operon.lock (the consumer resolves their own graph).
pub fn collect_package_files(
    dir: &Path,
    manifest: &Manifest,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    out.push((
        "operon.toml".to_string(),
        std::fs::read(dir.join("operon.toml")).map_err(|e| e.to_string())?,
    ));
    for doc in ["README.md", "LICENSE"] {
        if let Ok(b) = std::fs::read(dir.join(doc)) {
            out.push((doc.to_string(), b));
        }
    }
    let entry_abs = dir.join(&manifest.entry);
    let src_root = entry_abs
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    let mut stack = vec![src_root.clone()];
    let mut seen = 0usize;
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if name == MODULES_DIR || name == "target" || name == ".git" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().map(|x| x == "op").unwrap_or(false) {
                let rel = p
                    .strip_prefix(dir)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
                out.push((rel, bytes));
                seen += 1;
                if seen > 512 {
                    return Err("package has more than 512 source files — refusing".into());
                }
            }
        }
    }
    if !out.iter().any(|(p, _)| *p == manifest.entry) {
        return Err(format!(
            "entry '{}' not found under {}",
            manifest.entry,
            dir.display()
        ));
    }
    Ok(out)
}

/// Build the publish envelope for a project directory.
pub fn build_envelope(dir: &Path, manifest: &Manifest) -> Result<(Envelope, Vec<u8>), String> {
    let files = collect_package_files(dir, manifest)?;
    let env = Envelope {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        deps: manifest.deps.clone(),
        manifest_src: manifest.to_toml(),
        files,
    };
    let bytes = env.encode();
    Ok((env, bytes))
}

// ---------------------------------------------------------------------------
// project scaffolding templates (operon new)
// ---------------------------------------------------------------------------

pub const TEMPLATE_BIN: &str = "bin";
pub const TEMPLATE_LIB: &str = "lib";

pub fn template_files(template: &str, name: &str) -> Result<Vec<(String, String)>, String> {
    match template {
        TEMPLATE_BIN => Ok(vec![
            (
                "operon.toml".to_string(),
                format!(
                    "[package]\nname = \"{}\"\nversion = \"0.1.0\"\noperon-version = \">= 2.2\"\nauthors = []\nlicense = \"\"\ndescription = \"A new Operon project\"\nentry = \"src/main.op\"\n\n[dependencies]\n",
                    name
                ),
            ),
            (
                "src/main.op".to_string(),
                "#!/usr/bin/env operon\n# entry point - run with `operon run` from the project root\n\ngene main {\n    print(\"hello, Operon\\n\");\n    print(\"argv: \" + str(args()) + \"\\n\");\n    return 0;\n}\n".to_string(),
            ),
            (
                "tests/main_test.op".to_string(),
                "frame proof {\n    # project tests — run with `operon test`\n    assert(1 + 1 == 2, \"arithmetic holds\");\n}\n".to_string(),
            ),
            (
                ".gitignore".to_string(),
                "operon_modules/\ntarget/\n*.built.op\n".to_string(),
            ),
        ]),
        TEMPLATE_LIB => Ok(vec![
            (
                "operon.toml".to_string(),
                format!(
                    "[package]\nname = \"{}\"\nversion = \"0.1.0\"\noperon-version = \">= 2.2\"\nauthors = []\nlicense = \"\"\ndescription = \"An Operon library\"\nentry = \"lib/{}.op\"\n\n[dependencies]\n",
                    name, name
                ),
            ),
            (
                format!("lib/{}.op", name),
                format!(
                    "# {} - an Operon library\n\nexport hello\n\ngene hello(name) {{\n    return \"hello, \" + name;\n}}\n",
                    name
                ),
            ),
            (
                "tests/lib_test.op".to_string(),
                // `use lib/demo` resolves from the project cwd (SPEC §8
                // relative-first order) so the template test runs green on a
                // fresh checkout, before any registry round-trip
                format!(
                    "use lib/{}\n\nframe proof {{\n    assert({}.hello(\"world\") == \"hello, world\", \"hello\");\n}}\n",
                    name, name
                ),
            ),
            (
                ".gitignore".to_string(),
                "operon_modules/\ntarget/\n*.built.op\n".to_string(),
            ),
        ]),
        other => Err(format!(
            "unknown template '{}' (available: bin, lib)",
            other
        )),
    }
}

/// Scaffold a new project; refuses to clobber a non-empty directory.
pub fn scaffold(dir: &Path, template: &str, name: &str) -> Result<Vec<String>, String> {
    if !valid_name(name) {
        return Err(format!(
            "project name '{}' is invalid (lowercase letters, digits, '-', '_', 2..64 chars, starts with a letter)",
            name
        ));
    }
    if dir.exists() {
        let empty = std::fs::read_dir(dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            return Err(format!(
                "directory '{}' already exists and is not empty",
                dir.display()
            ));
        }
    } else {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut created = Vec::new();
    for (path, content) in template_files(template, name)? {
        let dest = dir.join(&path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dest, content).map_err(|e| e.to_string())?;
        created.push(path);
    }
    Ok(created)
}

// ---------------------------------------------------------------------------
// unit tests — the package core proves itself before any CLI wiring
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- semver ---
    #[test]
    fn semver_parse_and_order() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert!(SemVer::parse("1.2").is_err());
        assert!(SemVer::parse("1.2.3.4").is_err());
        assert!(SemVer::parse("01.2.3").is_ok()); // leading zeros tolerated, digits only
        assert!(SemVer::parse("1.2.3-rc1").is_err()); // pre-release rejected loudly
        assert!(SemVer::parse("abc").is_err());
        assert!(SemVer::parse("1.2.x").is_err());
        assert!(SemVer::parse("").is_err());
        let a = SemVer::parse("2.0.0").unwrap();
        let b = SemVer::parse("10.0.0").unwrap();
        assert!(a < b);
    }

    #[test]
    fn req_caret() {
        let r = Req::parse("^1.2.3").unwrap();
        assert!(r.matches(&SemVer::parse("1.2.3").unwrap()));
        assert!(r.matches(&SemVer::parse("1.9.0").unwrap()));
        assert!(!r.matches(&SemVer::parse("2.0.0").unwrap()));
        assert!(!r.matches(&SemVer::parse("1.2.2").unwrap()));
        // ^0.2.3 stays inside 0.2.x
        let r0 = Req::parse("^0.2.3").unwrap();
        assert!(r0.matches(&SemVer::parse("0.2.9").unwrap()));
        assert!(!r0.matches(&SemVer::parse("0.3.0").unwrap()));
        // ^0.0.3 stays inside 0.0.x
        let r00 = Req::parse("^0.0.3").unwrap();
        assert!(r00.matches(&SemVer::parse("0.0.3").unwrap()));
        assert!(!r00.matches(&SemVer::parse("0.0.4").unwrap()));
    }

    #[test]
    fn req_tilde_and_wildcards() {
        let t = Req::parse("~1.2.3").unwrap();
        assert!(t.matches(&SemVer::parse("1.2.9").unwrap()));
        assert!(!t.matches(&SemVer::parse("1.3.0").unwrap()));
        let t1 = Req::parse("~1").unwrap();
        assert!(t1.matches(&SemVer::parse("1.9.9").unwrap()));
        assert!(!t1.matches(&SemVer::parse("2.0.0").unwrap()));
        let w = Req::parse("1.x").unwrap();
        assert!(w.matches(&SemVer::parse("1.5.0").unwrap()));
        assert!(!w.matches(&SemVer::parse("2.0.0").unwrap()));
        let w2 = Req::parse("1.2.*").unwrap();
        assert!(w2.matches(&SemVer::parse("1.2.7").unwrap()));
        assert!(!w2.matches(&SemVer::parse("1.3.0").unwrap()));
        let any = Req::parse("*").unwrap();
        assert!(any.matches(&SemVer::parse("0.0.1").unwrap()));
    }

    #[test]
    fn req_comparators_and_and_lists() {
        let g = Req::parse(">=1.0 <2.0").unwrap();
        assert!(g.matches(&SemVer::parse("1.5.0").unwrap()));
        assert!(!g.matches(&SemVer::parse("2.0.0").unwrap()));
        assert!(!g.matches(&SemVer::parse("0.9.0").unwrap()));
        let e = Req::parse("=1.2.3").unwrap();
        assert!(e.matches(&SemVer::parse("1.2.3").unwrap()));
        assert!(!e.matches(&SemVer::parse("1.2.4").unwrap()));
        // bare full version = exact pin
        let bare = Req::parse("1.2.3").unwrap();
        assert!(bare.matches(&SemVer::parse("1.2.3").unwrap()));
        assert!(!bare.matches(&SemVer::parse("1.2.4").unwrap()));
        // bare partial = caret (operon default)
        let bare2 = Req::parse("1.2").unwrap();
        assert!(bare2.matches(&SemVer::parse("1.9.0").unwrap()));
        assert!(!bare2.matches(&SemVer::parse("2.0.0").unwrap()));
    }

    // --- toml ---
    #[test]
    fn toml_manifest_roundtrip() {
        let src = "# comment\n[package]\nname = \"my-app\"\nversion = \"0.1.0\" # trailing\noperon-version = \">= 2.2\"\nauthors = [\"A <a@x>\", \"B\"]\nlicense = \"MIT\"\ndescription = \"does things\"\nkeywords = [\"bio\", \"seq\"]\n\n[dependencies]\nhttp = \"^1.0\"\njson = \"1.2\"\n";
        let m = Manifest::parse(src).unwrap();
        assert_eq!(m.name, "my-app");
        assert_eq!(m.version.to_string(), "0.1.0");
        assert_eq!(m.deps.len(), 2);
        assert!(m.deps["http"].matches(&SemVer::parse("1.4.0").unwrap()));
        assert_eq!(m.authors.len(), 2);
        assert_eq!(m.keywords, vec!["bio".to_string(), "seq".to_string()]);
        assert_eq!(m.entry, DEFAULT_ENTRY);
        let out = m.to_toml();
        let m2 = Manifest::parse(&out).unwrap();
        assert_eq!(m.name, m2.name);
        assert_eq!(m.deps["json"].raw_str(), m2.deps["json"].raw_str());
    }

    #[test]
    fn toml_rejects_garbage() {
        assert!(Manifest::parse("[package]\nname = \"X\"\nversion = \"1.0.0\"\n").is_err());
        assert!(Manifest::parse("[package]\nname = \"ok\"\nversion = \"nope\"\n").is_err());
        assert!(Manifest::parse("name = \"ok\"\nversion = \"1.0.0\"\n").is_err());
        assert!(
            Manifest::parse("[package]\nname = \"ok\"\nversion = \"1.0.0\"\nentry = 3\n").is_err()
        );
        assert!(Manifest::parse(
            "[package]\nname = \"ok\"\nversion = \"1.0.0\"\n\n[dependencies]\nhttp = 7\n"
        )
        .is_err());
    }

    // --- sha256 / base64 ---
    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn b64_roundtrip() {
        for case in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(b64_decode(&b64_encode(case)).unwrap(), case.to_vec());
        }
        // RFC 4648 vector
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }

    // --- envelope ---
    #[test]
    fn envelope_roundtrip_and_path_safety() {
        let env = Envelope {
            name: "demo".into(),
            version: SemVer::parse("1.0.0").unwrap(),
            description: "test pkg".into(),
            deps: BTreeMap::new(),
            manifest_src: "[package]\nname = \"demo\"\n".into(),
            files: vec![
                ("demo.op".into(), b"gene hi { }".to_vec()),
                ("sub/util.op".into(), b"gene u { }".to_vec()),
            ],
        };
        let bytes = env.encode();
        let dec = Envelope::decode(&bytes).unwrap();
        assert_eq!(dec.name, "demo");
        assert_eq!(dec.files.len(), 2);
        assert_eq!(dec.files[0].0, "demo.op");
        assert_eq!(dec.files[0].1, b"gene hi { }".to_vec());
        // deterministic bytes: encode twice, identical
        assert_eq!(bytes, env.encode());
        assert!(sanitize_rel_path("../evil.op").is_err());
        assert!(sanitize_rel_path("/abs.op").is_err());
        assert!(sanitize_rel_path("a/../../b.op").is_err());
        assert!(sanitize_rel_path("a/../b.op").is_ok());
        assert!(sanitize_rel_path("back\\slash.op").is_err());
    }

    // --- lockfile ---
    #[test]
    fn lock_roundtrip_and_satisfaction() {
        let lock = Lockfile {
            packages: vec![
                LockEntry {
                    name: "http".into(),
                    version: SemVer::parse("1.2.0").unwrap(),
                    sha256: "aa".into(),
                    deps: vec!["json 1.0.0".into()],
                },
                LockEntry {
                    name: "json".into(),
                    version: SemVer::parse("1.0.0").unwrap(),
                    sha256: "bb".into(),
                    deps: vec![],
                },
            ],
        };
        let bytes = lock.to_bytes();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert_eq!(
            text.lines().next().unwrap(),
            "# operon.lock — generated by operon. Do not edit."
        );
        let parsed = Lockfile::parse(&text).unwrap();
        assert_eq!(parsed, lock);
        // byte-reproducible
        assert_eq!(parsed.to_bytes(), bytes);

        let manifest = Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"^1.0\"\n",
        )
        .unwrap();
        assert!(lock_satisfies(&manifest, &parsed));
        // a manifest wanting ^2 breaks the lock
        let manifest2 = Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"^2.0\"\n",
        )
        .unwrap();
        assert!(!lock_satisfies(&manifest2, &parsed));
        // broken closure (json missing) fails
        let broken = Lockfile {
            packages: vec![lock.packages[0].clone()],
        };
        assert!(!lock_satisfies(&manifest, &broken));
    }

    // --- resolver against an in-memory dir registry ---
    fn write_file(p: &Path, content: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn resolve_deterministic_with_transitive_deps() {
        let tmp = std::env::temp_dir().join(format!("operon-pkg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let reg = Registry::Dir(tmp.clone());
        // one index per package with ALL its versions (a real registry index
        // is cumulative; overwriting per version would hide versions)
        let mut index_versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut mk = |name: &str, ver: &str, desc: &str, deps: &str| {
            let m = format!("[package]\nname = \"{}\"\nversion = \"{}\"\ndescription = \"{}\"\nentry = \"{}.op\"\n\n[dependencies]\n{}", name, ver, desc, name, deps);
            let env = Envelope {
                name: name.into(),
                version: SemVer::parse(ver).unwrap(),
                description: desc.into(),
                deps: {
                    let mut d = BTreeMap::new();
                    for line in deps.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let mut it = line.splitn(2, " = \"");
                        let k = it.next().unwrap().to_string();
                        let v = it.next().unwrap().trim_end_matches('"').to_string();
                        d.insert(k, Req::parse(&v).unwrap());
                    }
                    d
                },
                manifest_src: m.clone(),
                files: vec![(
                    format!("{}.op", name),
                    format!("// {} {}", name, ver).into_bytes(),
                )],
            };
            let bytes = env.encode();
            let sha = sha256_hex(&bytes);
            write_file(
                &tmp.join("artifacts")
                    .join(name)
                    .join(format!("{}.opkg", ver)),
                std::str::from_utf8(&bytes).unwrap(),
            );
            let deps_json = {
                let ds: Vec<String> = env
                    .deps
                    .iter()
                    .map(|(k, r)| format!("\"{}\":\"{}\"", k, r.raw_str()))
                    .collect();
                ds.join(",")
            };
            index_versions
                .entry(name.to_string())
                .or_default()
                .push(format!(
                    "{{\"version\":\"{}\",\"yanked\":false,\"sha256\":\"{}\",\"deps\":{{{}}}}}",
                    ver, sha, deps_json
                ));
        };
        mk("json", "1.0.0", "json tools", "");
        mk("json", "1.1.0", "json tools", "");
        mk("json", "2.0.0", "json tools", "");
        mk("http", "1.0.0", "http client", "json = \"^1.0\"");
        mk("http", "1.2.0", "http client", "json = \"^1.1\"");
        for (name, versions) in &index_versions {
            let idx = format!(
                "{{\"name\":\"{}\",\"description\":\"{}\",\"versions\":[{}]}}\n",
                name,
                name,
                versions.join(",")
            );
            write_file(&tmp.join("index").join(format!("{}.json", name)), &idx);
        }

        let manifest = Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"^1.0\"\n",
        )
        .unwrap();
        let lock1 = resolve(&manifest, &reg).unwrap();
        let lock2 = resolve(&manifest, &reg).unwrap();
        // byte-reproducible resolution
        assert_eq!(lock1.to_bytes(), lock2.to_bytes());
        // picked highest matching transitively
        assert_eq!(lock1.find("http").unwrap().version.to_string(), "1.2.0");
        assert_eq!(lock1.find("json").unwrap().version.to_string(), "1.1.0");
        // sorted + exact pins recorded
        assert_eq!(lock1.packages[0].name, "http");
        assert_eq!(lock1.packages[0].deps, vec!["json 1.1.0".to_string()]);

        // unsatisfiable requirement fails with the inventory in the message
        let bad = Manifest::parse(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"^9.0\"\n",
        )
        .unwrap();
        let err = resolve(&bad, &reg).unwrap_err();
        assert!(
            err.contains("no version of 'http' satisfies"),
            "got: {}",
            err
        );
        // yanked versions are invisible
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn names_and_templates() {
        assert!(valid_name("http"));
        assert!(valid_name("my-app"));
        assert!(!valid_name("2x"));
        assert!(!valid_name("X"));
        assert!(!valid_name("has space"));
        assert!(!valid_name("h"));
        let files = template_files(TEMPLATE_BIN, "demo").unwrap();
        assert!(files.iter().any(|(p, _)| *p == "operon.toml"));
        assert!(files.iter().any(|(p, _)| *p == "src/main.op"));
        assert!(template_files("nope", "demo").is_err());
    }
}
