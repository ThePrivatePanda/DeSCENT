// One line of receiver output into an event, as web/js/lines.js reads it:
//   - the raw receiver: PKT, ERR, HB and #DG lines
//   - the old CSV receiver (V2_5_X_T_BEAM_DECODE_CSV): 35 columns, plus its header
//   - a ChipSat console hex dump, 55 bytes with spaces between
// An Arduino Serial Monitor "HH:MM:SS.mmm -> " prefix comes off and is the clock.
// Where lines.js leans on JS (Number(), trim(), \s, object key order) this does what
// JS does rather than what Rust would, so the two give the same JSON.
#![allow(dead_code)]   // the server starts using it in the next part of the 1.4 split

use crate::decode::{self, Decoded};
use crate::json::esc;

pub const CSV_COLUMNS: usize = 35;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    // kind is "packet", or "badlength" when the bytes are not 55
    Packet {
        source: &'static str,   // raw, csv or hexdump
        bytes: Vec<u8>,
        decoded: Result<Decoded, String>,
        rssi: f64,
        snr: f64,
        freq_err: f64,
        more: Option<Info>,     // only a PKT line has it
    },
    RxError { code: f64, rssi: f64, snr: f64 },
    Heartbeat { uptime_ms: f64, ok: f64, errors: f64 },
    // #DG,RX / #DG,BOOT / #DG,SRC; kind is rxinfo, boot or source
    Device { kind: &'static str, version: Option<String>, info: Info },
    RxFatal { code: f64 },
    CsvHeader,
    Empty,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub clock: Option<f64>,   // ms since midnight, from the Serial Monitor prefix
    pub text: String,         // the line without the prefix
}

// key=value pairs, in the order a JS object would list them
pub type Info = Vec<(String, String)>;

// What String.prototype.trim and the regex \s count as space
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

// An integer in base 2, 8 or 16 rounded to the nearest f64, as Number("0x...") does.
// Past 64 bits the bits below the top 64 only count as sticky, which is enough for
// round-to-nearest-even at 53.
fn radix_value(digits: &str, radix: u32) -> f64 {
    let per = radix.trailing_zeros();
    let mut bits: Vec<bool> = Vec::new();
    for c in digits.chars() {
        let d = c.to_digit(radix).unwrap();
        for i in (0..per).rev() {
            bits.push(d >> i & 1 == 1);
        }
    }
    let first = match bits.iter().position(|b| *b) {
        Some(i) => i,
        None => return 0.0,
    };
    let bits = &bits[first..];
    let top = bits.iter().take(64).fold(0u64, |m, b| m << 1 | *b as u64);
    if bits.len() <= 64 {
        return top as f64;
    }
    let sticky = bits[64..].iter().any(|b| *b) as u64;
    (top | sticky) as f64 * 2f64.powi(bits.len() as i32 - 64)
}

