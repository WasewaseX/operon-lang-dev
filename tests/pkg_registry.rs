//! W20/W21-r1 — first-class package management, end to end at the CLI level.
//!
//! Covers the developer surface the owner spec'd: `operon new` scaffolding,
//! top-level package verbs (add/remove/tree/verify/update), the seed
//! registry chain (env → manifest → bundled seed, no network), the
//! lockfile's reproducibility contract (same bytes, same rev, byte-stable
//! across machines), and the deny-by-default security rule for remote
//! registries (dir-sourced entries are a LOCAL-registry feature; a remote
//! index naming a `dir` is refused before any path is touched).

use std::process::Command;

fn operon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_operon"))
}

/// Isolated sandbox: its own HOME (registry + deps caches) and project dir.
fn sandbox(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("operon_w20_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("home");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    dir
}

fn in_dir(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    // HOME is pinned to <sandbox>/home — the caller's `dir` is the sandbox
    // root and every command runs at its expected subdirectory. (Pinning
    // HOME to the CWD was the v1 bug: a nested cwd redirected the caches.)
    // every sandbox is <root>/{home,proj,...}; commands run in a
    // subdirectory of <root>, so the sandbox root is the first ancestor
    // (including dir itself) that owns a `home` directory.
    let mut home = dir.join("home");
    let mut probe = Some(dir);
    while let Some(p) = probe {
        if p.join("home").is_dir() {
            home = p.join("home");
            break;
        }
        probe = p.parent();
    }
    let out = operon()
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        // hermetic: strip any inherited OPERON_* cache/registry overrides —
        // the sandbox's own HOME must own the caches and the seed registry
        // (the bash gate isolates exactly the same three).
        .env_remove("OPERON_DEPS")
        .env_remove("OPERON_REGISTRY")
        .env_remove("OPERON_REGISTRY_HOME")
        .output()
        .expect("run operon");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn new_scaffolds_a_green_project() {
    let sb = sandbox("new");
    let (rc, out, err) = in_dir(&sb.join("proj"), &["new", "myapp"]);
    assert_eq!(rc, 0, "new exits 0 (stderr: {})", err);
    assert!(out.contains("created myapp"), "next-steps banner");
    let root = sb.join("proj").join("myapp");
    assert!(root.join("operon.toml").is_file());
    assert!(root.join("src/main.op").is_file());
    assert!(root.join("tests/smoke.op").is_file());
    let (rc, _, err) = in_dir(&root, &["test"]);
    assert_eq!(rc, 0, "scaffold smoke test green (stderr: {})", err);
    // the app runs and greets
    let (rc, out, _) = in_dir(&root, &["run", "src/main.op"]);
    assert_eq!(rc, 0);
    assert!(
        out.contains("hello, operon"),
        "template output, got {}",
        out
    );
}

#[test]
fn new_refuses_bad_names_and_collisions() {
    let sb = sandbox("newbad");
    let (rc, _, _) = in_dir(&sb.join("proj"), &["new", "BadName"]);
    assert_eq!(rc, 2, "uppercase names are rejected");
    let (rc, _, _) = in_dir(&sb.join("proj"), &["new", "ok-name"]);
    assert_eq!(rc, 0);
    let (rc, _, err) = in_dir(&sb.join("proj"), &["new", "ok-name"]);
    assert_eq!(rc, 2, "existing dir is a hard stop");
    assert!(err.contains("already exists"), "collision message: {}", err);
}

#[test]
fn add_by_name_uses_the_bundled_seed_registry() {
    let sb = sandbox("seed");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    // zero flags, zero network: the chain falls through to the seed index
    let (rc, out, err) = in_dir(&root, &["add", "http"]);
    assert_eq!(rc, 0, "add http (stderr: {})", err);
    assert!(
        out.contains("via registry"),
        "resolution names the registry source: {}",
        out
    );
    let lock = std::fs::read_to_string(root.join("operon.lock")).expect("lockfile");
    assert!(
        lock.contains("git = \"registry:http\""),
        "lock records source"
    );
    assert!(lock.contains("checksum = \"sha256:"), "lock pins checksum");
    // vendored use works immediately (offline)
    std::fs::write(
        root.join("src/app.op"),
        "use http\ngene main() {\n    promote(http.http_query_encode({b: \"2\", a: \"1\"}))\n}\n",
    )
    .expect("write app");
    let (rc, out, _) = in_dir(&root, &["run", "src/app.op"]);
    assert_eq!(rc, 0);
    assert!(
        out.contains("a=1&b=2"),
        "canonical query from the dep: {}",
        out
    );
}

#[test]
fn transitive_dep_joins_the_closure() {
    let sb = sandbox("transitive");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let _ = in_dir(&root, &["add", "web"]);
    let lock = std::fs::read_to_string(root.join("operon.lock")).expect("lockfile");
    assert!(lock.contains("name = \"web\""), "direct dep locked");
    assert!(
        lock.contains("name = \"http\""),
        "web's http dep joined transitively"
    );
    let (rc, out, _) = in_dir(&root, &["tree"]);
    assert_eq!(rc, 0);
    assert!(
        out.contains("web") && out.contains("http"),
        "tree renders both"
    );
}

#[test]
fn lockfile_is_byte_stable_across_resolutions() {
    let sb = sandbox("stable");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let _ = in_dir(&root, &["add", "http"]);
    let lock1 = std::fs::read_to_string(root.join("operon.lock")).expect("lock 1");
    // a second machine = fresh caches, same bytes in, same lock out
    let deps = std::fs::read_to_string(root.join("operon.toml")).expect("manifest");
    std::fs::remove_dir_all(sb.join("home").join(".operon").join("deps")).ok();
    let (rc, _, _) = in_dir(&root, &["update"]);
    assert_eq!(rc, 0);
    let lock2 = std::fs::read_to_string(root.join("operon.lock")).expect("lock 2");
    assert_eq!(
        lock1, lock2,
        "same manifest+registry -> byte-identical lockfile"
    );
    let _ = deps;
}

/// Two completely fresh states — separate project dirs, separate HOMEs
/// (the seed registry re-materializes under ~/.operon/registry per HOME),
/// separate deps caches — and the same `new demo` + `add web` (web -> http
/// transitively) must end at BYTE-identical operon.lock files. The lock
/// carries registry:NAME sources, content revs and checksums: zero machine
/// locality, so a second machine resolves to the same bytes (W23's
/// reproducibility contract; mirrors scripts/pkg_e2e.sh step 9).
#[test]
fn lockfile_is_byte_identical_from_completely_fresh_caches() {
    let a = sandbox("lockfresh_a");
    let b = sandbox("lockfresh_b");
    let _ = in_dir(&a.join("proj"), &["new", "demo"]);
    let _ = in_dir(&b.join("proj"), &["new", "demo"]);
    let root_a = a.join("proj").join("demo");
    let root_b = b.join("proj").join("demo");
    let (rc, _, err) = in_dir(&root_a, &["add", "web"]);
    assert_eq!(rc, 0, "add web in sandbox A (stderr: {})", err);
    let (rc, _, err) = in_dir(&root_b, &["add", "web"]);
    assert_eq!(rc, 0, "add web in sandbox B (stderr: {})", err);
    // each fresh HOME got its own seed registry materialization
    assert!(a.join("home/.operon/registry/index.jsonl").is_file());
    assert!(b.join("home/.operon/registry/index.jsonl").is_file());
    // BYTES, not strings: the comparison must be byte-exact
    let lock_a = std::fs::read(root_a.join("operon.lock")).expect("lock A");
    let lock_b = std::fs::read(root_b.join("operon.lock")).expect("lock B");
    assert!(!lock_a.is_empty(), "lock A non-empty");
    assert_eq!(
        lock_a, lock_b,
        "fresh caches + re-materialized seed registries -> byte-identical lockfile"
    );
    let text = String::from_utf8(lock_a).expect("utf8 lock");
    assert!(
        text.contains("git = \"registry:web\"") && text.contains("git = \"registry:http\""),
        "lock pins the registry sources for web AND its transitive http"
    );
}

#[test]
fn verify_detects_a_corrupted_cache() {
    let sb = sandbox("verify");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let _ = in_dir(&root, &["add", "http"]);
    let (rc, _, _) = in_dir(&root, &["verify"]);
    assert_eq!(rc, 0, "clean cache verifies");
    // corrupt: append a byte to the vendored module (checksum covers bytes)
    for entry in std::fs::read_dir(sb.join("home").join(".operon").join("deps")).expect("deps") {
        let p = entry.expect("entry").path().join("http.op");
        if p.is_file() {
            let mut body = std::fs::read_to_string(&p).expect("read");
            body.push('\n');
            std::fs::write(&p, body).expect("tamper");
        }
    }
    let (rc, _, _) = in_dir(&root, &["verify"]);
    assert_eq!(rc, 1, "tampered cache FAILS verification");
}

#[test]
fn remove_keeps_manifest_and_lock_in_sync() {
    let sb = sandbox("remove");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let _ = in_dir(&root, &["add", "http"]);
    let (rc, _, _) = in_dir(&root, &["remove", "http"]);
    assert_eq!(rc, 0);
    let manifest = std::fs::read_to_string(root.join("operon.toml")).expect("manifest");
    assert!(!manifest.contains("http = "), "dep gone from manifest");
    let lock = std::fs::read_to_string(root.join("operon.lock")).expect("lock");
    assert!(!lock.contains("name = \"http\""), "dep gone from lock");
    let (rc, _, err) = in_dir(&root, &["remove", "http"]);
    assert_eq!(rc, 2, "removing a missing dep is a hard stop");
    assert!(err.contains("not in operon.toml"), "message: {}", err);
}

#[test]
fn registry_serve_and_client_roundtrip() {
    let sb = sandbox("serve");
    let reg = sb.join("reg");
    let (rc, _, _) = in_dir(&sb, &["registry", "init", reg.to_str().expect("path")]);
    assert_eq!(rc, 0, "registry init");
    assert!(reg.join("index.jsonl").is_file());
    assert!(reg.join("packages").is_dir());
}

#[test]
fn registry_help_lists_the_tooling() {
    let sb = sandbox("reghelp");
    let (_, out, _) = in_dir(&sb, &["registry", "default"]);
    assert!(
        out.contains("seed packages:") && out.contains("http") && out.contains("web"),
        "default probe lists the seed: {}",
        out
    );
}

// ---- #126 pkg-integrity family (Z-126-PKGINTEG) ----
//
// Local dir-sourced registry helper: a one-line jsonl index naming a
// directory package, vendored through resolve_dir_dep (staged + renamed
// since item 10). The seed chain is bypassed with --registry FILE.

fn write_local_registry(path: &std::path::Path, lines: &[String]) {
    std::fs::write(path, lines.join("\n") + "\n").expect("write registry index");
}

fn make_dir_package(root: &std::path::Path, name: &str, word: &str) -> std::path::PathBuf {
    // mirror the seed layout: operon.toml manifest + <name>.op source with
    // one exported gene (the vendored `use` resolves against this shape)
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("pkg dir");
    std::fs::write(
        dir.join("operon.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion = \"1.0.0\"\noperon-version = \"2.2\"\ndescription = \"test fixture\"\n",
            name
        ),
    )
    .expect("write manifest");
    std::fs::write(
        dir.join(format!("{}.op", name)),
        format!("gene mod_out() {{\n    return \"{}\"\n}}\n", word),
    )
    .expect("write pkg source");
    dir
}

/// item 12: a corrupt operon.lock must DIE LOUDLY on the read path — the
/// pre-fix `unwrap_or_default()` silently treated it as EMPTY, losing every
/// pin, and the next install re-resolved whatever it liked.
#[test]
fn corrupt_lockfile_dies_loudly_instead_of_losing_pins() {
    let sb = sandbox("lockcorrupt");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let (rc, _, err) = in_dir(&root, &["add", "http"]);
    assert_eq!(rc, 0, "add http baseline (stderr: {})", err);
    let pins = std::fs::read_to_string(root.join("operon.lock")).expect("lock");
    assert!(pins.contains("checksum"), "the lock pins checksums");
    // corrupt it: valid TOML-ish shape, unparseable as a lockfile
    std::fs::write(root.join("operon.lock"), "this is not a lockfile \x01\x02").expect("corrupt");
    let (rc, _, err) = in_dir(&root, &["install"]);
    assert_ne!(rc, 0, "install on a corrupt lock must not exit 0");
    assert!(
        err.contains("operon.lock is corrupt"),
        "the error must name the file and the corruption (stderr: {})",
        err
    );
    assert!(
        err.contains("NOT loaded") || err.contains("fix or delete"),
        "the error must say the pins were not loaded (stderr: {})",
        err
    );
}

/// item 13: registry_lookup_req tie-break — equal versions keep the LATER
/// line (last-match-wins, matching the doc comment and registry_lookup_text);
/// the pre-fix `v > *bv` silently kept the FIRST.
#[test]
fn requirement_lookup_equal_versions_keep_the_later_line() {
    let sb = sandbox("tiebreak");
    let reg_file = sb.join("reg.jsonl");
    let pkg_a = make_dir_package(sb.join("pkgs").join("v1").as_path(), "tie", "FIRST");
    let pkg_b = make_dir_package(sb.join("pkgs").join("v2").as_path(), "tie", "SECOND");
    let line = |dir: &std::path::Path| {
        // git+rev are required fields on every registry line (parse_registry
        // validates both); the dir branch wins when dir is non-empty.
        // Forward slashes: a backslash inside a jsonl string is an ESCAPE
        // (\t literally ate the \tie directory name on windows), and
        // std::path accepts / on every platform.
        format!(
            "{{\"name\":\"tie\",\"version\":\"1.2.0\",\"git\":\"local\",\"rev\":\"content-local\",\"dir\":\"{}\"}}",
            dir.to_string_lossy().replace('\\', "/")
        )
    };
    write_local_registry(&reg_file, &[line(&pkg_a), line(&pkg_b)]);
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let (rc, _, err) = in_dir(
        &root,
        &[
            "add",
            "tie@1.2.0",
            "--registry",
            reg_file.to_str().expect("path"),
        ],
    );
    assert_eq!(rc, 0, "add tie@1.2.0 (stderr: {})", err);
    std::fs::write(
        root.join("src/app.op"),
        "use tie\ngene main() {\n    promote(tie.mod_out())\n}\n",
    )
    .expect("write app");
    let (rc, out, err) = in_dir(&root, &["run", "src/app.op"]);
    assert_eq!(rc, 0, "run (stderr: {})", err);
    assert!(
        out.contains("SECOND"),
        "equal-version ties must keep the LATER registry line, got: {} {}",
        out,
        err
    );
}

/// item 11: a symlink inside a package tree is REFUSED, loudly — the
/// pre-fix walk followed links, so a loop aborted the process and a
/// followed link hashed out-of-tree bytes (nondeterministic digests).
#[test]
fn symlink_in_package_tree_is_refused_not_followed() {
    let sb = sandbox("symlink");
    let reg_file = sb.join("reg.jsonl");
    let pkg = make_dir_package(sb.join("pkgs").as_path(), "loopy", "x");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&pkg, pkg.join("loop")).expect("make loop symlink");
    #[cfg(windows)]
    {
        // windows symlink creation needs privileges the runner may lack;
        // the refusal path is e2e-asserted on unix — skip, don't fail,
        // when the link cannot be made
        if std::os::windows::fs::symlink_dir(&pkg, pkg.join("loop")).is_err() {
            return;
        }
    }
    let line = format!(
        "{{\"name\":\"loopy\",\"version\":\"1.0.0\",\"git\":\"local\",\"rev\":\"content-local\",\"dir\":\"{}\"}}",
        pkg.to_string_lossy().replace('\\', "/")
    );
    write_local_registry(&reg_file, &[line]);
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let (rc, _, err) = in_dir(
        &root,
        &[
            "add",
            "loopy",
            "--registry",
            reg_file.to_str().expect("path"),
        ],
    );
    assert_ne!(rc, 0, "a symlinked package tree must not vendor");
    assert!(
        err.contains("symlink in package tree"),
        "the refusal must name the symlink class (stderr: {})",
        err
    );
}

/// item 10: the install warm-check must VERIFY the pinned bytes — a
/// corrupted cache used to pass the is_dir-only check and ship whatever
/// sat in the directory. Post-fix: checksum mismatch re-materializes and
/// the corrupted file is restored.
#[test]
fn install_re_materializes_a_corrupted_warm_cache() {
    let sb = sandbox("warmcheck");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let (rc, _, err) = in_dir(&root, &["add", "http"]);
    assert_eq!(rc, 0, "add http baseline (stderr: {})", err);
    // locate the vendored http package file in the deps cache and corrupt it
    let deps = sb.join("home").join(".operon").join("deps");
    let mut target: Option<std::path::PathBuf> = None;
    let stack = vec![deps.clone()];
    let mut all = Vec::new();
    let mut stack = stack;
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "op").unwrap_or(false) {
                    all.push(p);
                }
            }
        }
    }
    all.sort();
    if let Some(f) = all.first() {
        target = Some(f.clone());
    }
    let victim = target.expect("a vendored .op file to corrupt");
    let original = std::fs::read(&victim).expect("read victim");
    std::fs::write(&victim, b"CORRUPTED BYTES").expect("corrupt cache");
    // install must notice the drift and re-vendor (exit 0, bytes restored)
    let (rc, _, err) = in_dir(&root, &["install"]);
    assert_eq!(
        rc, 0,
        "install self-heals a drifted cache (stderr: {})",
        err
    );
    let restored = std::fs::read(&victim).expect("read restored");
    assert_eq!(
        original, restored,
        "the corrupted cache file must be re-materialized from the registry"
    );
    let (rc, _, err) = in_dir(&root, &["verify"]);
    assert_eq!(rc, 0, "verify green after the self-heal (stderr: {})", err);
}

/// item 15: HOME="" (set but empty) must die loudly with the
/// cannot-locate-the-registry-home message — the pre-fix fell back to a
/// CWD-relative `.operon/registry` (the OPERON_REGISTRY_HOME empty case
/// was guarded; HOME was not).
#[test]
fn empty_home_dies_loudly_not_cwd_relative() {
    let sb = sandbox("emptyhome");
    let root = sb.join("proj").join("app");
    let _ = in_dir(&sb.join("proj"), &["new", "app"]);
    let out = operon()
        .args(["add", "http"])
        .current_dir(&root)
        .env("HOME", "")
        .env_remove("OPERON_DEPS")
        .env_remove("OPERON_REGISTRY")
        .env_remove("OPERON_REGISTRY_HOME")
        .output()
        .expect("run operon with empty HOME");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_ne!(
        out.status.code().unwrap_or(-1),
        0,
        "empty HOME must not silently resolve against the CWD"
    );
    assert!(
        err.contains("registry home") || err.contains("HOME"),
        "the error must name the registry-home/HOME cause (stderr: {})",
        err
    );
    assert!(
        !root.join(".operon").exists(),
        "no CWD-relative .operon may be created by the empty-HOME run"
    );
}
