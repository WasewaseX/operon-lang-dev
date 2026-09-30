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
    /// W21-r1: default registry source for this project (a registry file
    /// path or an http(s):// index URL). Empty = the chain resolves it:
    /// OPERON_REGISTRY env, then this field, then the bundled seed index.
    pub registry: String,
    /// One-line human description (publish uses it; parse-only otherwise).
    pub description: String,
    /// insertion-ordered; resolution sorts by name for determinism
    pub deps: Vec<(String, DepSpec)>,
}

impl Manifest {
    pub fn find_dep(&self, name: &str) -> Option<&DepSpec> {
        self.deps.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    /// W20-r1: true when the manifest declares any dependency at all.
    pub fn has_deps(&self) -> bool {
        !self.deps.is_empty()
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
            ("registry", "path") => m.registry = unquote(&val)?,
            ("package", "description") => m.description = unquote(&val)?,
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
    // ast-grep-ignore: no-unwrap-in-src
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
    if !m.description.is_empty() {
        out.push_str(&format!("description = \"{}\"\n", m.description));
    }
    if !m.registry.is_empty() {
        out.push_str("[registry]\n");
        out.push_str(&format!("path = \"{}\"\n\n", m.registry));
    }
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
    /// W21-r1: a directory source (the seed registry and local dev
    /// registries publish "dir" lines; the package tree is copied into
    /// the vendored cache instead of a git clone). Empty = git-sourced.
    pub dir: String,
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
        let dir = json_line_get(line, "dir").unwrap_or_default();
        out.push(RegistryEntry {
            name,
            version,
            git,
            rev,
            sha256,
            description,
            dir,
        });
    }
    Ok(out)
}

/// Lookup over already-read registry text (URL registries are fetched
/// once per command, then parsed here). Same last-match-wins rule.
fn registry_lookup_text(text: &str, name: &str, src_label: &str) -> RegistryEntry {
    let entries = parse_registry(text).unwrap_or_else(|e| die_pkg(&e));
    let hits: Vec<&RegistryEntry> = entries.iter().filter(|e| e.name == name).collect();
    match hits.last() {
        Some(e) => (*e).clone(),
        None => die_pkg(&format!(
            "'{}' not in registry '{}' (searched with `operon add {}`)",
            name, src_label, name
        )),
    }
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
    // ast-grep-ignore: no-std-process-exit-in-core
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
/// Two source classes: `git URL` (clone, shallow, rev-pinned) and
/// `registry:NAME` (the W21-r1 registry chain; dir-sourced seed packages
/// copy into the cache, git-sourced entries clone exactly as before).
fn resolve_dep(name: &str, spec: &DepSpec) -> LockEntry {
    if let Some(reg_name) = spec.git.strip_prefix("registry:") {
        return resolve_registry_dep(name, reg_name, spec.rev.as_deref());
    }
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

/// Resolve a dep through the registry chain. When `pinned` is set (every
/// lockfile-driven path), the registry MUST still carry that rev — a
/// registry that moved on is a hard error, never a silent re-resolve
/// (W23: the lockfile is the reproducibility contract, not a suggestion).
fn resolve_registry_dep(name: &str, reg_name: &str, pinned: Option<&str>) -> LockEntry {
    let src = registry_source(None);
    let text = registry_read(&src);
    let entries = parse_registry(&text).unwrap_or_else(|e| die_pkg(&e));
    let hits: Vec<&RegistryEntry> = entries.iter().filter(|e| e.name == reg_name).collect();
    if hits.is_empty() {
        die_pkg(&format!(
            "'{}' not in registry '{}' (operon add {} would fail the same way)",
            reg_name, src, reg_name
        ));
    }
    let entry = match pinned {
        Some(p) => hits
            .iter()
            .rev()
            .find(|e| e.rev == p || e.rev.starts_with(p))
            .copied()
            .unwrap_or_else(|| {
                die_pkg(&format!(
                    "registry '{}' no longer carries '{}' at rev {} (it moved on; run `operon update` to re-resolve, or restore the registry line)",
                    src, reg_name, p
                ))
            }),
        None => hits.last().copied().unwrap(),
    };
    // SECURITY (deny-by-default): a dir-sourced entry is a LOCAL-registry
    // feature. Honoring a remote index's `dir` field would let a remote
    // registry direct this machine to copy arbitrary local directories
    // into the dep cache — a remote-controlled file copy. Remote
    // registries publish git URLs, period.
    if !entry.dir.is_empty() && (src.starts_with("http://") || src.starts_with("https://")) {
        die_pkg(&format!(
            "registry '{}' is remote but its entry for '{}' is dir-sourced; remote registries publish git URLs only (dir sources are a local-registry feature)",
            src, reg_name
        ));
    }
    let mut out = if entry.dir.is_empty() {
        resolve_dep(
            name,
            &DepSpec {
                git: entry.git.clone(),
                rev: Some(entry.rev.clone()),
            },
        )
    } else {
        let d = std::path::PathBuf::from(&entry.dir);
        if !d.is_dir() {
            die_pkg(&format!(
                "registry entry '{}' points at directory '{}' which is missing (re-materialize the registry or fix the index line)",
                reg_name,
                d.display()
            ));
        }
        resolve_dir_dep(name, &d)
    };
    if !entry.sha256.is_empty() && out.checksum != entry.sha256 {
        die_pkg(&format!(
            "checksum mismatch for '{}' (registry published {}, vendored {}): the index and the package tree disagree, refusing to lock",
            name, entry.sha256, out.checksum
        ));
    }
    // the lock records WHERE the dep came from (registry:NAME), keeping
    // the lockfile byte-stable across machines; rev+checksum pin the bytes
    out.git = format!("registry:{}", reg_name);
    out
}

/// Resolve a directory-sourced package (seed registry, local dev
/// registries): copy the tree into the vendored cache. The rev IS the
/// content: `content-` + first 16 hex of the checksum, so the same bytes
/// always land in the same cache dir on every machine. The copy is
/// verified by re-checksumming the destination.
fn resolve_dir_dep(name: &str, dir: &Path) -> LockEntry {
    let checksum = checkout_checksum(dir);
    let rev = format!("content-{}", &checksum[..checksum.len().min(16)]);
    let dest = cache_dir_for(name, &rev).unwrap_or_else(|| die_pkg("cannot locate the deps cache"));
    let _ = std::fs::remove_dir_all(&dest);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| die_pkg(&format!("cache mkdir failed: {}", e)));
    }
    copy_tree(dir, &dest).unwrap_or_else(|e| die_pkg(&format!("cache copy failed: {}", e)));
    let got = checkout_checksum(&dest);
    if got != checksum {
        die_pkg(&format!(
            "vendored copy of '{}' does not match its source ({} != {}): refusing to lock",
            name, got, checksum
        ));
    }
    LockEntry {
        name: name.to_string(),
        git: format!("registry:{}", name),
        rev,
        checksum: got,
    }
}

/// Recursively copy a package tree (files + dirs, `.git` skipped). Used
/// by the dir-source resolution path; byte-exact by construction (the
/// caller re-checksums the destination).
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {}", dst.display(), e))?;
        let entries =
            std::fs::read_dir(src).map_err(|e| format!("readdir {}: {}", src.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry: {}", e))?;
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if name == ".git" || name == ".gitignore" {
                continue;
            }
            let s = src.join(&name);
            let d = dst.join(&name);
            if s.is_dir() {
                copy_tree(&s, &d)?;
            } else {
                std::fs::copy(&s, &d).map_err(|e| format!("copy {}: {}", s.display(), e))?;
            }
        }
        Ok(())
    } else {
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|e| format!("copy {}: {}", src.display(), e))
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
                registry: String::new(),
                description: String::new(),
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
            {
                // W21-r1: `add NAME` resolves through the registry chain
                // (explicit --registry, OPERON_REGISTRY, the manifest's
                // [registry] path, then the bundled seed index). A bare
                // git URL is still accepted: anything that is not a
                // legal package name is treated as a URL.
                let looks_like_name = valid_pkg_name(&target)
                    && !target.contains('/')
                    && !target.contains(':')
                    && !target.contains('.');
                if looks_like_name || reg_path.is_some() {
                    let src = registry_source(reg_path.as_deref());
                    let text = registry_read(&src);
                    let entry = registry_lookup_text(&text, &target, &src);
                    name = entry.name.clone();
                    url = format!("registry:{}", entry.name);
                    rev = Some(entry.rev.clone());
                    let source_note = if entry.dir.is_empty() {
                        "git"
                    } else {
                        "dir"
                    };
                    println!(
                        "resolved '{}' {} via registry {} ({} source, checksum {})",
                        name,
                        entry.version,
                        src,
                        source_note,
                        &entry.sha256[..entry.sha256.len().min(12)]
                    );
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
            // URL (--url or the 'origin' remote), and a WRITABLE registry
            // target (--registry FILE, OPERON_REGISTRY, or the manifest's
            // [registry] path — the bundled seed index is not publishable,
            // it is rebuilt from embedded bytes on toolchain upgrade, and
            // a remote index URL has no append semantics over HTTP).
            let m = read_manifest();
            let mut reg_path: Option<String> = None;
            {
                let mut i = 1;
                while i < rest.len() {
                    if rest[i] == "--registry" {
                        reg_path = Some(
                            rest.get(i + 1)
                                .cloned()
                                .unwrap_or_else(|| die_pkg("--registry needs a file path")),
                        );
                    }
                    i += 1;
                }
            }
            let reg_path = match reg_path {
                Some(r) => r,
                None => {
                    let env_reg =
                        std::env::var("OPERON_REGISTRY").ok().filter(|s| !s.trim().is_empty());
                    let man_reg = manifest_in(Path::new("."))
                        .map(|mm| mm.registry)
                        .filter(|s| !s.is_empty());
                    match env_reg.or(man_reg) {
                        Some(s) => s,
                        None => die_pkg(
                            "publish needs --registry FILE, OPERON_REGISTRY, or [registry] path in operon.toml (the bundled seed index is not publishable)",
                        ),
                    }
                }
            };
            if reg_path.starts_with("http://") || reg_path.starts_with("https://") {
                die_pkg(
                    "publish cannot append to a remote index URL — point --registry at a writable checkout of the registry file (git is the transport)",
                );
            }
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
                        // ast-grep-ignore: no-std-process-exit-in-core
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
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(1);
            }
        }
        other => die_pkg(&format!(
            "unknown subcommand '{}' (init | add | remove | update | install | tree | verify | publish)",
            other
        )),
    }
    // ast-grep-ignore: no-std-process-exit-in-core
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

