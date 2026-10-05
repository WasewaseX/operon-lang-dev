//! F5 (issue #49) — in-process coverage-guided target: the VM-load surface.
//!
//! The EXACT CLI run sequence in-process (mirrors the `run` arm of
//! src/main.rs, minus process exit and terminal I/O):
//!   1. tools::load_file        — read + parse + top-level statements
//!   2. entry-leg fuel cap      — step_budget 200k (the E2 profile)
//!   3. run-wide fuel pool      — 200M shared across spawned workers
//!                                (SPEC §9b: per run, not per interpreter)
//!   4. VM lane on, vm_opt 0    — the shipped default lane, unoptimized
//!   5. Caps::default           — secure by default: denying, zero grants
//!   6. stdout sunk (dx-r3)     — program promote() output captured into a
//!                                Vec, never printed, kept until drop
//!   7. tools::run_entry        — the entry gene (default entry resolution)
//!
//! An Err(Stress) return is the interpreter CONTAINING (interference,
//! interference-breach, fuel exhaustion, rescue) — that is the designed
//! behavior under test, not a crash. Only a panic / signal / in-process
//! exit counts (TRIAGE vocabulary).
//!
//! Coverage-only (built --sanitizer none) + -detect_leaks=0: the Rc
//! reference-cycle REMAIN (FUZZING.md harness note 2) and the
//! catch-and-retry stack economics (note 3) are inherent to the shipped
//! CLI model; they are documented REMAINs, not fuzzable crashes — the
//! rationale lives in FUZZING.md "The in-process layer".

#![no_main]
use libfuzzer_sys::fuzz_target;

mod common;

const ENTRY_FUEL: u64 = 200_000;
const RUN_WIDE_FUEL: i64 = 200_000_000;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data).into_owned();

    // The run surface reads real files (load_file takes a path), so the
    // input is materialized into a temp file inside the crate's target
    // scratch dir — the only writable location the harness touches.
    let mut path = std::env::temp_dir();
    path.push(format!("f5_run_input_{}.op", std::process::id()));
    if std::fs::write(&path, src.as_bytes()).is_err() {
        return; // scratch unavailable: skip, not a finding
    }

    common::with_big_stack(move || {
        let sink: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let opts = operon::tools::Opts {
            cell: None,
            variant: None,
            rna: None,
            entry: None,
            use_ires: false,
            frame: None,
            args: Vec::new(),
            quiet: true,
            caps: operon::interp::Caps::default(), // default-deny, zero grants
            profile: false,
            spans: false,
            stdout_sink: Some(sink), // dx-r3: sunk, never printed
            use_vm: true,            // the shipped default lane
        };
        let file = path.to_string_lossy().into_owned();
        if let Ok(mut l) = operon::tools::load_file(&file, &opts) {
            // the E2 entry-leg cap + the SPEC §9b run-wide pool
            l.interp.step_budget = ENTRY_FUEL;
            l.interp.fuel_pool = Some(std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
                RUN_WIDE_FUEL,
            )));
            // VM lane on, vm_opt 0 (shipped default, unoptimized)
            l.interp.vm = true;
            l.interp.vm_opt = 0;
            l.interp.vm_program = Some(operon::vm::VmProgram::default());
            // Err(Stress) = contained (the behavior under test)
            let _ = operon::tools::run_entry(&mut l, &opts);
        }
        let _ = std::fs::remove_file(&file); // best-effort scratch cleanup
    });
});
