//! lexer.rs — tokenizer. Produces raw tokens; keyword recognition happens in
//! the parser so the wobble ladder (rungs 2/3) can repair at the positions
//! where a keyword is actually required.

use crate::ast::Note;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),    // no interpolation present
    Interp(String), // raw content, contains {..} parts
    Mark(String),   // @word
    Newline,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    Colon,
    Arrow,    // ->
    FatArrow, // =>
    Plus,
    Minus,
    Star,
    Slash,
    DSlash,
    Percent,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    DSlashEq,
    PercentEq,
    StarStar,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Question,
    Eq,
    EqEq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    AmpAmp,
    PipePipe,
    Bang,
    Dot,
    Semi,
    Eof,
}

impl Tok {
    /// dx-r1: programmer-facing token name for diagnostics. Rust's Debug
    /// output leaked internal enum shapes like `Str("boom")` into user
    /// messages; describe() speaks in source terms instead.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier '{}'", s),
            Tok::Int(n) => format!("number {}", n),
            Tok::Float(f) => format!("number {}", f),
            Tok::Str(s) => format!("string \"{}\"", s),
            Tok::Interp(s) => format!("interpolated string \"{}\"", s),
            Tok::Mark(m) => format!("mark '@{}'", m),
            Tok::Newline => "end of line".to_string(),
            Tok::LBrace => "'{'".to_string(),
            Tok::RBrace => "'}'".to_string(),
            Tok::LParen => "'('".to_string(),
            Tok::RParen => "')'".to_string(),
            Tok::LBrack => "'['".to_string(),
            Tok::RBrack => "']'".to_string(),
            Tok::Comma => "','".to_string(),
            Tok::Colon => "':'".to_string(),
            Tok::Arrow => "'->'".to_string(),
            Tok::FatArrow => "'=>'".to_string(),
            Tok::Plus => "'+'".to_string(),
            Tok::Minus => "'-'".to_string(),
            Tok::Star => "'*'".to_string(),
            Tok::Slash => "'/'".to_string(),
            Tok::DSlash => "'//'".to_string(),
            Tok::Percent => "'%'".to_string(),
            Tok::PlusEq => "'+='".to_string(),
            Tok::MinusEq => "'-='".to_string(),
            Tok::StarEq => "'*='".to_string(),
            Tok::SlashEq => "'/='".to_string(),
            Tok::DSlashEq => "'//='".to_string(),
            Tok::PercentEq => "'%='".to_string(),
            Tok::StarStar => "'**'".to_string(),
            Tok::Amp => "'&'".to_string(),
            Tok::Pipe => "'|'".to_string(),
            Tok::Caret => "'^'".to_string(),
            Tok::Tilde => "'~'".to_string(),
            Tok::Shl => "'<<'".to_string(),
            Tok::Shr => "'>>'".to_string(),
            Tok::Question => "'?'".to_string(),
            Tok::Eq => "'='".to_string(),
            Tok::EqEq => "'=='".to_string(),
            Tok::Neq => "'!='".to_string(),
            Tok::Lt => "'<'".to_string(),
            Tok::Le => "'<='".to_string(),
            Tok::Gt => "'>'".to_string(),
            Tok::Ge => "'>='".to_string(),
            Tok::AmpAmp => "'&&'".to_string(),
            Tok::PipePipe => "'||'".to_string(),
            Tok::Bang => "'!'".to_string(),
            Tok::Dot => "'.'".to_string(),
            Tok::Semi => "';'".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }
}

pub struct Lexed {
    pub toks: Vec<(Tok, usize)>, // token + line
    pub notes: Vec<Note>,
}

