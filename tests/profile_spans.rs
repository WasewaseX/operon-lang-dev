//! W097-A — per-call span capture + Chrome Trace Format export
//! (`operon profile f.op --chrome trace.json`).
//!
//! The profiler's aggregate mode (call_counts / call_time_self, `--json`
//! format `operon-profile`) predates this; what was missing is the
//! per-call TIMELINE W096's Chrome-trace export needs (builder-A's PR #55
//! note: "spans remain the right next instrument"). Spans are captured in
//! `close_timing` — the shared call funnel's single pop choke point — so
//! both the VM and tree-walk lanes produce them, and only when
//! `--chrome` armed capture before load (dx-r1: top-level calls execute
//! during load_file, so the flag rides Opts, not post-load interp state).
//!
//! Pins are STRUCTURAL, never timing values: wall-clock ts/dur vary run to
//! run, so every assertion here is about event counts, names, depths,
//! interval nesting, and the self-describing metadata — the things that
//! must be deterministic. Each test drives the release binary (the
//! sec_regression / grn_trace pattern): the flag is an operator
//! diagnostic surface, so the pins are end-to-end. JSON is parsed by a
//! tiny in-test parser — the crate is dependency-free by policy (the
//! docgen.rs precedent).

use std::process::Command;

// ------------------------------------------------------------ mini JSON
/// Dependency-free JSON parser (objects/arrays/strings/numbers/null/bools),
/// exactly enough for the shapes write_chrome_trace emits.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // complete JSON grammar, though our trace only exercises a subset
enum J {
    Obj(Vec<(String, J)>),
    Arr(Vec<J>),
    S(String),
    N(f64),
    B(bool),
    Null,
}

impl J {
    fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            J::N(x) => Some(*x),
            _ => None,
        }
    }
    fn as_u64(&self) -> Option<u64> {
        match self {
            J::N(x) if *x >= 0.0 && x.fract() == 0.0 => Some(*x as u64),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            J::S(s) => Some(s),
            _ => None,
        }
    }
    fn as_arr(&self) -> Option<&[J]> {
        match self {
            J::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn val(&mut self) -> Option<J> {
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(J::Obj(pairs));
                }
                loop {
                    self.ws();
                    let k = self.str()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return None;
                    }
                    self.i += 1;
                    let v = self.val()?;
                    pairs.push((k, v));
                    self.ws();
                    match self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(J::Obj(pairs));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(J::Arr(items));
                }
                loop {
                    let v = self.val()?;
                    items.push(v);
                    self.ws();
                    match self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(J::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => Some(J::S(self.str()?)),
            _ => {
                let s = self.i;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
                    )
                {
                    self.i += 1;
                }
                if s == self.i {
                    return None;
                }
                std::str::from_utf8(&self.b[s..self.i])
                    .ok()?
                    .parse::<f64>()
                    .ok()
                    .map(J::N)
            }
        }
    }
    fn str(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.i += 1;
                    out.push(*self.b.get(self.i)? as char);
                    self.i += 1;
                }
                c => {
                    out.push(c as char);
                    self.i += 1;
                }
            }
        }
        None
    }
}

fn parse_trace(raw: &str) -> Option<J> {
    let mut p = P {
        b: raw.as_bytes(),
        i: 0,
    };
    let v = p.val()?;
    // full-consumption check: trailing garbage = malformed trace
    p.ws();
    if p.i != raw.len() {
        return None;
    }
    Some(v)
}

