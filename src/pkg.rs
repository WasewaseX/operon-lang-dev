//! pkg.rs, the package system (W19/W20/W23, ROADMAP-100).
//!
//! Scope-honest design, zero new dependencies:
//! - `operon mod init`    → writes `operon.toml` ([package] name/version)
//! - `operon mod add URL` → clones the dep (git CLI, shallow), pins the rev,
//!   vendors it under `~/.operon/deps/<name>-<rev>`
//! - `operon mod remove`  → drops the dep from manifest + lockfile
//! - `operon mod update`  → re-resolves every dep (rev if pinned, else HEAD)
//! - `operon mod install` → materializes every lockfile entry into the cache
//!   (offline whenever the cache is warm)
//! - `operon mod tree`    → prints the resolved dependency tree
//! - `operon.lock` (W23)  → resolved revs + content checksums; `--locked`
//!   fails on manifest↔lock drift
//!
//! The manifest is a DELIBERATE MINIMAL TOML SUBSET (the W22 rule: `.cell`
//! is runtime configuration and `operon.toml` is package metadata, the two
//! never merge). Deterministic resolution: deps are walked in sorted name
//! order, revs are pinned, and the checksum is a content digest of the
//! checkout (sha256 over a sorted (path, file-hash) manifest) so the same
//! rev always yields the same lockfile line on every machine.
//!
//! The checksum is implemented in-house (standard FIPS 180-4 constants, no
//! crates) because the tree carries zero runtime dependencies by policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- sha256

/// Standard SHA-256 (FIPS 180-4). Used ONLY as a content fingerprint for
/// lockfile pinning, not a security primitive (the capability sandbox is).
pub fn sha256_hex(data: &[u8]) -> String {
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
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|x| format!("{:08x}", x)).collect()
}

fn sha256_file(p: &Path) -> String {
    match std::fs::read(p) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------- manifest

#[derive(Debug, Clone)]
pub struct DepSpec {
    pub git: String,
    pub rev: Option<String>,
}

#[derive(Debug, Default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub operon_version: String,
    /// insertion-ordered; resolution sorts by name for determinism
    pub deps: Vec<(String, DepSpec)>,
}

impl Manifest {
    pub fn find_dep(&self, name: &str) -> Option<&DepSpec> {
        self.deps.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }
}

/// Minimal TOML subset parser for `operon.toml`. Accepts exactly the shape
/// `operon mod init` writes plus the `[deps]` table:
/// ```toml
/// [package]
/// name = "my-app"
/// version = "0.1.0"
/// operon-version = "2.2"
///
/// [deps]
/// my-lib = { git = "https://…", rev = "abc123" }
/// other  = { git = "file:///…" }
/// ```
/// Anything outside this subset is reported as an error line (never
/// silently ignored, a manifest that lies would corrupt the lockfile).
pub fn parse_manifest(src: &str) -> Result<Manifest, String> {
    let mut m = Manifest::default();
    let mut section = String::new();
    for (idx, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let (key, val) = split_kv(line, idx)?;
        match (section.as_str(), key.as_str()) {
            ("package", "name") => m.name = unquote(&val)?,
            ("package", "version") => m.version = unquote(&val)?,
            ("package", "operon-version") => m.operon_version = unquote(&val)?,
            ("deps", dep) => {
                let dep = dep.to_string();
                let git = extract_field(&val, "git", idx)?;
                let rev = extract_field(&val, "rev", idx).ok();
                m.deps.push((dep, DepSpec { git, rev }));
            }
            ("", k) => {
                return Err(format!(
                    "operon.toml line {}: '{}' outside a [package]/[deps] table",
                    idx + 1,
                    k
                ))
            }
            _ => {
                return Err(format!(
                    "operon.toml line {}: unknown key '{}' in [{}]",
                    idx + 1,
                    key,
                    section
                ))
            }
        }
    }
    if m.name.is_empty() {
        return Err("operon.toml: [package] name is missing".to_string());
    }
    Ok(m)
}

fn split_kv(line: &str, idx: usize) -> Result<(String, String), String> {
    let pos = line
        .find('=')
        .ok_or_else(|| format!("operon.toml line {}: expected 'key = value'", idx + 1))?;
    Ok((
        line[..pos].trim().trim_matches('"').to_string(),
        line[pos + 1..].trim().to_string(),
    ))
}

