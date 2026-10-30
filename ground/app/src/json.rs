// Four fields of JSON are not worth a serialisation crate, but they are worth
// escaping properly: a port error carries whatever the OS said.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
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
    out
}

// The one integer we read back out of a request body.
pub fn int_field(body: &str, name: &str) -> Option<i64> {
    let pat = format!("\"{}\"", name);
    let at = body.find(&pat)? + pat.len();
    let rest = &body[at..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

pub fn str_field(body: &str, name: &str) -> Option<String> {
    let pat = format!("\"{}\"", name);
    let at = body.find(&pat)? + pat.len();
    let rest = &body[at..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// A string somebody typed, which str_field cannot be trusted with: it stops at the
// first quote, so a name with a quote in it would be cut short there without a word.
// This one reads to the quote that ends the string and undoes the escapes on the way.
pub fn text_field(body: &str, name: &str) -> Option<String> {
    let pat = format!("\"{}\"", name);
    let at = body.find(&pat)? + pat.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let hex4 = |chars: &mut std::str::Chars| -> Option<u32> {
        let digits: String = chars.take(4).collect();
        if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_hexdigit()) {
            u32::from_str_radix(&digits, 16).ok()
        } else {
            None
        }
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let mut code = hex4(&mut chars)?;
                    // Anything past U+FFFF arrives as two of these, one after the other.
                    if (0xD800..0xDC00).contains(&code) {
                        if chars.next()? != '\\' || chars.next()? != 'u' {
                            return None;
                        }
                        let low = hex4(&mut chars)?;
                        if !(0xDC00..0xE000).contains(&low) {
                            return None;
                        }
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                    }
                    out.push(char::from_u32(code)?);
                }
                c @ ('"' | '\\' | '/') => out.push(c),
                _ => return None,
            },
            c => out.push(c),
        }
    }
}

// A number, for the model settings. Some(NaN) when the key is there with something that
// is not a number, so a typo is refused rather than ignored.
pub fn num_field(body: &str, name: &str) -> Option<f64> {
    let pat = format!("\"{}\"", name);
    let at = body.find(&pat)? + pat.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    let text: String = rest.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')).collect();
    Some(text.parse().unwrap_or(f64::NAN))
}

// true or false, which is all flight.json has that is neither a number nor a string.
pub fn bool_field(body: &str, name: &str) -> Option<bool> {
    let pat = format!("\"{}\"", name);
    let at = body.find(&pat)? + pat.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_field_reads_what_json_writes() {
        let body = "{\"lostAfterMs\": 600000, \"lowBatteryV\":3.45,\"weakRssiDbm\":-1.2e2,\"x\":\"no\"}";
        assert_eq!(num_field(body, "lostAfterMs"), Some(600000.0));
        assert_eq!(num_field(body, "lowBatteryV"), Some(3.45));
        assert_eq!(num_field(body, "weakRssiDbm"), Some(-120.0));
        assert!(num_field(body, "x").unwrap().is_nan());
        assert_eq!(num_field(body, "missing"), None);
    }

    #[test]
    fn reads_what_somebody_typed_escapes_and_all() {
        assert_eq!(text_field("{\"uid\":\"00\",\"name\":\"Flight spare 2\"}", "name").as_deref(), Some("Flight spare 2"));
        // As JSON.stringify sends a quote, a backslash, a line end and a tab.
        assert_eq!(
            text_field(r#"{"name":"the \"left\" one \\ two\nlines\there","uid":"00"}"#, "name").as_deref(),
            Some("the \"left\" one \\ two\nlines\there")
        );
        assert_eq!(text_field(r#"{"name":"caf\u00e9 / \/ \ud83d\ude80"}"#, "name").as_deref(), Some("caf\u{e9} / / \u{1F680}"));
        assert_eq!(text_field("{\"name\":\"caf\u{e9}\"}", "name").as_deref(), Some("caf\u{e9}"));
        assert_eq!(text_field("{\"name\" : \"\"}", "name").as_deref(), Some(""));
        // Not a string, never closed, or an escape that is not one.
        for no in [r#"{"name":12}"#, r#"{"name":"open"#, r#"{"name":"bad \q"}"#, r#"{"name":"bad \u12"}"#, r#"{"uid":"00"}"#] {
            assert_eq!(text_field(no, "name"), None, "{}", no);
        }
    }

    #[test]
    fn reads_a_flag() {
        assert_eq!(bool_field("{\"settingsBlock\": false, \"sf\": 7}", "settingsBlock"), Some(false));
        assert_eq!(bool_field("{\"settingsBlock\":true}", "settingsBlock"), Some(true));
        assert_eq!(bool_field("{\"settingsBlock\": 0}", "settingsBlock"), None);
        assert_eq!(bool_field("{\"sf\": 7}", "settingsBlock"), None);
    }

    #[test]
    fn escapes_what_would_break_a_line() {
        assert_eq!(esc("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te");
    }

    #[test]
    fn reads_the_fields_the_page_sends() {
        assert_eq!(int_field("{\"sf\":12}", "sf"), Some(12));
        assert_eq!(int_field("{ \"sf\" : 7 }", "sf"), Some(7));
        assert_eq!(int_field("{\"sf\":\"twelve\"}", "sf"), None);
        assert_eq!(int_field("{}", "sf"), None);
        assert_eq!(str_field("{\"image\":\"/tmp/a.bin\"}", "image"), Some("/tmp/a.bin".into()));
        assert_eq!(str_field("{\"sf\":12}", "image"), None);
    }
}
