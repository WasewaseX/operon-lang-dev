//! W101 (owner directive, 2026-09-29): excellent errors.
//!
//! One module owns the shape of an uncaught top-level stress so every
//! consumer (CLI text, --json-errors, later the LSP) renders from the same
//! catalog. Nothing here moves program behavior: stresses as VALUES (what a
//! rescue binding sees, what the oracle mirrors) keep their kind/message/
//! line/chain fields untouched. This module is presentation only, fed by the
//! runner's catch site in main.rs.
//!
//! Design notes:
//! - Codes are stable strings (E1xxx) documented in SPEC §9a. A code is
//!   derived from (kind, message); the message carries the capability verb
//!   in its text already, so the mapping stays a pure function.
//! - The snippet line comes from the script source the runner already
//!   loaded. Multi-line spans do not exist in the AST yet (dx-r2 spans are
//!   line-only), so the caret row is emitted only when the callee token
//!   named first in the message is found on the raise line; that heuristic
//!   is documented in SPEC §9a and degrades to no-caret honestly.
//! - Help lines are extracted from the message's own "grant with ..." tail
//!   (Caps::denied writes it), so the help text can never disagree with the
//!   denial that produced it.

use crate::value::Stress;

/// The error-code catalog (SPEC §9a). One code per user-visible failure
/// family, stable across releases; new families append, never renumber.
pub fn code_for(kind: &str, message: &str) -> &'static str {
    match kind {
        "interference" => interference_code(message),
        "missing" => "E1002",  // unknown name (gene, variable, module)
        "unfolded" => "E1003", // type/shape mismatch (incl. soft annotations)
        "unwrap" => "E1004",   // unwrap of none/err without a fallback
        "overflow" => "E1020", // arithmetic overflow (pinned contract, W32)
        "burned" => "E1021",   // fuel exhausted (recursion/spawn/regex/json)
        _ => "E1000",          // unclassified stress, kind is the truth
    }
}

/// Capability verbs, detected two ways: the message's leading verb
/// ("read denied, ...") and quoted verbs inside grant phrasing. Order
/// matters: first match wins, new verbs append here AND to SPEC 9a.
fn interference_code(message: &str) -> &'static str {
    let first: String = message
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    for (verb, code) in [
        ("read", "E1010"),
        ("write", "E1011"),
        ("append", "E1011"),
        ("net", "E1012"),
        ("http", "E1012"),
        ("serve", "E1012"),
        ("exit", "E1013"),
        ("env", "E1013"),
        ("spawn", "E1014"),
        ("py", "E1015"),
        ("clock", "E1016"),
    ] {
        if first == verb || message.contains(&format!("'{}'", verb)) {
            return code;
        }
    }
    "E1019"
}

/// Extract the `--allow-...` token(s) the denial message itself suggests.
/// The help text is a projection of the message, never an independent claim.
fn help_lines(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in message.split_whitespace() {
        if word.starts_with("--allow-") {
            let w = word.trim_end_matches(&[')', ',', '.'][..]);
            out.push(w.to_string());
        }
    }
    out.dedup();
    out
}

/// Locate the caret on the raise line, CAPABILITY DENIALS ONLY: the first
/// word of the message is the failing call/builtin name (read_file 'x': ...);
/// underline its first occurrence on the line. Other kinds honestly get no
/// caret (the AST is line-only today; dx-r2 spans do not carry columns).
/// Returns (col 0-based, len), or None.
fn locate_caret(kind: &str, line_text: &str, message: &str) -> Option<(usize, usize)> {
    if kind != "interference" {
        return None;
    }
    let name: String = message
        .split(|c: char| c.is_whitespace() || c == '(')
        .find(|w| !w.is_empty())?
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    let byte_col = line_text.find(&name)?;
    let col = line_text[..byte_col].chars().count();
    Some((col, name.chars().count()))
}

fn note_line(message: &str) -> Option<String> {
    // The explanatory "= ..." note: the grant clause of a denial, verbatim.
    let start = message.find("no capability grant")?;
    Some(message[start..].trim_end().to_string())
}

