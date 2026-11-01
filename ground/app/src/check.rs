// What a pull or a log says, for someone standing in a field between drops. Behind
// `descent-ground check` and /api/check. Only ever reads files: no port, no board, no server.
//
// Facts only, and only the firmware's own flags. Nothing here decides a value looks wrong. What
// a file does not hold is said to be missing, and what could not be read is counted.
//
// Three things in the firmware shape what can be said (fsw FlashLog.h, Health.cpp):
//   - flash holds packets, time anchors, the boot event and dropped events. Sensor recovery, BUS
//     lines and phase changes only ever go to the console, so a pull has none of them unless
//     a console was caught. Event kinds 0x10-0x3F are kept for them but nothing writes those yet.
//   - no record says which phase it was written in, so ground and air are read off the spacing
//     and called that. Going to air also writes the RAM ring, so the fast spacing starts about
//     30 s before take-off.
//   - the settings and RST lines in a dump.txt are the board restarting for the pull. They say
//     what it was set to then, not what an older boot in the log ran with.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::decode::{self, Decoded, VALIDITY_KEYS};
use crate::dump::{self, Dump, EventKind};
use crate::fleet::{Config, Fleet};
use crate::json::esc;
use crate::lines::{self, Kind};

// Picked, not measured. Records further apart than this are a gap, unless they are a steady
// run of it, which is what ground logging looks like (5 s and 10 s on the chips so far). The
// slowest air rate on any pull so far is one record a second, 1000 to 1005 ms.
const GAP_MS: u64 = 1500;
const STEADY_RUN: usize = 3;     // gaps in a row before it counts as steady
const STEADY_PERCENT: u64 = 10;  // either side of their middle value

const SENSORS: usize = 7;        // validity bits 0-6, bit 7 is the all-fresh flag and not a sensor
const FRESH_BIT: usize = 7;
const SPELLS_KEPT: usize = 50;   // per sensor in the JSON
const SHOWN: usize = 5;          // lines of any one list in the printed report
const SAMPLES: usize = 5;
const SAMPLE_CHARS: usize = 80;
const WALK_DEPTH: usize = 4;     // a day folder is <day>/pulls/<pull>/dump.txt

const DUMP_FIRMWARE_START: &str = "start test=flashdump";
const WATCHDOG_FLAGS: [&str; 2] = ["IWDG", "WWDG"];
// these two are the board's own housekeeping in a folder and not something left unread
const PULL_EXTRAS: [&str; 2] = ["board.txt", "report.json"];

pub const HELP: &str = "\
Reads a pull or a log and says what happened, worst first. Opens no port and starts no server.

  descent-ground check [--json] <path> ...

A path is a pull folder, a dump.txt, a ground log, or a folder of those (a day folder works).
--json prints what /api/check/<name> returns, or an array of those for several paths.
";

fn seconds(ms: u64) -> String {
    match ms {
        0..=999 => format!("{} ms", ms),
        1000..=119_999 => format!("{:.1} s", ms as f64 / 1000.0),
        120_000..=7_199_999 => format!("{:.1} min", ms as f64 / 60_000.0),
        _ => format!("{:.1} h", ms as f64 / 3_600_000.0),
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{} {}{}", n, one, if n == 1 { "" } else { "s" })
}

fn join_ids<T: ToString>(ids: &[T]) -> String {
    ids.iter().map(T::to_string).collect::<Vec<String>>().join(",")
}

fn quoted(items: &[String]) -> String {
    items.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<String>>().join(",")
}

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map(T::to_string).unwrap_or_else(|| String::from("null"))
}

fn opt_text(v: &Option<String>) -> String {
    v.as_ref().map(|s| format!("\"{}\"", esc(s))).unwrap_or_else(|| String::from("null"))
}

fn sample(line: &str) -> String {
    line.chars().take(SAMPLE_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect()
}

fn utc(unix_ms: u64) -> String {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms);
    humantime::format_rfc3339_seconds(at).to_string()
}

// The clock at the field, the same zone the day folders are named by
fn local(unix_ms: f64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(unix_ms as i64).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{} ms", unix_ms),
    }
}

// How the records of one boot sit in time

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Fast,
    Steady,
    Gap,
}

#[derive(Debug, Clone, PartialEq)]
struct Gap {
    at_ms: u64,   // uptime of the record before it
    len_ms: u64,
}

#[derive(Debug, Default, PartialEq)]
struct Spacing {
    fast_ms: u64,
    steady_ms: u64,
    gap_ms: u64,
    gaps: Vec<Gap>,               // longest first
    changes: Vec<(u64, bool)>,    // uptime, and true going to fast spacing
    backward: usize,              // steps where the uptime went down
}

fn middle(values: &[u64]) -> u64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    v[v.len() / 2]
}

fn spacing(ms: &[u64]) -> Spacing {
    let mut out = Spacing::default();
    let steps: Vec<u64> = ms.windows(2).map(|w| w[1].saturating_sub(w[0])).collect();
    out.backward = ms.windows(2).filter(|w| w[1] < w[0]).count();
    let mut kinds = vec![Step::Fast; steps.len()];
    let mut i = 0;
    while i < steps.len() {
        if steps[i] <= GAP_MS {
            i += 1;
            continue;
        }
        let end = steps[i..].iter().position(|s| *s <= GAP_MS).map_or(steps.len(), |n| i + n);
        let m = middle(&steps[i..end]);
        let near = |s: u64| s * 100 >= m * (100 - STEADY_PERCENT) && s * 100 <= m * (100 + STEADY_PERCENT);
        let steady = steps[i..end].iter().filter(|s| near(**s)).count() >= STEADY_RUN;
        for k in i..end {
            kinds[k] = if steady && near(steps[k]) { Step::Steady } else { Step::Gap };
        }
        i = end;
    }
    let mut last = None;
    for (k, step) in kinds.iter().enumerate() {
        match step {
            Step::Fast => out.fast_ms += steps[k],
            Step::Steady => out.steady_ms += steps[k],
            Step::Gap => {
                out.gap_ms += steps[k];
                out.gaps.push(Gap { at_ms: ms[k], len_ms: steps[k] });
                continue;
            }
        }
        if last.is_some() && last != Some(*step) {
            out.changes.push((ms[k], *step == Step::Fast));
        }
        last = Some(*step);
    }
    out.gaps.sort_by_key(|g| std::cmp::Reverse(g.len_ms));
    out
}

// One sensor's flag across a boot

#[derive(Debug, Clone, PartialEq)]
struct Spell {
    from_ms: u64,
    len_ms: u64,
    open: bool,   // still clear on the boot's last record
}

#[derive(Debug, Clone, PartialEq)]
struct Sensor {
    name: &'static str,
    clear: usize,
    records: usize,
    clear_ms: u64,
    spells: usize,
    longest: Vec<Spell>,   // longest first, SPELLS_KEPT of them
}

impl Sensor {
    fn never_set(&self) -> bool {
        self.records > 0 && self.clear == self.records
    }
}

fn sensors(rows: &[(u64, [bool; 8])]) -> Vec<Sensor> {
    (0..SENSORS)
        .map(|bit| {
            let mut spells: Vec<Spell> = Vec::new();
            let mut start = None;
            for (ms, valid) in rows {
                match (valid[bit], start) {
                    (false, None) => start = Some(*ms),
                    (true, Some(from)) => {
                        spells.push(Spell { from_ms: from, len_ms: ms.saturating_sub(from), open: false });
                        start = None;
                    }
                    _ => {}
                }
            }
            if let (Some(from), Some((last, _))) = (start, rows.last()) {
                spells.push(Spell { from_ms: from, len_ms: last.saturating_sub(from), open: true });
            }
            let clear_ms = spells.iter().map(|s| s.len_ms).sum();
            let count = spells.len();
            spells.sort_by_key(|s| std::cmp::Reverse(s.len_ms));
            spells.truncate(SPELLS_KEPT);
            Sensor {
                name: VALIDITY_KEYS[bit],
                clear: rows.iter().filter(|(_, v)| !v[bit]).count(),
                records: rows.len(),
                clear_ms,
                spells: count,
                longest: spells,
            }
        })
        .collect()
}

// Console lines. "I BOOT 1 c0: settings from=block csid=17 ..." is how Log.cpp writes every
// one: level, tag, uptime in ms, cycle, then a word and key=value pairs.

#[derive(Debug, PartialEq)]
struct ConsoleLine<'a> {
    level: char,
    tag: &'a str,
    ms: u64,
    word: &'a str,
    fields: Vec<(&'a str, &'a str)>,
    rest: &'a str,   // everything after the "c0: "
    cut: bool,       // another line starts inside this one, so it stopped short
}

impl ConsoleLine<'_> {
    fn field(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

// Where the line's text starts, past "c0: ", if a console line starts at `at`
fn console_at(b: &[u8], at: usize) -> Option<usize> {
    if !matches!(b.get(at)?, b'I' | b'W' | b'E' | b'D') || *b.get(at + 1)? != b' ' {
        return None;
    }
    let mut i = at + 2;
    let tag_at = i;
    while b.get(i).is_some_and(u8::is_ascii_uppercase) {
        i += 1;
    }
    if i == tag_at || *b.get(i)? != b' ' {
        return None;
    }
    i += 1;
    let ms_at = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == ms_at || *b.get(i)? != b' ' || *b.get(i + 1)? != b'c' {
        return None;
    }
    i += 2;
    let cycle_at = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == cycle_at || *b.get(i)? != b':' {
        return None;
    }
    Some(i + 1)
}

fn console_one(line: &str) -> Option<ConsoleLine<'_>> {
    let text_at = console_at(line.as_bytes(), 0)?;
    let mut head = line[..text_at].split(' ');
    let level = head.next()?.chars().next()?;
    let tag = head.next()?;
    let ms = head.next()?.parse().ok()?;
    let rest = line[text_at..].trim();
    let mut parts = rest.split_whitespace();
    let word = parts.next().unwrap_or("");
    let fields = parts.filter_map(|p| p.split_once('=')).collect();
    Some(ConsoleLine { level, tag, ms, word, fields, rest, cut: false })
}

// A reset or a lost byte joins lines together, and the port can carry junk in front of one.
// Each console line found in the text comes back, and all but the last were cut short.
fn console_lines(line: &str) -> Vec<ConsoleLine<'_>> {
    let b = line.as_bytes();
    let starts: Vec<usize> = (0..b.len()).filter(|at| console_at(b, *at).is_some()).collect();
    let mut out = Vec::new();
    for (i, at) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(line.len());
        if let Some(mut c) = line.get(*at..end).and_then(console_one) {
            c.cut = i + 1 < starts.len();
            out.push(c);
        }
    }
    out
}

// One run of a board as its console showed it, from a boot to the next
#[derive(Debug, Default, Clone, PartialEq)]
struct Session {
    source: String,       // the receiver key, or "dump.txt"
    first_t: Option<f64>,
    first_ms: u64,
    last_ms: u64,
    lines: usize,
    settings: Option<String>,   // the line as printed, from "from=" on
    settings_cut: bool,
    refused: Option<String>,
    csid: Option<String>,
    reset: Option<String>,      // reset flags, from the csr the line prints
    watchdog: bool,             // iwdg or wwdg set on that line
    watch: BTreeMap<String, usize>,          // "ENV aside", "BUS clear", ...
    longest_down: BTreeMap<String, u64>,     // tag to the longest downms it reported
    phases: Vec<(u64, String)>,
    air_ms: u64,
    ground_ms: u64,
    air_since: Option<(u64, bool)>,
    log_stopped: Option<String>,
    drop_last: Option<String>,               // CYC packet drop= on the last one
    others: BTreeMap<String, usize>,         // every other W and E line, by level, tag and word
    dump_firmware: bool,
}

impl Session {
    fn count(&mut self, label: String) {
        *self.watch.entry(label).or_insert(0) += 1;
    }

