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

/// W101 slice 5: a located range in one source line. Char columns feed the
/// JSON schema (SPEC 9a), byte offsets feed width-correct caret padding (CJK
/// text must not tear the underline off its target). `line` is 1-based; a
/// zero line means "unknown location", and an unknown location is NEVER
/// upgraded to a guessed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub byte_col: usize,
    pub byte_len: usize,
}

impl Span {
    /// The span of `name`'s first occurrence on `line_text`.
    pub fn of_token(line: usize, line_text: &str, name: &str) -> Option<Span> {
        let byte_col = line_text.find(name)?;
        Some(Span {
            line,
            col: line_text[..byte_col].chars().count() + 1,
            len: name.chars().count(),
            byte_col,
            byte_len: name.len(),
        })
    }

    /// W101 slice 6: the word-boundary variant. The first occurrence of
    /// `name` as a WHOLE token (a bare substring hit inside a longer
    /// identifier, `min` inside `admin`, would underline the wrong word).
    /// None when the name never appears as a token on the line.
    pub fn of_token_word(line: usize, line_text: &str, name: &str) -> Option<Span> {
        let is_id = |i: usize| {
            line_text[i..]
                .chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric() || c == '_')
                .unwrap_or(false)
        };
        let mut from = 0usize;
        while let Some(pos) = line_text[from..].find(name) {
            let abs = from + pos;
            let after = abs + name.len();
            let left_ok = abs == 0 || !is_id(abs - 1);
            let right_ok = after >= line_text.len() || !is_id(after);
            if left_ok && right_ok {
                return Some(Span {
                    line,
                    col: line_text[..abs].chars().count() + 1,
                    len: name.chars().count(),
                    byte_col: abs,
                    byte_len: name.len(),
                });
            }
            from = abs + 1;
        }
        None
    }
}

/// W101 slice 5: one underlined range plus its message. Primary labels mark
/// the caret row (`^^^^`), secondary labels add context (`----`); a label
/// with empty text underlines silently. This is the engine's input shape:
/// today's heuristic locate site produces zero or one primary label, and the
/// multi-label path exists so the next consumers (did-you-mean, check) never
/// grow a second, divergent renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub text: String,
    pub primary: bool,
}

/// Locate the caret on the raise line, CAPABILITY DENIALS ONLY: the first
/// word of the message is the failing call/builtin name (read_file 'x': ...);
/// underline its first occurrence on the line. Other kinds honestly get no
/// caret (the AST is line-only today; dx-r2 spans do not carry columns).
/// Returns None when there is no honest caret.
fn locate_span(kind: &str, line: usize, line_text: &str, message: &str) -> Option<Span> {
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
    Span::of_token(line, line_text, &name)
}

/// W101 slice 5: the multi-label locate site. Today's heuristic produces at
/// most ONE primary label (the capability-denial caret); the function is the
/// single place later consumers extend, so the renderer never grows a second
/// locate path.
fn located_labels(kind: &str, line: usize, line_text: &str, message: &str) -> Vec<Label> {
    match locate_span(kind, line, line_text, message) {
        Some(span) => vec![Label {
            span,
            text: String::new(),
            primary: true,
        }],
        None => Vec::new(),
    }
}

/// W101 slice 5: render the underline row(s) for one source line's labels.
/// Same-line labels render on ONE row, left to right, primary `^` and
/// secondary `-`; a secondary that overlaps an earlier segment is dropped
/// honestly (an underline that starts inside another is a lie about where
/// things are). Each label's text renders after its own segment. The math is
/// display-width based, so CJK source keeps the underline under its target.
fn label_rows(
    line_text: &str,
    labels: &[&Label],
    pad: usize,
    dim: &str,
    reset: &str,
    primary_c: &str,
    secondary_c: &str,
) -> String {
    let mut segs: Vec<&&Label> = labels.iter().collect();
    segs.sort_by_key(|l| l.span.byte_col);
    let mut row = String::new();
    let mut cursor = 0usize; // byte offset in line_text where the next mark may start
    let mut drew_any = false;
    for l in segs {
        if l.span.byte_col < cursor || l.span.byte_col > line_text.len() {
            continue; // overlap or out of range: degrade, never lie
        }
        let lead = str_width(&line_text[cursor..l.span.byte_col]);
        let mark = str_width(&line_text[l.span.byte_col..l.span.byte_col + l.span.byte_len]).max(1);
        row.push_str(&" ".repeat(lead));
        let c = if l.primary { primary_c } else { secondary_c };
        row.push_str(c);
        let ch = if l.primary { '^' } else { '-' };
        row.push_str(&ch.to_string().repeat(mark));
        row.push_str(reset);
        if !l.text.is_empty() {
            row.push(' ');
            row.push_str(&l.text);
        }
        cursor = l.span.byte_col + l.span.byte_len;
        drew_any = true;
    }
    if !drew_any {
        return String::new();
    }
    format!("{}{:>w$} | {}{}\n", dim, "", reset, row, w = pad + 1)
}

