// `descent-ground check-receiver`: reads what an SDR receiver program printed and says which
// lines break docs/sdr-receiver.md, one message per line, then what it heard. The app reads the
// same lines with lines.rs and receivers.rs and says nothing about one it cannot use, so this is
// where whoever writes the program finds out. A line has to be one the app takes and also keep
// the spec's stricter rules.
use crate::decode;
use crate::lines::{self, Kind};
use crate::receivers;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};

pub const HELP: &str = "\
Reads what an SDR receiver program printed and says which lines break docs/sdr-receiver.md,
one message per line, then what it heard. Exits 1 if any line breaks a rule.

  descent-ground check-receiver <file>
  your_program | descent-ground check-receiver -

With - it reads until the pipe closes, so give the program a time limit:
  timeout 120 your_program | descent-ground check-receiver -
";

// The header is asked for every 30 s and HB every 5 s. The only clock in the lines is HB's
// uptime, and a header is dated by the HB before it, so each limit has one HB of slack.
const HEADER_MS: f64 = 35_000.0;
const HB_MS: f64 = 6_000.0;

#[derive(Clone, Copy)]
struct Beat {
    up: f64,
    ok: u64,
    errors: u64,
    ovf: Option<u64>,
    drop: Option<u64>,
}

#[derive(Default)]
struct Checker {
    problems: usize,
    lines: usize,
    id: Option<String>,
    fw: Option<String>,
    sfs: Vec<u8>,
    headers: usize,
    header_at: Option<f64>,    // uptime of the HB before the last header
    header_late: bool,         // already said for this gap
    beat: Option<Beat>,
    header_since_beat: bool,   // a restart starts its uptime again, after a header
    fatal_at: Option<usize>,
    by_sf: BTreeMap<u8, u64>,
    crc_ok: u64,
    crc_bad: u64,
    other_length: u64,
    codes: BTreeMap<String, u64>,
    boards: BTreeSet<u8>,
}

fn plural(n: usize, one: &str) -> String {
    format!("{} {}{}", n, one, if n == 1 { "" } else { "s" })
}

// -12, 7, 9.25: digits with an optional minus and an optional fraction, nothing else
fn decimal(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (whole, fraction) = s.split_once('.').unwrap_or((s, "0"));
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit());
    digits(whole) && digits(fraction)
}

fn count(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn upper_hex(s: &str) -> bool {
    !s.is_empty() && s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

fn reserved(key: &str) -> Option<String> {
    match key {
        "name" => Some(String::from("name= is left out: in a header it makes the app send #SET,name commands to the program")),
        "rx" => Some(String::from("rx= is a T-Beam key the app acts on (its radio in receive); leave it out")),
        "t_ns" | "mod" | "ch" => Some(format!("{}= is reserved for a later version; leave it out", key)),
        _ => None,
    }
}

// The key=value fields after a line's fixed ones
fn pairs<'a>(fields: &[&'a str], said: &mut Vec<String>) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for f in fields {
        match f.split_once('=') {
            Some((k, v)) if !k.is_empty() => {
                if out.iter().any(|(have, _)| *have == k) {
                    said.push(format!("{}= is there twice", k));
                }
                said.extend(reserved(k));
                out.push((k, v));
            }
            _ => said.push(format!("\"{}\" is not key=value, which every field after the fixed ones is", f)),
        }
    }
    out
}

fn value<'a>(kv: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

// A damaged header or a payload cut short can leave no SNR to give, so an ERR -24 or -1 may
// leave it empty
fn radio(rssi: &str, snr: &str, snr_or_empty: bool, said: &mut Vec<String>) {
    if !rssi.is_empty() && !decimal(rssi) {
        said.push(format!("RSSI \"{}\" must be a decimal number in dBm, or empty when it is not calibrated", rssi));
    }
    if !decimal(snr) && !(snr_or_empty && snr.is_empty()) {
        said.push(format!("SNR \"{}\" must be a decimal number in dB{}", snr, if snr_or_empty { ", or empty" } else { "" }));
    }
}