fn unquote(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
        Ok(t[1..t.len() - 1].to_string())
    } else {
        Err(format!("expected a quoted string, got '{}'", t))
    }
}

fn extract_field(val: &str, field: &str, idx: usize) -> Result<String, String> {
    let needle = format!("{} =", field);
    let alt = format!("{}=", field);
    let start = val
        .find(&needle)
        .or_else(|| val.find(&alt))
        .ok_or_else(|| {
            format!(
                "operon.toml line {}: dep field '{}' missing",
                idx + 1,
                field
            )
        })?;
    let rest = &val[start..];
    let eq = rest.find('=').unwrap();
    let after = rest[eq + 1..].trim();
    let quote = after.find('"').ok_or_else(|| {
        format!(
            "operon.toml line {}: '{}' needs a quoted value",
            idx + 1,
            field
        )
    })?;
    let rest2 = &after[quote + 1..];
    let end = rest2
        .find('"')
        .ok_or_else(|| format!("operon.toml line {}: unterminated string", idx + 1))?;
    Ok(rest2[..end].to_string())
}

pub fn emit_manifest(m: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{}\"\n", m.name));
    out.push_str(&format!("version = \"{}\"\n", m.version));
    if !m.operon_version.is_empty() {
        out.push_str(&format!("operon-version = \"{}\"\n", m.operon_version));
    }
    out.push('\n');
    if !m.deps.is_empty() {
        out.push_str("[deps]\n");
        let mut sorted = m.deps.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, d) in &sorted {
            match &d.rev {
                Some(r) => out.push_str(&format!(
                    "{} = {{ git = \"{}\", rev = \"{}\" }}\n",
                    name, d.git, r
                )),
                None => out.push_str(&format!("{} = {{ git = \"{}\" }}\n", name, d.git)),
            }
        }
    }
    out
}

// ---------------------------------------------------------------- lockfile

#[derive(Debug, Clone)]
pub struct LockEntry {
    pub name: String,
    pub git: String,
    pub rev: String,
    /// content digest of the vendored checkout (sha256 of the file manifest)
    pub checksum: String,
}

/// The lockfile is a deterministic, human-readable table (sorted by name).
pub fn emit_lock(entries: &BTreeMap<String, LockEntry>) -> String {
    let mut out = String::new();
    out.push_str("# operon.lock, resolved dependencies (W23).\n");
    out.push_str(
        "# Generated by `operon mod add/update/install`. Checked in; --locked fails on drift.\n",
    );
    for e in entries.values() {
        out.push_str(&format!(
            "[[dep]]\nname = \"{}\"\ngit = \"{}\"\nrev = \"{}\"\nchecksum = \"sha256:{}\"\n\n",
            e.name, e.git, e.rev, e.checksum
        ));
    }
    out
}

pub fn parse_lock(src: &str) -> Result<BTreeMap<String, LockEntry>, String> {
    let mut out = BTreeMap::new();
    let mut cur: Option<LockEntry> = None;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[dep]]" {
            if let Some(e) = cur.take() {
                out.insert(e.name.clone(), e);
            }
            cur = Some(LockEntry {
                name: String::new(),
                git: String::new(),
                rev: String::new(),
                checksum: String::new(),
            });
            continue;
        }
        let e = match cur.as_mut() {
            Some(e) => e,
            None => continue,
        };
        let (k, v) = split_kv(line, 0)?;
        let v = unquote(&v)?;
        match k.as_str() {
            "name" => e.name = v,
            "git" => e.git = v,
            "rev" => e.rev = v,
            "checksum" => e.checksum = v.trim_start_matches("sha256:").to_string(),
            _ => return Err(format!("operon.lock: unknown key '{}'", k)),
        }
    }
    if let Some(e) = cur.take() {
        out.insert(e.name.clone(), e);
    }
    Ok(out)
}

// ---------------------------------------------------------------- checkout

pub fn deps_cache_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("OPERON_DEPS") {
        if !d.is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".operon").join("deps"))
}

pub fn cache_dir_for(name: &str, rev: &str) -> Option<PathBuf> {
    deps_cache_dir().map(|d| d.join(format!("{}-{}", name, &rev[..rev.len().min(12)])))
}

