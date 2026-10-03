// mini_json.rs — compact JSON parser (std-only, no serde).
// This file exists BECAUSE Rust ships no JSON runtime: a real LOC cost of
// the native-no-runtime choice, counted honestly in the benchmark report.
// Supports the subset yt-dlp -J / ffprobe emit: objects, arrays, strings
// (with \u escapes), numbers, true/false/null. Depth-capped, no recursion
// bombs on adversarial input.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => "",
        }
    }
    pub fn as_arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }
}

pub fn parse(src: &str) -> Result<Json, String> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let v = parse_value(b, &mut i, 0)?;
    skip_ws(b, &mut i);
    if i != b.len() {
        return Err(format!("trailing bytes at {}", i));
    }
    Ok(v)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn parse_value(b: &[u8], i: &mut usize, depth: usize) -> Result<Json, String> {
    if depth > 128 {
        return Err("depth cap exceeded".into());
    }
    skip_ws(b, i);
    if *i >= b.len() {
        return Err("unexpected end".into());
    }
    match b[*i] {
        b'{' => parse_obj(b, i, depth),
        b'[' => parse_arr(b, i, depth),
        b'"' => Ok(Json::Str(parse_str(b, i)?)),
        b't' => lit(b, i, "true", Json::Bool(true)),
        b'f' => lit(b, i, "false", Json::Bool(false)),
        b'n' => lit(b, i, "null", Json::Null),
        _ => parse_num(b, i),
    }
}

fn lit(b: &[u8], i: &mut usize, s: &str, v: Json) -> Result<Json, String> {
    if b[*i..].starts_with(s.as_bytes()) {
        *i += s.len();
        Ok(v)
    } else {
        Err(format!("bad literal at {}", i))
    }
}

fn parse_num(b: &[u8], i: &mut usize) -> Result<Json, String> {
    let start = *i;
    while *i < b.len()
        && matches!(b[*i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
    {
        *i += 1;
    }
    std::str::from_utf8(&b[start..*i])
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(Json::Num)
        .ok_or_else(|| format!("bad number at {}", start))
}

fn parse_str(b: &[u8], i: &mut usize) -> Result<String, String> {
    *i += 1; // opening quote
    let mut out = String::new();
    while *i < b.len() {
        match b[*i] {
            b'"' => {
                *i += 1;
                return Ok(out);
            }
            b'\\' => {
                *i += 1;
                if *i >= b.len() {
                    return Err("bad escape".into());
                }
                match b[*i] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'u' => {
                        if *i + 4 >= b.len() {
                            return Err("bad \\u".into());
                        }
                        let hex = std::str::from_utf8(&b[*i + 1..*i + 5])
                            .map_err(|_| "bad \\u".to_string())?;
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| "bad \\u hex".to_string())?;
                        out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        *i += 4;
                    }
                    _ => return Err("bad escape char".into()),
                }
                *i += 1;
            }
            _ => {
                // consume one UTF-8 scalar
                let start = *i;
                let len = utf8_len(b[*i]);
                *i += len;
                out.push_str(
                    std::str::from_utf8(&b[start..(*i).min(b.len())])
                        .map_err(|_| "bad utf8".to_string())?,
                );
            }
        }
    }
    Err("unterminated string".into())
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn parse_arr(b: &[u8], i: &mut usize, depth: usize) -> Result<Json, String> {
    *i += 1;
    let mut out = Vec::new();
    loop {
        skip_ws(b, i);
        if *i < b.len() && b[*i] == b']' {
            *i += 1;
            return Ok(Json::Arr(out));
        }
        out.push(parse_value(b, i, depth + 1)?);
        skip_ws(b, i);
        if *i < b.len() && b[*i] == b',' {
            *i += 1;
        } else if *i < b.len() && b[*i] == b']' {
            *i += 1;
            return Ok(Json::Arr(out));
        } else {
            return Err(format!("bad array at {}", i));
        }
    }
}

fn parse_obj(b: &[u8], i: &mut usize, depth: usize) -> Result<Json, String> {
    *i += 1;
    let mut out = Vec::new();
    loop {
        skip_ws(b, i);
        if *i < b.len() && b[*i] == b'}' {
            *i += 1;
            return Ok(Json::Obj(out));
        }
        if *i >= b.len() || b[*i] != b'"' {
            return Err(format!("expected key at {}", i));
        }
        let k = parse_str(b, i)?;
        skip_ws(b, i);
        if *i >= b.len() || b[*i] != b':' {
            return Err(format!("expected ':' at {}", i));
        }
        *i += 1;
        let v = parse_value(b, i, depth + 1)?;
        out.push((k, v));
        skip_ws(b, i);
        if *i < b.len() && b[*i] == b',' {
            *i += 1;
        } else if *i < b.len() && b[*i] == b'}' {
            *i += 1;
            return Ok(Json::Obj(out));
        } else {
            return Err(format!("bad object at {}", i));
        }
    }
}