// ------------------------------------------------------- W21-r1: the chain

/// The seed package table: embedded at compile time, materialized under
/// the registry home on first use. Embedded strings mean `operon add
/// http` works offline on a fresh machine with zero network — the
/// mainstream first-run experience, byte-identical everywhere.
struct SeedPkg {
    name: &'static str,
    version: &'static str,
    description: &'static str,
    files: &'static [(&'static str, &'static str)],
}

const SEED_PACKAGES: &[SeedPkg] = &[
    SeedPkg {
        name: "http",
        version: "0.1.0",
        description: "HTTP envelope toolkit: request/response, headers, query, cookies, URL join",
        files: &[
            (
                "operon.toml",
                include_str!("../registry/packages/http/operon.toml"),
            ),
            ("http.op", include_str!("../registry/packages/http/http.op")),
            (
                "test_http.op",
                include_str!("../registry/packages/http/test_http.op"),
            ),
        ],
    },
    SeedPkg {
        name: "json",
        version: "0.1.0",
        description:
            "JSON toolkit: canonical form, RFC 6901 pointers, schema-lite, JSON Lines, diff",
        files: &[
            (
                "operon.toml",
                include_str!("../registry/packages/json/operon.toml"),
            ),
            ("json.op", include_str!("../registry/packages/json/json.op")),
            (
                "test_json.op",
                include_str!("../registry/packages/json/test_json.op"),
            ),
        ],
    },
    SeedPkg {
        name: "postgres",
        version: "0.1.0",
        description:
            "PostgreSQL text layer: DSN parse/build, quoting, placeholders, arrays, LIMIT guard",
        files: &[
            (
                "operon.toml",
                include_str!("../registry/packages/postgres/operon.toml"),
            ),
            (
                "postgres.op",
                include_str!("../registry/packages/postgres/postgres.op"),
            ),
            (
                "test_postgres.op",
                include_str!("../registry/packages/postgres/test_postgres.op"),
            ),
        ],
    },
    SeedPkg {
        name: "web",
        version: "0.1.0",
        description: "Routing layer: :param/*splat router, request envelopes, forms, responses",
        files: &[
            (
                "operon.toml",
                include_str!("../registry/packages/web/operon.toml"),
            ),
            ("web.op", include_str!("../registry/packages/web/web.op")),
            (
                "test_web.op",
                include_str!("../registry/packages/web/test_web.op"),
            ),
        ],
    },
];