/// Content digest of a vendored checkout: sha256 over the sorted
/// `(relative path, file digest)` pairs, `.git` excluded (the rev pins the
/// tree; the digest pins the BYTES so a corrupted cache is detectable).
pub fn checkout_checksum(dir: &Path) -> String {
    let mut rows: Vec<String> = Vec::new();
    walk_files(dir, dir, &mut rows);
    rows.sort();
    rows.dedup();
    sha256_hex(rows.join("\n").as_bytes())
}

fn walk_files(root: &Path, dir: &Path, rows: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name.starts_with('#') {
                continue;
            }
            if p.is_dir() {
                walk_files(root, &p, rows);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                rows.push(format!("{}:{}", rel, sha256_file(&p)));
            }
        }
    }
}

// ---------------------------------------------------------------- registry
// W21: the cheap first registry is a static git-index file: JSON LINES,
// one flat object per line, append-only by convention (a later line for
// the same name supersedes an earlier one). No server, no crates: the
// file is shareable through git, which is the whole hosting model at
// this stage. `operon mod add NAME --registry FILE` resolves NAME through
// the index and then uses the ordinary git machinery; `operon mod publish`
// appends the caller's own package as one line.

#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub git: String,
    pub rev: String,
    pub sha256: String,
    pub description: String,
}

/// Pull one `"key": "value"` string pair out of a flat JSON object line.
/// Values must be JSON strings (no nesting, no numbers: every registry
/// field is a string). Returns None when the key is absent.
fn json_line_get(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let mut rest = line;
    loop {
        let i = rest.find(&pat)?;
        let after = &rest[i + pat.len()..];
        let after = after.trim_start();
        if !after.starts_with(':') {
            rest = after;
            continue;
        }
        let after = after[1..].trim_start();
        if !after.starts_with('"') {
            return None;
        }
        let mut out = String::new();
        let mut chars = after[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => return None,
                },
                other => out.push(other),
            }
        }
        return None;
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Parse a registry index. Line-precise rejections (a registry that lies
/// would resolve installs against the wrong code, so malformed lines are
/// hard errors, never skipped).
pub fn parse_registry(src: &str) -> Result<Vec<RegistryEntry>, String> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |why: &str| format!("registry line {}: {}", i + 1, why);
        if !line.starts_with('{') || !line.ends_with('}') {
            return Err(bad("expected a flat JSON object"));
        }
        let name = json_line_get(line, "name").ok_or_else(|| bad("missing \"name\""))?;
        if name.is_empty() {
            return Err(bad("empty \"name\""));
        }
        let git = json_line_get(line, "git").ok_or_else(|| bad("missing \"git\""))?;
        let rev = json_line_get(line, "rev").ok_or_else(|| bad("missing \"rev\""))?;
        let version = json_line_get(line, "version").unwrap_or_default();
        let sha256 = json_line_get(line, "sha256").unwrap_or_default();
        let description = json_line_get(line, "description").unwrap_or_default();
        out.push(RegistryEntry {
            name,
            version,
            git,
            rev,
            sha256,
            description,
        });
    }
    Ok(out)
}

/// Resolve `name` against a registry file. The LAST matching line wins
/// (append-only convention: republished versions land later in the file).
fn registry_lookup(path: &str, name: &str) -> RegistryEntry {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| die_pkg(&format!("cannot read registry '{}': {}", path, e)));
    let entries = parse_registry(&src).unwrap_or_else(|e| die_pkg(&e));
    let hits: Vec<&RegistryEntry> = entries.iter().filter(|e| e.name == name).collect();
    match hits.last() {
        Some(e) => (*e).clone(),
        None => die_pkg(&format!("'{}' not in registry '{}'", name, path)),
    }
}

/// Pull the `--registry FILE` flag out of a flag list (shared by publish).
fn registry_path_from(rest: &[String], start: usize) -> (String, usize) {
    let mut i = start;
    while i < rest.len() {
        if rest[i] == "--registry" {
            let p = rest
                .get(i + 1)
                .cloned()
                .unwrap_or_else(|| die_pkg("--registry needs a file path"));
            return (p, i);
        }
        i += 1;
    }
    die_pkg("this subcommand needs --registry FILE (the static git index)");
}