fn dbfs(kv: &[(&str, &str)], said: &mut Vec<String>) {
    if let Some(v) = value(kv, "dbfs").filter(|v| !decimal(v)) {
        said.push(format!("dbfs=\"{}\" must be a decimal number", v));
    }
}

impl Checker {
    // What is wrong with line n, each as "line n: ..."
    fn line(&mut self, n: usize, raw: &[u8]) -> Vec<String> {
        self.lines += 1;
        let mut said = Vec::new();
        if let Some(at) = self.fatal_at {
            said.push(format!("comes after the #DG,FATAL on line {}, where the program should have stopped", at));
        }
        if !raw.is_ascii() {
            said.push(String::from("has bytes that are not ASCII"));
        }
        let text = String::from_utf8_lossy(raw);
        // printf's "\n" on Windows comes out as \r\n, and the app takes either
        let text = text.strip_suffix('\r').unwrap_or(&text);
        if text.chars().any(char::is_control) {
            said.push(String::from("has a tab or another control character in it"));
        }
        if n == 1 && !text.starts_with("#DG,RX,") {
            said.push(String::from("the first line must be the #DG,RX header, so the app knows which receiver this is"));
        }
        if text.starts_with("#DG,RX,") {
            self.header(text, &mut said);
        } else if text.starts_with("PKT,") {
            self.packet(text, &mut said);
        } else if text.starts_with("ERR,") {
            self.error(text, &mut said);
        } else if text.starts_with("HB,") {
            self.heartbeat(text, &mut said);
        } else if let Some(code) = text.strip_prefix("#DG,FATAL,") {
            if !["1", "2", "3", "4"].contains(&code) {
                said.push(format!(
                    "#DG,FATAL takes 2 (no SDR plugged in), 3 (the SDR is held by another program), 4 (the SDR went away) or 1 (anything else), not \"{}\"",
                    code
                ));
            }
            self.fatal_at.get_or_insert(n);
        } else if text.is_empty() {
            said.push(String::from("is empty"));
        } else {
            let start: String = text.chars().take(40).collect();
            said.push(format!("is not a line of this protocol: \"{}\"", start));
        }
        self.problems += said.len();
        said.into_iter().map(|s| format!("line {}: {}", n, s)).collect()
    }

    fn header(&mut self, text: &str, said: &mut Vec<String>) {
        let parts: Vec<&str> = text.split(',').collect();
        if parts.get(2) != Some(&"v1") {
            said.push(format!("the version after #DG,RX must be v1, not \"{}\"", parts.get(2).unwrap_or(&"")));
        }
        let kv = pairs(parts.get(3..).unwrap_or(&[]), said);
        for key in ["id", "f", "bw", "cr", "sync", "pre", "kind", "sfs", "fw"] {
            if value(&kv, key).is_none() {
                said.push(format!("has no {}=", key));
            }
        }
        for (k, v) in &kv {
            let wrong = match *k {
                "id" if receivers::an_id(v).is_none() => {
                    Some(format!("id \"{}\" must be 6 hex characters (0-9, A-F), chosen once and kept", v))
                }
                "id" if v.bytes().any(|b| b.is_ascii_lowercase()) => Some(format!("id \"{}\" must be uppercase", v)),
                "f" if !(decimal(v) && v.parse::<f64>() == Ok(915.0)) => Some(format!("f=\"{}\" must be 915.0, the ChipSats' frequency in MHz", v)),
                "bw" if !(decimal(v) && v.parse::<f64>() == Ok(125.0)) => Some(format!("bw=\"{}\" must be 125.0, the bandwidth in kHz", v)),
                "cr" if *v != "7" => Some(format!("cr=\"{}\" must be 7, for coding rate 4/7", v)),
                "sync" if *v != "0x12" => Some(format!("sync=\"{}\" must be 0x12", v)),
                "pre" if *v != "8" => Some(format!("pre=\"{}\" must be 8, the preamble in symbols", v)),
                "kind" if *v != "sdr" => Some(format!("kind=\"{}\" must be sdr", v)),
                "fw" if receivers::a_version(v).is_none() => {
                    Some(format!("fw=\"{}\" must be digits and dots, starting with a digit, 16 characters at most", v))
                }
                "sfs" => {
                    let sfs: Option<Vec<u8>> = v.split('+').map(|s| count(s).filter(|n| (7..=12).contains(n)).map(|n| n as u8)).collect();
                    match sfs.filter(|s| s.windows(2).all(|w| w[0] < w[1])) {
                        Some(sfs) => {
                            self.sfs = sfs;
                            None
                        }
                        None => Some(format!("sfs=\"{}\" must be the spreading factors it decodes, 7 to 12, smallest first, joined by +", v)),
                    }
                }
                "sf" => Some(String::from(
                    "sf= in a header tells the app the receiver hears one spreading factor; an SDR says sfs= and puts sf= on each PKT and ERR",
                )),
                "sfchip" => Some(String::from("sfchip= is a T-Beam key the app acts on; leave it out")),
                "gain" if !(*v == "auto" || (decimal(v) && !v.starts_with('-'))) => {
                    Some(format!("gain=\"{}\" must be the tuner gain in dB, 0 or more, or auto", v))
                }
                _ => None,
            };
            said.extend(wrong);
        }
        if said.is_empty() && receivers::header(text).is_none() {
            said.push(String::from("the app does not take this as a header"));
        }
        if let Some(id) = value(&kv, "id").and_then(receivers::an_id) {
            match &self.id {
                Some(was) if *was != id => said.push(format!("id changed from {} to {}; choose it once and keep it", was, id)),
                _ => self.id = Some(id),
            }
        }
        self.fw = value(&kv, "fw").and_then(receivers::a_version).or(self.fw.take());
        self.headers += 1;
        self.header_at = Some(self.beat.map_or(0.0, |b| b.up));
        self.header_late = false;
        self.header_since_beat = true;
    }