/// Render one finding (the W041-coded lint/check streams) as a located
/// block in the same shape as the fatal renderer, severity-prefixed:
/// `error[E01]:`, `warning[W07]:`, `style[N12]:`. Findings carry
/// line-only locations today, so no caret row is faked (SPEC 9a.1);
/// the note line names the owning rule, which is the finding's provenance.
pub fn render_finding(
    file: &str,
    src: &str,
    sev: &str,
    code: &str,
    rule: &str,
    line: usize,
    message: &str,
) -> String {
    let mut out = format!("{}[{}]: {}\n", sev, code, message);
    if line == 0 {
        return out;
    }
    let line_text = src.lines().nth(line - 1).unwrap_or("");
    let num = line.to_string();
    let pad = num.len();
    out.push_str(&format!("\n  --> {}:{}\n", file, line));
    out.push_str(&format!("{:>w$} |\n", "", w = pad + 1));
    out.push_str(&format!("{} | {}\n", num, line_text));
    out.push_str(&format!("{:>w$} |\n", "", w = pad + 1));
    out.push_str(&format!("{:>w$} = rule: {}\n", "", rule, w = pad + 1));
    out
}

/// The rendered text block (rustc shape). The W007 chain lines are appended
/// in the runner's established format after the block so existing parsers of
/// our stderr (cookbook expected files, redteam rc checks) keep working.
pub fn render(file: &str, src: &str, s: &Stress) -> String {
    let code = code_for(&s.kind, &s.message);
    let mut out = format!("error[{}]: {}\n", code, s.message);
    if s.line == 0 {
        out.push('\n');
        return out;
    }
    let line_text = src.lines().nth(s.line - 1).unwrap_or("");
    let num = s.line.to_string();
    let pad = num.len();
    out.push_str(&format!("\n  --> {}:{}\n", file, s.line));
    out.push_str(&format!("{:>w$} |\n", "", w = pad + 1));
    out.push_str(&format!("{} | {}\n", num, line_text));
    if let Some((col, len)) = locate_caret(&s.kind, line_text, &s.message) {
        out.push_str(&format!(
            "{:>w$} | {}{}\n",
            "",
            " ".repeat(col),
            "^".repeat(len.max(1)),
            w = pad + 1
        ));
    }
    out.push_str(&format!("{:>w$} |\n", "", w = pad + 1));
    if let Some(note) = note_line(&s.message) {
        out.push_str(&format!("{:>w$} = {}\n", "", note, w = pad + 1));
    } else {
        out.push_str(&format!("{:>w$} = kind: {}\n", "", s.kind, w = pad + 1));
    }
    if !help_lines(&s.message).is_empty() {
        out.push_str("\nhelp: run with:\n");
        for h in help_lines(&s.message) {
            out.push_str(&format!("      {}\n", h));
        }
    }
    out
}

/// Machine-readable form (--json-errors). One object per fatal stress; the
/// schema is SPEC §9a's table. Fields a caller cannot know (column without a
/// located caret) are null, never guessed.
pub fn render_json(file: &str, src: &str, s: &Stress) -> String {
    let code = code_for(&s.kind, &s.message);
    let line_text = if s.line > 0 {
        src.lines().nth(s.line - 1).unwrap_or("")
    } else {
        ""
    };
    let (col, len) = locate_caret(&s.kind, line_text, &s.message)
        .map(|(c, l)| (c.to_string(), l.to_string()))
        .unwrap_or_else(|| ("null".into(), "null".into()));
    let chain: Vec<String> = s
        .chain
        .iter()
        .map(|(name, line)| {
            format!(
                "{{\"gene\": {}, \"line\": {}}}",
                json_str(name),
                if *line > 0 {
                    line.to_string()
                } else {
                    "null".into()
                }
            )
        })
        .collect();
    let help: Vec<String> = help_lines(&s.message).iter().map(|h| json_str(h)).collect();
    format!(
        "{{\"code\": {}, \"kind\": {}, \"message\": {}, \"file\": {}, \
         \"line\": {}, \"column\": {}, \"length\": {}, \"chain\": [{}], \"help\": [{}]}}\n",
        json_str(code),
        json_str(&s.kind),
        json_str(&s.message),
        json_str(file),
        if s.line > 0 {
            s.line.to_string()
        } else {
            "null".into()
        },
        col,
        len,
        chain.join(", "),
        help.join(", "),
    )
}

fn json_str(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
