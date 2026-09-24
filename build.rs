//! build.rs — compiles the load-bearing native kernels into the Rust build.
//!
//! sec-r2 (audit A15): the C runtime kernel (runtime/operon_rt.c) is DELETED.
//! Its interning table now lives in src/ffi.rs as ordinary Rust ownership —
//! the entire use-after-free class the ASan audit proved (C-5/C-6) is
//! structurally impossible, and the Win32/POSIX shim (SRWLOCK vs pthread,
//! QPC vs clock_gettime) went with it. `std::time::Instant` is monotonic on
//! every supported platform.
//!
//! What remains — mirrors scripts/build.sh:
//!   [1] runtime/codon_kernel.cpp — C++ codon kernel (bit-parallel Myers,
//!       codon scoring). Pure computation: no allocation, no retained
//!       pointers, no exceptions across the ABI, DP budget guarded in Rust.
//!
//! Portability notes (t3c, after release-matrix failures):
//!   * `.std()` — cc-rs maps it per compiler family: `-std=c++17` for
//!     gcc/clang, `/std:c++17` for MSVC. Hand-writing `-std=...` flags
//!     hard-fails MSVC (D8021).
//!   * `flag_if_supported()` — gcc/clang-only flags (`-Wall`, `-Wextra`,
//!     `-fno-exceptions`) are probed and silently skipped on compilers that
//!     reject them; cc-rs applies its own warning set (/W4) on MSVC.
//!   * C++ runtime linking is delegated to cc-rs: libc++ on Apple, libstdc++
//!     on linux-gnu, nothing extra on MSVC. Never hand-write
//!     `cargo:rustc-link-lib=stdc++` — that broke macOS ('stdc++' not found).
//!   * No pthread link: the C kernel's mutex was the only consumer; the codon
//!     kernel is single-threaded pure math.

fn main() {
    // --- C++ codon kernel ---------------------------------------------------
    // -fno-exceptions (where supported) matches scripts/build.sh: the kernel
    // never unwinds.
    let mut codon = cc::Build::new();
    codon
        .file("runtime/codon_kernel.cpp")
        .cpp(true)
        .std("c++17")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .warnings_into_errors(false)
        .compile("operon_codon_cpp");

    println!("cargo:rerun-if-changed=runtime/codon_kernel.cpp");
}