    // sf=, which a PKT and an ERR must carry
    fn sf(&self, kv: &[(&str, &str)], said: &mut Vec<String>) -> Option<u8> {
        let Some(v) = value(kv, "sf") else {
            said.push(String::from("has no sf=, the spreading factor it came in on"));
            return None;
        };
        let Some(sf) = count(v).filter(|n| (7..=12).contains(n)).map(|n| n as u8) else {
            said.push(format!("sf=\"{}\" must be a spreading factor from 7 to 12", v));
            return None;
        };
        if !self.sfs.is_empty() && !self.sfs.contains(&sf) {
            said.push(format!("sf={} is not one of the header's sfs=", sf));
        }
        Some(sf)
    }

    fn packet(&mut self, text: &str, said: &mut Vec<String>) {
        let p: Vec<&str> = text.split(',').collect();
        if p.len() < 6 {
            said.push(String::from("a PKT line is PKT,<len>,<hex>,<rssi>,<snr>,<freqErr>,sf=<n>, and this has fewer fields"));
            return;
        }
        let len = count(p[1]).filter(|n| (1..=255).contains(n));
        if len.is_none() {
            said.push(format!("length \"{}\" must be a whole number of bytes, 1 to 255", p[1]));
        }
        let hex_ok = upper_hex(p[2]);
        if !hex_ok {
            said.push(String::from("the payload must be uppercase hex, two characters a byte, no spaces"));
        }
        if let (Some(len), true) = (len, hex_ok) {
            if len as usize * 2 != p[2].len() {
                said.push(format!("length says {} bytes, but the hex holds {}", len, p[2].len() / 2));
            }
        }
        radio(p[3], p[4], false, said);
        if !decimal(p[5]) {
            said.push(format!("frequency error \"{}\" must be a decimal number in Hz", p[5]));
        }
        let kv = pairs(&p[6..], said);
        let sf = self.sf(&kv, said);
        dbfs(&kv, said);
        if said.is_empty() && !matches!(lines::parse_line(text).kind, Kind::Packet { .. }) {
            said.push(String::from("the app does not take this as a packet"));
        }
        if let Some(sf) = sf {
            *self.by_sf.entry(sf).or_default() += 1;
        }
        let Some(bytes) = decode::hex_to_bytes(p[2]).filter(|_| hex_ok) else { return };
        match decode::decode(&bytes) {
            Ok(d) if d.crc_ok => {
                self.crc_ok += 1;
                self.boards.insert(d.csid());
            }
            Ok(_) => self.crc_bad += 1,
            Err(_) => self.other_length += 1,
        }
    }

