//! lib.rs, Operon language core.
//!
//! One module tree backs both the `operon` CLI binary and the cargo test
//! targets. The native C++ kernel (runtime/codon_kernel.cpp) is compiled and
//! linked by build.rs; its bindings plus the Rust symbol table/clock live in
//! `ffi`. (sec-r2, audit A15: the C runtime kernel was deleted, interning
//! is ordinary Rust ownership now.)

pub mod ast;
pub mod ffi;
pub mod genes;
pub mod graph;
pub mod interp;
pub mod lexer;
pub mod lint;
pub mod ls;
pub mod parser;
pub mod pkg;
pub mod pybridge;
pub mod rna2;
pub mod tools;
pub mod types;
pub mod unicode_tables;
pub mod value;
pub mod vm;

/// Fatal CLI error: print to stderr and exit with status 2.
/// Lives here (not in the binary) because tool-layer entry points rely on it.
pub fn die(msg: &str) -> ! {
    eprintln!("operon: {}", msg);
    // ast-grep-ignore: no-std-process-exit-in-core
    std::process::exit(2);
}