    fn take(&mut self, c: &ConsoleLine) {
        self.lines += 1;
        self.last_ms = self.last_ms.max(c.ms);
        match (c.tag, c.word) {
            ("BOOT", "start") if c.field("test") == Some("flashdump") => self.dump_firmware = true,
            ("BOOT", "settings") => {
                self.settings = Some(c.rest.trim_start_matches("settings").trim().to_string());
                self.settings_cut = c.cut;
                self.refused = c.field("refused").map(String::from);
                self.csid = c.field("csid").filter(|v| !v.is_empty()).map(String::from);
            }
            ("RST", "cause") => {
                let csr = c.field("csr").and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
                self.reset = Some(csr.map_or_else(|| c.rest.to_string(), dump::reset_words));
                self.watchdog = c.field("iwdg") == Some("1") || c.field("wwdg") == Some("1");
            }
            (tag, "recover") => {
                let step = c.field("step").unwrap_or("?");
                let word = match step {
                    "aside" => "set aside",
                    "ok" => "recovered",
                    other => other,
                };
                self.count(format!("{} {}", tag, word));
                if step == "reset" && c.field("ok") == Some("0") {
                    self.count(format!("{} reset failed", tag));
                }
                if let Some(down) = c.field("downms").and_then(|v| v.parse::<u64>().ok()) {
                    let top = self.longest_down.entry(tag.to_string()).or_insert(0);
                    *top = (*top).max(down);
                }
            }
            ("BUS", word) => {
                match (word, c.field("part")) {
                    ("clear", _) if c.field("ok") == Some("0") => self.count(String::from("BUS clear failed")),
                    ("clear", _) => self.count(String::from("BUS clear")),
                    (_, Some(part)) => self.count(format!("BUS {} {}", word, part)),
                    _ => self.count(format!("BUS {}", word)),
                }
            }
            ("PHASE", "log") => {
                self.phases.push((c.ms, format!("log air={}", c.field("air").unwrap_or("?"))));
                if let Some(air) = c.field("air").map(|v| v == "1") {
                    self.phase_until(c.ms);
                    self.air_since = Some((c.ms, air));
                }
            }
            ("PHASE", _) => self.phases.push((c.ms, c.rest.to_string())),
            ("FLASH", "stopped") => self.log_stopped = Some(c.rest.to_string()),
            ("CYC", "packet") => self.drop_last = c.field("drop").map(String::from),
            _ if matches!(c.level, 'W' | 'E') => {
                *self.others.entry(format!("{} {} {}", c.level, c.tag, c.word)).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    fn phase_until(&mut self, ms: u64) {
        match self.air_since {
            Some((from, true)) => self.air_ms += ms.saturating_sub(from),
            Some((from, false)) => self.ground_ms += ms.saturating_sub(from),
            None => {}
        }
    }

    fn close(mut self) -> Session {
        self.phase_until(self.last_ms);
        self.air_since = None;
        self
    }

    fn watch_words(&self) -> String {
        self.watch.iter().map(|(k, n)| format!("{} x{}", k, n)).collect::<Vec<String>>().join(", ")
    }

    fn rank(&self) -> (bool, bool, usize, usize) {
        (self.watchdog, self.log_stopped.is_some(), self.watch.values().sum(), self.others.values().sum())
    }

    fn worth_showing(&self) -> bool {
        self.settings.is_some() || self.reset.is_some() || !self.watch.is_empty() || !self.phases.is_empty()
            || !self.others.is_empty() || self.log_stopped.is_some()
    }

    fn json(&self) -> String {
        let counts = |m: &BTreeMap<String, usize>| {
            m.iter().map(|(k, n)| format!("\"{}\":{}", esc(k), n)).collect::<Vec<String>>().join(",")
        };
        let down: Vec<String> = self.longest_down.iter().map(|(k, n)| format!("\"{}\":{}", esc(k), n)).collect();
        let phases: Vec<String> = self.phases.iter().map(|(ms, s)| format!("{{\"ms\":{},\"text\":\"{}\"}}", ms, esc(s))).collect();
        format!(
            "{{\"source\":\"{}\",\"firstT\":{},\"firstMs\":{},\"lastMs\":{},\"lines\":{},\"settings\":{},\"settingsCut\":{},\"refused\":{},\"reset\":{},\"hardwareWatchdog\":{},\"watch\":{{{}}},\"longestDownMs\":{{{}}},\"phases\":[{}],\"airMs\":{},\"groundMs\":{},\"logStopped\":{},\"droppedLastSeen\":{},\"others\":{{{}}}}}",
            esc(&self.source),
            opt(&self.first_t),
            self.first_ms,
            self.last_ms,
            self.lines,
            opt_text(&self.settings),
            self.settings_cut,
            opt_text(&self.refused),
            opt_text(&self.reset),
            self.watchdog,
            counts(&self.watch),
            down.join(","),
            phases.join(","),
            self.air_ms,
            self.ground_ms,
            opt_text(&self.log_stopped),
            opt_text(&self.drop_last),
            counts(&self.others)
        )
    }
}

// Lines off one port go into sessions. Uptime going down is a new boot.
#[derive(Default)]
struct Consoles {
    open: HashMap<String, Session>,
    done: Vec<Session>,
}

impl Consoles {
    fn take(&mut self, source: &str, t: Option<f64>, c: &ConsoleLine) {
        let ended = self.open.get(source).is_some_and(|s| s.lines > 0 && c.ms < s.last_ms);
        if ended {
            let s = self.open.remove(source).unwrap();
            self.done.push(s.close());
        }
        let s = self.open.entry(source.to_string()).or_default();
        if s.lines == 0 {
            s.source = source.to_string();
            s.first_ms = c.ms;
            s.first_t = t;
        }
        s.take(c);
    }

    // The dump firmware's own few lines say nothing about the board's run
    fn close(mut self) -> Vec<Session> {
        let mut rest: Vec<Session> = self.open.drain().map(|(_, s)| s.close()).collect();
        rest.sort_by(|a, b| a.source.cmp(&b.source));
        self.done.extend(rest);
        self.done.into_iter().filter(|s| !s.dump_firmware).collect()
    }
}

// worst first when sorted high to low, the fields in the order rank() gives them
type BootRank = (bool, u32, bool, usize, usize, u64, u32);
type PullRank = (bool, bool, usize, BootRank);

#[derive(Debug, PartialEq)]
struct BootCheck {
    n: u32,
    records: usize,
    first_ms: u64,
    last_ms: u64,
    start_utc: Option<String>,
    anchors: usize,
    anchors_rejected: usize,
    csids: Vec<u8>,
    reset: Option<String>,
    reset_csr: Option<u32>,
    dropped: u32,
    dropped_seen: Vec<(u64, u32)>,   // every dropped event, uptime and the count so far
    other_events: Vec<(u64, u8)>,
    space: Spacing,
    sensors: Vec<Sensor>,
    fresh: usize,
    bad_packet_crc: usize,
    verdict: String,
}

impl BootCheck {
    fn hardware_watchdog(&self) -> bool {
        let words = self.reset.as_deref().unwrap_or("");
        words.split('+').any(|w| WATCHDOG_FLAGS.contains(&w))
    }

    fn flagged(&self) -> usize {
        self.sensors.iter().filter(|s| s.clear > 0).count()
    }

    fn longest_gap(&self) -> u64 {
        self.space.gaps.first().map_or(0, |g| g.len_ms)
    }

    // Worst first, in this order. A newer boot goes first among equals.
    fn rank(&self) -> BootRank {
        (
            self.hardware_watchdog(),
            self.dropped,
            self.bad_packet_crc > 0,
            self.flagged(),
            self.space.gaps.len(),
            self.longest_gap(),
            self.n,
        )
    }

    fn make_verdict(&mut self, several_boards: bool) {
        let mut said: Vec<String> = Vec::new();
        if self.hardware_watchdog() {
            said.push(String::from("hardware watchdog reset"));
        }
        if self.records == 0 {
            said.push(String::from("no records"));
        }
        if self.dropped > 0 {
            said.push(format!("{} dropped", self.dropped));
        }
        if self.bad_packet_crc > 0 {
            said.push(format!("{} failed the packet CRC", self.bad_packet_crc));
        }
        if !self.space.gaps.is_empty() {
            said.push(format!("{} up to {}", plural(self.space.gaps.len(), "gap"), seconds(self.longest_gap())));
        }
        if self.space.backward > 0 {
            said.push(format!("uptime went back {}", plural(self.space.backward, "time")));
        }
        let names = |never: bool| -> Vec<&str> {
            self.sensors.iter().filter(|s| s.clear > 0 && s.never_set() == never).map(|s| s.name).collect()
        };
        if !names(true).is_empty() {
            said.push(format!("never set: {}", names(true).join(" ")));
        }
        if !names(false).is_empty() {
            said.push(format!("clear at times: {}", names(false).join(" ")));
        }
        let reset = match &self.reset {
            Some(r) => format!("reset {}", r),
            None => String::from("reset cause not in the log"),
        };
        let whose = if several_boards && !self.csids.is_empty() { format!(" (board {})", join_ids(&self.csids)) } else { String::new() };
        let head = format!("boot {}{}, {}, {}", self.n, whose, seconds(self.last_ms.saturating_sub(self.first_ms)), plural(self.records, "record"));
        self.verdict = if said.is_empty() {
            format!("{}: nothing flagged, {}", head, reset)
        } else {
            format!("{}: {}, {}", head, said.join(", "), reset)
        };
    }

    fn text(&self, out: &mut String) {
        let line = |out: &mut String, s: String| out.push_str(&format!("    {}\n", s));
        if self.records > 0 {
            out.push_str(&format!("\n  boot {}: uptime {} to {} ms\n", self.n, self.first_ms, self.last_ms));
        } else {
            out.push_str(&format!("\n  boot {}: no records\n", self.n));
        }
        match &self.start_utc {
            Some(at) => line(out, format!("first record {} UTC, GPS time from {}{}", at, plural(self.anchors, "anchor"), rejected(self.anchors_rejected))),
            None => line(out, format!("no GPS time in this boot{}", rejected(self.anchors_rejected))),
        }
        if self.csids.len() > 1 {
            line(out, format!("packets carry more than one board id: {}", join_ids(&self.csids)));
        }
        if self.dropped > 0 {
            let first = self.dropped_seen.first().map_or(0, |d| d.0);
            let last = self.dropped_seen.last().map_or(0, |d| d.0);
            if first == last {
                line(out, format!("ring: {} dropped, said once at up={} ms", self.dropped, first));
            } else {
                line(out, format!("ring: {} dropped, {} from up={} to up={} ms", self.dropped, plural(self.dropped_seen.len(), "event"), first, last));
            }
        } else {
            line(out, String::from("ring: no dropped events"));
        }
        for (ms, kind) in self.other_events.iter().take(SHOWN) {
            line(out, format!("event of kind 0x{:02X} at up={} ms, not one this reads", kind, ms));
        }
        if self.records > 1 {
            line(
                out,
                format!(
                    "spacing, since flash keeps no phase: {} fast (air rate), {} steady and slow (ground rate), {} in gaps",
                    seconds(self.space.fast_ms),
                    seconds(self.space.steady_ms),
                    seconds(self.space.gap_ms)
                ),
            );
            for (ms, fast) in self.space.changes.iter().take(SHOWN) {
                line(out, format!("spacing changed at up={} ms to {}", ms, if *fast { "fast" } else { "steady and slow" }));
            }
            if self.space.changes.len() > SHOWN {
                line(out, format!("and {} more spacing changes", self.space.changes.len() - SHOWN));
            }
        }
        for g in self.space.gaps.iter().take(SHOWN) {
            line(out, format!("gap of {} after up={} ms", seconds(g.len_ms), g.at_ms));
        }
        if self.space.gaps.len() > SHOWN {
            line(out, format!("and {} more gaps, {} of gaps in all", self.space.gaps.len() - SHOWN, seconds(self.space.gap_ms)));
        }
        if self.space.backward > 0 {
            line(out, format!("uptime went backwards {}", plural(self.space.backward, "time")));
        }
        let all_set: Vec<&str> = self.sensors.iter().filter(|s| s.clear == 0 && s.records > 0).map(|s| s.name).collect();
        if !all_set.is_empty() {
            line(out, format!("flags set on every record: {}", all_set.join(" ")));
        }
        let never: Vec<&str> = self.sensors.iter().filter(|s| s.never_set()).map(|s| s.name).collect();
        if !never.is_empty() {
            line(out, format!("never set on any of {}: {}", plural(self.records, "record"), never.join(" ")));
        }
        let mut bad: Vec<&Sensor> = self.sensors.iter().filter(|s| s.clear > 0 && !s.never_set()).collect();
        bad.sort_by_key(|s| std::cmp::Reverse(s.clear_ms));
        // the four IMU flags usually fall together, and four lines saying one thing hide the rest
        let mut same: Vec<(Vec<&str>, &Sensor)> = Vec::new();
        for s in bad {
            match same.iter_mut().find(|(_, o)| o.clear == s.clear && o.spells == s.spells && o.longest == s.longest) {
                Some((names, _)) => names.push(s.name),
                None => same.push((vec![s.name], s)),
            }
        }
        for (names, s) in same {
            let worst = &s.longest[0];
            line(
                out,
                format!(
                    "{}: clear on {} of {} records, {} in {}, longest {} from up={} ms{}",
                    names.join(" "),
                    s.clear,
                    s.records,
                    seconds(s.clear_ms),
                    plural(s.spells, "spell"),
                    seconds(worst.len_ms),
                    worst.from_ms,
                    if worst.open { " to the end of the boot" } else { "" }
                ),
            );
        }
        if self.records > 0 {
            line(out, format!("all-fresh flag set on {} of {} records", self.fresh, self.records));
        }
    }

    fn json(&self) -> String {
        let gaps: Vec<String> = self.space.gaps.iter().map(|g| format!("{{\"afterMs\":{},\"ms\":{}}}", g.at_ms, g.len_ms)).collect();
        let changes: Vec<String> = self
            .space
            .changes
            .iter()
            .map(|(ms, fast)| format!("{{\"ms\":{},\"to\":\"{}\"}}", ms, if *fast { "fast" } else { "steady" }))
            .collect();
        let dropped: Vec<String> = self.dropped_seen.iter().map(|(ms, n)| format!("{{\"ms\":{},\"count\":{}}}", ms, n)).collect();
        let other: Vec<String> = self.other_events.iter().map(|(ms, k)| format!("{{\"ms\":{},\"kind\":{}}}", ms, k)).collect();
        let sensors: Vec<String> = self
            .sensors
            .iter()
            .map(|s| {
                let spells: Vec<String> = s
                    .longest
                    .iter()
                    .map(|p| format!("{{\"fromMs\":{},\"ms\":{},\"toEnd\":{}}}", p.from_ms, p.len_ms, p.open))
                    .collect();
                format!(
                    "{{\"name\":\"{}\",\"clear\":{},\"records\":{},\"clearMs\":{},\"spells\":{},\"longest\":[{}]}}",
                    s.name,
                    s.clear,
                    s.records,
                    s.clear_ms,
                    s.spells,
                    spells.join(",")
                )
            })
            .collect();
        format!(
            "{{\"n\":{},\"records\":{},\"firstMs\":{},\"lastMs\":{},\"startUtc\":{},\"anchors\":{},\"anchorsRejected\":{},\"csids\":[{}],\"resetCsr\":{},\"reset\":{},\"hardwareWatchdog\":{},\"dropped\":{},\"droppedEvents\":[{}],\"otherEvents\":[{}],\"fastMs\":{},\"steadyMs\":{},\"gapMs\":{},\"gaps\":[{}],\"spacingChanges\":[{}],\"uptimeBackward\":{},\"sensors\":[{}],\"fresh\":{},\"badPacketCrc\":{},\"verdict\":\"{}\"}}",
            self.n,
            self.records,
            self.first_ms,
            self.last_ms,
            opt_text(&self.start_utc),
            self.anchors,
            self.anchors_rejected,
            join_ids(&self.csids),
            opt(&self.reset_csr),
            opt_text(&self.reset),
            self.hardware_watchdog(),
            self.dropped,
            dropped.join(","),
            other.join(","),
            self.space.fast_ms,
            self.space.steady_ms,
            self.space.gap_ms,
            gaps.join(","),
            changes.join(","),
            self.space.backward,
            sensors.join(","),
            self.fresh,
            self.bad_packet_crc,
            esc(&self.verdict)
        )
    }
}

fn rejected(n: usize) -> String {
    if n > 0 { format!(", {} rejected", n) } else { String::new() }
}

fn check_boot(dump: &Dump, b: &dump::BootReport, several_boards: bool) -> BootCheck {
    let mine: Vec<&dump::Record> = dump.records.iter().filter(|r| r.boot == b.n).collect();
    let ms: Vec<u64> = mine.iter().map(|r| r.ms).collect();
    let packets: Vec<(u64, Result<Decoded, String>)> = mine
        .iter()
        .map(|r| (r.ms, decode::hex_to_bytes(&r.hex).map_or_else(|| Err(String::from("not hex")), |bytes| decode::decode(&bytes))))
        .collect();
    let good: Vec<(u64, [bool; 8])> = packets
        .iter()
        .filter_map(|(ms, d)| d.as_ref().ok().filter(|d| d.crc_ok).map(|d| (*ms, d.valid)))
        .collect();
    let mut events: Vec<&dump::Event> = dump.events.iter().filter(|e| e.boot == b.n).collect();
    events.sort_by_key(|e| e.ms);
    let mut boot = BootCheck {
        n: b.n,
        records: mine.len(),
        first_ms: b.first_ms,
        last_ms: b.last_ms,
        start_utc: b.anchored.then(|| utc((b.uptime0_ms + b.first_ms as i64).max(0) as u64)),
        anchors: b.anchors_used,
        anchors_rejected: b.anchors_rejected,
        csids: b.csids.clone(),
        reset: b.reset_csr.map(dump::reset_words),
        reset_csr: b.reset_csr,
        dropped: b.dropped.unwrap_or(0),
        dropped_seen: events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Dropped { count } => Some((e.ms, count)),
                _ => None,
            })
            .collect(),
        other_events: events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Other(kind) => Some((e.ms, kind)),
                _ => None,
            })
            .collect(),
        space: spacing(&ms),
        sensors: sensors(&good),
        fresh: good.iter().filter(|(_, v)| v[FRESH_BIT]).count(),
        bad_packet_crc: packets.len() - good.len(),
        verdict: String::new(),
    };
    boot.make_verdict(several_boards);
    boot
}