// Number(s) for a string
pub fn js_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(rest) = t.strip_prefix(prefix) {
            if !rest.is_empty() && rest.chars().all(|c| c.is_digit(radix)) {
                return radix_value(rest, radix);
            }
            return f64::NAN;
        }
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    if body == "Infinity" {
        return if t.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // StrUnsignedDecimalLiteral: digits, an optional point, digits, an optional exponent,
    // with at least one digit before the exponent
    let b = body.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let mut n = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

// lines.js num(): an absent or empty field, or one that is not a finite number, is NaN
fn num(s: Option<&str>) -> f64 {
    match s {
        Some(s) if !s.is_empty() => {
            let v = js_number(s);
            if v.is_finite() {
                v
            } else {
                f64::NAN
            }
        }
        _ => f64::NAN,
    }
}

// A JS object puts keys that are array indexes first, in number order, then the rest in
// the order they were set. Setting a key again keeps its place.
fn index_key(k: &str) -> Option<u32> {
    let v: u32 = k.parse().ok()?;
    (v != u32::MAX && v.to_string() == k).then_some(v)
}

fn key_values(parts: &[&str]) -> Info {
    let mut info: Info = Vec::new();
    for p in parts {
        // indexOf('=') > 0: a pair needs a key
        let Some(eq) = p.find('=').filter(|eq| *eq > 0) else { continue };
        let (k, v) = (&p[..eq], &p[eq + 1..]);
        // Setting __proto__ to a string does nothing to a plain object
        if k == "__proto__" {
            continue;
        }
        match info.iter_mut().find(|(have, _)| have == k) {
            Some(slot) => slot.1 = v.to_string(),
            None => info.push((k.to_string(), v.to_string())),
        }
    }
    let (mut idx, rest): (Info, Info) = info.into_iter().partition(|(k, _)| index_key(k).is_some());
    idx.sort_by_key(|(k, _)| index_key(k));
    idx.extend(rest);
    idx
}

fn packet(source: &'static str, bytes: Vec<u8>, rssi: f64, snr: f64, freq_err: f64) -> Kind {
    let decoded = decode::decode(&bytes);
    Kind::Packet { source, bytes, decoded, rssi, snr, freq_err, more: None }
}

// PKT,<len>,<hex>,<rssi>,<snr>,<freqErr>; a record off a chip leaves the radio figures
// empty and goes on with up=<ms>,n=<index>
fn parse_raw(parts: &[&str]) -> Kind {
    let Some(bytes) = decode::hex_to_bytes(parts.get(2).copied().unwrap_or("")) else { return Kind::Other };
    let mut k = packet("raw", bytes, num(parts.get(3).copied()), num(parts.get(4).copied()), num(parts.get(5).copied()));
    if let Kind::Packet { more, .. } = &mut k {
        *more = Some(key_values(parts.get(6..).unwrap_or(&[])));
    }
    k
}

fn parse_csv(parts: &[&str]) -> Option<Kind> {
    let mut values = [0.0; 24];
    for (i, v) in values.iter_mut().enumerate() {
        *v = num(Some(js_trim(parts[i])));
        if v.is_nan() {
            return None;
        }
    }
    // That receiver printed the battery byte as percent (byte / 2); lines.js turns it
    // back into the byte and then volts so the rebuilt bytes and CRC come out right
    values[22] = (values[22] * 2.0) / 100.0 + 2.5;
    let crc = num(Some(parts[32]));
    if crc.is_nan() {
        return None;
    }
    let bytes = decode::encode(&values, crc).to_vec();
    Some(packet("csv", bytes, num(Some(parts[33])), num(Some(parts[34])), f64::NAN))
}

fn parse_device(parts: &[&str]) -> Kind {
    let device = |kind| Kind::Device {
        kind,
        version: parts.get(2).map(|s| s.to_string()),
        info: key_values(parts.get(3..).unwrap_or(&[])),
    };
    match parts.get(1).copied() {
        Some("RX") => device("rxinfo"),
        Some("BOOT") => device("boot"),
        Some("SRC") => device("source"),
        Some("FATAL") => Kind::RxFatal { code: num(parts.get(2).copied()) },
        _ => Kind::Other,
    }
}

fn digits_at(s: &[u8], at: usize, n: usize) -> Option<u32> {
    let d = s.get(at..at + n)?;
    d.iter().all(u8::is_ascii_digit).then(|| d.iter().fold(0, |v, c| v * 10 + (c - b'0') as u32))
}

// ^(\d{1,2}):(\d{2}):(\d{2})\.(\d{3})\s*(?:->)?\s*  as (clock ms, length matched)
fn time_prefix(line: &str) -> Option<(f64, usize)> {
    let b = line.as_bytes();
    let hl = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if !(1..=2).contains(&hl) || b.get(hl) != Some(&b':') {
        return None;
    }
    let h = digits_at(b, 0, hl)?;
    let m = digits_at(b, hl + 1, 2)?;
    if b.get(hl + 3) != Some(&b':') {
        return None;
    }
    let s = digits_at(b, hl + 4, 2)?;
    if b.get(hl + 6) != Some(&b'.') {
        return None;
    }
    let ms = digits_at(b, hl + 7, 3)?;
    let mut rest = line[hl + 10..].trim_start_matches(js_space);
    rest = rest.strip_prefix("->").unwrap_or(rest);
    rest = rest.trim_start_matches(js_space);
    let clock = (((h * 60 + m) * 60 + s) * 1000 + ms) as f64;
    Some((clock, line.len() - rest.len()))
}

// ^(?:[0-9A-Fa-f]{2}\s+){54}[0-9A-Fa-f]{2}$
fn is_hex_dump(line: &str) -> bool {
    let mut chars = line.chars().peekable();
    for i in 0..55 {
        for _ in 0..2 {
            match chars.next() {
                Some(c) if c.is_ascii_hexdigit() => {}
                _ => return false,
            }
        }
        if i == 54 {
            return chars.next().is_none();
        }
        let mut gap = 0;
        while chars.peek().is_some_and(|c| js_space(*c)) {
            chars.next();
            gap += 1;
        }
        if gap == 0 {
            return false;
        }
    }
    false
}

pub fn parse_line(input: &str) -> Event {
    let line = input.strip_prefix('\u{feff}').unwrap_or(input);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut line = js_trim(line);
    let mut clock = None;
    if let Some((c, len)) = time_prefix(line) {
        clock = Some(c);
        line = js_trim(&line[len..]);
    }
    let kind = if line.is_empty() {
        Kind::Empty
    } else if line.starts_with("PKT,") {
        parse_raw(&line.split(',').collect::<Vec<_>>())
    } else if line.starts_with("ERR,") {
        let p: Vec<&str> = line.split(',').collect();
        Kind::RxError { code: num(p.get(1).copied()), rssi: num(p.get(2).copied()), snr: num(p.get(3).copied()) }
    } else if line.starts_with("HB,") {
        let p: Vec<&str> = line.split(',').collect();
        Kind::Heartbeat { uptime_ms: num(p.get(1).copied()), ok: num(p.get(2).copied()), errors: num(p.get(3).copied()) }
    } else if line.starts_with("#DG,") {
        parse_device(&line.split(',').collect::<Vec<_>>())
    } else if line.starts_with("Latitude_deg,") {
        Kind::CsvHeader
    } else if is_hex_dump(line) {
        packet("hexdump", decode::hex_to_bytes(line).unwrap(), f64::NAN, f64::NAN, f64::NAN)
    } else {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() == CSV_COLUMNS {
            parse_csv(&parts).unwrap_or(Kind::Other)
        } else {
            Kind::Other
        }
    };
    Event { kind, clock, text: line.to_string() }
}

fn info_json(info: &Info) -> String {
    let pairs: Vec<String> = info.iter().map(|(k, v)| format!("\"{}\":\"{}\"", esc(k), esc(v))).collect();
    format!("{{{}}}", pairs.join(","))
}

impl Event {
    // The event as JSON.stringify writes lines.js's, with hex in place of the byte array
    pub fn to_json(&self) -> String {
        let n = decode::js_num;
        let body = match &self.kind {
            Kind::Packet { source, bytes, decoded, rssi, snr, freq_err, more } => {
                let kind = if decoded.is_ok() { "packet" } else { "badlength" };
                let more = more.as_ref().map(|m| format!(",\"more\":{}", info_json(m))).unwrap_or_default();
                format!(
                    "\"kind\":\"{}\",\"source\":\"{}\",\"hex\":\"{}\",\"decoded\":{},\"rssi\":{},\"snr\":{},\"freqErr\":{}{}",
                    kind,
                    source,
                    decode::bytes_to_hex(bytes),
                    decode::to_json(decoded),
                    n(*rssi),
                    n(*snr),
                    n(*freq_err),
                    more
                )
            }
            Kind::RxError { code, rssi, snr } => format!("\"kind\":\"rxerror\",\"code\":{},\"rssi\":{},\"snr\":{}", n(*code), n(*rssi), n(*snr)),
            Kind::Heartbeat { uptime_ms, ok, errors } => {
                format!("\"kind\":\"heartbeat\",\"uptimeMs\":{},\"ok\":{},\"errors\":{}", n(*uptime_ms), n(*ok), n(*errors))
            }
            // JSON.stringify leaves out a version that is undefined
            Kind::Device { kind, version, info } => format!(
                "\"kind\":\"{}\",{}\"info\":{}",
                kind,
                version.as_ref().map(|v| format!("\"version\":\"{}\",", esc(v))).unwrap_or_default(),
                info_json(info)
            ),
            Kind::RxFatal { code } => format!("\"kind\":\"rxfatal\",\"code\":{}", n(*code)),
            Kind::CsvHeader => "\"kind\":\"csvheader\"".into(),
            Kind::Empty => "\"kind\":\"empty\"".into(),
            Kind::Other => "\"kind\":\"other\"".into(),
        };
        let clock = self.clock.map(n).unwrap_or_else(|| "null".into());
        format!("{{{},\"clock\":{},\"text\":\"{}\"}}", body, clock, esc(&self.text))
    }
}

// The lines of a log file that recorder.js parseLogFile hands to parseLine: after the
// v1 header, the text after the second tab of each line with a number for its time;
// in any other file every line that is not blank.
pub fn log_texts(content: &str) -> Vec<&str> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let lines: Vec<&str> = content.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if lines[0].starts_with("# descent-ground log v1") {
        let mut out = Vec::new();
        for l in &lines[1..] {
            let Some(a) = l.find('\t') else { continue };
            let Some(b) = l[a + 1..].find('\t').map(|b| a + 1 + b) else { continue };
            if js_number(&l[..a]).is_finite() {
                out.push(&l[b + 1..]);
            }
        }
        return out;
    }
    lines.into_iter().filter(|l| !js_trim(l).is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected JSON in these tests is what lines.js parseLine printed for the same line
    // (node, JSON.stringify with bytes left out), not worked out by hand.
    fn json(line: &str) -> String {
        parse_line(line).to_json()
    }

    // Real lines off the logs (session logs, a pull, test/data/csv_receiver.txt and the
    // 2026-09-09 lab log), each with what lines.js printed for it
    const REAL: [(&str, &str, &str); 10] = [
        (
            "a radio packet",
            r##"PKT,55,00000000000000000000000000000000000000000000000000000000000000000000000000000000FF7F0000000000000000FF0000CCE0,-92.0,9.25,226"##,
            r##"{"kind":"packet","source":"raw","hex":"00000000000000000000000000000000000000000000000000000000000000000000000000000000FF7F0000000000000000FF0000CCE0","decoded":{"ok":true,"crcOk":true,"crcReceived":57548,"crcCalculated":57548,"values":{"lat":0,"lon":0,"gpsAlt":0,"envAlt":0,"ax":0,"ay":0,"az":0,"gx":0,"gy":0,"gz":0,"mx":0,"my":0,"mz":0,"qi":0,"qj":0,"qk":0,"qr":1,"temp":0,"pressure":0,"humidity":0,"counter":0,"csid":255,"battery":2.5,"validity":0},"valid":{"accel":false,"gyro":false,"mag":false,"quat":false,"gps":false,"soc":false,"env":false,"fresh":false},"saturated":false},"rssi":-92,"snr":9.25,"freqErr":226,"more":{},"clock":null,"text":"PKT,55,00000000000000000000000000000000000000000000000000000000000000000000000000000000FF7F0000000000000000FF0000CCE0,-92.0,9.25,226"}"##,
        ),
        (
            "GPS fix, CRC failed",
            r##"PKT,55,99CA4F194D627BD216870700000000002000DAFFDDFFF8FF14000C00E2005300EBFEA4008C04BF41B96D00000000000025040D173F541A,-44.0,8.75,1686"##,
            r##"{"kind":"packet","source":"raw","hex":"99CA4F194D627BD216870700000000002000DAFFDDFFF8FF14000C00E2005300EBFEA4008C04BF41B96D00000000000025040D173F541A","decoded":{"ok":true,"crcOk":false,"crcReceived":6740,"crcCalculated":31316,"values":{"lat":42.4659609,"lon":-76.3665843,"gpsAlt":493.334,"envAlt":0,"ax":0.32,"ay":-0.38,"az":-0.35,"gx":-0.8,"gy":2,"gz":1.2,"mx":22.6,"my":8.3,"mz":-27.7,"qi":0.0050050355540635395,"qj":0.035523545030060735,"qk":0.5136570329905087,"qr":0.8572344126712851,"temp":0,"pressure":0,"humidity":0,"counter":1061,"csid":13,"battery":2.73,"validity":63},"valid":{"accel":true,"gyro":true,"mag":true,"quat":true,"gps":true,"soc":true,"env":false,"fresh":false},"saturated":false},"rssi":-44,"snr":8.75,"freqErr":1686,"more":{},"clock":null,"text":"PKT,55,99CA4F194D627BD216870700000000002000DAFFDDFFF8FF14000C00E2005300EBFEA4008C04BF41B96D00000000000025040D173F541A,-44.0,8.75,1686"}"##,
        ),
        (
            "off a chip, accel saturated",
            r##"PKT,55,00000000000000000000000084B800004EE12A0D040604D3F5B1B9EFC4FF1B0195FE60C0B2D35ADAA15E29036A25CD0E2302098B6F0AEE,,,,up=294332,n=2235"##,
            r##"{"kind":"packet","source":"raw","hex":"00000000000000000000000084B800004EE12A0D040604D3F5B1B9EFC4FF1B0195FE60C0B2D35ADAA15E29036A25CD0E2302098B6F0AEE","decoded":{"ok":true,"crcOk":true,"crcReceived":60938,"crcCalculated":60938,"values":{"lat":0,"lon":0,"gpsAlt":0,"envAlt":472.36,"ax":-78.58,"ay":33.7,"az":15.4,"gx":-1151.6,"gy":-1997.9,"gz":-416.7,"mx":-6,"my":28.3,"mz":-36.3,"qi":-0.4970854823450423,"qj":-0.34614093447676014,"qk":-0.29413739432966096,"qr":0.739310892056032,"temp":8.09,"pressure":957.8,"humidity":37.89,"counter":547,"csid":9,"battery":3.8899999999999997,"validity":111},"valid":{"accel":true,"gyro":true,"mag":true,"quat":true,"gps":false,"soc":true,"env":true,"fresh":false},"saturated":true},"rssi":null,"snr":null,"freqErr":null,"more":{"up":"294332","n":"2235"},"clock":null,"text":"PKT,55,00000000000000000000000084B800004EE12A0D040604D3F5B1B9EFC4FF1B0195FE60C0B2D35ADAA15E29036A25CD0E2302098B6F0AEE,,,,up=294332,n=2235"}"##,
        ),
        (
            "short",
            r##"PKT,55,0000000000000000000000005658000000000000FEFF000B,440063,421,2"##,
            r##"{"kind":"badlength","source":"raw","hex":"0000000000000000000000005658000000000000FEFF000B","decoded":{"ok":false,"error":"length 24, expected 55"},"rssi":440063,"snr":421,"freqErr":2,"more":{},"clock":null,"text":"PKT,55,0000000000000000000000005658000000000000FEFF000B,440063,421,2"}"##,
        ),
        (
            "console hex dump",
            r##"00 00 00 00 00 00 00 00 00 00 00 00 7B 4B 00 00 D5 FF 17 00 05 00 F8 FF F0 FF 01 00 63 00 EE FF BE FD D8 FE AE 09 25 5A 57 5A 22 08 AF 26 EE 15 07 00 09 A0 6F AA B8"##,
            r##"{"kind":"packet","source":"hexdump","hex":"0000000000000000000000007B4B0000D5FF17000500F8FFF0FF01006300EEFFBEFDD8FEAE09255A575A2208AF26EE15070009A06FAAB8","decoded":{"ok":true,"crcOk":true,"crcReceived":47274,"crcCalculated":47274,"values":{"lat":0,"lon":0,"gpsAlt":0,"envAlt":193.23,"ax":-0.43,"ay":0.23,"az":0.05,"gx":-0.8,"gy":-1.6,"gz":0.1,"mx":9.9,"my":-1.8,"mz":-57.8,"qi":-0.009033478804895169,"qj":0.07562486648152104,"qk":0.7042756431775872,"qr":0.7058015686513871,"temp":20.82,"pressure":990.3,"humidity":56.14,"counter":7,"csid":9,"battery":4.1,"validity":111},"valid":{"accel":true,"gyro":true,"mag":true,"quat":true,"gps":false,"soc":true,"env":true,"fresh":false},"saturated":false},"rssi":null,"snr":null,"freqErr":null,"clock":null,"text":"00 00 00 00 00 00 00 00 00 00 00 00 7B 4B 00 00 D5 FF 17 00 05 00 F8 FF F0 FF 01 00 63 00 EE FF BE FD D8 FE AE 09 25 5A 57 5A 22 08 AF 26 EE 15 07 00 09 A0 6F AA B8"}"##,
        ),
        (
            "CSV receiver with a Serial Monitor clock",
            r##"16:38:21.952 -> 0.0000000,0.0000000,-17.000,250.43,0.00,0.00,0.00,0.0,0.0,0.0,-13.8,10.4,31.8,-0.00519,-0.01263,-0.46144,0.88708,34.34,983.5,24.85,0,7,63.0,111,1,1,1,1,0,1,1,0,37681,-51.0,11.5"##,
            r##"{"kind":"packet","source":"csv","hex":"000000000000000098BDFFFFD361000000000000000000000000000076FF68003E0156FF62FEF0C48B716A0D6B26B5090000077E6F3193","decoded":{"ok":true,"crcOk":true,"crcReceived":37681,"crcCalculated":37681,"values":{"lat":0,"lon":0,"gpsAlt":-17,"envAlt":250.43,"ax":0,"ay":0,"az":0,"gx":0,"gy":0,"gz":0,"mx":-13.8,"my":10.4,"mz":31.8,"qi":-0.005188146610919523,"qj":-0.012634662923062838,"qk":-0.46143986327707753,"qr":0.8870815149388104,"temp":34.34,"pressure":983.5,"humidity":24.85,"counter":0,"csid":7,"battery":3.76,"validity":111},"valid":{"accel":true,"gyro":true,"mag":true,"quat":true,"gps":false,"soc":true,"env":true,"fresh":false},"saturated":false},"rssi":-51,"snr":11.5,"freqErr":null,"clock":59901952,"text":"0.0000000,0.0000000,-17.000,250.43,0.00,0.00,0.00,0.0,0.0,0.0,-13.8,10.4,31.8,-0.00519,-0.01263,-0.46144,0.88708,34.34,983.5,24.85,0,7,63.0,111,1,1,1,1,0,1,1,0,37681,-51.0,11.5"}"##,
        ),
        (
            "receiver error",
            r##"ERR,-7,-94.0,2.75"##,
            r##"{"kind":"rxerror","code":-7,"rssi":-94,"snr":2.75,"clock":null,"text":"ERR,-7,-94.0,2.75"}"##,
        ),
        (
            "heartbeat",
            r##"HB,9965,0,0"##,
            r##"{"kind":"heartbeat","uptimeMs":9965,"ok":0,"errors":0,"clock":null,"text":"HB,9965,0,0"}"##,
        ),
        (
            "receiver info",
            r##"#DG,RX,v1,id=72E53C,f=915.0,bw=125.0,sf=7,cr=7,sync=0x12,pre=8"##,
            r##"{"kind":"rxinfo","version":"v1","info":{"id":"72E53C","f":"915.0","bw":"125.0","sf":"7","cr":"7","sync":"0x12","pre":"8"},"clock":null,"text":"#DG,RX,v1,id=72E53C,f=915.0,bw=125.0,sf=7,cr=7,sync=0x12,pre=8"}"##,
        ),
        (
            "source",
            r##"#DG,SRC,v1,kind=flash,from=/dev/ttyUSB0,boot=0,packets=714,time_source=none,uptime0_ms=-4361"##,
            r##"{"kind":"source","version":"v1","info":{"kind":"flash","from":"/dev/ttyUSB0","boot":"0","packets":"714","time_source":"none","uptime0_ms":"-4361"},"clock":null,"text":"#DG,SRC,v1,kind=flash,from=/dev/ttyUSB0,boot=0,packets=714,time_source=none,uptime0_ms=-4361"}"##,
        ),
    ];

    #[test]
    fn real_lines_parse_as_lines_js_parses_them() {
        for (what, line, want) in REAL {
            assert_eq!(json(line), want, "{}", what);
        }
    }

    #[test]
    fn number_reads_as_js_number_does() {
        for (s, want) in [("", 0.0), ("  ", 0.0), (" -51.0 ", -51.0), ("0x1F", 31.0), ("0b101", 5.0), ("0o17", 15.0), (".5", 0.5), ("5.", 5.0), ("1e3", 1000.0), ("+7", 7.0)] {
            assert_eq!(js_number(s), want, "{:?}", s);
        }
        for s in [".", "-0x10", "1e", "0x", "1_000", "nan", "inf", "12abc", "1,5"] {
            assert!(js_number(s).is_nan(), "{:?}", s);
        }
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
        assert!(num(Some("Infinity")).is_nan());
        assert!(num(None).is_nan());
        assert!(num(Some("")).is_nan());
        assert_eq!(radix_value("1FFFFFFFFFFFFF1", 16), 144115188075855860.0);
    }

    // Not a fixture: the Rust half of the check against every real log, which
    // ground/tools/golden.sh runs and compares with lines.js line by line.
    // DG_GOLDEN_LIST=<file of paths> DG_GOLDEN_OUT=<out.jsonl> cargo test lines::tests::golden
    #[test]
    fn golden() {
        use std::io::Write;
        let Ok(list) = std::env::var("DG_GOLDEN_LIST") else { return };
        let out = std::env::var("DG_GOLDEN_OUT").expect("DG_GOLDEN_OUT is where the lines go");
        let list = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("{}: {}", list, e));
        let mut w = std::io::BufWriter::new(std::fs::File::create(&out).unwrap_or_else(|e| panic!("{}: {}", out, e)));
        for path in list.lines().filter(|l| !l.is_empty()) {
            let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
            writeln!(w, "{{\"file\":\"{}\"}}", esc(path)).unwrap();
            for text in log_texts(&String::from_utf8_lossy(&bytes)) {
                writeln!(w, "{}", parse_line(text).to_json()).unwrap();
            }
        }
        w.flush().unwrap();
    }

    #[test]
    fn object_keys_go_in_js_order() {
        let kv = key_values(&["b=1", "10=x", "a=2", "2=y", "b=3", "=no", "noeq", "01=z", "__proto__=p"]);
        let keys: Vec<&str> = kv.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["2", "10", "b", "a", "01"]);
        assert_eq!(kv[2].1, "3");
    }
}