pub fn lex(src: &str) -> Lexed {
    let mut toks: Vec<(Tok, usize)> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1usize;
    let n = chars.len();

    macro_rules! push {
        ($t:expr) => {
            toks.push(($t, line))
        };
    }

    while i < n {
        let c = chars[i];
        // whitespace (not newline)
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            continue;
        }
        if c == '\n' {
            // collapse repeated newlines
            if let Some((Tok::Newline, _)) = toks.last() {
                i += 1;
                line += 1;
                continue;
            }
            push!(Tok::Newline);
            i += 1;
            line += 1;
            continue;
        }
        // comments
        if c == '#' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // strings
        if c == '"' {
            let (tok, note) = lex_string(&chars, &mut i, &mut line, '"');
            if let Some(msg) = note {
                notes.push(Note {
                    line,
                    rung: 4,
                    message: msg,
                });
            }
            push!(tok);
            continue;
        }
        // single-quote string → wobble: treat as double-quoted (rung 4)
        if c == '\'' {
            notes.push(Note {
                line,
                rung: 4,
                message: "single-quoted string repaired to double quotes".into(),
            });
            let (tok, _note) = lex_string(&chars, &mut i, &mut line, '\'');
            push!(tok);
            continue;
        }
        // marks
        if c == '@' {
            let start = i + 1;
            let mut j = start;
            while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[start..j].iter().collect();
            if word.is_empty() {
                notes.push(Note {
                    line,
                    rung: 4,
                    message: "stray '@' skipped".into(),
                });
            } else {
                push!(Tok::Mark(word));
            }
            i = j;
            continue;
        }
        // numbers
        if c.is_ascii_digit() {
            let start = i;
            let mut is_float = false;
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                if chars[i] == '.' {
                    // don't consume a dot that isn't part of a number (e.g. 1.map)
                    if i + 1 >= n || !chars[i + 1].is_ascii_digit() {
                        break;
                    }
                    is_float = true;
                }
                i += 1;
            }
            // exponent
            if i < n && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < n && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < n && chars[j].is_ascii_digit() {
                    is_float = true;
                    i = j;
                    while i < n && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            if is_float {
                match text.parse::<f64>() {
                    Ok(f) => push!(Tok::Float(f)),
                    Err(_) => notes.push(Note {
                        line,
                        rung: 4,
                        message: format!("malformed number '{}' treated as 0", text),
                    }),
                }
            } else {
                match text.parse::<i64>() {
                    Ok(v) => push!(Tok::Int(v)),
                    Err(_) => notes.push(Note {
                        line,
                        rung: 4,
                        message: format!("integer '{}' out of range treated as 0", text),
                    }),
                }
            }
            continue;
        }
        // identifiers / keywords — every name is interned into the C table,
        // which becomes the canonical record of all symbols in all files
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            crate::ffi::intern(&word);
            push!(Tok::Ident(word));
            continue;
        }
        // operators
        match c {
            '{' => {
                push!(Tok::LBrace);
                i += 1;
            }
            '}' => {
                push!(Tok::RBrace);
                i += 1;
            }
            '(' => {
                push!(Tok::LParen);
                i += 1;
            }
            ')' => {
                push!(Tok::RParen);
                i += 1;
            }
            '[' => {
                push!(Tok::LBrack);
                i += 1;
            }
            ']' => {
                push!(Tok::RBrack);
                i += 1;
            }
            ',' => {
                push!(Tok::Comma);
                i += 1;
            }
            ':' => {
                push!(Tok::Colon);
                i += 1;
            }
            '.' => {
                push!(Tok::Dot);
                i += 1;
            }
            ';' => {
                push!(Tok::Semi);
                i += 1;
            }
            '+' => {
                if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::PlusEq);
                    i += 2;
                } else {
                    push!(Tok::Plus);
                    i += 1;
                }
            }
            '*' => {
                if i + 1 < n && chars[i + 1] == '*' {
                    push!(Tok::StarStar);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::StarEq);
                    i += 2;
                } else {
                    push!(Tok::Star);
                    i += 1;
                }
            }
            '/' => {
                if i + 1 < n && chars[i + 1] == '/' {
                    if i + 2 < n && chars[i + 2] == '=' {
                        push!(Tok::DSlashEq);
                        i += 3;
                    } else {
                        push!(Tok::DSlash);
                        i += 2;
                    }
                } else if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::SlashEq);
                    i += 2;
                } else {
                    push!(Tok::Slash);
                    i += 1;
                }
            }
            '%' => {
                if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::PercentEq);
                    i += 2;
                } else {
                    push!(Tok::Percent);
                    i += 1;
                }
            }
            '-' => {
                if i + 1 < n && chars[i + 1] == '>' {
                    push!(Tok::Arrow);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::MinusEq);
                    i += 2;
                } else {
                    push!(Tok::Minus);
                    i += 1;
                }
            }
            '=' => {
                if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::EqEq);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '>' {
                    push!(Tok::FatArrow);
                    i += 2;
                } else {
                    push!(Tok::Eq);
                    i += 1;
                }
            }
            '!' => {
                if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::Neq);
                    i += 2;
                } else {
                    push!(Tok::Bang);
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < n && chars[i + 1] == '<' {
                    push!(Tok::Shl);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::Le);
                    i += 2;
                } else {
                    push!(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < n && chars[i + 1] == '>' {
                    push!(Tok::Shr);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '=' {
                    push!(Tok::Ge);
                    i += 2;
                } else {
                    push!(Tok::Gt);
                    i += 1;
                }
            }
            '&' => {
                if i + 1 < n && chars[i + 1] == '&' {
                    push!(Tok::AmpAmp);
                    i += 2;
                } else {
                    push!(Tok::Amp);
                    i += 1;
                }
            }
            '|' => {
                if i + 1 < n && chars[i + 1] == '|' {
                    push!(Tok::PipePipe);
                    i += 2;
                } else {
                    push!(Tok::Pipe);
                    i += 1;
                }
            }
            '^' => {
                push!(Tok::Caret);
                i += 1;
            }
            '~' => {
                push!(Tok::Tilde);
                i += 1;
            }
            '?' => {
                push!(Tok::Question);
                i += 1;
            }
            other => {
                notes.push(Note {
                    line,
                    rung: 4,
                    message: format!("unexpected character '{}' skipped", other),
                });
                i += 1;
            }
        }
    }

    push!(Tok::Eof);
    Lexed { toks, notes }
}