/// W101 slice 6: did-you-mean, the render-side twin of the runtime wobble
/// ladder. Same thresholds as the interpreter's nearest-callable repair (the
/// two must never disagree about what "near" means): distance 1 for short
/// names, 2 otherwise; zero-dependency via ffi::edit_distance. Sorted by
/// (distance, name) so the output is deterministic; capped at 3 because a
/// longer list is noise, not help.
pub fn did_you_mean(name: &str, candidates: &[&str]) -> Vec<String> {
    let max = if name.chars().count() <= 4 { 1 } else { 2 };
    let mut hits: Vec<(i32, String)> = Vec::new();
    for c in candidates {
        if *c == name {
            continue; // the name itself is not a suggestion
        }
        let d = crate::ffi::edit_distance(name, c);
        if d <= max {
            hits.push((d, c.to_string()));
        }
    }
    hits.sort();
    hits.dedup();
    hits.truncate(3);
    hits.into_iter().map(|(_, c)| c).collect()
}

/// W101 slice 6: a machine-applicable source edit. Char column + length feed
/// the JSON schema (same convention as the labels array); `note` says what
/// applying it does, in the operator's words, not the engine's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedFix {
    pub line: usize,
    pub column: usize,
    pub length: usize,
    pub replacement: String,
    pub note: String,
}

impl SuggestedFix {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"line\": {}, \"column\": {}, \"length\": {}, \"replacement\": {}, \"note\": {}}}",
            self.line,
            self.column,
            self.length,
            json_str(&self.replacement),
            json_str(&self.note),
        )
    }
}

/// W101 slice 7: the parse/repair note-code catalog (SPEC 9a.1). Notes are
/// the Total Grammar transparency channel (repair, never reject); their
/// codes are DERIVED from the message by this pure function, the same
/// pattern as `code_for` for stresses, so no emit site had to change. The
/// matching is ordered substring phrases: specific families first, generic
/// families last. New families APPEND; codes never renumber. A note whose
/// message matches nothing renders uncoded (honest absence, not a guess).
/// Full emit-site inventory: docs/diagnostics-inventory.md.
pub fn note_code(message: &str) -> &'static str {
    // -- cap / suppression (lexer + parser + interp share the phrasing)
    if message.contains("note cap") {
        return "E2000";
    }
    // -- lexer repairs (E2001-E2009)
    if message.contains("single-quoted string repaired") {
        return "E2001";
    }
    if message.contains("single-quoted bytes literal repaired") {
        return "E2002";
    }
    if message.contains("malformed \\x escape") {
        return "E2003";
    }
    if message.contains("non-ASCII char in bytes literal") {
        return "E2004";
    }
    if message.contains("unclosed bytes literal") {
        return "E2005";
    }
    if message.contains("unclosed multiline string") || message.contains("unclosed raw string") {
        return "E2006";
    }
    if message.contains("stray '@' skipped") {
        return "E2007";
    }
    if message.contains("out of range treated as 0") {
        return "E2008";
    }
    if message.contains("malformed number") {
        return "E2009";
    }
    // -- parser repairs (E2010-E2019)
    if message.starts_with("synonym ") {
        return "E2010";
    }
    if message.starts_with("wobble: ") {
        return "E2011";
    }
    if message.contains("unmatched '}' skipped") {
        return "E2012";
    }
    if message.contains("auto-closed") {
        return "E2013";
    }
    if message.contains("unknown mark '@") {
        return "E2018";
    }
    if message.contains("binds null") {
        return "E2015";
    }
    if message.contains("needs ") || message.contains("missing 'in'") {
        return "E2016";
    }
    if message.contains("treated as") {
        return "E2017";
    }
    if message.contains("null substituted") {
        return "E2019";
    }
    if message.contains("skipped") {
        return "E2014";
    }
    // -- runtime operational notes (E203x; families append)
    if message.starts_with("phantom call to ") {
        return "E2030";
    }
    ""
}

/// The help line(s) for a suggestion list: one candidate reads as a direct
/// question, several as an honest shortlist, none renders nothing.
/// Public because print_diag composes phantom messages with the same wording
/// (one source of truth for the did-you-mean sentence shape).
pub fn suggestion_help(suggestions: &[String]) -> Option<String> {
    match suggestions.len() {
        0 => None,
        1 => Some(format!("did you mean '{}'?", suggestions[0])),
        _ => {
            let list: Vec<String> = suggestions.iter().map(|s| format!("'{}'", s)).collect();
            Some(format!("some similar names: {}", list.join(", ")))
        }
    }
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
    render_finding_labeled(file, src, sev, code, rule, line, message, color, &[])
}