// A pull: one chip's log, from a dump.txt or from the ground.log made out of one

#[derive(Debug, PartialEq)]
struct PullCheck {
    name: String,
    file: String,
    from_log: bool,          // read from a ground.log, so no events and no slot counts
    uid: Option<String>,
    format: &'static str,
    found: Option<usize>,
    good: Option<usize>,
    bad: Option<usize>,
    read: usize,
    failed: usize,
    missing: usize,
    ignored_old: usize,
    complete: bool,
    wrapped: bool,
    printings: usize,
    bytes: usize,
    csids: Vec<u8>,
    unread: usize,
    samples: Vec<String>,
    missing_boots: Vec<u32>,
    lock_events: usize,
    boots: Vec<BootCheck>,   // worst first
    consoles: Vec<Session>,
}

impl PullCheck {
    fn board(&self) -> String {
        if !self.csids.is_empty() {
            return format!("board {}", join_ids(&self.csids));
        }
        match self.consoles.iter().rev().find_map(|s| s.csid.clone()) {
            Some(id) => format!("board {}", id),
            None => String::from("board unknown"),
        }
    }

    fn records(&self) -> usize {
        self.boots.iter().map(|b| b.records).sum()
    }

    fn settings(&self) -> Option<&Session> {
        self.consoles.iter().rev().find(|s| s.settings.is_some())
    }

    fn csid_differs(&self) -> Option<String> {
        let said: u8 = self.settings()?.csid.as_ref()?.parse().ok()?;
        (!self.csids.is_empty() && !self.csids.contains(&said)).then(|| {
            format!("settings say csid={} but the packets in the log say {}", said, join_ids(&self.csids))
        })
    }

    fn hardware_watchdogs(&self) -> usize {
        self.boots.iter().filter(|b| b.hardware_watchdog()).count()
    }

    fn line(&self) -> String {
        let mut bits = vec![
            plural(self.boots.len(), "boot"),
            plural(self.records(), "record"),
            if self.from_log {
                String::from("slot counts not in this file")
            } else if self.complete {
                String::from("all read")
            } else {
                String::from("NOT all read")
            },
        ];
        let watchdogs = self.hardware_watchdogs();
        if watchdogs > 0 {
            bits.push(format!("{} hardware watchdog", plural(watchdogs, "reset")));
        }
        let dropped: u64 = self.boots.iter().map(|b| b.dropped as u64).sum();
        if dropped > 0 {
            bits.push(format!("{} dropped", dropped));
        }
        let gaps: usize = self.boots.iter().map(|b| b.space.gaps.len()).sum();
        if gaps > 0 {
            bits.push(plural(gaps, "gap"));
        }
        let flagged: BTreeSet<&str> = self.boots.iter().flat_map(|b| b.sensors.iter().filter(|s| s.clear > 0).map(|s| s.name)).collect();
        if !flagged.is_empty() {
            bits.push(format!("flags clear: {}", flagged.into_iter().collect::<Vec<&str>>().join(" ")));
        }
        if self.unread > 0 {
            bits.push(format!("{} unread", plural(self.unread, "line")));
        }
        format!("{}  {}  {}", self.board(), self.name, bits.join(", "))
    }

    // How complete the file is comes first, because everything else leans on it
    fn rank(&self) -> PullRank {
        let top = self.boots.first().map(BootCheck::rank).unwrap_or_default();
        (self.boots.is_empty(), !self.complete && !self.from_log, self.failed + self.missing, top)
    }

    fn text(&self, out: &mut String) {
        out.push_str(&format!("{}  {}\n", self.board(), self.file));
        let line = |out: &mut String, s: String| out.push_str(&format!("  {}\n", s));
        if let Some(uid) = &self.uid {
            line(out, format!("chip id {}", uid));
        }
        if self.from_log {
            line(out, String::from("read from a log made out of a pull, which keeps no reset causes, no dropped counts and no slot counts: dump.txt has those"));
        } else if self.format == "none" {
            line(out, format!("no log in this file: {} bytes, nothing that reads as a record", self.bytes));
        } else {
            let of = self.good.or(self.found).map_or(String::from("the board never gave its count"), |n| format!("of {} the board counted", n));
            line(
                out,
                format!(
                    "log: {} slots read {}, {} failed their check, {} missing, {}",
                    self.read,
                    of,
                    self.failed,
                    self.missing,
                    if self.complete { "complete" } else { "NOT complete" }
                ),
            );
            if let Some(bad) = self.bad.filter(|n| *n > 0) {
                line(out, format!("the board itself called {} bad", plural(bad, "slot")));
            }
            line(out, String::from(if self.wrapped { "wrapped: the oldest records were written over" } else { "not wrapped" }));
        }
        if self.printings > 1 {
            line(out, format!("{} printings in the file, the last whole one is what this reads", self.printings));
        }
        if self.ignored_old > 0 {
            line(out, format!("{} old-format lines of an earlier printing left out", self.ignored_old));
        }
        if self.lock_events > 0 {
            line(out, format!("{} from the older ring, counted and not read", plural(self.lock_events, "lock event")));
        }
        if !self.missing_boots.is_empty() {
            line(out, format!("boots not in the log: {}", join_ids(&self.missing_boots)));
        }
        match self.settings() {
            Some(s) => {
                line(out, format!("settings {}{}", s.settings.as_deref().unwrap_or(""), if s.settings_cut { " (line cut short)" } else { "" }));
                line(out, String::from("  as printed when the board restarted for the pull, flash keeps none. fixedms are transmit gaps, ground first. Log gaps are not printed"));
                if let Some(why) = &s.refused {
                    line(out, format!("  REFUSED: {}, so the block was not used", why));
                }
            }
            None => line(out, String::from("settings: no settings line in this file")),
        }
        if let Some(differs) = self.csid_differs() {
            line(out, differs);
        }
        console_text(&self.consoles, false, "  ", out);
        if self.consoles.iter().all(|s| s.watch.is_empty()) {
            line(out, String::from("sensor recovery and BUS lines: none in this file. They go to the console only, never to flash, so this does not mean there were none"));
        }
        if !self.boots.is_empty() {
            line(out, String::from("boots, worst first:"));
        }
        for b in &self.boots {
            line(out, format!("  {}", b.verdict));
        }
        for b in &self.boots {
            b.text(out);
        }
        unread_text(self.unread, &self.samples, "  ", out);
    }

