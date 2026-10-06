//! operon-ffi — the C-ABI bridge that hosts the Operon core inside the
//! TubeForge Android app as `liboperon.so`.
//!
//! Surface (called from `app/src/main/cpp/jni_glue.c`):
//! - `operon_version() -> const char*`      static core version string
//! - `operon_run_file(path) -> char*`       run one .op program under the
//!                                          default-deny sandbox, return a
//!                                          JSON result document
//! - `operon_string_free(ptr)`              free a returned string
//!
//! The result JSON shape (hand-serialized, zero dependencies — the core
//! stays zero-external-crates and this crate only depends on `operon`):
//! `{"ok":true|false,"version":"...","stdout":["..."],"notes":["..."],
//!   "stress":{"kind":"...","message":"..."}|null,"elapsed_ms":N}`
//!
//! The program runs EXACTLY like the CLI's default lane: no capability
//! grants (policy scripts are pure computation — `promote()` lines only),
//! VM lane on, notes captured instead of printed.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::rc::Rc;
use std::cell::RefCell;

use operon::interp::Caps;
use operon::tools::{self, Opts};

static VERSION_C: &[u8] = b"operon-ffi/0.1.0\0";

/// Hand-rolled JSON string escaping (the only JSON this crate emits).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
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

fn json_array(items: &[String]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|s| format!("\"{}\"", json_escape(s)))
        .collect();
    format!("[{}]", parts.join(","))
}

/// Run one .op program exactly like `operon run <path>` with no grants.
/// Returns the JSON result document (caller frees via operon_string_free).
unsafe fn run_file_impl(path: &str) -> *mut c_char {
    let t0 = std::time::Instant::now();
    let sink: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let opts = Opts {
        cell: None,
        variant: None,
        rna: None,
        entry: None,
        use_ires: false,
        frame: None,
        args: vec![],
        quiet: true,
        caps: Caps::default(), // deny-by-default: the TubeForge contract
        profile: false,
        stdout_sink: Some(sink.clone()),
        use_vm: true,
        spans: false,
    };
    let mut doc = String::from("{\"ok\":");
    match tools::load_file(path, &opts) {
        Ok(mut loaded) => {
            let stress = tools::run_entry(&mut loaded, &opts);
            let stdout: Vec<String> = sink.borrow().clone();
            let notes: Vec<String> = loaded
                .interp
                .notes
                .iter()
                .map(|n| n.message.clone())
                .collect();
            match stress {
                Ok(_) => doc.push_str("true"),
                Err(s) => doc.push_str(&format!(
                    "false,\"stress\":{{\"kind\":\"{}\",\"message\":\"{}\"}}",
                    json_escape(&s.kind),
                    json_escape(&s.message)
                )),
            }
            doc.push_str(&format!(
                ",\"version\":\"{}\",\"stdout\":{},\"notes\":{}",
                json_escape(VERSION),
                json_array(&stdout),
                json_array(&notes)
            ));
        }
        Err(e) => {
            // load failure: unreadable file, no entry gene, ...
            doc.push_str(&format!(
                "false,\"stress\":{{\"kind\":\"load\",\"message\":\"{}\"}},\"version\":\"{}\",\"stdout\":[],\"notes\":[]",
                json_escape(&e),
                json_escape(VERSION)
            ));
        }
    }
    doc.push_str(&format!(",\"elapsed_ms\":{}}}", t0.elapsed().as_millis()));
    match CString::new(doc) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[no_mangle]
pub extern "C" fn operon_version() -> *const c_char {
    VERSION_C.as_ptr() as *const c_char
}

#[no_mangle]
pub unsafe extern "C" fn operon_run_file(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let p = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    run_file_impl(p)
}

#[no_mangle]
pub unsafe extern "C" fn operon_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// Sanity counter for the smoke test lane (returns 7 — a canary that the
/// .so loaded and its C ABI is intact).
#[no_mangle]
pub extern "C" fn operon_ffi_canary() -> c_int {
    7
}