// ------------------------------------------------------------ harness
/// Per-CALL unique temp dir (the grn_trace lesson: shared per-pid dirs
/// raced between parallel tests in one binary; unique dirs make the race
/// structurally impossible).
struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "operon_w097a_{}_{}_{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }
    fn op(&self, name: &str, src: &str) -> std::path::PathBuf {
        let f = self.dir.join(name);
        std::fs::write(&f, src).unwrap();
        f
    }
    fn trace(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(args: &[&std::path::Path]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .args(args)
        .output()
        .expect("run operon");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const FIB5: &str = r#"gene fib(n) {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
gene main() {
    promote("fib(5) = {fib(5)}")
}
"#;

fn x_events(d: &J) -> Vec<&J> {
    d.get("traceEvents")
        .and_then(|t| t.as_arr())
        .expect("traceEvents array")
        .iter()
        .filter(|e| e.get("ph").and_then(|p| p.as_str()) == Some("X"))
        .collect()
}

/// C(5) = 15 fib calls (C(n) = 1 + C(n-1) + C(n-2)) + 1 main = 16 spans;
/// recursion depth: main=0, fib(5)=1 .. fib(1)=5.
#[test]
fn spans_capture_call_counts_names_and_depths() {
    let fx = Fixture::new("spans1");
    let f = fx.op("fib5.op", FIB5);
    let tf = fx.trace("fib5.json");
    let (rc, out, _err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
        tf.as_path(),
    ]);
    assert_eq!(rc, 0, "profile succeeds:\n{out}");
    assert!(
        out.contains("16 span(s), 0 dropped"),
        "operator notice reports the honest counts:\n{out}"
    );
    let raw = std::fs::read_to_string(&tf).unwrap_or_default();
    let d = parse_trace(&raw).expect("trace is valid JSON");
    let events = x_events(&d);
    assert_eq!(events.len(), 16, "one X event per call");
    let mut fib = 0usize;
    let mut main = 0usize;
    for e in &events {
        match e.get("name").and_then(|n| n.as_str()).unwrap() {
            "fib" => fib += 1,
            "main" => main += 1,
            other => panic!("unexpected span name {other}"),
        }
        assert_eq!(e.get("cat").and_then(|c| c.as_str()), Some("gene"));
        assert_eq!(e.get("pid").and_then(|p| p.as_u64()), Some(1));
        assert_eq!(e.get("tid").and_then(|t| t.as_u64()), Some(1));
        assert!(e.get("ts").and_then(|t| t.as_f64()).unwrap() >= 0.0);
        assert!(e.get("dur").and_then(|t| t.as_f64()).unwrap() >= 0.0);
    }
    assert_eq!((fib, main), (15, 1), "name counts match the aggregate view");
    let mut depths: Vec<u64> = events
        .iter()
        .map(|e| {
            e.get("args")
                .and_then(|a| a.get("depth"))
                .and_then(|d| d.as_u64())
                .expect("depth arg")
        })
        .collect();
    depths.sort_unstable();
    depths.dedup();
    assert_eq!(
        depths,
        vec![0, 1, 2, 3, 4, 5],
        "depths = ancestor counts through the fib(5) chain"
    );
}

/// Every span with depth > 0 is INTERVAL-CONTAINED in some depth-1 parent:
/// ts_parent <= ts && ts + dur <= ts_parent + dur_parent. This is the
/// property Chrome/Perfetto use to reconstruct the call tree from X events
/// (and the cross-check that `depth` means what the doc says).
#[test]
fn spans_nest_by_interval_containment() {
    let fx = Fixture::new("spans2");
    let f = fx.op("fib5.op", FIB5);
    let tf = fx.trace("nest.json");
    let (rc, _out, _err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
        tf.as_path(),
    ]);
    assert_eq!(rc, 0);
    let raw = std::fs::read_to_string(&tf).unwrap_or_default();
    let d = parse_trace(&raw).expect("valid trace JSON");
    let events: Vec<(f64, f64, u64)> = x_events(&d)
        .into_iter()
        .map(|e| {
            (
                e.get("ts").and_then(|t| t.as_f64()).unwrap(),
                e.get("dur").and_then(|t| t.as_f64()).unwrap(),
                e.get("args")
                    .and_then(|a| a.get("depth"))
                    .and_then(|d| d.as_u64())
                    .unwrap(),
            )
        })
        .collect();
    assert!(!events.is_empty());
    for &(ts, dur, depth) in &events {
        if depth == 0 {
            // outermost frames have no parent by definition
            continue;
        }
        let contained = events
            .iter()
            .any(|&(pts, pdur, pdepth)| pdepth + 1 == depth && pts <= ts && ts + dur <= pts + pdur);
        assert!(
            contained,
            "depth-{depth} span ts={ts} dur={dur} has no containing depth-{} parent",
            depth.saturating_sub(1)
        );
    }
}