    fn json(&self) -> String {
        let boots: Vec<String> = self.boots.iter().map(BootCheck::json).collect();
        let consoles: Vec<String> = self.consoles.iter().map(Session::json).collect();
        format!(
            "{{\"kind\":\"pull\",\"name\":\"{}\",\"file\":\"{}\",\"line\":\"{}\",\"board\":\"{}\",\"uid\":{},\"fromLog\":{},\"format\":\"{}\",\"found\":{},\"boardGood\":{},\"boardBad\":{},\"read\":{},\"failedCheck\":{},\"missing\":{},\"ignoredOldFormat\":{},\"complete\":{},\"wrapped\":{},\"printings\":{},\"bytes\":{},\"csids\":[{}],\"unreadLines\":{},\"unreadSamples\":[{}],\"bootsNotInLog\":[{}],\"lockEvents\":{},\"csidDiffers\":{},\"boots\":[{}],\"consoles\":[{}]}}",
            esc(&self.name),
            esc(&self.file),
            esc(&self.line()),
            esc(&self.board()),
            opt_text(&self.uid),
            self.from_log,
            self.format,
            opt(&self.found),
            opt(&self.good),
            opt(&self.bad),
            self.read,
            self.failed,
            self.missing,
            self.ignored_old,
            self.complete,
            self.wrapped,
            self.printings,
            self.bytes,
            join_ids(&self.csids),
            self.unread,
            quoted(&self.samples),
            join_ids(&self.missing_boots),
            self.lock_events,
            opt_text(&self.csid_differs()),
            boots.join(","),
            consoles.join(",")
        )
    }
}

fn console_text(sessions: &[Session], with_settings: bool, pad: &str, out: &mut String) {
    let mut shown: Vec<&Session> = sessions.iter().filter(|s| s.worth_showing()).collect();
    if shown.is_empty() {
        return;
    }
    out.push_str(&format!("{}console caught: {}, worst first\n", pad, plural(sessions.len(), "run")));
    shown.sort_by_key(|s| std::cmp::Reverse(s.rank()));
    let quieter = shown.len().saturating_sub(SHOWN * 2);
    for s in shown.iter().take(SHOWN * 2) {
        let mut head = format!("{}  {}:", pad, s.source);
        if let Some(t) = s.first_t {
            head.push_str(&format!(" {},", local(t)));
        }
        head.push_str(&format!(" {} over up={} to {} ms", plural(s.lines, "line"), s.first_ms, s.last_ms));
        if let Some(reset) = &s.reset {
            head.push_str(&format!(", reset {}", reset));
        }
        if s.watchdog {
            head.push_str(", HARDWARE WATCHDOG");
        }
        out.push_str(&head);
        out.push('\n');
        if let (true, Some(settings)) = (with_settings, &s.settings) {
            out.push_str(&format!("{}    settings {}{}\n", pad, settings, if s.settings_cut { " (line cut short)" } else { "" }));
            if let Some(why) = &s.refused {
                out.push_str(&format!("{}    REFUSED: {}, so the block was not used\n", pad, why));
            }
        }
        if !s.watch.is_empty() {
            out.push_str(&format!("{}    recovery: {}\n", pad, s.watch_words()));
        }
        for (tag, ms) in &s.longest_down {
            out.push_str(&format!("{}    {} back after {} down at the longest\n", pad, tag, seconds(*ms)));
        }
        for (ms, what) in s.phases.iter().take(SHOWN) {
            out.push_str(&format!("{}    phase at up={} ms: {}\n", pad, ms, what));
        }
        if s.phases.len() > SHOWN {
            out.push_str(&format!("{}    and {} more phase lines\n", pad, s.phases.len() - SHOWN));
        }
        if s.air_ms + s.ground_ms > 0 {
            out.push_str(&format!("{}    from the phase lines: {} air, {} ground\n", pad, seconds(s.air_ms), seconds(s.ground_ms)));
        }
        if let Some(stopped) = &s.log_stopped {
            out.push_str(&format!("{}    flash log stopped: {}\n", pad, stopped));
        }
        if let Some(n) = &s.drop_last {
            out.push_str(&format!("{}    last packet line said drop={}\n", pad, n));
        }
        let mut others: Vec<(&String, &usize)> = s.others.iter().collect();
        others.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (what, n) in others.iter().take(SHOWN) {
            out.push_str(&format!("{}    {} x{}\n", pad, what, n));
        }
        if others.len() > SHOWN {
            out.push_str(&format!("{}    and {} other kinds of warning\n", pad, others.len() - SHOWN));
        }
    }
    if quieter > 0 {
        out.push_str(&format!("{}  and {} quieter\n", pad, plural(quieter, "run")));
    }
}

fn unread_text(unread: usize, samples: &[String], pad: &str, out: &mut String) {
    if unread == 0 {
        out.push_str(&format!("\n{}unread lines: 0\n", pad));
        return;
    }
    out.push_str(&format!("\n{}unread lines: {}\n", pad, unread));
    for s in samples {
        out.push_str(&format!("{}  {}\n", pad, s));
    }
}

fn boots_not_in_log(boots: &[u32]) -> Vec<u32> {
    let (Some(first), Some(last)) = (boots.first(), boots.last()) else { return Vec::new() };
    (*first..*last).filter(|n| !boots.contains(n)).collect()
}

// What is in a dump besides the log itself
fn dump_console(text: &str) -> (Vec<Session>, usize, Vec<String>, usize) {
    let mut consoles = Consoles::default();
    let (mut unread, mut samples, mut printings) = (0, Vec::new(), 0);
    let hex_line = |l: &str| l.split_whitespace().count() == dump::PACKET_BYTES && l.split_whitespace().all(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()));
    for line in text.lines() {
        let line = line.trim();
        let first = line.split_whitespace().next().unwrap_or("");
        if line.is_empty() || matches!(first, "raw" | "record" | "time") || hex_line(line) || line.starts_with("send D to print") {
            continue;
        }
        if first == "found" {
            printings += 1;
            continue;
        }
        let found = console_lines(line);
        if found.is_empty() || !line.starts_with(|c| matches!(c, 'I' | 'W' | 'E' | 'D')) {
            unread += 1;
            if samples.len() < SAMPLES {
                samples.push(sample(line));
            }
        }
        for c in &found {
            consoles.take("dump.txt", None, c);
        }
    }
    (consoles.close(), unread, samples, printings)
}

// pull.rs reads only what arrived after the reset, because the tail of an older printing can
// still be on the port and would count as cut slots of this one. A file has no reset to go by
// but the dump firmware's first line. It seems to always be there, and when it is not, or
// nothing follows it, the whole file is read.
fn read_printing(text: &str) -> Dump {
    if let Some(at) = text.rfind(DUMP_FIRMWARE_START) {
        let parsed = dump::parse_what_arrived(&text[at..]);
        if !parsed.records.is_empty() {
            return parsed;
        }
    }
    dump::parse_what_arrived(text)
}

fn check_dump(name: &str, file: &str, text: &str, bytes: usize, uid: Option<String>) -> PullCheck {
    let parsed = read_printing(text);
    let (consoles, unread, samples, printings) = dump_console(text);
    pull_from(name, file, &parsed, bytes, uid, consoles, unread, samples, printings, false)
}

#[allow(clippy::too_many_arguments)]
fn pull_from(
    name: &str,
    file: &str,
    parsed: &Dump,
    bytes: usize,
    uid: Option<String>,
    consoles: Vec<Session>,
    unread: usize,
    samples: Vec<String>,
    printings: usize,
    from_log: bool,
) -> PullCheck {
    let report = dump::report(parsed, bytes, 0);
    let mut boots: Vec<BootCheck> = report.boots.iter().map(|b| check_boot(parsed, b, report.csids.len() > 1)).collect();
    let numbers: Vec<u32> = boots.iter().map(|b| b.n).collect();
    boots.sort_by_key(|b| std::cmp::Reverse(b.rank()));
    let kept = parsed.records.len() + parsed.anchors.len() + parsed.events.len();
    PullCheck {
        name: name.to_string(),
        file: file.to_string(),
        from_log,
        uid,
        format: if from_log {
            "log"
        } else if parsed.raw {
            "raw"
        } else if parsed.records.is_empty() {
            "none"
        } else {
            "text"
        },
        found: report.found,
        good: report.board_good,
        bad: report.board_bad,
        read: report.read,
        failed: report.failed_check,
        missing: report.missing,
        ignored_old: report.ignored_old_format,
        complete: report.complete,
        wrapped: report.wrapped,
        printings,
        bytes,
        csids: report.csids,
        unread,
        samples,
        missing_boots: boots_not_in_log(&numbers),
        lock_events: report.read.saturating_sub(kept),
        boots,
        consoles,
    }
}

// A ground.log out of a pull, back into records: boot from the receiver name, uptime and index
// from the end of each row (dump.rs to_log). For when the dump.txt is not beside it.
fn check_flash_log(name: &str, file: &str, text: &str, bytes: usize) -> PullCheck {
    let mut parsed = Dump::default();
    let (mut unread, mut samples) = (0, Vec::new());
    for l in lines::log_events(text, name, 0.0) {
        let ev = lines::parse_line(l.text);
        let boot = l.rx.strip_prefix("flash-boot").and_then(|n| n.parse::<u32>().ok());
        let row = match (&ev.kind, boot) {
            (Kind::Packet { bytes, more: Some(more), .. }, Some(boot)) => {
                let field = |key: &str| more.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.parse::<u64>().ok());
                field("up").map(|ms| dump::Record {
                    n: field("n").unwrap_or(parsed.records.len() as u64),
                    addr: None,
                    boot,
                    ms,
                    hex: decode::bytes_to_hex(bytes),
                })
            }
            (Kind::Device { .. }, _) => continue,
            _ => None,
        };
        match row {
            Some(r) => parsed.records.push(r),
            None => {
                unread += 1;
                if samples.len() < SAMPLES {
                    samples.push(sample(l.text));
                }
            }
        }
    }
    parsed.read_slots = parsed.records.len();
    let mut check = pull_from(name, file, &parsed, bytes, None, Vec::new(), unread, samples, 0, true);
    check.format = "log";
    check
}

// A log: what the receivers heard, and any console that was on a port

#[derive(Debug, Default, Clone, PartialEq)]
struct Heard {
    rx: String,
    packets: usize,
    rssi: Option<(f64, f64)>,
    snr: Option<(f64, f64)>,
}