/// Digest over the whole embedded seed table (names + every file's
/// bytes, in table order). The registry marker stores this: when the
/// toolchain's seed changes, the digest changes, the fast path misses,
/// and the registry rebuilds from the new embedded bytes.
fn seed_digest() -> String {
    let mut buf: Vec<u8> = Vec::new();
    for pkg in SEED_PACKAGES {
        buf.extend_from_slice(pkg.name.as_bytes());
        buf.extend_from_slice(pkg.version.as_bytes());
        buf.extend_from_slice(pkg.description.as_bytes());
        for (n, b) in pkg.files {
            buf.extend_from_slice(n.as_bytes());
            buf.extend_from_slice(b.as_bytes());
        }
    }
    sha256_hex(&buf)
}

/// The registry home: `OPERON_REGISTRY_HOME` or `~/.operon/registry`.
pub fn registry_home() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("OPERON_REGISTRY_HOME") {
        if !d.is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".operon").join("registry"))
}

/// Materialize the bundled seed registry (idempotent, self-verifying).
/// Returns the index path. The packages/ tree is rebuilt whenever the
/// marker does not match the current binary's seed content, so `operon
/// add http` after an upgrade serves the NEW http, never a stale copy.
fn materialize_seed() -> String {
    let home = registry_home().unwrap_or_else(|| {
        die_pkg("cannot locate the registry home (set HOME or OPERON_REGISTRY_HOME)")
    });
    let index = home.join("index.jsonl");
    let marker = home.join("seed.sha256");
    // fast path: already materialized AND the marker matches THIS binary's
    // embedded seed content (a toolchain upgrade rebuilds; stale packages
    // are never served after an upgrade)
    if index.is_file() && marker.is_file() {
        if let Ok(want) = std::fs::read_to_string(&marker) {
            if want.trim() == seed_digest() {
                let mut fresh = true;
                for pkg in SEED_PACKAGES {
                    let dir = home.join("packages").join(pkg.name);
                    if !dir.is_dir() || checkout_checksum(&dir).is_empty() {
                        fresh = false;
                        break;
                    }
                }
                if fresh {
                    return index.to_string_lossy().to_string();
                }
            }
        }
    }
    // rebuild: packages from embedded bytes, index from the written bytes
    std::fs::create_dir_all(home.join("packages"))
        .unwrap_or_else(|e| die_pkg(&format!("registry mkdir failed: {}", e)));
    let mut lines = vec![
        "# operon seed registry (W21-r1).".to_string(),
        "# Materialized from the toolchain's embedded packages on first use;".to_string(),
        "# every line pins rev + sha256 and a dir source under packages/.".to_string(),
        "# Append your own lines (or point OPERON_REGISTRY at your own index)".to_string(),
        "# to grow beyond the seed. LAST matching line for a name wins.".to_string(),
    ];
    for pkg in SEED_PACKAGES {
        let dir = home.join("packages").join(pkg.name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| die_pkg(&format!("package mkdir failed: {}", e)));
        for (name, body) in pkg.files {
            std::fs::write(dir.join(name), body)
                .unwrap_or_else(|e| die_pkg(&format!("seed write {} failed: {}", name, e)));
        }
        let sha = checkout_checksum(&dir);
        lines.push(format!(
            "{{\"name\": \"{}\", \"version\": \"{}\", \"git\": \"seed:{}\", \"rev\": \"content-{}\", \"sha256\": \"{}\", \"dir\": \"{}\", \"description\": \"{}\"}}",
            pkg.name,
            pkg.version,
            pkg.name,
            &sha[..sha.len().min(16)],
            sha,
            json_escape(&dir.to_string_lossy()),
            json_escape(pkg.description),
        ));
    }
    let body = lines.join("\n") + "\n";
    std::fs::write(&index, &body)
        .unwrap_or_else(|e| die_pkg(&format!("index write failed: {}", e)));
    // the marker records the digest of the embedded seed content: the
    // next binary with different seed bytes rebuilds the registry
    let _ = std::fs::write(&marker, seed_digest());
    index.to_string_lossy().to_string()
}