/// Shallow-clone `url` at `rev` (or HEAD when None) into `dest`; returns the
/// resolved full rev. Uses the git CLI (no crates, by policy).
pub fn git_checkout(url: &str, rev: Option<&str>, dest: &Path) -> Result<String, String> {
    let _ = std::fs::remove_dir_all(dest);
    let out = std::process::Command::new("git")
        .args(["clone", "--quiet", url, &dest.to_string_lossy()])
        .output()
        .map_err(|e| format!("git clone failed: {} (is git installed?)", e))?;
    if !out.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let rev_arg: [&str; 2] = ["checkout", rev.unwrap_or("HEAD")];
    let out = std::process::Command::new("git")
        .args(["-C", &dest.to_string_lossy()])
        .args(rev_arg)
        .output()
        .map_err(|e| format!("git checkout failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git checkout '{}-' failed: {}",
            rev.unwrap_or("HEAD"),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let out = std::process::Command::new("git")
        .args(["-C", &dest.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-parse failed: {}", e))?;
    if !out.status.success() {
        return Err("git rev-parse HEAD failed".to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Read the dep's OWN manifest from its checkout (for transitive closure).
pub fn manifest_in(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join("operon.toml")).ok()?;
    parse_manifest(&text).ok()
}

// ------------------------------------------------------------ CLI surface

/// Populate the interpreter's lock_dirs from the nearest operon.lock
/// (program dir first, then CWD). Missing lockfile = no roots (the
/// standard resolution chain is untouched).
pub fn apply_lock(interp: &mut crate::interp::Interp) {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(b) = &interp.base_dir {
        roots.push(PathBuf::from(b).join("operon.lock"));
    }
    roots.push(PathBuf::from("operon.lock"));
    for lock_path in roots {
        if let Ok(text) = std::fs::read_to_string(&lock_path) {
            match parse_lock(&text) {
                Ok(entries) => {
                    for (name, e) in entries {
                        if let Some(dir) = cache_dir_for(&name, &e.rev) {
                            interp
                                .lock_dirs
                                .push((name, dir.to_string_lossy().to_string()));
                        }
                    }
                    return; // nearest lockfile wins
                }
                Err(m) => {
                    interp.note(
                        0,
                        4,
                        format!(
                            "operon.lock at '{}' unreadable ({}); vendored deps ignored",
                            lock_path.to_string_lossy(),
                            m
                        ),
                    );
                }
            }
        }
    }
}

fn die_pkg(msg: &str) -> ! {
    eprintln!("operon mod: {}", msg);
    std::process::exit(2);
}

fn read_manifest() -> Manifest {
    let src = match std::fs::read_to_string("operon.toml") {
        Ok(s) => s,
        Err(_) => die_pkg(
            "no operon.toml here, run `operon mod init` first (package metadata lives in operon.toml, never in .cell)",
        ),
    };
    match parse_manifest(&src) {
        Ok(m) => m,
        Err(e) => die_pkg(&e),
    }
}

fn write_manifest(m: &Manifest) {
    std::fs::write("operon.toml", emit_manifest(m)).expect("write operon.toml");
}

fn read_or_new_lock() -> BTreeMap<String, LockEntry> {
    match std::fs::read_to_string("operon.lock") {
        Ok(text) => parse_lock(&text).unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    }
}

/// Resolve `name` at `spec` into the vendored cache; returns the LockEntry.
/// Deterministic: same rev + same bytes → same cache dir + same checksum.
fn resolve_dep(name: &str, spec: &DepSpec) -> LockEntry {
    let tmp = deps_cache_dir()
        .unwrap_or_else(|| die_pkg("cannot locate the deps cache (set HOME or OPERON_DEPS)"))
        .join("tmp-checkout");
    let rev = match git_checkout(&spec.git, spec.rev.as_deref(), &tmp) {
        Ok(r) => r,
        Err(e) => die_pkg(&format!("resolving '{}': {}", name, e)),
    };
    let checksum = checkout_checksum(&tmp);
    let dest = cache_dir_for(name, &rev).unwrap_or_else(|| die_pkg("cannot locate the deps cache"));
    let _ = std::fs::remove_dir_all(&dest);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::rename(&tmp, &dest).unwrap_or_else(|e| die_pkg(&format!("cache move failed: {}", e)));
    LockEntry {
        name: name.to_string(),
        git: spec.git.clone(),
        rev,
        checksum,
    }
}

/// Walk the dependency closure: the manifest's deps plus every dep's own
/// operon.toml deps, transitively, cycle-safe, SORTED name order so the
/// lockfile is byte-identical on every machine.
fn resolve_closure(m: &Manifest) -> BTreeMap<String, LockEntry> {
    let mut out: BTreeMap<String, LockEntry> = BTreeMap::new();
    let mut queue: Vec<(String, DepSpec)> = m.deps.clone();
    queue.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen: Vec<String> = Vec::new();
    while let Some((name, spec)) = queue.pop() {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let entry = resolve_dep(&name, &spec);
        if let Some(dir) = cache_dir_for(&name, &entry.rev) {
            if let Some(sub) = manifest_in(&dir) {
                let mut subdeps = sub.deps;
                subdeps.sort_by(|a, b| a.0.cmp(&b.0));
                queue.extend(subdeps);
            }
        }
        out.insert(name, entry);
    }
    out
}

fn check_locked(m: &Manifest) {
    // W23 --locked: manifest ↔ lockfile drift is a hard failure (CI pins it).
    // The lockfile legitimately contains TRANSITIVE deps (a dep's own
    // operon.toml [deps]), those are checked against the vendored
    // manifests, not the app manifest, so the stale rule never false-positives.
    let lock_text = match std::fs::read_to_string("operon.lock") {
        Ok(t) => t,
        Err(_) => die_pkg("--locked: operon.lock missing (run `operon mod install` first)"),
    };
    let lock = parse_lock(&lock_text).unwrap_or_else(|e| die_pkg(&e));
    for (name, spec) in &m.deps {
        match lock.get(name) {
            None => die_pkg(&format!(
                "--locked: dep '{}' missing from operon.lock",
                name
            )),
            Some(e) => {
                if e.git != spec.git {
                    die_pkg(&format!(
                        "--locked: dep '{}' git drift (lock: {})",
                        name, e.git
                    ));
                }
                if let Some(r) = &spec.rev {
                    if !e.rev.starts_with(r) {
                        die_pkg(&format!(
                            "--locked: dep '{}' rev drift (lock: {})",
                            name, e.rev
                        ));
                    }
                }
            }
        }
    }
    // transitive names: every vendored manifest's own [deps] justifies lock
    // entries (alpha's manifest lists beta, so beta is legal in the lock).
    // Collect from ALL vendored manifests, direct deps included, not just
    // the non-direct entries.
    let mut transitive: Vec<String> = Vec::new();
    for (name, e) in &lock {
        if let Some(dir) = cache_dir_for(name, &e.rev) {
            if let Some(sub) = manifest_in(&dir) {
                for (sn, _sd) in &sub.deps {
                    transitive.push(sn.clone());
                }
            }
        }
    }
    for name in lock.keys() {
        if m.deps.iter().any(|(n, _)| n == name) {
            continue;
        }
        if !transitive.contains(name) {
            die_pkg(&format!("--locked: lockfile has stale dep '{}'", name));
        }
    }
}

/// The `operon mod <subcommand>` group (W19/W20).
pub fn mod_command(rest: &[String]) -> ! {
    let sub = match rest.first() {
        Some(s) => s.as_str(),
        None => die_pkg(
            "mod needs a subcommand: init | add | remove | update | install | tree | verify",
        ),
    };
    match sub {
        "init" => {
            let name = rest.get(1).cloned().unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|d| d.file_name().map(|s| s.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "my-package".to_string())
            });
            if std::path::Path::new("operon.toml").exists() {
                die_pkg("operon.toml already exists");
            }
            let m = Manifest {
                name: name.clone(),
                version: "0.1.0".to_string(),
                operon_version: env!("CARGO_PKG_VERSION").to_string(),
                deps: Vec::new(),
            };
            write_manifest(&m);
            println!("initialized package '{}' (operon.toml)", name);
        }
        "add" => {
            let mut m = read_manifest();
            let target = rest
                .get(1)
                .cloned()
                .unwrap_or_else(|| die_pkg("add needs a git URL or a registry name"));
            // W21: `add NAME --registry FILE` resolves NAME through the
            // static git index first; everything else behaves like before.
            let mut reg_path: Option<String> = None;
            {
                let mut i = 2;
                while i < rest.len() {
                    if rest[i] == "--registry" {
                        reg_path = Some(rest.get(i + 1).cloned().unwrap_or_else(|| {
                            die_pkg("--registry needs a file path")
                        }));
                    }
                    i += 1;
                }
            }
            let url;
            let mut name;
            let mut rev: Option<String> = None;
            if let Some(rp) = &reg_path {
                let entry = registry_lookup(rp, &target);
                name = entry.name.clone();
                url = entry.git.clone();
                rev = Some(entry.rev.clone());
                if !entry.sha256.is_empty() {
                    println!(
                        "resolved '{}' {} via registry (checksum {})",
                        name,
                        entry.version,
                        &entry.sha256[..entry.sha256.len().min(12)]
                    );
                }
            } else {
                // name: derived from the URL's basename, or --as NAME
                name = target
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("dep")
                    .trim_end_matches(".git")
                    .to_string();
                url = target.clone();
            }
            let mut i = 2;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--registry" => {
                        i += 1; // consumed above
                    }
                    "--as" => {
                        i += 1;
                        name = rest
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| die_pkg("--as needs a name"));
                    }
                    "--rev" => {
                        i += 1;
                        rev = Some(
                            rest.get(i)
                                .cloned()
                                .unwrap_or_else(|| die_pkg("--rev needs a revision")),
                        );
                    }
                    other => die_pkg(&format!("unknown flag '{}'", other)),
                }
                i += 1;
            }
            if m.deps.iter().any(|(n, _)| n == &name) {
                die_pkg(&format!("dep '{}' already present (remove it first)", name));
            }
            m.deps.push((name.clone(), DepSpec { git: url, rev }));
            let lock = resolve_closure(&m);
            write_manifest(&m);
            std::fs::write("operon.lock", emit_lock(&lock)).expect("write operon.lock");
            println!("added '{}' ({} dep(s) locked)", name, lock.len());
        }
        "remove" => {
            let mut m = read_manifest();
            let name = rest
                .get(1)
                .cloned()
                .unwrap_or_else(|| die_pkg("remove needs a package name"));
            let before = m.deps.len();
            m.deps.retain(|(n, _)| n != &name);
            if m.deps.len() == before {
                die_pkg(&format!("dep '{}' not in operon.toml", name));
            }
            write_manifest(&m);
            let lock = read_or_new_lock();
            let mut lock = lock;
            lock.remove(&name);
            std::fs::write("operon.lock", emit_lock(&lock)).expect("write operon.lock");
            println!("removed '{}'", name);
        }
        "update" => {
            let m = read_manifest();
            let lock = resolve_closure(&m);
            std::fs::write("operon.lock", emit_lock(&lock)).expect("write operon.lock");
            println!("re-resolved {} dep(s)", lock.len());
        }
        "install" => {
            let m = read_manifest();
            let existing = read_or_new_lock();
            // Install from the lock when it agrees with the manifest (the
            // offline path); resolve anything missing, then re-emit.
            let mut lock = existing;
            for (name, spec) in &m.deps {
                if !lock.contains_key(name) {
                    let e = resolve_dep(name, spec);
                    lock.insert(name.clone(), e);
                }
            }
            // materialize every entry into the cache (no-op when warm)
            let missing: Vec<(String, DepSpec)> = lock
                .iter()
                .filter(|(name, e)| {
                    !cache_dir_for(name, &e.rev)
                        .map(|d| d.is_dir())
                        .unwrap_or(false)
                })
                .map(|(name, e)| {
                    (
                        name.clone(),
                        DepSpec {
                            git: e.git.clone(),
                            rev: Some(e.rev.clone()),
                        },
                    )
                })
                .collect();
            for (name, spec) in missing {
                let fresh = resolve_dep(&name, &spec);
                lock.insert(name, fresh);
            }
            std::fs::write("operon.lock", emit_lock(&lock)).expect("write operon.lock");
            println!("installed {} dep(s)", lock.len());
        }
        "publish" => {
            // W21: append this package to a static registry file as one
            // JSON line. Requires: a git repo (for the HEAD rev), a git
            // URL (--url or the 'origin' remote), and --registry FILE.
            let m = read_manifest();
            let (reg_path, _) = registry_path_from(rest, 1);
            let mut url: Option<String> = None;
            let mut desc = String::new();
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--registry" => {
                        i += 1;
                    }
                    "--url" => {
                        i += 1;
                        url = Some(
                            rest.get(i)
                                .cloned()
                                .unwrap_or_else(|| die_pkg("--url needs a git URL")),
                        );
                    }
                    "--desc" => {
                        i += 1;
                        desc = rest
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| die_pkg("--desc needs text"));
                    }
                    other => die_pkg(&format!("unknown flag '{}'", other)),
                }
                i += 1;
            }
            if url.is_none() {
                let out = std::process::Command::new("git")
                    .args(["remote", "get-url", "origin"])
                    .output();
                if let Ok(o) = out {
                    if o.status.success() {
                        url = Some(String::from_utf8_lossy(&o.stdout).trim().to_string());
                    }
                }
            }
            let url = match url {
                Some(u) => u,
                None => die_pkg("publish needs --url or a git 'origin' remote"),
            };
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output();
            let rev = match out {
                Ok(o) if o.status.success() => {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                }
                _ => die_pkg("publish needs a git repo with at least one commit"),
            };
            let checksum = checkout_checksum(std::path::Path::new("."));
            let line = format!(
                "{{\"name\": \"{}\", \"version\": \"{}\", \"git\": \"{}\", \"rev\": \"{}\", \"sha256\": \"{}\", \"description\": \"{}\"}}",
                json_escape(&m.name),
                json_escape(&m.version),
                json_escape(&url),
                json_escape(&rev),
                json_escape(&checksum),
                json_escape(&desc)
            );
            // idempotence: the same (name, version, rev) is not appended twice
            if let Ok(existing) = std::fs::read_to_string(&reg_path) {
                if let Ok(entries) = parse_registry(&existing) {
                    if entries
                        .iter()
                        .any(|e| e.name == m.name && e.version == m.version && e.rev == rev)
                    {
                        println!(
                            "{} {} already in registry at {}",
                            m.name, m.version, &rev[..rev.len().min(7)]
                        );
                        std::process::exit(0);
                    }
                }
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&reg_path)
                .unwrap_or_else(|e| die_pkg(&format!("cannot open registry '{}': {}", reg_path, e)));
            writeln!(f, "{}", line)
                .unwrap_or_else(|e| die_pkg(&format!("cannot write registry: {}", e)));
            println!(
                "published {} {} ({} rev {})",
                m.name,
                m.version,
                url,
                &rev[..rev.len().min(7)]
            );
        }
        "tree" => {
            let m = read_manifest();
            let lock = read_or_new_lock();
            println!("{} {}", m.name, m.version);
            print_tree(&m, &lock, 1, &mut Vec::new());
        }
        "verify" => {
            // W23 honesty probe: every lock entry's vendored bytes must match
            // its checksum (detects a corrupted/partial cache).
            let lock = read_or_new_lock();
            let mut bad = 0usize;
            for (name, e) in &lock {
                match cache_dir_for(name, &e.rev) {
                    Some(d) if d.is_dir() => {
                        let got = checkout_checksum(&d);
                        if got != e.checksum {
                            println!("MISMATCH {} ({} != {})", name, got, e.checksum);
                            bad += 1;
                        } else {
                            println!("ok {}", name);
                        }
                    }
                    _ => {
                        println!("MISSING {} (not installed)", name);
                        bad += 1;
                    }
                }
            }
            if bad > 0 {
                std::process::exit(1);
            }
        }
        other => die_pkg(&format!(
            "unknown subcommand '{}' (init | add | remove | update | install | tree | verify | publish)",
            other
        )),
    }
    std::process::exit(0);
}