/// W101 slice 6: the labeled variant — same block, plus underline rows for
/// the caller's labels. Phantoms underline their call token here; findings
/// that have no honest token stay on the unlabeled path (empty slice).
#[allow(clippy::too_many_arguments)] // the labeled finding shape IS 9-wide
pub fn render_finding_labeled(
    file: &str,
    src: &str,
    sev: &str,
    code: &str,
    rule: &str,
    line: usize,
    message: &str,
    color: bool,
    labels: &[Label],
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
    if !labels.is_empty() {
        let refs: Vec<&Label> = labels.iter().collect();
        let (pc, sc) = if color {
            (ansi(sev_code(sev)), c_blue())
        } else {
            (String::new(), String::new())
        };
        out.push_str(&label_rows(line_text, &refs, pad, &dim, &reset, &pc, &sc));
    }
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
/// `suggestions` (W101 slice 6) are did-you-mean names for the failing name;
/// they render as the FIRST help section (name-level help before grant-level
/// help), and keep rendering when the block is header-only (line 0), which
/// is the typo'd --entry shape.
pub fn render(file: &str, src: &str, s: &Stress, color: bool, suggestions: &[String]) -> String {
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
        if let Some(h) = suggestion_help(suggestions) {
            out.push_str(&format!("\nhelp: {}\n", h));
        }
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
    let labels = located_labels(&s.kind, s.line, line_text, &s.message);
    let refs: Vec<&Label> = labels.iter().collect();
    let (pc, sc) = if color {
        (c_red(), c_blue())
    } else {
        (String::new(), String::new())
    };
    out.push_str(&label_rows(line_text, &refs, pad, &dim, &reset, &pc, &sc));
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
    if let Some(h) = suggestion_help(suggestions) {
        out.push_str(&format!("\nhelp: {}\n", h));
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
/// located caret) are null, never guessed. `labels` (W101 slice 5) carries
/// every located range as {line, column, length, text, primary}; an error
/// with no honest location renders an empty array, not a fabricated one.
/// `suggestions` (W101 slice 6) is the did-you-mean shortlist.
pub fn render_json(file: &str, src: &str, s: &Stress, suggestions: &[String]) -> String {
    let code = code_for(&s.kind, &s.message);
    let line_text = if s.line > 0 {
        src.lines().nth(s.line - 1).unwrap_or("")
    } else {
        ""
    };
    let labels = located_labels(&s.kind, s.line, line_text, &s.message);
    let (col, len) = labels
        .first()
        .map(|l| (l.span.col.to_string(), l.span.len.to_string()))
        .unwrap_or_else(|| ("null".into(), "null".into()));
    let labels_json: Vec<String> = labels
        .iter()
        .map(|l| {
            format!(
                "{{\"line\": {}, \"column\": {}, \"length\": {}, \"text\": {}, \"primary\": {}}}",
                l.span.line,
                l.span.col,
                l.span.len,
                json_str(&l.text),
                l.primary,
            )
        })
        .collect();
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
    let sug: Vec<String> = suggestions.iter().map(|s| json_str(s)).collect();
    format!(
        "{{\"code\": {}, \"kind\": {}, \"message\": {}, \"file\": {}, \
         \"line\": {}, \"column\": {}, \"length\": {}, \"chain\": [{}], \"help\": [{}], \"labels\": [{}], \"suggestions\": [{}]}}\n",
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
        labels_json.join(", "),
        sug.join(", "),
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
        // locate_span call so they cannot disagree
        let line = "\u{6f2c}\u{5b57} read_file('x')";
        let span = locate_span("interference", 3, line, "read_file denied: no grant").unwrap();
        assert_eq!((span.col, span.len), (4, 9));
        assert_eq!((span.byte_col, span.byte_len), (7, 9));
        assert_eq!(span.line, 3);
    }

    #[test]
    fn multi_label_row_orders_segments_and_marks_kinds() {
        // two same-line labels: primary under the target, secondary context
        // left of it, each with its text; secondary renders dashes
        let line = "let x = handle(a, b)";
        let labels = vec![
            Label {
                span: Span::of_token(1, line, "handle").unwrap(),
                text: String::new(),
                primary: true,
            },
            Label {
                span: Span::of_token(1, line, "b").unwrap(),
                text: "second arg".into(),
                primary: false,
            },
        ];
        let refs: Vec<&Label> = labels.iter().collect();
        let row = label_rows(line, &refs, 1, "", "", "", "");
        let body = row.split("| ").nth(1).unwrap();
        // byte order wins: the primary underline under `handle` renders
        // first, then the secondary with its text after it
        assert!(body.contains("^^^^^^"));
        assert!(body.contains("- second arg"));
        assert!(body.find("^^^^^^").unwrap() < body.find("- second").unwrap());
    }

    #[test]
    fn overlapping_secondary_label_is_dropped_not_lied_about() {
        // a secondary span starting INSIDE the primary's underline would
        // double-mark the same bytes; the honest render drops it
        let line = "read_file('x')";
        let labels = vec![
            Label {
                span: Span::of_token(1, line, "read_file").unwrap(),
                text: String::new(),
                primary: true,
            },
            Label {
                span: Span::of_token(1, line, "ile").unwrap(),
                text: "overlaps".into(),
                primary: false,
            },
        ];
        let refs: Vec<&Label> = labels.iter().collect();
        let row = label_rows(line, &refs, 1, "", "", "", "");
        assert!(!row.contains("overlaps"));
        assert!(row.contains("^^^^"));
    }

    #[test]
    fn label_rows_degrade_to_empty_without_labels() {
        // no honest location: no caret row at all (never a guessed one)
        let line = "let ok = 1";
        let row = label_rows(line, &[], 1, "", "", "", "");
        assert!(row.is_empty());
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
    fn did_you_mean_matches_the_runtime_wobble_thresholds() {
        // the render-side suggestion and the interpreter's nearest-callable
        // repair must agree on what "near" means: distance 1 for short
        // names, 2 for longer ones
        let cands = vec!["main", "mini", "promote", "print"];
        assert_eq!(did_you_mean("maiin", &cands), vec!["main".to_string()]);
        // 6 chars, distance 2 to BOTH "promote" (sub n→m, insert o) and
        // "print" (sub o→i, delete e) → both in range, deterministic order
        assert_eq!(
            did_you_mean("pronte", &cands),
            vec!["print".to_string(), "promote".to_string()]
        );
        // 5 chars, distance 3 to the nearest ("maaai" → "main") → honest empty
        assert!(did_you_mean("maaai", &cands).is_empty());
        // the name itself is never its own suggestion
        assert!(did_you_mean("main", &cands).is_empty());
        // far-off names suggest nothing, honestly
        assert!(did_you_mean("zzzzzz", &cands).is_empty());
        // deterministic order, capped at 3
        let many = vec!["aa", "ab", "ac", "ad", "ae"];
        let got = did_you_mean("ax", &many);
        assert_eq!(
            got,
            vec!["aa".to_string(), "ab".to_string(), "ac".to_string()]
        );
    }

    #[test]
    fn note_codes_cover_the_repair_families() {
        // lexer repairs map by phrase; the cap notice is one family for all
        // three emitters (lex/parse/interp share the wording)
        assert_eq!(
            note_code("lex note cap (10000) reached - further notes suppressed"),
            "E2000"
        );
        assert_eq!(
            note_code("single-quoted string repaired to double quotes"),
            "E2001"
        );
        assert_eq!(
            note_code("malformed \\x escape in bytes literal kept verbatim"),
            "E2003"
        );
        assert_eq!(
            note_code("unclosed bytes literal consumed to end of input"),
            "E2005"
        );
        assert_eq!(
            note_code("integer '99999999999999999999' out of range treated as 0"),
            "E2008"
        );
        // parser repairs: rung phrasings first, generic skipped last
        assert_eq!(note_code("synonym 'elseif' repaired to 'elif'"), "E2010");
        assert_eq!(
            note_code("wobble: unknown gene 'mainn' repaired to builtin 'main'"),
            "E2011"
        );
        assert_eq!(note_code("'let x' without value binds null"), "E2015");
        assert_eq!(note_code("unknown mark '@zzz' skipped"), "E2018");
        assert_eq!(note_code("unmatched '}' skipped"), "E2012");
        assert_eq!(
            note_code("unexpected token 'end of line' in expression; null substituted"),
            "E2019"
        );
        assert_eq!(
            note_code("bare name block 'elseif' treated as gene definition"),
            "E2017"
        );
        // runtime operational note family
        assert_eq!(note_code("phantom call to 'foo'; result null"), "E2030");
        // no family match → honestly uncoded
        assert_eq!(note_code("cell config 'x.cell' unreadable: ENOENT"), "");
    }

    #[test]
    fn of_token_word_respects_identifier_boundaries() {
        let line = "admin(min)";
        // "min" inside "admin" is NOT the token; the standalone min() call is
        let span = Span::of_token_word(1, line, "min").unwrap();
        assert_eq!(span.byte_col, 6);
        assert_eq!(span.col, 7);
        // no token occurrence → no span, never a guessed one
        assert!(Span::of_token_word(1, line, "pad").is_none());
    }

    #[test]
    fn suggestion_help_reads_like_a_question() {
        assert!(suggestion_help(&[]).is_none());
        assert_eq!(
            suggestion_help(&["main".to_string()]).unwrap(),
            "did you mean 'main'?"
        );
        assert_eq!(
            suggestion_help(&["a".to_string(), "b".to_string()]).unwrap(),
            "some similar names: 'a', 'b'"
        );
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
