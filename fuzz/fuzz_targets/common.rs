//! F5 (issue #49) — shared harness plumbing for the in-process targets.
//!
//! The big-stack worker: every input runs on a thread with a 512 MB stack
//! so the ASan build fits the SAME shipped 4096 nesting thresholds the CLI
//! ships (ASan inflates recursive-descent frames several-fold; without
//! this the shipped thresholds cannot stay under test — see FUZZING.md
//! harness note 1). Panics re-propagate on the joining thread so libFuzzer
//! sees them as crashes (a panic swallowed inside a worker would be a
//! silent false-pass).

#![allow(dead_code)]

pub const WORKER_STACK_BYTES: usize = 512 * 1024 * 1024;

/// Run `f` on a big-stack worker thread; re-propagate any panic.
pub fn with_big_stack<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .stack_size(WORKER_STACK_BYTES)
        .spawn(f)
        .expect("fuzz harness: failed to spawn big-stack worker");
    match handle.join() {
        Ok(()) => {}
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