/// The trace file must be self-describing (W096's law: a trace never lies
/// silent about its own limits) — format tag, version, source file, unit,
/// clock, and the honest drop accounting.
#[test]
fn chrome_file_is_self_describing() {
    let fx = Fixture::new("spans3");
    let f = fx.op("fib5.op", FIB5);
    let tf = fx.trace("meta.json");
    let (rc, _out, _err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
        tf.as_path(),
    ]);
    assert_eq!(rc, 0);
    let raw = std::fs::read_to_string(&tf).unwrap_or_default();
    let d = parse_trace(&raw).expect("valid trace JSON");
    let od = d.get("otherData").expect("otherData block");
    assert_eq!(
        od.get("format").and_then(|v| v.as_str()),
        Some("operon-chrome-trace")
    );
    assert_eq!(
        od.get("version").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(
        od.get("file")
            .and_then(|v| v.as_str())
            .unwrap()
            .ends_with("fib5.op"),
        "source file recorded"
    );
    assert_eq!(
        od.get("unit").and_then(|v| v.as_str()),
        Some("microseconds")
    );
    assert_eq!(od.get("clock").and_then(|v| v.as_str()), Some("monotonic"));
    assert_eq!(od.get("total_spans").and_then(|v| v.as_u64()), Some(16));
    assert_eq!(od.get("dropped_spans").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(od.get("span_cap").and_then(|v| v.as_u64()), Some(1_000_000));
    assert_eq!(
        d.get("displayTimeUnit").and_then(|v| v.as_str()),
        Some("us")
    );
    let meta: Vec<&J> = d
        .get("traceEvents")
        .and_then(|t| t.as_arr())
        .unwrap()
        .iter()
        .filter(|e| e.get("ph").and_then(|p| p.as_str()) == Some("M"))
        .collect();
    assert_eq!(meta.len(), 2, "process_name + thread_name metadata events");
    assert_eq!(
        meta[0]
            .get("args")
            .and_then(|a| a.get("name"))
            .and_then(|n| n.as_str()),
        Some("operon")
    );
}

/// dx-r1 discipline: calls that execute during LOAD (top-level statements,
/// no main() gene) must be in the timeline too — the flag arms capture
/// before load_file, not after.
#[test]
fn load_time_calls_are_in_the_timeline() {
    let fx = Fixture::new("spans4");
    let src = "gene greet() {\n    return \"hi\"\n}\ngreet()\ngreet()\n";
    let f = fx.op("toplevel.op", src);
    let tf = fx.trace("tl.json");
    let (rc, _out, _err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
        tf.as_path(),
    ]);
    assert_eq!(rc, 0, "mainless profile succeeds");
    let raw = std::fs::read_to_string(&tf).unwrap_or_default();
    let d = parse_trace(&raw).expect("valid trace JSON");
    let events = x_events(&d);
    assert_eq!(events.len(), 2, "both top-level greet() calls captured");
    assert!(events
        .iter()
        .all(|e| e.get("name").and_then(|n| n.as_str()) == Some("greet")));
    assert!(
        events.iter().all(|e| e
            .get("args")
            .and_then(|a| a.get("depth"))
            .and_then(|x| x.as_u64())
            == Some(0)),
        "top-level calls have no ancestors"
    );
}

/// --json stays byte-compatible (schema pinned since W096 stage 1): with
/// --chrome also given, stdout is STILL pure machine-readable JSON and the
/// operator notice rides stderr instead of corrupting the stream.
#[test]
fn json_output_stays_pure_with_chrome() {
    let fx = Fixture::new("spans5");
    let f = fx.op("fib5.op", FIB5);
    let tf = fx.trace("both.json");
    let (rc, out, err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
        tf.as_path(),
        std::path::Path::new("--json"),
    ]);
    assert_eq!(rc, 0);
    // The program's own stdout (promote) legitimately precedes the JSON
    // block — the pin is that the notice NEVER lands on stdout (it rides
    // stderr) and the JSON block stays intact, not that stdout is 100% JSON
    // (only --json-pure programs could promise that).
    assert!(
        out.contains("{\"format\":\"operon-profile\""),
        "stdout keeps the operon-profile JSON block:\n{out}"
    );
    assert!(
        out.contains("\"total_self_us\":"),
        "aggregate schema fields intact"
    );
    assert!(
        err.contains("chrome trace:"),
        "notice rides stderr under --json:\n{err}"
    );
    assert!(tf.exists(), "trace file still written");
}

/// Default behavior unchanged: `operon profile` without --chrome prints no
/// chrome notice and writes no trace — the aggregate table surface is
/// byte-stable for existing consumers.
#[test]
fn no_chrome_flag_no_trace_surface() {
    let fx = Fixture::new("spans6");
    let f = fx.op("fib5.op", FIB5);
    let tf = fx.trace("absent.json");
    let (rc, out, err) = run(&[std::path::Path::new("profile"), f.as_path()]);
    assert_eq!(rc, 0);
    assert!(!out.contains("chrome trace:"), "no notice without --chrome");
    assert!(!err.contains("chrome"), "no stderr noise without --chrome");
    assert!(!tf.exists(), "no trace file without --chrome");
    assert!(
        out.contains("operon profile:"),
        "aggregate table intact:\n{out}"
    );
}

/// Negative: a bare --chrome with no path dies with a usage note naming
/// the flag (never a silent default or a panic).
#[test]
fn chrome_without_path_is_refused() {
    let fx = Fixture::new("spans7");
    let f = fx.op("fib5.op", FIB5);
    let (rc, _out, err) = run(&[
        std::path::Path::new("profile"),
        f.as_path(),
        std::path::Path::new("--chrome"),
    ]);
    assert_ne!(rc, 0, "bare --chrome refused");
    assert!(
        err.contains("--chrome needs a file path"),
        "usage note names the flag:\n{err}"
    );
}