/// The registry resolution chain (W21-r1). Order: explicit --registry,
/// OPERON_REGISTRY env, the project manifest's [registry] path, and
/// finally the bundled seed index (materialized on first use). The first
/// source that is SET wins — a project can pin its own registry even
/// when the env var is exported, by... no: env beats manifest (an
/// explicit machine-level override should win over a checked-in file),
/// explicit beats both.
fn registry_source(explicit: Option<&str>) -> String {
    if let Some(e) = explicit {
        if !e.is_empty() {
            return e.to_string();
        }
    }
    if let Ok(v) = std::env::var("OPERON_REGISTRY") {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    if let Some(m) = manifest_in(Path::new(".")) {
        if !m.registry.is_empty() {
            return m.registry;
        }
    }
    materialize_seed()
}

/// Read a registry source: a local file path, or an http(s):// index
/// fetched with curl (the same trust class as the git CLI this toolchain
/// already shells out to; see docs/REGISTRY.md §5 for the threat note).
fn registry_read(src: &str) -> String {
    if src.starts_with("http://") || src.starts_with("https://") {
        let out = std::process::Command::new("curl")
            .args(["-sSL", "--max-time", "30", src])
            .output();
        return match out {
            Ok(o) if o.status.success() => {
                let body = String::from_utf8_lossy(&o.stdout).to_string();
                if body.trim().is_empty() {
                    die_pkg(&format!(
                        "registry '{}': fetched an empty index (wrong URL? server down?)",
                        src
                    ));
                }
                body
            }
            Ok(o) => die_pkg(&format!(
                "registry '{}': curl exited {} ({} bytes)",
                src,
                o.status,
                o.stdout.len()
            )),
            Err(e) => die_pkg(&format!(
                "registry '{}': curl unavailable ({}) — install curl or use a file registry",
                src, e
            )),
        };
    }
    std::fs::read_to_string(src)
        .unwrap_or_else(|e| die_pkg(&format!("cannot read registry '{}': {}", src, e)))
}

/// True when `name` is a legal package name (lowercase ident-ish: the
/// same character class operon.toml's name field accepts).
fn valid_pkg_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        if !(first.is_ascii_lowercase() || first == '_') {
            return false;
        }
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

// ------------------------------------------------------ W20-r1: operon new

/// `operon new NAME [--lib] [--here]` — scaffold a project.
///   default : operon.toml + src/main.op + tests/smoke.op (runs green)
///   --lib   : operon.toml + src/NAME.op with a self-test proof frame
///   --here  : scaffold into the current directory instead of NAME/
pub fn new_command(rest: &[String]) -> ! {
    let mut name: Option<String> = None;
    let mut lib = false;
    let mut here = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--lib" => lib = true,
            "--here" => here = true,
            other => {
                if other.starts_with('-') {
                    die_pkg(&format!(
                        "unknown flag '{}' (new takes NAME, --lib, --here)",
                        other
                    ));
                }
                if name.is_some() {
                    die_pkg("new takes exactly one NAME");
                }
                name = Some(other.to_string());
            }
        }
        i += 1;
    }
    let name = match name {
        Some(n) => n,
        None => die_pkg("new needs a project name: operon new myapp"),
    };
    if !valid_pkg_name(&name) {
        die_pkg(&format!(
            "'{}' is not a valid package name (lowercase letters, digits, '-' and '_' only)",
            name
        ));
    }
    let root = if here {
        PathBuf::from(".")
    } else {
        PathBuf::from(&name)
    };
    if !here && root.exists() {
        die_pkg(&format!("'{}' already exists", name));
    }
    if root.join("operon.toml").exists() {
        die_pkg("there is already an operon.toml here");
    }
    std::fs::create_dir_all(root.join("src"))
        .unwrap_or_else(|e| die_pkg(&format!("mkdir failed: {}", e)));
    if !lib {
        std::fs::create_dir_all(root.join("tests"))
            .unwrap_or_else(|e| die_pkg(&format!("mkdir failed: {}", e)));
    }
    let version = env!("CARGO_PKG_VERSION").to_string();
    let mut toml = String::new();
    toml.push_str(&format!(
        "# {} — created by `operon new`\n\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\noperon-version = \"{}\"\n\n[deps]\n",
        name, name, version
    ));
    std::fs::write(root.join("operon.toml"), toml)
        .unwrap_or_else(|e| die_pkg(&format!("write operon.toml failed: {}", e)));
    let gene_prefix = name.replace('-', "_");
    if lib {
        let lib_src = format!(
            "# {name} — library scaffolded by `operon new --lib`.\n# Consumers import it with `use {name}` (after this package is a dependency).\n\ngene {gp}_double(n) {{\n    return n * 2\n}}\n\nframe proof {{\n    assert({gp}_double(21) == 42, \"double\")\n    assert({gp}_double(0) == 0, \"zero\")\n}}\n",
            name = name,
            gp = gene_prefix
        );
        std::fs::write(root.join("src").join(format!("{}.op", name)), lib_src)
            .unwrap_or_else(|e| die_pkg(&format!("write src failed: {}", e)));
        println!("created {} (library)", name);
    } else {
        let main_src = format!(
            "# {name} — scaffolded by `operon new`.\n# Run it:            operon run src/main.op\n# Run the tests:     operon test\n# Add a dependency:  operon add http        (then `use http` in your code)\n\ngene greet(who) {{\n    return \"hello, {r}\"\n}}\n\ngene main() {{\n    promote(greet(\"operon\"))\n}}\n",
            name = name,
            r = "{who}"
        );
        std::fs::write(root.join("src").join("main.op"), main_src)
            .unwrap_or_else(|e| die_pkg(&format!("write src failed: {}", e)));
        let smoke = "# smoke test — `operon test` runs every proof frame it finds.\nuse src/main\n\nframe proof {\n    assert(greet(\"operon\") == \"hello, operon\", \"greet\")\n    assert(greet(\"\") == \"hello, \", \"empty name is total\")\n}\n"
            .to_string();
        std::fs::write(root.join("tests").join("smoke.op"), smoke)
            .unwrap_or_else(|e| die_pkg(&format!("write tests failed: {}", e)));
        println!("created {} (application)", name);
    }
    println!("\nnext steps:");
    if !here {
        println!("  cd {}", name);
    }
    println!("  operon run src/main.op     # (applications)");
    println!("  operon test                # run the smoke test");
    println!("  operon add <package>       # pull a dependency from the registry");
    // ast-grep-ignore: no-std-process-exit-in-core
    std::process::exit(0);
}