fn print_tree(
    m: &Manifest,
    lock: &BTreeMap<String, LockEntry>,
    depth: usize,
    seen: &mut Vec<String>,
) {
    let mut names: Vec<&String> = m.deps.iter().map(|(n, _)| n).collect();
    names.sort();
    names.dedup();
    for n in names {
        let pad = "  ".repeat(depth);
        match lock.get(n) {
            Some(e) => {
                if seen.contains(n) {
                    println!("{}{} {} (…)", pad, n, &e.rev[..e.rev.len().min(7)]);
                    continue;
                }
                println!("{}{} {} {}", pad, n, &e.rev[..e.rev.len().min(7)], e.git);
                seen.push(n.clone());
                if let Some(dir) = cache_dir_for(n, &e.rev) {
                    if let Some(sub) = manifest_in(&dir) {
                        print_tree(&sub, lock, depth + 1, seen);
                    }
                }
            }
            None => println!("{}{} (unresolved, run operon mod install)", pad, n),
        }
    }
}

/// W23 `--locked`: verify the current operon.toml against operon.lock and
/// die on drift (missing dep, changed URL, stale lock line). A pinned CI
/// run refuses to execute against an unpinned dependency set.
pub fn check_locked_manifest() {
    let m = read_manifest();
    check_locked(&m);
}
