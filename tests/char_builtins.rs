//! fress-port: tests for the chars/utf8_bytes/str_from_bytes builtins.

use operon::interp::Interp;
use operon::parser;

fn eval(src: &str) -> String {
    let prog = parser::parse(&format!("return {src}"));
    let mut interp = Interp::new();
    let env = operon::interp::Env::new(None);
    let mut last = String::new();
    for stmt in &prog.stmts {
        match interp.exec_stmt(&env, stmt) {
            Ok(operon::interp::Flow::Ret(v)) => last = v.display(),
            Ok(_) => {}
            Err(st) => panic!("stress [{}]: {}", st.kind, st.message),
        }
    }
    last
}

#[test]
fn chars_splits_per_codepoint() {
    // ascii
    assert_eq!(eval("json_str(chars(\"ab c\"))"), r#"["a","b"," ","c"]"#);
    // multibyte: one element per CHARACTER, not per byte
    assert_eq!(eval("len(chars(\"日本\"))"), "2");
    assert_eq!(eval("len(chars(\"\"))"), "0");
    assert_eq!(eval("chars(42)"), "[]");
}

#[test]
fn utf8_bytes_encodes_and_str_from_bytes_roundtrips() {
    // ASCII: byte == codepoint
    assert_eq!(eval("utf8_bytes(\"ab\")"), "[97, 98]");
    // multibyte: 3 bytes for U+65E5
    assert_eq!(eval("len(utf8_bytes(\"日\"))"), "3");
    assert_eq!(eval("str_from_bytes(utf8_bytes(\"日\"))"), "日");
    assert_eq!(eval("str_from_bytes(utf8_bytes(\"abc\"))"), "abc");
    assert_eq!(eval("str_from_bytes([])"), "");
}

#[test]
fn str_from_bytes_is_lossy_like_rust() {
    // 0xFF is invalid UTF-8 -> U+FFFD replacement, matching from_utf8_lossy
    assert_eq!(eval("str_from_bytes([255, 98])"), "\u{FFFD}b".to_string());
    // out-of-range ints are dropped by the filter
    assert_eq!(eval("str_from_bytes([65, 999, -1])"), "A");
}

#[test]
fn empty_and_wrong_types_degrade_to_empty() {
    assert_eq!(eval("chars(\"\")"), "[]");
    assert_eq!(eval("utf8_bytes(\"\")"), "[]");
    assert_eq!(eval("utf8_bytes(12)"), "[]");
    assert_eq!(eval("str_from_bytes(\"no\")"), "");
}