/// Lex a double-quoted (or repaired single-quoted) string starting at the
/// opening quote. Handles escapes; returns Interp token if {..} parts exist.
fn lex_string(
    chars: &[char],
    i: &mut usize,
    line: &mut usize,
    quote: char,
) -> (Tok, Option<String>) {
    let n = chars.len();
    *i += 1; // skip opening quote
    let mut raw = String::new();
    let mut closed = false;
    let mut has_interp = false;
    let mut depth = 0usize;
    while *i < n {
        let c = chars[*i];
        if c == '\n' {
            *line += 1;
            raw.push('\n');
            *i += 1;
            continue;
        }
        if c == '\\' && *i + 1 < n {
            let e = chars[*i + 1];
            match e {
                'n' => raw.push('\n'),
                't' => raw.push('\t'),
                '\\' => raw.push('\\'),
                '"' => raw.push('"'),
                '{' => raw.push('{'),
                '}' => raw.push('}'),
                other => {
                    raw.push('\\');
                    raw.push(other);
                }
            }
            *i += 2;
            continue;
        }
        if depth == 0 && c == quote {
            *i += 1;
            closed = true;
            break;
        }
        if c == '{' {
            depth += 1;
            has_interp = true;
        }
        if c == '}' && depth > 0 {
            depth -= 1;
        }
        raw.push(c);
        *i += 1;
    }
    let note = if closed {
        None
    } else {
        Some("unclosed string consumed to end of line".to_string())
    };
    let tok = if has_interp {
        Tok::Interp(raw)
    } else {
        Tok::Str(raw)
    };
    (tok, note)
}