// ------------------------------------------------- W21-r1: registry server

/// `operon registry init DIR` — create an empty local registry: an index
/// (JSON lines, comment-headed) plus a packages/ tree to publish into.
fn registry_init(dir: &str) -> ! {
    let root = PathBuf::from(dir);
    if root.join("index.jsonl").exists() {
        die_pkg(&format!("'{}' already has an index.jsonl", dir));
    }
    std::fs::create_dir_all(root.join("packages"))
        .unwrap_or_else(|e| die_pkg(&format!("mkdir failed: {}", e)));
    let index = "# my operon registry.\n# One JSON object per line:\n# {\"name\": \"beta\", \"version\": \"0.1.0\", \"git\": \"https://...\", \"rev\": \"...\", \"sha256\": \"...\", \"description\": \"...\"}\n# Dir-sourced packages add \"dir\": \"/abs/path/to/package-tree\" instead of a clone.\n# LAST matching line for a name wins (append-only by convention).\n";
    std::fs::write(root.join("index.jsonl"), index)
        .unwrap_or_else(|e| die_pkg(&format!("write index failed: {}", e)));
    let readme = format!(
        "# operon registry\n\nServe it:      operon registry serve {} --port 7331\nClients add:   OPERON_REGISTRY=http://127.0.0.1:7331/index.jsonl\n               operon add <package>\nPublish into it: operon publish --registry {}/index.jsonl\n",
        dir, dir
    );
    std::fs::write(root.join("README.md"), readme)
        .unwrap_or_else(|e| die_pkg(&format!("write README failed: {}", e)));
    println!("created registry at {} (index.jsonl + packages/)", dir);
    // ast-grep-ignore: no-std-process-exit-in-core
    std::process::exit(0);
}