    fn error(&mut self, text: &str, said: &mut Vec<String>) {
        let p: Vec<&str> = text.split(',').collect();
        if p.len() < 4 {
            said.push(String::from("an ERR line is ERR,<code>,<rssi>,<snr>,sf=<n>, and this has fewer fields"));
            return;
        }
        if !["-7", "-24", "-1"].contains(&p[1]) {
            said.push(format!("code \"{}\" must be -7 (payload CRC), -24 (header damaged) or -1 (anything else)", p[1]));
        }
        radio(p[2], p[3], ["-24", "-1"].contains(&p[1]), said);
        let kv = pairs(&p[4..], said);
        self.sf(&kv, said);
        if let Some(v) = value(&kv, "hex").filter(|v| !upper_hex(v)) {
            said.push(format!("hex=\"{}\" must be uppercase hex, two characters a byte", v));
        }
        dbfs(&kv, said);
        if said.is_empty() && !matches!(lines::parse_line(text).kind, Kind::RxError { .. }) {
            said.push(String::from("the app does not take this as an error"));
        }
        *self.codes.entry(p[1].to_string()).or_default() += 1;
    }

    fn heartbeat(&mut self, text: &str, said: &mut Vec<String>) {
        let p: Vec<&str> = text.split(',').collect();
        if p.len() < 4 {
            said.push(String::from("an HB line is HB,<uptime ms>,<ok>,<errors>, and this has fewer fields"));
            return;
        }
        let whole = |what: &str, v: &str, said: &mut Vec<String>| {
            let n = count(v);
            if n.is_none() {
                said.push(format!("{} \"{}\" must be a whole number", what, v));
            }
            n
        };
        let up = whole("uptime", p[1], said);
        let ok = whole("ok", p[2], said);
        let errors = whole("errors", p[3], said);
        let kv = pairs(&p[4..], said);
        let ovf = value(&kv, "ovf").and_then(|v| whole("ovf=", v, said));
        let drop = value(&kv, "drop").and_then(|v| whole("drop=", v, said));
        let clip = value(&kv, "clip");
        if let Some(v) = clip.filter(|v| !(decimal(v) && v.parse::<f64>().is_ok_and(|c| (0.0..=100.0).contains(&c)))) {
            said.push(format!("clip=\"{}\" must be the percent of samples at the ADC rail, 0 to 100", v));
        }
        match value(&kv, "sat") {
            Some(v) if v != "0" && v != "1" => said.push(format!("sat=\"{}\" must be 1 or 0", v)),
            Some("0") if clip.and_then(|c| c.parse::<f64>().ok()).is_some_and(|c| c > 0.0) => {
                said.push(String::from("sat=0 with clip= above 0: a clipped sample is a saturated one"))
            }
            _ => {}
        }
        if let Some(v) = value(&kv, "floor").filter(|v| !decimal(v)) {
            said.push(format!("floor=\"{}\" must be a decimal number in dBFS", v));
        }
        if said.is_empty() && !matches!(lines::parse_line(text).kind, Kind::Heartbeat { .. }) {
            said.push(String::from("the app does not take this as a heartbeat"));
        }
        let (Some(up), Some(ok), Some(errors)) = (up, ok, errors) else { return };
        let now = Beat { up: up as f64, ok, errors, ovf, drop };
        if let Some(was) = self.beat {
            if now.up < was.up {
                if self.header_since_beat {
                    self.header_at = Some(0.0);
                } else {
                    said.push(format!("uptime went back from {} to {} ms with no header first; after a restart the header comes first", was.up, now.up));
                }
            } else {
                if now.up - was.up > HB_MS {
                    said.push(format!("{:.1} s since the last HB, which is asked for every 5 s", (now.up - was.up) / 1000.0));
                }
                let counters = [("ok", was.ok, now.ok), ("errors", was.errors, now.errors)];
                let optional = [("ovf=", was.ovf, now.ovf), ("drop=", was.drop, now.drop)];
                for (what, before, after) in counters.into_iter().chain(optional.into_iter().filter_map(|(w, b, a)| Some((w, b?, a?)))) {
                    if after < before {
                        said.push(format!("{} went down from {} to {}; it counts from the program's start", what, before, after));
                    }
                }
            }
        }
        if let Some(at) = self.header_at.filter(|at| now.up - at > HEADER_MS && !self.header_late) {
            said.push(format!("no #DG,RX header since uptime {} ms, {:.0} s ago; it is asked for every 30 s", at, (now.up - at) / 1000.0));
            self.header_late = true;
        }
        self.beat = Some(now);
        self.header_since_beat = false;
    }

