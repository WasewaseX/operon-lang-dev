//! lib.rs — Operon language core.
//!
//! One module tree backs both the `operon` CLI binary and the cargo test
//! targets. The native C/C++ kernels (runtime/) are compiled and linked by
//! build.rs; their bindings live in `ffi`.

pub mod ast;
pub mod ffi;
pub mod genes;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod tools;
pub mod value;

/// Fatal CLI error: print to stderr and exit with status 2.
/// Lives here (not in the binary) because tool-layer entry points rely on it.
pub fn die(msg: &str) -> ! {
    eprintln!("operon: {}", msg);
    std::process::exit(2);
}