/// `operon registry serve DIR [--port N] [--host H]` — a read-only HTTP
/// server over the registry directory. Serves exactly three route
/// classes, nothing else, no listing, no dotfiles, no path traversal:
///   GET /health        → "ok"
///   GET /index.jsonl   → the index file
///   GET /pkg/NAME/FILE → a file under DIR/packages/NAME/ (validated)
/// deny-by-default is the whole security model: every request that is
/// not one of these shapes gets a 404 before any path is opened.
fn registry_serve(rest: &[String]) -> ! {
    let mut dir = ".".to_string();
    let mut port: u16 = 7331;
    let mut host = "127.0.0.1".to_string();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--port" => {
                i += 1;
                port = rest
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| die_pkg("--port needs a number"));
            }
            "--host" => {
                i += 1;
                host = rest
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die_pkg("--host needs an address"));
            }
            other => {
                if other.starts_with('-') {
                    die_pkg(&format!("unknown flag '{}'", other));
                }
                dir = other.to_string();
            }
        }
        i += 1;
    }
    let root = std::fs::canonicalize(PathBuf::from(&dir))
        .unwrap_or_else(|e| die_pkg(&format!("cannot serve '{}': {}", dir, e)));
    if !root.join("index.jsonl").is_file() {
        die_pkg(&format!(
            "'{}' has no index.jsonl (run `operon registry init {}` first)",
            dir, dir
        ));
    }
    let listener = std::net::TcpListener::bind((host.as_str(), port))
        .unwrap_or_else(|e| die_pkg(&format!("cannot bind {}:{}: {}", host, port, e)));
    println!("operon registry serving {}", root.display());
    println!("  index:   http://{}:{}/index.jsonl", host, port);
    println!("  health:  http://{}:{}/health", host, port);
    println!(
        "  clients: OPERON_REGISTRY=http://{}:{}/index.jsonl operon add <package>",
        host, port
    );
    let root = std::sync::Arc::new(root);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let root = std::sync::Arc::clone(&root);
                std::thread::spawn(move || {
                    serve_one(s, &root);
                });
            }
            Err(_) => continue,
        }
    }
    // ast-grep-ignore: no-std-process-exit-in-core
    std::process::exit(0);
}