fn widen(range: &mut Option<(f64, f64)>, v: f64) {
    if v.is_finite() {
        *range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct BoardHeard {
    csid: u8,
    packets: u64,
    runs: usize,
    missed: u64,
    restarts: u64,
    from_chip: bool,
    first_t: f64,
    last_t: f64,
    longest_quiet_ms: u64,
    rssi: Option<(f64, f64)>,
    snr: Option<(f64, f64)>,
    heard: Vec<Heard>,
    clear: [u64; SENSORS],
}

impl BoardHeard {
    fn sensor_words(&self) -> String {
        (0..SENSORS)
            .filter(|i| self.clear[*i] > 0)
            .map(|i| format!("{} {}", VALIDITY_KEYS[i], self.clear[i]))
            .collect::<Vec<String>>()
            .join(", ")
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct RxHeard {
    key: String,
    id: Option<String>,
    sf: Option<String>,
    lines: usize,
    packets: u64,
    bad_crc: u64,
    radio_errors: u64,
    boots: usize,
    first_t: Option<f64>,
    last_t: Option<f64>,
    longest_quiet_ms: u64,
}

#[derive(Debug, PartialEq)]
struct LogCheck {
    name: String,
    file: String,
    bytes: usize,
    dated: bool,       // our own log, whose first column is real time
    first_t: Option<f64>,
    last_t: Option<f64>,
    lines: usize,
    boards: Vec<BoardHeard>,   // worst first
    receivers: Vec<RxHeard>,
    bad_crc: u64,
    bad_length: u64,
    duplicates: u64,
    unread: usize,
    samples: Vec<String>,
    consoles: Vec<Session>,
}

fn range_words(r: &Option<(f64, f64)>, unit: &str) -> String {
    match r {
        Some((lo, hi)) => format!("{} to {} {}", lo, hi, unit),
        None => format!("no {}", unit),
    }
}

fn range_json(r: &Option<(f64, f64)>) -> String {
    match r {
        Some((lo, hi)) => format!("[{},{}]", lo, hi),
        None => String::from("null"),
    }
}

impl LogCheck {
    fn when(&self, t: f64) -> String {
        if self.dated { local(t) } else { format!("{} s into the file", (t / 1000.0).round()) }
    }

    fn line(&self) -> String {
        let packets: u64 = self.boards.iter().map(|b| b.packets).sum();
        let missed: u64 = self.boards.iter().map(|b| b.missed).sum();
        let restarts: u64 = self.boards.iter().map(|b| b.restarts).sum();
        let mut bits = vec![plural(self.boards.len(), "board"), format!("{} packets", packets), plural(self.receivers.len(), "receiver")];
        if missed > 0 {
            bits.push(format!("{} missed", missed));
        }
        if restarts > 0 {
            bits.push(plural(restarts as usize, "restart"));
        }
        if self.bad_crc > 0 {
            bits.push(format!("{} bad CRC", self.bad_crc));
        }
        if self.consoles.iter().any(|s| s.watchdog) {
            bits.push(String::from("hardware watchdog on a console"));
        }
        if self.unread > 0 {
            bits.push(format!("{} unread", plural(self.unread, "line")));
        }
        format!("log  {}  {}", self.name, bits.join(", "))
    }

    fn rank(&self) -> (bool, u64, u64) {
        let restarts = self.boards.iter().map(|b| b.restarts).sum();
        let missed = self.boards.iter().map(|b| b.missed).sum();
        (self.consoles.iter().any(|s| s.watchdog), restarts, missed)
    }

    fn text(&self, out: &mut String) {
        out.push_str(&format!("log  {}\n", self.file));
        match (self.first_t, self.last_t) {
            (Some(a), Some(b)) => out.push_str(&format!("  {} to {}, {}\n", self.when(a), self.when(b), plural(self.lines, "line"))),
            _ => out.push_str("  no lines in this log\n"),
        }
        if self.boards.is_empty() {
            out.push_str("  no board heard\n");
        }
        for b in &self.boards {
            let missed = if b.from_chip { String::from("missed not counted for records off a chip") } else { format!("{} missed", b.missed) };
            out.push_str(&format!(
                "  board {}: {} packets, {}, {}, RSSI {}, SNR {}, last heard {}, longest quiet {}\n",
                b.csid,
                b.packets,
                missed,
                plural(b.restarts as usize, "restart"),
                range_words(&b.rssi, "dBm"),
                range_words(&b.snr, "dB"),
                self.when(b.last_t),
                seconds(b.longest_quiet_ms)
            ));
            for h in &b.heard {
                out.push_str(&format!("    {}: {} packets, RSSI {}, SNR {}\n", h.rx, h.packets, range_words(&h.rssi, "dBm"), range_words(&h.snr, "dB")));
            }
            let words = b.sensor_words();
            if !words.is_empty() {
                out.push_str(&format!("    flags clear, packets out of {}: {}\n", b.packets, words));
            }
        }
        for r in &self.receivers {
            out.push_str(&format!(
                "  receiver {}{}{}: {}, {} packets, {} bad CRC, {} radio errors, {}, last line {}, longest quiet {}\n",
                r.key,
                r.id.as_ref().map(|i| format!(" id {}", i)).unwrap_or_default(),
                r.sf.as_ref().map(|s| format!(" (last set to sf {})", s)).unwrap_or_default(),
                plural(r.lines, "line"),
                r.packets,
                r.bad_crc,
                r.radio_errors,
                plural(r.boots, "restart line"),
                r.last_t.map_or(String::from("none"), |t| self.when(t)),
                seconds(r.longest_quiet_ms)
            ));
        }
        if self.bad_length > 0 {
            out.push_str(&format!("  {} packets of the wrong length\n", self.bad_length));
        }
        if self.duplicates > 0 {
            out.push_str(&format!("  {} of the same bytes inside the same-packet window, counted once in a board's packets\n", plural(self.duplicates as usize, "repeat")));
        }
        console_text(&self.consoles, true, "  ", out);
        if self.consoles.is_empty() {
            out.push_str("  console: none in this log, so no sensor recovery, BUS or phase lines\n");
        }
        unread_text(self.unread, &self.samples, "  ", out);
    }

    fn json(&self) -> String {
        let boards: Vec<String> = self
            .boards
            .iter()
            .map(|b| {
                let heard: Vec<String> = b
                    .heard
                    .iter()
                    .map(|h| format!("{{\"rx\":\"{}\",\"packets\":{},\"rssi\":{},\"snr\":{}}}", esc(&h.rx), h.packets, range_json(&h.rssi), range_json(&h.snr)))
                    .collect();
                let clear: Vec<String> = (0..SENSORS).map(|i| format!("\"{}\":{}", VALIDITY_KEYS[i], b.clear[i])).collect();
                format!(
                    "{{\"csid\":{},\"packets\":{},\"runs\":{},\"missed\":{},\"restarts\":{},\"fromChip\":{},\"firstT\":{},\"lastT\":{},\"longestQuietMs\":{},\"rssi\":{},\"snr\":{},\"receivers\":[{}],\"clear\":{{{}}}}}",
                    b.csid,
                    b.packets,
                    b.runs,
                    b.missed,
                    b.restarts,
                    b.from_chip,
                    b.first_t,
                    b.last_t,
                    b.longest_quiet_ms,
                    range_json(&b.rssi),
                    range_json(&b.snr),
                    heard.join(","),
                    clear.join(",")
                )
            })
            .collect();
        let receivers: Vec<String> = self
            .receivers
            .iter()
            .map(|r| {
                format!(
                    "{{\"key\":\"{}\",\"id\":{},\"sf\":{},\"lines\":{},\"packets\":{},\"badCrc\":{},\"radioErrors\":{},\"boots\":{},\"firstT\":{},\"lastT\":{},\"longestQuietMs\":{}}}",
                    esc(&r.key),
                    opt_text(&r.id),
                    opt_text(&r.sf),
                    r.lines,
                    r.packets,
                    r.bad_crc,
                    r.radio_errors,
                    r.boots,
                    opt(&r.first_t),
                    opt(&r.last_t),
                    r.longest_quiet_ms
                )
            })
            .collect();
        let consoles: Vec<String> = self.consoles.iter().map(Session::json).collect();
        format!(
            "{{\"kind\":\"log\",\"name\":\"{}\",\"file\":\"{}\",\"line\":\"{}\",\"bytes\":{},\"dated\":{},\"firstT\":{},\"lastT\":{},\"lines\":{},\"badCrc\":{},\"badLength\":{},\"duplicates\":{},\"unreadLines\":{},\"unreadSamples\":[{}],\"boards\":[{}],\"receivers\":[{}],\"consoles\":[{}]}}",
            esc(&self.name),
            esc(&self.file),
            esc(&self.line()),
            self.bytes,
            self.dated,
            opt(&self.first_t),
            opt(&self.last_t),
            self.lines,
            self.bad_crc,
            self.bad_length,
            self.duplicates,
            self.unread,
            quoted(&self.samples),
            boards.join(","),
            receivers.join(","),
            consoles.join(",")
        )
    }
}

fn check_log(name: &str, file: &str, text: &str, bytes: usize) -> LogCheck {
    let events = lines::log_events(text, name, 0.0);
    let v1 = text.trim_start_matches('\u{feff}').starts_with("# descent-ground log v1");
    let written = text.lines().skip(usize::from(v1)).filter(|l| !l.trim().is_empty()).count();
    let mut fleet = Fleet::new(Config::default());
    let mut consoles = Consoles::default();
    let mut boards: BTreeMap<u8, BoardHeard> = BTreeMap::new();
    let mut heard: BTreeMap<(u8, String), Heard> = BTreeMap::new();
    let mut rx_extra: BTreeMap<String, RxHeard> = BTreeMap::new();
    let mut quiet: HashMap<String, f64> = HashMap::new();
    // a v1 row that lost its columns never reaches the loop below, and is unread all the same
    let mut unread = written - events.len().min(written);
    let mut samples: Vec<String> = if unread > 0 {
        text.lines().skip(1).filter(|l| !l.trim().is_empty() && l.split('\t').count() < 3).take(SAMPLES).map(sample).collect()
    } else {
        Vec::new()
    };
    let (mut first_t, mut last_t) = (None::<f64>, None::<f64>);

    for l in &events {
        first_t = Some(first_t.map_or(l.t, |t| t.min(l.t)));
        last_t = Some(last_t.map_or(l.t, |t| t.max(l.t)));
        let ev = lines::parse_line(l.text);
        match &ev.kind {
            Kind::Other => {
                let found = console_lines(&ev.text);
                if found.is_empty() {
                    unread += 1;
                    if samples.len() < SAMPLES {
                        samples.push(sample(&ev.text));
                    }
                }
                for c in &found {
                    consoles.take(&l.rx, v1.then_some(l.t), c);
                }
                continue;
            }
            Kind::Empty => continue,
            _ => {}
        }
        let rx = rx_extra.entry(l.rx.clone()).or_default();
        rx.key = l.rx.clone();
        rx.lines += 1;
        if matches!(ev.kind, Kind::Device { kind: "boot", .. }) {
            rx.boots += 1;
        }
        if let Some(before) = quiet.insert(l.rx.clone(), l.t) {
            rx.longest_quiet_ms = rx.longest_quiet_ms.max((l.t - before).max(0.0) as u64);
        }
        fleet.ingest(&ev, &l.rx, l.t);
        let Kind::Packet { decoded: Ok(d), rssi, snr, .. } = &ev.kind else { continue };
        if !d.crc_ok {
            continue;
        }
        let csid = d.csid();
        let h = heard.entry((csid, l.rx.clone())).or_default();
        h.rx = l.rx.clone();
        h.packets += 1;
        widen(&mut h.rssi, *rssi);
        widen(&mut h.snr, *snr);
        let b = boards.entry(csid).or_insert_with(|| BoardHeard { csid, first_t: l.t, last_t: l.t, ..BoardHeard::default() });
        b.longest_quiet_ms = b.longest_quiet_ms.max((l.t - b.last_t).max(0.0) as u64);
        b.last_t = b.last_t.max(l.t);
        widen(&mut b.rssi, *rssi);
        widen(&mut b.snr, *snr);
        // the same bytes off a second receiver are the same reading
        if fleet.last_result == "new" {
            for (i, clear) in b.clear.iter_mut().enumerate() {
                *clear += u64::from(!d.valid[i]);
            }
        }
    }

    for u in fleet.all_units() {
        let Some(b) = boards.get_mut(&u.csid) else { continue };
        b.packets += u.packets;
        b.runs += 1;
        b.missed += u.missed;
        b.restarts += u.resets;
        b.from_chip |= u.from_chip;
    }
    for b in boards.values_mut() {
        b.restarts += b.runs.saturating_sub(1) as u64;
    }
    for ((csid, _), h) in heard {
        if let Some(b) = boards.get_mut(&csid) {
            b.heard.push(h);
        }
    }
    let mut boards: Vec<BoardHeard> = boards.into_values().collect();
    boards.sort_by_key(|b| std::cmp::Reverse((b.restarts, b.missed)));
    let receivers = fleet
        .receivers(0.0)
        .iter()
        .map(|r| {
            let extra = rx_extra.remove(&r.key).unwrap_or_default();
            RxHeard {
                key: r.key.clone(),
                id: r.id.clone(),
                sf: r.info.iter().find(|(k, _)| k == "sf").map(|(_, v)| v.clone()),
                lines: extra.lines,
                packets: r.packets,
                bad_crc: r.bad_crc,
                radio_errors: r.radio_errors,
                boots: extra.boots,
                first_t: r.first_t,
                last_t: r.last_t,
                longest_quiet_ms: extra.longest_quiet_ms,
            }
        })
        .collect();
    LogCheck {
        name: name.to_string(),
        file: file.to_string(),
        bytes,
        dated: v1,
        first_t,
        last_t,
        lines: events.len(),
        boards,
        receivers,
        bad_crc: fleet.bad_crc,
        bad_length: fleet.bad_length,
        duplicates: fleet.duplicates,
        unread,
        samples,
        consoles: consoles.close(),
    }
}

#[derive(Debug)]
enum Item {
    Pull(PullCheck),
    Log(LogCheck),
}

impl Item {
    fn line(&self) -> String {
        match self {
            Item::Pull(p) => p.line(),
            Item::Log(l) => l.line(),
        }
    }

    fn json(&self) -> String {
        match self {
            Item::Pull(p) => p.json(),
            Item::Log(l) => l.json(),
        }
    }

    fn text(&self, out: &mut String) {
        match self {
            Item::Pull(p) => p.text(out),
            Item::Log(l) => l.text(out),
        }
    }

    // Pulls before logs: a pull is one board's whole story
    fn rank(&self) -> (bool, PullRank, (bool, u64, u64)) {
        match self {
            Item::Pull(p) => (true, p.rank(), Default::default()),
            Item::Log(l) => (false, Default::default(), l.rank()),
        }
    }
}

#[derive(Debug)]
pub struct Check {
    asked: String,
    items: Vec<Item>,
    not_read: Vec<String>,    // files in a folder that are neither a pull nor a log, or would not open
}

fn looks_like_dump(text: &str) -> bool {
    text.lines().any(|l| {
        let first = l.split_whitespace().next().unwrap_or("");
        (first == "raw" && l.split_whitespace().count() >= 3) || (first == "record" && l.contains(" boot ") && l.contains(" ms "))
            || (first == "found" && l.contains(" records"))
    })
}

fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

fn read_text(path: &Path) -> Result<(String, usize), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), bytes.len()))
}

