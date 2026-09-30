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

/// W101 slice 3: color policy. Auto: only a terminal gets ANSI codes;
/// NO_COLOR (the de-facto no-color.org convention) always wins; pipes,
/// files and the golden gate stay byte-exact plain text. There is no
/// always-on flag on purpose: the golden fixtures are the contract, and
/// a --color=always that leaks into them would be a lie generator.
pub fn color_enabled(stream_is_tty: bool) -> bool {
    std::env::var_os("NO_COLOR").is_none() && stream_is_tty
}

/// Escape sequences are BUILT at runtime from pieces: no ANSI byte
/// sequence appears literally in this source file, so nothing in the
/// pipeline can strip a parameter and silently change the palette.
fn ansi(params: &str) -> String {
    let mut s = String::new();
    s.push('\u{1b}');
    s.push('[');
    s.push_str(params);
    s
}
fn c_reset() -> String {
    ansi("0m")
}
fn c_dim() -> String {
    ansi("2m")
}
fn c_bold() -> String {
    ansi("1m")
}
fn c_red() -> String {
    ansi("31m")
}
#[allow(dead_code)] // W101 palette: reserved for the warning sev rendering
fn c_yellow() -> String {
    ansi("33m")
}
#[allow(dead_code)] // W101 palette: reserved for the info sev rendering
fn c_cyan() -> String {
    ansi("36m")
}
fn c_blue() -> String {
    ansi("34m")
}

/// Severity word -> ANSI parameter. Unknown words render plain rather
/// than guessing a palette entry.
fn sev_code(sev: &str) -> &'static str {
    match sev {
        "error" => "31m",
        "warning" => "33m",
        "style" => "36m",
        _ => "",
    }
}

