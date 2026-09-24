//! build.rs — compiles the load-bearing native kernels into the Rust build.
//!
//! Mirrors scripts/build.sh exactly (same sources, same standards, same flags):
//!   [1] runtime/operon_rt.c       — C runtime kernel (interning, arena, clock)
//!   [2] runtime/codon_kernel.cpp  — C++ codon kernel (bit-parallel Myers, scoring)
//!
//! The kernels are linked statically into every target (lib and bin), so
//! `cargo build` is self-contained: no pre-step, no stale build/ artifacts.
//!
//! Portability notes (t3c, after release-matrix failures):
//!   * `.std()` — cc-rs maps it per compiler family: `-std=c17` for gcc/clang,
//!     `/std:c17` for MSVC. Hand-writing `-std=...` flags hard-fails MSVC (D8021).
//!   * `flag_if_supported()` — gcc/clang-only flags (`-Wall`, `-Wextra`,
//!     `-fno-exceptions`) are probed and silently skipped on compilers that
//!     reject them; cc-rs applies its own warning set (/W4) on MSVC.
//!   * C++ runtime linking is delegated to cc-rs: libc++ on Apple, libstdc++
//!     on linux-gnu, nothing extra on MSVC. Never hand-write
//!     `cargo:rustc-link-lib=stdc++` — that broke macOS ('stdc++' not found).

fn main() {
    // --- C runtime kernel ------------------------------------------------
    let mut rt = cc::Build::new();
    rt.file("runtime/operon_rt.c")
        .std("c17")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .warnings_into_errors(false)
        .compile("operon_rt_c");

    // --- C++ codon kernel -------------------------------------------------
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

    // pthread comes from glibc >= 2.34, but link it explicitly where it is
    // still a separate library. C++ stdlib: handled by cc-rs (see header note).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    println!("cargo:rerun-if-changed=runtime/operon_rt.c");
    println!("cargo:rerun-if-changed=runtime/operon_rt.h");
    println!("cargo:rerun-if-changed=runtime/codon_kernel.cpp");
}