fn board_uid(pull: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pull.join("board.txt")).ok()?;
    let uid = text.trim();
    (!uid.is_empty()).then(|| uid.to_string())
}

fn check_pull_folder(pull: &Path) -> Result<Item, String> {
    let file = pull.join("dump.txt");
    let (text, bytes) = read_text(&file)?;
    Ok(Item::Pull(check_dump(&folder_name(pull), &file.display().to_string(), &text, bytes, board_uid(pull))))
}

fn check_file(path: &Path) -> Result<Item, String> {
    let (text, bytes) = read_text(path)?;
    let name = folder_name(path);
    let shown = path.display().to_string();
    if text.starts_with("# descent-ground log v1 from flash") {
        // dump.txt is the richer of the two whenever it is there
        return match path.parent().filter(|p| p.join("dump.txt").is_file()) {
            Some(pull) => check_pull_folder(pull),
            None => Ok(Item::Pull(check_flash_log(&name, &shown, &text, bytes))),
        };
    }
    if name == "dump.txt" || looks_like_dump(&text) {
        let (label, uid) = match path.parent().filter(|_| name == "dump.txt") {
            Some(pull) => (folder_name(pull), board_uid(pull)),
            None => (name, None),
        };
        return Ok(Item::Pull(check_dump(&label, &shown, &text, bytes, uid)));
    }
    Ok(Item::Log(check_log(&name, &shown, &text, bytes)))
}

fn walk(dir: &Path, depth: usize, items: &mut Vec<Item>, not_read: &mut Vec<String>) -> Result<(), String> {
    if dir.join("dump.txt").is_file() {
        match check_pull_folder(dir) {
            Ok(item) => items.push(item),
            Err(e) => not_read.push(e),
        }
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = folder_name(&path);
        if path.is_dir() {
            if depth == 0 {
                not_read.push(format!("{}: deeper than this looks", path.display()));
            } else if let Err(e) = walk(&path, depth - 1, items, not_read) {
                not_read.push(e);
            }
        } else if name.ends_with(".log") {
            match check_file(&path) {
                Ok(item) => items.push(item),
                Err(e) => not_read.push(e),
            }
        } else if !PULL_EXTRAS.contains(&name.as_str()) {
            not_read.push(path.display().to_string());
        }
    }
    Ok(())
}

// A file, or a folder of anything, down to a pull in a day folder
pub fn check_path(path: &Path) -> Result<Check, String> {
    let mut check = Check { asked: path.display().to_string(), items: Vec::new(), not_read: Vec::new() };
    if path.is_dir() {
        walk(path, WALK_DEPTH, &mut check.items, &mut check.not_read)?;
    } else {
        check.items.push(check_file(path)?);
    }
    check.items.sort_by_key(|i| std::cmp::Reverse(i.rank()));
    Ok(check)
}

impl Check {
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.items.is_empty() {
            out.push_str(&format!("{}: no pull or log found\n", self.asked));
        }
        if self.items.len() > 1 {
            out.push_str(&format!("{}: {}, worst first\n", self.asked, plural(self.items.len(), "file")));
            for item in &self.items {
                out.push_str(&format!("  {}\n", item.line()));
            }
            out.push('\n');
        }
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            item.text(&mut out);
        }
        if !self.not_read.is_empty() {
            out.push_str(&format!("\nnot read: {}\n", plural(self.not_read.len(), "file")));
            for n in self.not_read.iter().take(SHOWN) {
                out.push_str(&format!("  {}\n", n));
            }
            if self.not_read.len() > SHOWN {
                out.push_str(&format!("  and {} more\n", self.not_read.len() - SHOWN));
            }
        }
        out
    }

    pub fn json(&self) -> String {
        let items: Vec<String> = self.items.iter().map(Item::json).collect();
        format!(
            "{{\"asked\":\"{}\",\"lines\":[{}],\"items\":[{}],\"notRead\":[{}]}}",
            esc(&self.asked),
            quoted(&self.items.iter().map(Item::line).collect::<Vec<String>>()),
            items.join(","),
            quoted(&self.not_read)
        )
    }
}

