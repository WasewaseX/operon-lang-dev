//! build.rs — compiles the load-bearing native kernels into the Rust build.
//!
//! Mirrors scripts/build.sh exactly (same sources, same standards, same flags):
//!   [1] runtime/operon_rt.c       — C runtime kernel (interning, arena, clock)
//!   [2] runtime/codon_kernel.cpp  — C++ codon kernel (bit-parallel Myers, scoring)
//!
//! The kernels are linked statically into every target (lib and bin), so
//! `cargo build` is self-contained: no pre-step, no stale build/ artifacts.

fn main() {
    // --- C runtime kernel ------------------------------------------------
    let mut rt = cc::Build::new();
    rt.file("runtime/operon_rt.c")
        .flag("-std=c17")
        .flag("-Wall")
        .flag("-Wextra")
        .warnings_into_errors(false)
        .compile("operon_rt_c");

    // --- C++ codon kernel -------------------------------------------------
    // -fno-exceptions matches scripts/build.sh: the kernel never unwinds.
    let mut codon = cc::Build::new();
    codon.file("runtime/codon_kernel.cpp")
        .cpp(true)
        .flag("-std=c++17")
        .flag("-fno-exceptions")
        .flag("-Wall")
        .flag("-Wextra")
        .warnings_into_errors(false)
        .compile("operon_codon_cpp");

    // The C++ kernel's runtime support; pthread comes from glibc >= 2.34 but
    // link it explicitly where it is still a separate library.
    println!("cargo:rustc-link-lib=dylib=stdc++");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    println!("cargo:rerun-if-changed=runtime/operon_rt.c");
    println!("cargo:rerun-if-changed=runtime/operon_rt.h");
    println!("cargo:rerun-if-changed=runtime/codon_kernel.cpp");
}
