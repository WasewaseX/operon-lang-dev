//! fress-port: tests for the net server's raw-byte file responses
//! (`interp::write_file_response` — the testable core of the `send_file`
//! builtin). Byte fidelity is the whole point: a PNG served through an
//! operon String would be silently corrupted by UTF-8 lossy conversion,
//! so these tests use genuinely non-UTF-8 bytes.

use operon::interp::write_file_response;
use std::io::Cursor;

#[test]
fn sends_exact_head_and_binary_bytes_unchanged() {
    let dir = std::env::temp_dir().join(format!("operon-sendfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("blob.bin");
    // 0xFF 0xFE 0x00 0x80 are invalid UTF-8 — a lossy String round-trip
    // would corrupt them. write_file_response must not.
    let bytes: Vec<u8> = vec![0xFF, 0xFE, 0x00, 0x80, b'a', b'b', 0xC3, 0x28, 0x89, b'P'];
    std::fs::write(&path, &bytes).unwrap();
    let path_str = path.to_string_lossy().to_string();

    let mut buf: Vec<u8> = Vec::new();
    write_file_response(&mut Cursor::new(&mut buf), "200", "image/png", &path_str).unwrap();

    let head_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("head terminator");
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = &buf[head_end + 4..];
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Content-Type: image/png\r\n"));
    assert!(head.contains(&format!("Content-Length: {}\r\n", bytes.len())));
    assert!(head.ends_with("Connection: close"), "actual head: {head:?}");
    assert_eq!(body, &bytes[..], "raw bytes must arrive unchanged");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn content_length_counts_bytes_not_chars() {
    let dir = std::env::temp_dir().join(format!("operon-sendfile-len-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("utf8.txt");
    // "é" is 2 bytes in UTF-8; a char-count bug would report 1.
    std::fs::write(&path, "café — 3 multibyte".as_bytes()).unwrap();
    let path_str = path.to_string_lossy().to_string();
    let mut buf: Vec<u8> = Vec::new();
    write_file_response(&mut Cursor::new(&mut buf), "200", "text/plain", &path_str).unwrap();
    let head = String::from_utf8_lossy(&buf).to_string();
    let expected = "café — 3 multibyte".as_bytes().len();
    assert!(head.contains(&format!("Content-Length: {}\r\n", expected)));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn refuses_directories_and_missing_files() {
    let dir = std::env::temp_dir();
    let dir_str = dir.to_string_lossy().to_string();
    let mut buf: Vec<u8> = Vec::new();
    let err = write_file_response(
        &mut Cursor::new(&mut buf),
        "200",
        "application/octet-stream",
        &dir_str,
    )
    .unwrap_err();
    assert!(err.contains("not a regular file"), "got: {err}");

    let missing = dir.join("operon-no-such-file-xyz.bin");
    let err = write_file_response(
        &mut Cursor::new(&mut buf),
        "200",
        "application/octet-stream",
        &missing.to_string_lossy(),
    )
    .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn refuses_files_over_the_64mib_ceiling() {
    // sparse file: instant, no real disk usage
    let dir = std::env::temp_dir().join(format!("operon-sendfile-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.bin");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(f);
    let mut buf: Vec<u8> = Vec::new();
    let err = write_file_response(
        &mut Cursor::new(&mut buf),
        "200",
        "application/octet-stream",
        &path.to_string_lossy(),
    )
    .unwrap_err();
    assert!(err.contains("64 MiB ceiling"), "got: {err}");
    let _ = std::fs::remove_file(&path);
}