// `descent-ground check ...`, the exit code back to main. 1 only when something could not be
// read at all: a report full of faults is still a report that worked.
pub fn run(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", HELP);
        return 0;
    }
    if let Some(wrong) = args.iter().find(|a| a.starts_with("--") && *a != "--json") {
        println!("check does not know {}\n\n{}", wrong, HELP);
        return 1;
    }
    let paths: Vec<&String> = args.iter().filter(|a| *a != "--json").collect();
    if paths.is_empty() {
        print!("{}", HELP);
        return 1;
    }
    let mut code = 0;
    let mut answers = Vec::new();
    for path in paths {
        match check_path(Path::new(path)) {
            Ok(c) => answers.push(c),
            Err(e) => {
                println!("{}", e);
                code = 1;
            }
        }
    }
    let printed = if !json {
        answers.iter().map(Check::text).collect::<Vec<String>>().join("\n")
    } else if let [one] = answers.as_slice() {
        one.json() + "\n"
    } else {
        format!("[{}]\n", answers.iter().map(Check::json).collect::<Vec<String>>().join(","))
    };
    // not print!: quitting less part way closes the pipe, and print! panics on that
    use std::io::Write;
    let _ = std::io::stdout().write_all(printed.as_bytes());
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SET: u8 = 0x7F;
    const FRESH: u8 = 0x80;
    const IMU: u8 = 0x0F;
    const GPS: u8 = 0x10;

    // A packet the way packet.js lays it out, zero but for what these tests read
    fn packet(counter: u16, csid: u8, validity: u8) -> [u8; 55] {
        let mut p = [0u8; 55];
        p[48..50].copy_from_slice(&counter.to_le_bytes());
        p[50] = csid;
        p[52] = validity;
        let crc = dump::crc16(&p[..53]);
        p[53..].copy_from_slice(&crc.to_le_bytes());
        p
    }

    // A slot the way FlashLog.cpp encodeRecord does it, as flash_dump prints it
    fn raw(n: u64, kind: u8, boot: u16, ms: u32, payload: &[u8]) -> String {
        let mut b = [0u8; 64];
        b[0] = kind;
        b[1..3].copy_from_slice(&boot.to_le_bytes());
        b[3..7].copy_from_slice(&ms.to_le_bytes());
        b[7..7 + payload.len()].copy_from_slice(payload);
        let crc = dump::crc16(&b[..62]);
        b[62..].copy_from_slice(&crc.to_le_bytes());
        format!("raw {} {:06X} {}\r\n", n, n * 64, decode::bytes_to_hex(&b))
    }

    fn rec(n: u64, boot: u16, ms: u32, validity: u8) -> String {
        raw(n, 0xA5, boot, ms, &packet(0, 17, validity))
    }

    fn event(n: u64, boot: u16, ms: u32, kind: u8, word: u32) -> String {
        let mut p = vec![kind];
        p.extend_from_slice(&word.to_le_bytes());
        raw(n, 0xA7, boot, ms, &p)
    }

    fn found(n: usize) -> String {
        format!("found {} records, {} good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x100\r\n", n, n)
    }

    const PIN_BOR_SFT: u32 = 0x1C01C600;
    const PIN_IWDG: u32 = 0x24000000;

    const CONSOLE: &str = "W GPS 82481 c24: read ok=0 reason=nofix fix=0\r\n\
I BOOT 1 c0: start log=4\r\n\
I BOOT 1 c0: settings from=block csid=17 module=hp sf=12 dbm=14 fixedms=0,1000,1000,0 soc=0,0,0,0\r\n\
I RST 6 c0: cause csr=0x1401C600 lpwr=0 wwdg=0 iwdg=0 sft=1 bor=0 pin=1 obl=0 rfilar=0\r\n\
I BOOT 0 c0: start test=flashdump\r\n";

    fn pull_text() -> String {
        [
            CONSOLE.to_string(),
            event(0, 0, 3700, 1, PIN_BOR_SFT),
            rec(1, 0, 4122, ALL_SET),
            rec(2, 0, 4172, ALL_SET),
            rec(3, 0, 7172, ALL_SET),
            event(4, 0, 7200, 3, 52),
            rec(5, 0, 7222, ALL_SET),
            found(6),
            String::from("send D to print the log again, E to erase the whole chip\r\n"),
        ]
        .concat()
    }

    fn pull(text: &str) -> PullCheck {
        check_dump("1945-board17", "dump.txt", text, text.len(), None)
    }

    fn printed(p: &PullCheck) -> String {
        let mut s = String::new();
        p.text(&mut s);
        s
    }

    #[test]
    fn a_pull_says_its_settings_resets_drops_and_gaps() {
        let p = pull(&pull_text());
        assert_eq!(p.board(), "board 17");
        assert!(p.complete);
        assert_eq!(p.boots.len(), 1);
        let b = &p.boots[0];
        assert_eq!((b.first_ms, b.last_ms, b.records), (4122, 7222, 4));
        // the boot's own reset, off flash, and not the one the console showed at the pull
        assert_eq!(b.reset.as_deref(), Some("PIN+BOR+SFT"));
        assert_eq!(p.consoles.last().unwrap().reset.as_deref(), Some("PIN+SFT"));
        assert_eq!(b.dropped, 52);
        assert_eq!(b.space.gaps, vec![Gap { at_ms: 4172, len_ms: 3000 }]);
        let text = printed(&p);
        assert!(text.contains("settings from=block csid=17 module=hp sf=12 dbm=14 fixedms=0,1000,1000,0 soc=0,0,0,0\n"), "{}", text);
        assert!(!text.contains("REFUSED"), "{}", text);
        assert!(text.contains("boot 0, 3.1 s, 4 records: 52 dropped, 1 gap up to 3.0 s, reset PIN+BOR+SFT\n"), "{}", text);
        assert!(text.contains("ring: 52 dropped, said once at up=7200 ms"), "{}", text);
        assert!(text.contains("not wrapped"), "{}", text);
        assert!(text.contains("unread lines: 0\n"), "{}", text);
    }

    #[test]
    fn nothing_flagged_is_only_said_of_a_boot_with_nothing_to_flag() {
        let text = [event(0, 3, 300, 1, PIN_BOR_SFT), rec(1, 3, 1000, ALL_SET), rec(2, 3, 1050, ALL_SET), found(3)].concat();
        let p = pull(&text);
        assert_eq!(p.boots[0].verdict, "boot 3, 50 ms, 2 records: nothing flagged, reset PIN+BOR+SFT");
    }

    #[test]
    fn a_refused_block_is_said_out_loud() {
        let line = "I BOOT 1 c0: settings from=build csid=255 module=le sf=7 dbm=14 fixedms=60000,1000,1000,60000 soc=0,0,0,0 refused=sf\r\n";
        let p = pull(&[line.to_string(), rec(0, 0, 100, ALL_SET), found(1)].concat());
        let text = printed(&p);
        assert!(text.contains("settings from=build csid=255"), "{}", text);
        assert!(text.contains("REFUSED: sf, so the block was not used"), "{}", text);
    }

    #[test]
    fn settings_that_disagree_with_the_packets_are_said() {
        let line = "I BOOT 1 c0: settings from=block csid=16 module=hp sf=12 dbm=14 fixedms=0,1000,1000,0 soc=0,0,0,0\r\n";
        let p = pull(&[line.to_string(), rec(0, 0, 100, ALL_SET), found(1)].concat());
        assert_eq!(p.csid_differs().as_deref(), Some("settings say csid=16 but the packets in the log say 17"));
    }

    #[test]
    fn no_settings_line_is_said_and_not_guessed() {
        let p = pull(&[rec(0, 0, 100, ALL_SET), found(1)].concat());
        let text = printed(&p);
        assert!(text.contains("settings: no settings line in this file"), "{}", text);
        assert!(text.contains("reset cause not in the log"), "{}", text);
        assert!(text.contains("sensor recovery and BUS lines: none in this file"), "{}", text);
    }

    #[test]
    fn a_steady_slow_run_is_ground_rate_and_a_lone_hole_is_a_gap() {
        // 50 ms air, a 3 s hole, more air, then 5 s ground
        let ms = [0, 50, 100, 3100, 3150, 8150, 13_151, 18_149, 18_199];
        let s = spacing(&ms);
        assert_eq!(s.gaps, vec![Gap { at_ms: 100, len_ms: 3000 }]);
        assert_eq!(s.steady_ms, 15_000 - 1);
        assert_eq!(s.changes, vec![(3150, false), (18_149, true)]);
        assert_eq!(s.fast_ms + s.steady_ms + s.gap_ms, 18_199);
    }

    #[test]
    fn stalls_between_bursts_stay_gaps_however_alike_they_are() {
        // what board 17 did at SF12: a second of records, then near 3 s of nothing
        let mut ms = Vec::new();
        for burst in 0..5u64 {
            ms.extend((0..20).map(|i| burst * 3800 + i * 50));
        }
        let s = spacing(&ms);
        assert_eq!(s.gaps.len(), 4);
        assert_eq!(s.steady_ms, 0);
    }

    #[test]
    fn a_hole_inside_a_steady_run_is_still_a_gap() {
        let s = spacing(&[0, 5000, 10_000, 15_000, 45_000, 50_000, 55_000, 60_000]);
        assert_eq!(s.gaps, vec![Gap { at_ms: 15_000, len_ms: 30_000 }]);
        assert_eq!(s.steady_ms, 30_000);
    }

    #[test]
    fn uptime_going_back_is_counted() {
        assert_eq!(spacing(&[100, 150, 120, 170]).backward, 1);
    }

    #[test]
    fn a_sensor_is_bad_from_its_flag_going_clear_until_it_comes_back() {
        let text = [
            rec(0, 0, 1000, ALL_SET & !GPS),
            rec(1, 0, 1050, ALL_SET & !IMU & !GPS),
            rec(2, 0, 1100, ALL_SET & !IMU & !GPS),
            rec(3, 0, 1400, ALL_SET & !GPS),
            rec(4, 0, 1450, ALL_SET & !IMU & !GPS),
            found(5),
        ]
        .concat();
        let p = pull(&text);
        let b = &p.boots[0];
        let accel = b.sensors.iter().find(|s| s.name == "accel").unwrap();
        assert_eq!((accel.clear, accel.spells, accel.clear_ms), (3, 2, 350));
        assert_eq!(accel.longest[0], Spell { from_ms: 1050, len_ms: 350, open: false });
        assert_eq!(accel.longest[1], Spell { from_ms: 1450, len_ms: 0, open: true });
        assert!(b.sensors.iter().find(|s| s.name == "gps").unwrap().never_set());
        assert_eq!(b.sensors.iter().find(|s| s.name == "env").unwrap().clear, 0);
        let text = printed(&p);
        assert!(text.contains("never set on any of 5 records: gps\n"), "{}", text);
        assert!(text.contains("accel gyro mag quat: clear on 3 of 5 records, 350 ms in 2 spells, longest 350 ms from up=1050 ms\n"), "{}", text);
        assert!(text.contains("flags set on every record: soc env\n"), "{}", text);
    }

    #[test]
    fn only_the_flags_count_and_a_value_is_never_judged() {
        // a battery of 0 V and a counter that cannot be: every flag set, so nothing to say
        let mut p = packet(65535, 17, ALL_SET | FRESH);
        p[51] = 255;
        p[8..12].copy_from_slice(&i32::MIN.to_le_bytes());
        let crc = dump::crc16(&p[..53]);
        p[53..].copy_from_slice(&crc.to_le_bytes());
        let check = pull(&[raw(0, 0xA5, 0, 1000, &p), found(1)].concat());
        assert!(check.boots[0].verdict.contains("nothing flagged"), "{}", check.boots[0].verdict);
        assert_eq!(check.boots[0].fresh, 1);
    }

    #[test]
    fn whatever_cannot_be_read_is_counted() {
        let mut damaged = rec(1, 0, 1050, ALL_SET).into_bytes();
        damaged[40] = if damaged[40] == b'0' { b'1' } else { b'0' };
        let text = [
            rec(0, 0, 1000, ALL_SET),
            String::from_utf8(damaged).unwrap(),
            String::from("\u{fffd}\u{fffd}line noise\r\n"),
            event(3, 0, 1100, 0x12, 7),
            rec(5, 0, 1150, ALL_SET),
            found(6),
        ]
        .concat();
        let p = pull(&text);
        assert_eq!(p.failed, 1, "slot that failed its CRC");
        assert_eq!(p.missing, 2, "nothing at all came for 2 or 4");
        assert!(!p.complete);
        assert_eq!(p.unread, 1);
        assert_eq!(p.samples, vec!["\u{fffd}\u{fffd}line noise".to_string()]);
        assert_eq!(p.boots[0].other_events, vec![(1100, 0x12)]);
        let text = printed(&p);
        assert!(text.contains("NOT complete"), "{}", text);
        assert!(text.contains("event of kind 0x12 at up=1100 ms, not one this reads"), "{}", text);
        assert!(text.contains("unread lines: 1\n"), "{}", text);
    }

    #[test]
    fn a_tail_of_an_older_printing_is_left_out_like_the_pull_left_it_out() {
        // a slot line cut short before the reset, as in 1521-board13
        let tail = "raw 651 00A2C0 A50000D09300000000\r\n";
        let text = [tail.to_string(), CONSOLE.to_string(), rec(0, 0, 1000, ALL_SET), found(1)].concat();
        let p = pull(&text);
        assert_eq!(p.failed, 0);
        assert!(p.complete);
    }

    #[test]
    fn a_dump_with_no_firmware_line_is_read_whole() {
        let p = pull(&[rec(0, 0, 1000, ALL_SET), found(1)].concat());
        assert_eq!(p.read, 1);
        // and a firmware line with nothing after it does not lose the log
        let p = pull(&[rec(0, 0, 1000, ALL_SET), found(1), String::from("I BOOT 0 c0: start test=flashdump\r\n")].concat());
        assert_eq!(p.read, 1);
    }

    #[test]
    fn the_old_text_format_still_reads() {
        let bytes = packet(4, 13, ALL_SET).map(|b| format!("{:02X}", b)).join(" ");
        let text = format!("record 0 boot 2 ms 2517\n{}\nrecord 1 boot 2 ms 2567\n{}\nfound 2 records, 2 good (0 time anchors), 0 bad, from 1 boots, log ends at 0x70\n", bytes, bytes);
        let p = pull(&text);
        assert_eq!(p.format, "text");
        assert_eq!(p.board(), "board 13");
        assert_eq!(p.boots[0].records, 2);
        assert!(p.boots[0].reset.is_none());
    }

    #[test]
    fn a_wrapped_chip_says_so() {
        let text = [rec(0, 5, 1000, ALL_SET), rec(64, 4, 1000, ALL_SET), found(65)].concat();
        let p = pull(&text);
        assert!(p.wrapped);
        assert!(printed(&p).contains("wrapped: the oldest records were written over"));
    }

    #[test]
    fn boots_missing_from_the_middle_are_named() {
        assert_eq!(boots_not_in_log(&[2, 3, 6]), vec![4, 5]);
        assert!(boots_not_in_log(&[]).is_empty());
    }

    #[test]
    fn a_hardware_watchdog_goes_first_then_drops() {
        let text = [
            event(0, 0, 300, 1, PIN_BOR_SFT),
            rec(1, 0, 1000, ALL_SET),
            event(2, 1, 300, 1, PIN_BOR_SFT),
            event(3, 1, 400, 3, 9),
            rec(4, 1, 1000, ALL_SET),
            event(5, 2, 300, 1, PIN_IWDG),
            rec(6, 2, 1000, ALL_SET),
            found(7),
        ]
        .concat();
        let p = pull(&text);
        assert_eq!(p.boots.iter().map(|b| b.n).collect::<Vec<u32>>(), vec![2, 1, 0]);
        assert!(p.boots[0].verdict.contains("hardware watchdog reset"), "{}", p.boots[0].verdict);
        assert!(p.boots[0].verdict.contains("reset PIN+IWDG"), "{}", p.boots[0].verdict);
        assert!(p.line().contains("1 reset hardware watchdog"), "{}", p.line());
    }

    #[test]
    fn a_chip_with_two_boards_says_whose_each_boot_was() {
        let text = [raw(0, 0xA5, 0, 1000, &packet(0, 14, ALL_SET)), raw(1, 0xA5, 1, 1000, &packet(0, 15, ALL_SET)), found(2)].concat();
        let p = pull(&text);
        assert_eq!(p.board(), "board 14,15");
        assert!(p.boots.iter().any(|b| b.verdict.starts_with("boot 1 (board 15), ")));
    }

    #[test]
    fn console_lines_are_found_however_the_port_mangled_them() {
        let c = console_lines("I BOOT 1 c0: settings from=block csid=17");
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].level, c[0].tag, c[0].ms, c[0].word), ('I', "BOOT", 1, "settings"));
        assert_eq!(c[0].field("csid"), Some("17"));
        assert!(!c[0].cut);
        // real, off a 2026-10 pull: the reset landed in the middle of a line
        let c = console_lines("I BOOT 0 c0: settings from=block csid=W GPS 14508 c2: read ok=0 reason=nofix");
        assert_eq!(c.len(), 2);
        assert!(c[0].cut);
        assert_eq!((c[1].tag, c[1].ms), ("GPS", 14508));
        assert!(console_lines("\u{fffd}\u{fffd}!W ENV 9 c1: recover step=retry").len() == 1);
        for no in ["HB,9965,0,0", "I saw it", "I BOOT x c0: start", "I BOOT 1 0: start", "raw 0 000000 A5"] {
            assert!(console_lines(no).is_empty(), "{}", no);
        }
    }

    fn log_line(t: u64, rx: &str, text: &str) -> String {
        format!("{}\t{}\t{}\n", t, rx, text)
    }

    fn pkt(counter: u16, csid: u8, rssi: f64, validity: u8) -> String {
        format!("PKT,55,{},{},9.5,100", decode::bytes_to_hex(&packet(counter, csid, validity)), rssi)
    }

    fn log_text() -> String {
        [
            String::from("# descent-ground log v1 started 2026-10-17T18:34:17.099Z\n"),
            log_line(1000, "rx1", "#DG,RX,v1,id=9E8FF4,f=915.0,bw=125.0,sf=12,cr=7,sync=0x12,pre=8"),
            log_line(2000, "rx1", &pkt(1, 13, -90.0, ALL_SET)),
            log_line(2001, "rx2", &pkt(1, 13, -70.0, ALL_SET)),
            log_line(3000, "rx1", &pkt(4, 13, -110.0, ALL_SET & !GPS)),
            log_line(4000, "rx1", &pkt(0, 13, -100.5, ALL_SET)),
            log_line(5000, "rx2", "ERR,-7,-94.0,2.75"),
            log_line(6000, "rx1", "garbage off the port"),
            String::from("a row with no columns\n"),
            log_line(7000, "rx1", &format!("PKT,55,{},-90,9,1", "00".repeat(55))),
        ]
        .concat()
    }

    #[test]
    fn a_ground_log_says_what_each_board_did_and_who_heard_it() {
        let text = log_text();
        let l = check_log("1434-mark-ground.log", "x", &text, text.len());
        assert_eq!(l.boards.len(), 1);
        let b = &l.boards[0];
        assert_eq!(b.csid, 13);
        assert_eq!(b.packets, 3, "the second receiver's copy is the same packet");
        assert_eq!(b.missed, 2);
        assert_eq!(b.restarts, 1);
        assert_eq!(b.rssi, Some((-110.0, -70.0)));
        assert_eq!(b.last_t, 4000.0);
        assert_eq!(b.heard.iter().map(|h| h.rx.as_str()).collect::<Vec<&str>>(), vec!["rx1", "rx2"]);
        assert_eq!(b.clear[4], 1);
        assert_eq!(l.duplicates, 1);
        assert_eq!(l.bad_crc, 1);
        assert_eq!(l.receivers[0].id.as_deref(), Some("9E8FF4"));
        assert_eq!(l.receivers.iter().find(|r| r.key == "rx2").unwrap().radio_errors, 1);
        assert_eq!(l.unread, 2);
        assert!(l.samples.iter().any(|s| s == "garbage off the port"), "{:?}", l.samples);
        assert!(l.samples.iter().any(|s| s == "a row with no columns"), "{:?}", l.samples);
        let mut out = String::new();
        l.text(&mut out);
        assert!(out.contains("console: none in this log"), "{}", out);
    }

    #[test]
    fn an_empty_log_says_so() {
        let text = "# descent-ground log v1 started 2026-10-31T13:31:00.000Z\n";
        let l = check_log("0931-ground.log", "x", text, text.len());
        let mut out = String::new();
        l.text(&mut out);
        assert!(out.contains("no lines in this log"), "{}", out);
        assert_eq!(l.unread, 0);
    }

    #[test]
    fn a_console_in_a_ground_log_gives_recovery_bus_and_phase() {
        let text = [
            String::from("# descent-ground log v1 started 2026-10-29T23:19:05.014Z\n"),
            log_line(1, "rx1", "I BOOT 1 c0: settings from=block csid=15 module=hp sf=7 dbm=14 fixedms=0,1000,1000,0 soc=0,0,0,0"),
            log_line(2, "rx1", "I RST 6 c0: cause csr=0x24000000 lpwr=0 wwdg=0 iwdg=1 sft=0 bor=0 pin=1 obl=0 rfilar=0"),
            log_line(3, "rx1", "W BUS 853 c0: held sda=0 scl=0 part=imu addr=0x4A when=after"),
            log_line(4, "rx1", "W BUS 1230 c0: clear sda=0 scl=1 pulses=0 ok=1 n=1"),
            log_line(5, "rx1", "W BUS 1688 c2: clear sda=1 scl=0 pulses=0 ok=0 sclheld=0 retryms=1000 n=2"),
            log_line(6, "rx1", "W ENV 3337 c9: recover step=retry reason=notready"),
            log_line(7, "rx1", "W ENV 9661 c35: recover step=reset how=start attempt=1 reason=notready ok=0 ms=3"),
            log_line(8, "rx1", "W ENV 10858 c40: recover step=aside reason=notready backoffms=1000 attempts=1"),
            log_line(9, "rx1", "I ENV 12000 c41: recover step=ok downms=8663 attempts=1"),
            log_line(10, "rx1", "I PHASE 20000 c50: log air=1"),
            log_line(11, "rx1", "I PHASE 30000 c60: log air=0"),
            log_line(12, "rx1", "I CYC 40000 c70: packet ctr=9 valid=0x7F fresh=1 gate=1 imuready=1 imusleep=0 phase=0 rec=88 drop=12"),
            log_line(13, "rx1", "W FLASH 45000 c75: stopped records=88 dropped=12"),
            log_line(14, "rx1", "E IMU 45100 c76: init ok=0"),
            log_line(15, "rx1", "I BOOT 1 c0: start log=4"),
        ]
        .concat();
        let l = check_log("1919-ground.log", "x", &text, text.len());
        assert_eq!(l.unread, 0);
        assert_eq!(l.consoles.len(), 2, "uptime going down is a new run");
        let s = &l.consoles[0];
        assert_eq!(s.reset.as_deref(), Some("PIN+IWDG"));
        assert!(s.watchdog);
        assert_eq!(s.watch["BUS clear"], 1);
        assert_eq!(s.watch["BUS clear failed"], 1);
        assert_eq!(s.watch["BUS held imu"], 1);
        assert_eq!(s.watch["ENV retry"], 1);
        assert_eq!(s.watch["ENV reset"], 1);
        assert_eq!(s.watch["ENV reset failed"], 1);
        assert_eq!(s.watch["ENV set aside"], 1);
        assert_eq!(s.watch["ENV recovered"], 1);
        assert_eq!(s.longest_down["ENV"], 8663);
        assert_eq!((s.air_ms, s.ground_ms), (10_000, 15_100));
        assert_eq!(s.drop_last.as_deref(), Some("12"));
        assert_eq!(s.log_stopped.as_deref(), Some("stopped records=88 dropped=12"));
        assert_eq!(s.others["E IMU init"], 1);
        assert!(l.line().contains("hardware watchdog on a console"), "{}", l.line());
        let mut out = String::new();
        l.text(&mut out);
        assert!(out.contains("HARDWARE WATCHDOG"), "{}", out);
        assert!(out.contains("settings from=block csid=15"), "{}", out);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dg-check-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_pull(dir: &Path, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("dump.txt"), text).unwrap();
        std::fs::write(dir.join("board.txt"), "20393146324550100033003A\n").unwrap();
        std::fs::write(dir.join("report.json"), "{}").unwrap();
    }

    #[test]
    fn a_log_made_from_a_pull_reads_without_its_dump_and_says_what_it_lacks() {
        let d = scratch("flash-log");
        let parsed = dump::parse_what_arrived(&pull_text());
        let log = dump::to_log(&parsed, "/dev/ttyACM0");
        let alone = d.join("alone.log");
        std::fs::write(&alone, &log).unwrap();
        let Item::Pull(p) = check_file(&alone).unwrap() else { panic!("not read as a pull") };
        assert!(p.from_log);
        assert_eq!(p.board(), "board 17");
        assert_eq!((p.boots[0].first_ms, p.boots[0].last_ms, p.boots[0].records), (4122, 7222, 4));
        assert_eq!(p.boots[0].space.gaps.len(), 1);
        assert!(p.boots[0].reset.is_none());
        assert_eq!(p.unread, 0);
        assert!(printed(&p).contains("dump.txt has those"));

        // beside its dump.txt, the dump is what gets read
        let beside = d.join("1945-board17");
        write_pull(&beside, &pull_text());
        std::fs::write(beside.join("ground.log"), &log).unwrap();
        let Item::Pull(p) = check_file(&beside.join("ground.log")).unwrap() else { panic!("not read as a pull") };
        assert!(!p.from_log);
        assert_eq!(p.boots[0].dropped, 52);
        assert_eq!(p.uid.as_deref(), Some("20393146324550100033003A"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_gets_one_line_each_worst_first_then_the_detail() {
        let d = scratch("folder");
        let day = d.join("2026-11-01");
        write_pull(&day.join("pulls/1000-board9"), &[rec(0, 0, 1000, ALL_SET), found(1)].concat());
        write_pull(&day.join("pulls/1010-board17"), &pull_text());
        std::fs::create_dir_all(day.join("logs")).unwrap();
        std::fs::write(day.join("logs/0900-ground.log"), log_text()).unwrap();
        std::fs::write(day.join("notes.bin"), [0u8, 1, 2]).unwrap();
        let c = check_path(&d).unwrap();
        assert_eq!(c.items.len(), 3);
        assert_eq!(c.not_read, vec![day.join("notes.bin").display().to_string()]);
        let text = c.text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with("3 files, worst first"), "{}", text);
        assert!(lines[1].contains("1010-board17"), "the one with drops first: {}", text);
        assert!(lines[2].contains("1000-board9"), "{}", text);
        assert!(lines[3].starts_with("  log  0900-ground.log"), "{}", text);
        assert!(text.contains("not read: 1 file\n"), "{}", text);
        let json = c.json();
        assert!(json.contains("\"kind\":\"pull\""), "{}", json);
        assert!(json.contains("\"kind\":\"log\""), "{}", json);
        assert!(json.contains("\"reset\":\"PIN+BOR+SFT\""), "{}", json);
        assert!(json.contains("\"dropped\":52"), "{}", json);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pull_with_nothing_in_it_says_so() {
        let text = "I BOOT 0 c0: st\u{fffd}I BOOT 0 c0: start test=flashdum";
        let p = pull(text);
        assert_eq!(p.board(), "board unknown");
        assert!(p.boots.is_empty());
        assert!(printed(&p).contains("no log in this file"));
    }

    #[test]
    fn a_path_that_is_not_there_is_an_error_and_a_bad_flag_is_refused() {
        assert_eq!(run(&[String::from("/nowhere/at/all")]), 1);
        assert_eq!(run(&[String::from("--nope"), String::from("x")]), 1);
        assert_eq!(run(&[]), 1);
        assert_eq!(run(&[String::from("--help")]), 0);
    }
}