use std::io::{Read as IoRead, Write as IoWrite};

/// One served connection. Bounded request head (16 KB), one response,
/// close. Every file read goes through `serve_safe_path`.
fn serve_one(mut stream: std::net::TcpStream, root: &Path) {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let (status, ctype, body) = if method != "GET" {
        (
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed\n".to_vec(),
        )
    } else {
        match route_registry(path, root) {
            Some((ctype, body)) => ("200 OK", ctype, body),
            None => ("404 Not Found", "text/plain", b"not found\n".to_vec()),
        }
    };
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        ctype,
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

/// Route a validated GET. Returns (content-type, body) or None.
fn route_registry(path: &str, root: &Path) -> Option<(&'static str, Vec<u8>)> {
    if path == "/health" {
        return Some(("text/plain", b"ok\n".to_vec()));
    }
    if path == "/index.jsonl" {
        let body = std::fs::read(root.join("index.jsonl")).ok()?;
        return Some(("application/x-ndjson", body));
    }
    if let Some(rest) = path.strip_prefix("/pkg/") {
        // /pkg/NAME/FILE… — every segment validated BEFORE any path is
        // built: a traversal attempt never becomes a path, it becomes a 404.
        let segs: Vec<&str> = rest.split('/').collect();
        if segs.len() < 2 {
            return None;
        }
        let name = segs[0];
        if !valid_pkg_name(name) {
            return None;
        }
        let mut target = root.join("packages").join(name);
        for seg in &segs[1..] {
            if seg.is_empty()
                || *seg == "."
                || *seg == ".."
                || seg.contains('\\')
                || seg.contains('%')
            {
                return None;
            }
            if seg.starts_with('.') {
                return None; // no dotfiles, no dot-directories
            }
            target = target.join(seg);
        }
        let body = std::fs::read(&target).ok()?;
        let ctype = if target.to_string_lossy().ends_with(".op")
            || target.to_string_lossy().ends_with(".toml")
        {
            "text/plain; charset=utf-8"
        } else {
            "application/octet-stream"
        };
        return Some((ctype, body));
    }
    None
}

/// `operon registry init|serve|default` — the registry tooling group.
pub fn registry_command(rest: &[String]) -> ! {
    match rest.first().map(|s| s.as_str()) {
        Some("init") => {
            let dir = rest
                .get(1)
                .cloned()
                .unwrap_or_else(|| die_pkg("registry init needs a directory"));
            registry_init(&dir);
        }
        Some("serve") => registry_serve(&rest[1..]),
        Some("default") => {
            // DX probe: print the resolved chain without touching anything
            let explicit = std::env::var("OPERON_REGISTRY")
                .ok()
                .filter(|s| !s.trim().is_empty());
            match explicit {
                Some(e) => {
                    println!("OPERON_REGISTRY: {}", e);
                }
                None => {
                    if let Some(m) = manifest_in(Path::new(".")) {
                        if !m.registry.is_empty() {
                            println!("operon.toml [registry]: {}", m.registry);
                            // ast-grep-ignore: no-std-process-exit-in-core
                            std::process::exit(0);
                        }
                    }
                    println!("default: the bundled seed registry (materialized on first add)");
                    if let Some(h) = registry_home() {
                        println!("home:    {}", h.join("index.jsonl").display());
                    }
                    println!("seed packages:");
                    for pkg in SEED_PACKAGES {
                        println!("  {:<10} {} — {}", pkg.name, pkg.version, pkg.description);
                    }
                }
            }
            // ast-grep-ignore: no-std-process-exit-in-core
            std::process::exit(0);
        }
        other => {
            let _ = other;
            die_pkg("registry needs a subcommand: init | serve | default");
        }
    }
}