    fn end(&mut self) -> Vec<String> {
        let mut said = Vec::new();
        if self.lines == 0 {
            said.push(String::from("no lines at all"));
        } else if self.headers == 0 {
            said.push(String::from("no #DG,RX header line at all"));
        }
        if self.lines > 0 && self.beat.is_none() && self.fatal_at.is_none() {
            said.push(String::from("no HB line at all, so nothing says whether the program is alive or how often it speaks"));
        }
        self.problems += said.len();
        said.into_iter().map(|s| format!("end: {}", s)).collect()
    }

    fn summary(&self) -> String {
        let counted = |m: &BTreeMap<String, u64>| m.iter().map(|(k, n)| format!("{} {}", k, n)).collect::<Vec<String>>().join(", ");
        let by_sf = self.by_sf.iter().map(|(sf, n)| format!("SF{} {}", sf, n)).collect::<Vec<String>>().join(", ");
        let dropped = |key: &str, n: Option<u64>, what: &str| n.map_or(format!("no {}", key), |n| format!("{} {} ({})", n, what, key));
        let beat = self.beat;
        let mut out = format!(
            "receiver {}, fw {}: {}, {}\n",
            self.id.as_deref().unwrap_or("with no id"),
            self.fw.as_deref().unwrap_or("not said"),
            plural(self.lines, "line"),
            plural(self.headers, "header")
        );
        out += &format!("packets by spreading factor: {}\n", if by_sf.is_empty() { String::from("none") } else { by_sf });
        out += &format!(
            "payload CRC of the 55-byte packets: {} ok, {} bad; {} of another length\n",
            self.crc_ok, self.crc_bad, self.other_length
        );
        out += &format!("ERR by code: {}\n", if self.codes.is_empty() { String::from("none") } else { counted(&self.codes) });
        out += &format!(
            "dropped, by the last HB: {}, {}\n",
            dropped("ovf=", beat.and_then(|b| b.ovf), "samples"),
            dropped("drop=", beat.and_then(|b| b.drop), "lines")
        );
        let boards: Vec<String> = self.boards.iter().map(u8::to_string).collect();
        out += &format!("boards heard (csid, CRC good): {}\n", if boards.is_empty() { String::from("none") } else { boards.join(", ") });
        out += &if self.problems == 0 { String::from("no problems\n") } else { format!("{}\n", plural(self.problems, "problem")) };
        out
    }
}