/// Display width of one char, the East-Asian-ambiguous-free subset: wide
/// ranges get 2, the common combining block gets 0, everything else 1.
/// Hand-rolled because the runtime is zero-dependency by policy; the
/// ranges below cover CJK ideographs, kana, Hangul and fullwidth forms,
/// which is every script our own corpus renders in carets today.
fn char_width(c: char) -> usize {
    let u = c as u32;
    if (0x0300..=0x036F).contains(&u) {
        return 0; // combining diacritics
    }
    let wide = (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0x303E).contains(&u)
        || (0x3041..=0x33FF).contains(&u)
        || (0x3400..=0x4DBF).contains(&u)
        || (0x4E00..=0x9FFF).contains(&u)
        || (0xA000..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE4F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x20000..=0x2FFFD).contains(&u)
        || (0x30000..=0x3FFFD).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

/// Display width of a str (caret alignment must survive CJK source text).
fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

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
/// Returns (char column, char length, byte offset, byte length): the char
/// pair feeds the JSON schema, the byte pair feeds width-correct caret
/// padding (CJK text must not tear the underline off its target).
/// Returns None when there is no honest caret.
fn locate_caret(
    kind: &str,
    line_text: &str,
    message: &str,
) -> Option<(usize, usize, usize, usize)> {
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
    let char_col = line_text[..byte_col].chars().count();
    Some((char_col, name.chars().count(), byte_col, name.len()))
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
#[allow(clippy::too_many_arguments)] // the finding shape IS 8-wide (SPEC 9a.1)
pub fn render_finding(
    file: &str,
    src: &str,
    sev: &str,
    code: &str,
    rule: &str,
    line: usize,
    message: &str,
    color: bool,
) -> String {
    let c = if color { sev_code(sev) } else { "" };
    let mut out = if c.is_empty() {
        format!("{}[{}]: {}\n", sev, code, message)
    } else {
        format!(
            "{}{}{}[{}]{}: {}\n",
            c_bold(),
            ansi(c),
            sev,
            code,
            c_reset(),
            message
        )
    };
    if line == 0 {
        return out;
    }
    let line_text = src.lines().nth(line - 1).unwrap_or("");
    let num = line.to_string();
    let pad = num.len();
    let dim = if color { c_dim() } else { String::new() };
    let blue = if color { c_blue() } else { String::new() };
    let reset = if color { c_reset() } else { String::new() };
    out.push_str(&format!("\n  --> {}:{}\n", file, line));
    out.push_str(&format!("{}{:>w$} |{}\n", dim, "", reset, w = pad + 1));
    out.push_str(&format!("{}{} |{} {}\n", blue, num, reset, line_text));
    out.push_str(&format!("{}{:>w$} |{}\n", dim, "", reset, w = pad + 1));
    out.push_str(&format!(
        "{}{:>w$} = rule: {}{}\n",
        dim,
        "",
        rule,
        reset,
        w = pad + 1
    ));
    out
}

/// The rendered text block (rustc shape). The W007 chain lines are appended
/// in the runner's established format after the block so existing parsers of
/// our stderr (cookbook expected files, redteam rc checks) keep working.
pub fn render(file: &str, src: &str, s: &Stress, color: bool) -> String {
    let code = code_for(&s.kind, &s.message);
    let header = if color {
        format!(
            "{}{}error[{}]{}: {}\n",
            c_bold(),
            c_red(),
            code,
            c_reset(),
            s.message
        )
    } else {
        format!("error[{}]: {}\n", code, s.message)
    };
    let mut out = header;
    if s.line == 0 {
        out.push('\n');
        return out;
    }
    let line_text = src.lines().nth(s.line - 1).unwrap_or("");
    let num = s.line.to_string();
    let pad = num.len();
    let dim = if color { c_dim() } else { String::new() };
    let blue = if color { c_blue() } else { String::new() };
    let reset = if color { c_reset() } else { String::new() };
    out.push_str(&format!("\n  --> {}:{}\n", file, s.line));
    out.push_str(&format!("{}{:>w$} |{}\n", dim, "", reset, w = pad + 1));
    out.push_str(&format!("{}{} |{} {}\n", blue, num, reset, line_text));
    if let Some((_cc, _cl, bc, bl)) = locate_caret(&s.kind, line_text, &s.message) {
        let lead = str_width(&line_text[..bc]);
        let mark = str_width(&line_text[bc..bc + bl]).max(1);
        let caret = if color { c_red() } else { String::new() };
        out.push_str(&format!(
            "{}{:>w$} | {}{}{}{}\n",
            dim,
            "",
            reset,
            " ".repeat(lead),
            caret,
            "^".repeat(mark),
            w = pad + 1,
        ));
    }
    out.push_str(&format!("{}{:>w$} |{}\n", dim, "", reset, w = pad + 1));
    if let Some(note) = note_line(&s.message) {
        out.push_str(&format!(
            "{}{:>w$} = {}{}\n",
            dim,
            "",
            note,
            reset,
            w = pad + 1
        ));
    } else {
        out.push_str(&format!(
            "{}{:>w$} = kind: {}{}\n",
            dim,
            "",
            s.kind,
            reset,
            w = pad + 1
        ));
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
        .map(|(c, l, _, _)| (c.to_string(), l.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_survives_cjk_source_text() {
        // the caret must land under the call name even when CJK text
        // precedes it: display width, not char count, aligns the row
        let line = "let \u{8def}\u{5f84} = read_file('x')";
        assert_eq!(str_width("\u{8def}\u{5f84}"), 4);
        assert_eq!(str_width(line), line.chars().count() + 2);
    }

    #[test]
    fn json_columns_stay_char_based() {
        // JSON consumers get the char column (SPEC 9a schema), the text
        // renderer gets the byte offset for padding; both come from one
        // locate_caret call so they cannot disagree
        let line = "\u{6f2c}\u{5b57} read_file('x')";
        let (cc, cl, bc, bl) =
            locate_caret("interference", line, "read_file denied: no grant").unwrap();
        assert_eq!((cc, cl), (3, 9));
        assert_eq!((bc, bl), (7, 9));
    }

    #[test]
    fn color_is_off_for_pipes_and_honors_no_color() {
        // pipes/files/goldens never get ANSI codes; NO_COLOR wins even on
        // a (pretend) tty
        assert!(!color_enabled(false));
        std::env::set_var("NO_COLOR", "1");
        assert!(!color_enabled(true));
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn plain_and_colored_blocks_carry_the_same_content() {
        let src = "gene main() {\n  let dead = 1\n}\n";
        let plain = render_finding(
            "f.op",
            src,
            "warning",
            "W07",
            "unused-binding",
            2,
            "msg",
            false,
        );
        assert!(!plain.contains('\x1b'), "pipe output must be plain");
        let colored = render_finding(
            "f.op",
            src,
            "warning",
            "W07",
            "unused-binding",
            2,
            "msg",
            true,
        );
        assert!(colored.contains(&c_yellow()));
        assert!(colored.contains(&c_reset()));
        // the visible words are identical: severity+code header, location
        // and rule note appear in both; escapes wrap, never replace, words
        assert!(colored.contains("warning[W07]"));
        assert!(colored.contains(": msg"));
        for marker in ["--> f.op:2", "let dead = 1", "= rule: unused-binding"] {
            assert!(plain.contains(marker), "plain missing: {}", marker);
            assert!(colored.contains(marker), "colored missing: {}", marker);
        }
    }
}