// Each line's problems as they come, so a program piped in live is answered as it goes, then
// the summary. The answer is the number of problems.
fn check_all(mut input: impl BufRead, out: &mut impl Write) -> std::io::Result<usize> {
    let mut c = Checker::default();
    let mut raw = Vec::new();
    let mut n = 0;
    loop {
        raw.clear();
        if input.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        n += 1;
        let ended = raw.last() == Some(&b'\n');
        let mut said = c.line(n, raw.strip_suffix(b"\n").unwrap_or(&raw));
        if !ended {
            c.problems += 1;
            said.push(format!("line {}: has no \\n at its end, so the app never sees it", n));
        }
        for s in said {
            writeln!(out, "{}", s)?;
        }
    }
    for s in c.end() {
        writeln!(out, "{}", s)?;
    }
    write!(out, "{}", c.summary())?;
    Ok(c.problems)
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", HELP);
        return 0;
    }
    let [path] = args else {
        print!("{}", HELP);
        return 1;
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let checked = if path == "-" {
        check_all(std::io::stdin().lock(), &mut out)
    } else {
        match std::fs::File::open(path) {
            Ok(f) => check_all(std::io::BufReader::new(f), &mut out),
            Err(e) => {
                let _ = writeln!(out, "{}: {}", path, e);
                return 1;
            }
        }
    };
    match checked {
        Ok(0) => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLES: &str = include_str!("../../docs/sdr-receiver-examples.txt");
    const HEADER: &str = "#DG,RX,v1,id=5D4E01,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=7+8+9+10+11+12,fw=0.1.0";

    fn check(text: &str) -> (usize, String) {
        let mut out = Vec::new();
        let problems = check_all(text.as_bytes(), &mut out).unwrap();
        (problems, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_examples_in_the_spec_keep_every_rule() {
        let (problems, out) = check(EXAMPLES);
        assert_eq!(problems, 0, "{}", out);
        assert!(out.starts_with("receiver 5D4E01, fw 0.1.0: 48 lines, 3 headers\n"), "{}", out);
        assert!(out.contains("packets by spreading factor: SF7 6, SF10 6, SF11 6, SF12 12\n"), "{}", out);
        assert!(out.contains("payload CRC of the 55-byte packets: 30 ok, 0 bad; 0 of another length\n"), "{}", out);
        assert!(out.contains("ERR by code: -24 1, -7 1\n"), "{}", out);
        assert!(out.contains("boards heard (csid, CRC good): 5, 9, 12, 13\n"), "{}", out);
        assert!(out.ends_with("no problems\n"), "{}", out);
        // and the app reads every line as the kind it is
        for l in EXAMPLES.lines() {
            assert!(!matches!(lines::parse_line(l).kind, Kind::Other | Kind::Empty), "{}", l);
        }
    }

    #[test]
    fn a_broken_session_gets_one_message_per_fault_on_the_line_it_is_on() {
        let broken = [
            "PKT,2,00FF,,9.25,-12,sf=9",
            "#DG,RX,v1,id=sdr01,f=868.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=7+8+9,fw=v0.1,name=",
            "PKT,3,00ff,-50,9.25,-12,sf=9",
            "PKT,2,00FF,-50.0,9.25,-12",
            "PKT,2,00FF,-50.0,9.25,-12,sf=12,t_ns=5",
            "ERR,-6,,2.75,sf=9,hex=0",
            "HB,5000,1,0",
            "HB,12000,0,0",
            "HB,50000,3,0",
            "#DG,RX,v1,id=5D4E02,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=7+8+9,fw=0.1.0,sf=9",
            "garbage",
            "#DG,FATAL,3",
            "HB,51000,3,0",
        ]
        .join("\n");
        let (problems, out) = check(&broken);
        let want = [
            "line 1: the first line must be the #DG,RX header, so the app knows which receiver this is",
            "line 2: name= is left out: in a header it makes the app send #SET,name commands to the program",
            "line 2: id \"sdr01\" must be 6 hex characters (0-9, A-F), chosen once and kept",
            "line 2: f=\"868.0\" must be 915.0, the ChipSats' frequency in MHz",
            "line 2: fw=\"v0.1\" must be digits and dots, starting with a digit, 16 characters at most",
            "line 3: the payload must be uppercase hex, two characters a byte, no spaces",
            "line 4: has no sf=, the spreading factor it came in on",
            "line 5: t_ns= is reserved for a later version; leave it out",
            "line 5: sf=12 is not one of the header's sfs=",
            "line 6: code \"-6\" must be -7 (payload CRC), -24 (header damaged) or -1 (anything else)",
            "line 6: hex=\"0\" must be uppercase hex, two characters a byte",
            "line 8: 7.0 s since the last HB, which is asked for every 5 s",
            "line 8: ok went down from 1 to 0; it counts from the program's start",
            "line 9: 38.0 s since the last HB, which is asked for every 5 s",
            "line 9: no #DG,RX header since uptime 0 ms, 50 s ago; it is asked for every 30 s",
            "line 10: sf= in a header tells the app the receiver hears one spreading factor; an SDR says sfs= and puts sf= on each PKT and ERR",
            "line 11: is not a line of this protocol: \"garbage\"",
            "line 13: comes after the #DG,FATAL on line 12, where the program should have stopped",
            "line 13: has no \\n at its end, so the app never sees it",
        ];
        for w in want {
            assert!(out.contains(&format!("{}\n", w)), "missing {:?} in\n{}", w, out);
        }
        assert_eq!(problems, want.len(), "{}", out);
        assert!(out.ends_with(&format!("{} problems\n", want.len())), "{}", out);
    }

    #[test]
    fn the_v0_2_keys_are_checked_and_a_damaged_header_may_have_no_snr() {
        let good = [
            "#DG,RX,v1,id=EEF522,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=10+11+12,gain=auto,fw=0.2.0",
            "HB,23,0,0,ovf=0,drop=0,clip=0.00,sat=0",
            "ERR,-24,,,sf=11",
            "ERR,-1,,,sf=12",
            "HB,5024,0,2,ovf=0,drop=0,clip=12.30,sat=1,floor=-40.2",
            "#DG,RX,v1,id=EEF522,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=10+11+12,gain=20.7,fw=0.2.0",
            "HB,6000,0,2,ovf=0,drop=0,clip=0.00,sat=1,floor=-55.3",
            "#DG,FATAL,2",
            "",
        ];
        let (problems, out) = check(&good.join("\n"));
        assert_eq!(problems, 0, "{}", out);

        let bad = [
            "#DG,RX,v1,id=EEF522,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=10+11+12,gain=-3,fw=0.2.0",
            "ERR,-7,,,sf=11",
            "HB,5000,0,1,clip=120,sat=yes,floor=low",
            "HB,10000,0,1,clip=0.50,sat=0",
            "#DG,FATAL,7",
            "",
        ];
        let (problems, out) = check(&bad.join("\n"));
        let want = [
            "line 1: gain=\"-3\" must be the tuner gain in dB, 0 or more, or auto",
            "line 2: SNR \"\" must be a decimal number in dB",
            "line 3: clip=\"120\" must be the percent of samples at the ADC rail, 0 to 100",
            "line 3: sat=\"yes\" must be 1 or 0",
            "line 3: floor=\"low\" must be a decimal number in dBFS",
            "line 4: sat=0 with clip= above 0: a clipped sample is a saturated one",
            "line 5: #DG,FATAL takes 2 (no SDR plugged in), 3 (the SDR is held by another program), 4 (the SDR went away) or 1 (anything else), not \"7\"",
        ];
        for w in want {
            assert!(out.contains(&format!("{}\n", w)), "missing {:?} in\n{}", w, out);
        }
        assert_eq!(problems, want.len(), "{}", out);
    }

    #[test]
    fn an_id_that_changes_is_said() {
        let (problems, out) = check(&format!("{}\nHB,5000,0,0\n{}\n", HEADER, HEADER.replace("5D4E01", "5D4E02")));
        assert_eq!(problems, 1, "{}", out);
        assert!(out.contains("line 3: id changed from 5D4E01 to 5D4E02; choose it once and keep it\n"), "{}", out);
    }

    #[test]
    fn a_restart_may_start_uptime_again_after_a_header() {
        let text = [HEADER, "HB,5000,0,0", "HB,10000,0,0", HEADER, "HB,4000,0,0", "HB,9000,0,0", ""].join("\n");
        let (problems, out) = check(&text);
        assert_eq!(problems, 0, "{}", out);
        let (problems, out) = check(&[HEADER, "HB,5000,0,0", "HB,4000,0,0", ""].join("\n"));
        assert_eq!(problems, 1, "{}", out);
        assert!(out.contains("line 3: uptime went back from 5000 to 4000 ms with no header first"), "{}", out);
    }

    #[test]
    fn numbers_are_plain_decimals() {
        for yes in ["0", "-12", "9.25", "-101.5", "1234567"] {
            assert!(decimal(yes), "{}", yes);
        }
        for no in ["", "-", ".5", "5.", "1e3", "+7", "0x12", " 7", "nan", "1,5"] {
            assert!(!decimal(no), "{}", no);
        }
    }
}
