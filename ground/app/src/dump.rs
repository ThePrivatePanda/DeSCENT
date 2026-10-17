// What flash_dump prints, turned into a log the dashboard already replays.
//
// The board prints one line for each 64 byte slot of its log, as the slot sits on the chip:
//
//   raw 12 000300 A5060040E201 ... 128 hex characters, no spaces ...
//
// read off the sketch's format string, "raw %lu %06lX ": the index is decimal and the flash
// address is hex. The slot is laid out in the flight repo's FlashLog.h: byte 0 the type,
// 1-2 the boot count, 3-6 ms since boot, 7-61 a 55 byte packet or a time anchor, 62-63 a
// CRC16 over 0-61 with the low byte first. Everything little endian. The boot and the
// uptime are inside that check, so a digit lost on the wire costs one slot instead of
// turning into a timestamp.
//
// Dumps taken before 2026-10-04 have those fields outside any check. They print one line
// naming a record and one line of hex for its payload:
//
//   record 0 boot 0 ms 2517
//   00 00 00 ... 55 bytes ...
//
// and, when it has ever had a GPS fix, anchors tying one boot's uptime to real time:
//
//   time 12 boot 0 ms 41230 epoch 1790258783 nano 216000000 bits 07 accns 25
//
// read off the board's own format string, which is
//   "time %lu boot %u ms %lu epoch %lu nano %ld bits %02X accns %lu"
// so nano is signed and bits is hex. Fields are found by their labels rather than by
// position, and only boot, ms, epoch and nano are used.
//
// Both end with a line saying how many it found, which is how we know a dump finished
// rather than stopped halfway.
//
// Everything else on that port is console chatter and is ignored.
use crate::json::esc;
use std::collections::BTreeSet;

pub const PACKET_BYTES: usize = 55;
// Where the board's id sits in a packet: one byte, after the counter. This is the one
// thing in this file that depends on the packet layout, which lives in web/js/packet.js
// and nowhere else. Nothing else of a packet is read here. test/packet.test.js pins
// the offset, so a layout change fails there and points here.
const PACKET_CSID_AT: usize = 50;
const SLOT_BYTES: usize = 64;
const SLOT_CRC_AT: usize = 62;
const SLOT_PACKET: u8 = 0xA5;
const SLOT_TIME: u8 = 0xA6;
const BOOT_GAP_MS: u64 = 1000;
// How far one anchor may sit from the middle of the anchors written around it and still be
// believed. Picked, not measured. A digit lost from an uptime of more than a few seconds
// moves an anchor further than this, and a clock drifting by a percent moves neighbours
// 10 s apart by a tenth of a second.
const ANCHOR_SPREAD_MS: i128 = 2000;
const ANCHOR_NEIGHBOURS: usize = 4;   // either side; anchors are written every 10 s

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub n: u64,
    // Where the slot is on the chip. The old format does not print it. Nothing reads it
    // yet; it is kept so a row can be traced back to the chip.
    pub addr: Option<u32>,
    pub boot: u32,
    pub ms: u64,
    pub hex: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub boot: u32,
    pub ms: u64,
    pub unix_ms: u64,
}

#[derive(Debug, Default)]
pub struct Dump {
    pub records: Vec<Record>,
    pub anchors: Vec<Anchor>,
    pub found: Option<usize>,   // the board's own count, when it said
    pub good: Option<usize>,    // how many of those passed their check on the board
    pub bad: Option<usize>,
    pub raw: bool,              // printed as whole slots, not as record and time lines
    pub failed: usize,          // raw lines that never gave a slot that passed
    pub missing: usize,         // indices nothing at all was seen of
    pub ignored_old: usize,     // record and time lines left out of a printing of whole slots
}

impl Dump {
    // The board got to its found line. Whether every slot it counted also arrived here is
    // a different question, and the report answers that one.
    pub fn complete(&self) -> bool {
        self.found.is_some()
    }
    pub fn boots(&self) -> Vec<u32> {
        let mut b: Vec<u32> = self.records.iter().map(|r| r.boot).collect();
        b.sort_unstable();
        b.dedup();
        b
    }
}

fn after<'a>(parts: &[&'a str], key: &str) -> Option<&'a str> {
    parts.iter().position(|p| *p == key).and_then(|i| parts.get(i + 1)).copied()
}

// The found line puts its numbers in front of their words: "6627 good", "0 bad,".
fn before<'a>(parts: &[&'a str], key: &str) -> Option<&'a str> {
    let at = parts.iter().position(|p| p.trim_end_matches(',') == key)?;
    parts.get(at.checked_sub(1)?).copied()
}

// 55 bytes as the board prints them: space separated, uppercase, one line.
fn hex_line(line: &str) -> Option<String> {
    let mut out = String::with_capacity(PACKET_BYTES * 2);
    let mut seen = 0usize;
    for tok in line.split_whitespace() {
        if tok.len() != 2 || !tok.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        out.push_str(&tok.to_ascii_uppercase());
        seen += 1;
    }
    if seen == PACKET_BYTES { Some(out) } else { None }
}

// CRC-16/CCITT-FALSE, the same as calculateCRC16 in the flight repo's Telemetry.cpp:
// starts at 0xFFFF, polynomial 0x1021, high bit first, nothing done to the result.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for b in bytes {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

// The board prints nano with %ld: it is signed and can be negative, so the fraction is
// subtracted as often as added. Reading it unsigned turns a negative into zero and quietly
// loses up to a second, the wrong way.
fn unix_ms(epoch: u32, nano: i64) -> u64 {
    ((epoch as i64) * 1000 + nano / 1_000_000).max(0) as u64
}

enum Slot {
    Packet { boot: u32, ms: u64, hex: String },
    Time(Anchor),
}

// None for anything decodeRecord in FlashLog.cpp would also turn down, or that did not
// arrive as 64 whole bytes.
fn slot(text: &str) -> Option<Slot> {
    if text.len() != SLOT_BYTES * 2 {
        return None;
    }
    let mut b = [0u8; SLOT_BYTES];
    for (i, pair) in text.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        b[i] = (hi * 16 + lo) as u8;
    }
    let crc = u16::from_le_bytes([b[SLOT_CRC_AT], b[SLOT_CRC_AT + 1]]);
    if crc != crc16(&b[..SLOT_CRC_AT]) {
        return None;
    }
    let boot = u16::from_le_bytes([b[1], b[2]]) as u32;
    let ms = u32::from_le_bytes([b[3], b[4], b[5], b[6]]) as u64;
    match b[0] {
        SLOT_PACKET => {
            // Every character was a hex digit, so cutting by bytes cannot split one.
            let hex = text[7 * 2..(7 + PACKET_BYTES) * 2].to_ascii_uppercase();
            Some(Slot::Packet { boot, ms, hex })
        }
        SLOT_TIME => {
            let epoch = u32::from_le_bytes([b[7], b[8], b[9], b[10]]);
            let nano = i32::from_le_bytes([b[11], b[12], b[13], b[14]]);
            Some(Slot::Time(Anchor { boot, ms, unix_ms: unix_ms(epoch, nano as i64) }))
        }
        _ => None,
    }
}

// One printing while it is being read. The sets are only needed to count at the end.
#[derive(Default)]
struct Printing {
    dump: Dump,
    seen: BTreeSet<u64>,     // every index a line arrived for, readable or not
    kept: BTreeSet<u64>,
    failed: BTreeSet<u64>,
    blind: usize,            // raw lines so damaged that the index could not be read either
    // What record and time lines gave, held apart until the printing is over. One
    // printing is one format: the sketch prints whole slots or it prints these, never
    // both. So when a printing has slots in it, these are the end of some earlier
    // printing that was still on the port, from this board or from another, and are
    // counted and left out. It cannot be settled a line at a time, because the old
    // lines come first.
    old_records: Vec<Record>,
    old_anchors: Vec<Anchor>,
    old_seen: BTreeSet<u64>,
}

impl Printing {
    // raw <index> <address> <128 hex characters>
    fn raw(&mut self, parts: &[&str]) {
        self.dump.raw = true;
        let n = if parts.len() == 4 { parts[1].parse::<u64>().ok() } else { None };
        let Some(n) = n else {
            self.blind += 1;
            return;
        };
        self.seen.insert(n);
        // The index and address are outside the slot's check. An address that does not
        // read is no reason to drop a slot that passed.
        let addr = u32::from_str_radix(parts[2], 16).ok();
        match slot(parts[3]) {
            // A second line for an index already read is left alone: the first one passed.
            Some(_) if self.kept.contains(&n) => {}
            Some(Slot::Packet { boot, ms, hex }) => {
                self.kept.insert(n);
                self.dump.records.push(Record { n, addr, boot, ms, hex });
            }
            Some(Slot::Time(anchor)) => {
                self.kept.insert(n);
                self.dump.anchors.push(anchor);
            }
            None => {
                self.failed.insert(n);
            }
        }
    }

    fn close(mut self) -> Dump {
        if self.dump.raw {
            self.dump.ignored_old = self.old_records.len() + self.old_anchors.len();
        } else {
            self.dump.records.append(&mut self.old_records);
            self.dump.anchors.append(&mut self.old_anchors);
            self.seen.append(&mut self.old_seen);
        }
        self.dump.failed = self.blind + self.failed.difference(&self.kept).count();
        // The board counts every slot up to the first erased one, so its count is also one
        // past the last index. With no count, the highest index seen has to do.
        let top = self.seen.iter().next_back().map(|n| n + 1).unwrap_or(0);
        let expected = top.max(self.dump.found.unwrap_or(0) as u64);
        self.dump.missing = (expected - self.seen.len() as u64) as usize;
        self.dump
    }
}

fn printings(text: &str) -> Vec<Dump> {
    let mut runs: Vec<Printing> = vec![Printing::default()];
    let mut pending: Option<Record> = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rec) = pending.take() {
            if let Some(hex) = hex_line(line) {
                let run = runs.last_mut().unwrap();
                run.old_seen.insert(rec.n);
                run.old_records.push(Record { hex, ..rec });
                continue;
            }
            // No payload followed, so that record header was noise or a truncated line.
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("raw") => runs.last_mut().unwrap().raw(&parts),
            Some("record") => {
                let n = parts.get(1).and_then(|v| v.parse().ok());
                let boot = after(&parts, "boot").and_then(|v| v.parse().ok());
                let ms = after(&parts, "ms").and_then(|v| v.parse().ok());
                if let (Some(n), Some(boot), Some(ms)) = (n, boot, ms) {
                    pending = Some(Record { n, addr: None, boot, ms, hex: String::new() });
                }
            }
            Some("time") => {
                let boot = after(&parts, "boot").and_then(|v| v.parse::<u32>().ok());
                let ms = after(&parts, "ms").and_then(|v| v.parse::<u64>().ok());
                // 32 bits on the chip. Anything longer is two numbers run together.
                let epoch = after(&parts, "epoch").and_then(|v| v.parse::<u32>().ok());
                let nano = after(&parts, "nano").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                if let (Some(boot), Some(ms), Some(epoch)) = (boot, ms, epoch) {
                    let run = runs.last_mut().unwrap();
                    // An anchor takes an index of its own, between the records.
                    if let Some(n) = parts.get(1).and_then(|v| v.parse::<u64>().ok()) {
                        run.old_seen.insert(n);
                    }
                    run.old_anchors.push(Anchor { boot, ms, unix_ms: unix_ms(epoch, nano) });
                }
            }
            Some("found") => {
                let run = runs.last_mut().unwrap();
                run.dump.found = parts.get(1).and_then(|v| v.parse().ok());
                run.dump.good = before(&parts, "good").and_then(|v| v.parse().ok());
                run.dump.bad = before(&parts, "bad").and_then(|v| v.parse().ok());
                runs.push(Printing::default());   // anything after this belongs to the next printing
            }
            _ => {}
        }
    }
    runs.into_iter().map(Printing::close).collect()
}

// The board reprints its whole log on demand, so a port opened mid-printing catches the
// tail of an older one. Everything is read, then the last run that reached its "found"
// line is the one taken. Deciding afterwards rather than while reading is what keeps this
// from depending on when the port happened to open.
pub fn parse(text: &str) -> Dump {
    printings(text)
        .into_iter()
        .filter(|r| r.complete() && !r.records.is_empty())
        .last()
        .unwrap_or_default()
}

// The same choice, except that when no printing finished the last one to get anywhere is
// handed over as it stands. For a pull that was cut short: part of a flight log is worth
// having, as long as it is not passed off as all of it, and found stays None to say so.
pub fn parse_what_arrived(text: &str) -> Dump {
    let whole = parse(text);
    if !whole.records.is_empty() {
        return whole;
    }
    printings(text).into_iter().filter(|r| !r.records.is_empty()).last().unwrap_or_default()
}

// What uptime 0 of one boot was in epoch ms, and which anchors said so.
pub struct Clock {
    pub boot: u32,
    pub uptime0_ms: i64,
    pub anchor: Option<Anchor>,   // the earliest of the ones believed; None for an undated boot
    pub said: Vec<(u64, i64)>,    // each believed anchor's uptime and the uptime 0 it gives, in uptime order
    pub used: usize,
    pub rejected: usize,
}

// Garbage on a text line can be any size, and the sums are done wide so that it cannot
// overflow on the way to being thrown out.
fn narrow(v: i128) -> i64 {
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

// Each anchor of a boot says when uptime 0 was: its real time less its uptime. An anchor is
// believed when it agrees with the ones written around it, so one damaged anchor costs
// nothing, where taking the earliest lets it date the whole boot. It is judged against its
// neighbours and not against the boot as a whole because the board's clock is not GPS's:
// if it runs a fraction of a percent off, anchors an hour apart honestly disagree by tens
// of seconds, and one figure for the boot would throw most of them out. Not measured on a
// board yet; boot 6 of the 2026-09-29 pull overran the start of boot 7 by about 52 s in
// 1 h 56 min, which drift of 0.75% would explain and so would one damaged anchor.
// An anchor can arrive at the end and still date everything before it, which is the whole
// reason for anchoring rather than stamping each record.
//
// A boot with no anchor that can be believed has no real time at all. Those are laid end to
// end from epoch 0, so the axis is unmistakably not a real date: a plausible-looking wrong
// time invites someone to read it as measured, and 1970 does not. Two anchors that disagree
// end up here too, because nothing says which one is wrong.
pub fn clocks(dump: &Dump) -> Vec<Clock> {
    let mut out = Vec::new();
    let mut base: i64 = 0;

    for boot in dump.boots() {
        let mut mine: Vec<&Anchor> = dump.anchors.iter().filter(|a| a.boot == boot).collect();
        mine.sort_by_key(|a| a.ms);
        let said: Vec<i128> = mine.iter().map(|a| a.unix_ms as i128 - a.ms as i128).collect();
        let believed: Vec<(&Anchor, i64)> = mine
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let from = i.saturating_sub(ANCHOR_NEIGHBOURS);
                let to = (i + ANCHOR_NEIGHBOURS + 1).min(said.len());
                (said[*i] - middle(&said[from..to])).abs() <= ANCHOR_SPREAD_MS
            })
            .map(|(i, a)| (*a, narrow(said[i])))
            .collect();
        if let Some((first, uptime0)) = believed.first() {
            out.push(Clock {
                boot,
                uptime0_ms: *uptime0,
                anchor: Some((*first).clone()),
                said: believed.iter().map(|(a, o)| (a.ms, *o)).collect(),
                used: believed.len(),
                rejected: mine.len() - believed.len(),
            });
            continue;
        }
        let uptimes = dump.records.iter().filter(|r| r.boot == boot).map(|r| r.ms);
        let first = uptimes.clone().min().unwrap_or(0);
        let last = uptimes.max().unwrap_or(0);
        out.push(Clock {
            boot,
            uptime0_ms: narrow(base as i128 - first as i128),
            anchor: None,
            said: Vec::new(),
            used: 0,
            rejected: mine.len(),
        });
        base = narrow(base as i128 + (last - first) as i128 + BOOT_GAP_MS as i128);
    }
    out
}

// The middle of a few values; of an even number, halfway between the middle two.
fn middle(values: &[i128]) -> i128 {
    let mut v = values.to_vec();
    v.sort_unstable();
    let half = v.len() / 2;
    if v.len() % 2 == 1 { v[half] } else { (v[half - 1] + v[half]) / 2 }
}

// When uptime 0 was, as seen from one moment of the boot: what the anchors either side of
// that moment say, in proportion to how near each is. Before the first anchor and after
// the last there is only the one to go on. Sliding between them rather than jumping at the
// halfway point keeps two records 50 ms apart from swapping places.
fn uptime0_at(clock: &Clock, ms: u64) -> i128 {
    let said = &clock.said;
    let Some(next) = said.iter().position(|(at, _)| *at >= ms) else {
        return said.last().map(|(_, o)| *o as i128).unwrap_or(clock.uptime0_ms as i128);
    };
    if next == 0 {
        return said[0].1 as i128;
    }
    let (a_ms, a) = said[next - 1];
    let (b_ms, b) = said[next];
    if b_ms == a_ms {
        return a as i128;
    }
    a as i128 + (b as i128 - a as i128) * (ms - a_ms) as i128 / (b_ms - a_ms) as i128
}

// One boot's records in the order the chip was written, each with its time.
fn rows<'a>(dump: &'a Dump, clock: &Clock) -> Vec<(u64, &'a Record)> {
    let mut mine: Vec<&Record> = dump.records.iter().filter(|r| r.boot == clock.boot).collect();
    mine.sort_by_key(|r| r.n);
    mine.into_iter()
        .map(|r| {
            let t = (uptime0_at(clock, r.ms) + r.ms as i128).clamp(0, u64::MAX as i128);
            (t as u64, r)
        })
        .collect()
}

// Every record with its time, in one list. Boot by boot, not sorted by time: boots that
// were dated wrongly can overlap, and each one still has to come out in one piece. Only
// the tests want the one list; to_log goes a boot at a time.
#[cfg(test)]
pub fn date(dump: &Dump) -> (Vec<(u64, &Record)>, &'static str) {
    let clocks = clocks(dump);
    let anchored = clocks.iter().any(|c| c.anchor.is_some());
    let dated = clocks.iter().flat_map(|c| rows(dump, c)).collect();
    (dated, if anchored { "anchor" } else { "none" })
}

// 2026-09-26T16:45:50.000Z. An anchor's epoch is 32 bits, so this is always before 2107
// and inside what humantime will print.
fn utc(unix_ms: u64) -> String {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms);
    humantime::format_rfc3339_millis(at).to_string()
}

// The dashboard's own log format, so the file opens with Open log like any other. Each
// boot gets one SRC line and one BOOT line, then its rows. The page splits the SRC line on
// commas, so no value in it may hold one. A row carries its own uptime and its index on
// the chip after the three radio fields a flash record does not have: a boot's dates slide
// with its anchors, so uptime cannot be worked back out of them exactly.
pub fn to_log(dump: &Dump, origin: &str) -> String {
    let mut s = String::from("# descent-ground log v1 from flash\n");

    for clock in &clocks(dump) {
        let rows = rows(dump, clock);
        let Some((t, _)) = rows.first() else { continue };
        let rx = format!("flash-boot{}", clock.boot);
        let anchor_bits = match &clock.anchor {
            Some(a) => format!(
                ",anchor_uptime_ms={},anchor_unix_ms={},anchor_utc={},anchors={},anchors_rejected={}",
                a.ms,
                a.unix_ms,
                utc(a.unix_ms),
                clock.used,
                clock.rejected
            ),
            None => String::new(),
        };
        s.push_str(&format!(
            "{}\t{}\t#DG,SRC,v1,kind=flash,from={},boot={},packets={},time_source={},uptime0_ms={}{}\n",
            t,
            rx,
            esc(origin).replace(',', "_"),
            clock.boot,
            rows.len(),
            if clock.anchor.is_some() { "anchor" } else { "none" },
            clock.uptime0_ms,
            anchor_bits
        ));
        s.push_str(&format!("{}\t{}\t#DG,BOOT,v1,n={}\n", t, rx, clock.boot));
        for (t, r) in &rows {
            s.push_str(&format!("{}\t{}\tPKT,55,{},,,,up={},n={}\n", t, rx, r.hex, r.ms, r.n));
        }
    }
    s
}

// The board counts milliseconds off its internal oscillator, which nothing trims, so over a
// boot its uptime pulls away from GPS. Each anchor says when uptime 0 was, and a clock that
// runs fast makes later anchors put it earlier, so the slope of a straight line through
// them is the rate. Parts per million, positive when the board counts too many. A minute of
// anchors at least: over less than that the receiver's own millisecond jitter is most of it.
const DRIFT_SPAN_MS: u64 = 60_000;

fn clock_fast_ppm(said: &[(u64, i64)]) -> Option<i64> {
    let (first, last) = (said.first()?, said.last()?);
    if said.len() < 3 || last.0 - first.0 < DRIFT_SPAN_MS {
        return None;
    }
    let n = said.len() as f64;
    let at = said.iter().map(|(ms, _)| (*ms - first.0) as f64);
    let was = said.iter().map(|(_, o)| (*o - first.1) as f64);
    let mean_at = at.clone().sum::<f64>() / n;
    let mean_was = was.clone().sum::<f64>() / n;
    let over: f64 = at.clone().map(|x| (x - mean_at) * (x - mean_at)).sum();
    let under: f64 = at.zip(was).map(|(x, y)| (x - mean_at) * (y - mean_was)).sum();
    if over == 0.0 {
        return None;
    }
    // The line gives ms of GPS lost per ms of uptime; the board's excess over GPS is that
    // over what is left.
    let lost = -under / over;
    Some((lost / (1.0 - lost) * 1e6).round() as i64)
}

pub struct BootReport {
    pub n: u32,
    pub records: usize,
    pub first_ms: u64,   // uptime of its first and last record, on the boot's own clock
    pub last_ms: u64,
    pub anchored: bool,
    pub uptime0_ms: i64,
    pub anchors_used: usize,
    pub anchors_rejected: usize,
    pub clock_fast_ppm: Option<i64>,   // how far the board's clock runs ahead of GPS; None when there is too little to tell
    pub csids: Vec<u8>,   // the board ids its packets carry, ascending
}

// What was read set against what the board said it has, for whoever has to decide whether
// to pull again.
pub struct Report {
    pub raw: bool,
    pub found: Option<usize>,
    pub board_good: Option<usize>,
    pub board_bad: Option<usize>,
    pub read: usize,           // slots that passed here, anchors included, as the board counts them
    pub failed_check: usize,
    pub missing: usize,
    // Old-format records and anchors found among whole slots and left out. They are the
    // tail of some earlier printing, and not from this dump whatever they say.
    pub ignored_old_format: usize,
    pub complete: bool,
    pub longest_reader_gap_ms: u64,
    pub bytes: usize,
    // Every board id the packets carry, ascending. More than one is not a fault: a chip
    // keeps the boots from before a board was given a new id.
    pub csids: Vec<u8>,
    pub boots: Vec<BootReport>,
}

// The distinct board ids a set of records carries. A raw record is only here because its
// slot passed its check, so a damaged one names nobody. The older text format has no
// check on a record, and there every record that was printed counts.
fn csids<'a>(records: impl Iterator<Item = &'a Record>) -> Vec<u8> {
    let ids: BTreeSet<u8> = records
        .filter_map(|r| r.hex.get(PACKET_CSID_AT * 2..PACKET_CSID_AT * 2 + 2))
        .filter_map(|pair| u8::from_str_radix(pair, 16).ok())
        .collect();
    ids.into_iter().collect()
}

fn numbers(ids: &[u8]) -> String {
    ids.iter().map(|n| n.to_string()).collect::<Vec<String>>().join(",")
}

pub fn report(dump: &Dump, bytes: usize, longest_reader_gap_ms: u64) -> Report {
    let read = dump.records.len() + dump.anchors.len();
    let boots = clocks(dump)
        .iter()
        .map(|c| {
            let uptimes = dump.records.iter().filter(|r| r.boot == c.boot).map(|r| r.ms);
            BootReport {
                n: c.boot,
                records: uptimes.clone().count(),
                first_ms: uptimes.clone().min().unwrap_or(0),
                last_ms: uptimes.max().unwrap_or(0),
                anchored: c.anchor.is_some(),
                uptime0_ms: c.uptime0_ms,
                anchors_used: c.used,
                anchors_rejected: c.rejected,
                clock_fast_ppm: clock_fast_ppm(&c.said),
                csids: csids(dump.records.iter().filter(|r| r.boot == c.boot)),
            }
        })
        .collect();
    Report {
        raw: dump.raw,
        found: dump.found,
        board_good: dump.good,
        board_bad: dump.bad,
        read,
        failed_check: dump.failed,
        missing: dump.missing,
        ignored_old_format: dump.ignored_old,
        // A slot the board itself called bad fails here as well and is not a loss on the
        // way, which is why this is measured against its good count and not against found.
        complete: dump.found.is_some() && Some(read) == dump.good.or(dump.found) && dump.missing == 0,
        longest_reader_gap_ms,
        bytes,
        csids: csids(dump.records.iter()),
        boots,
    }
}

fn or_null(v: Option<usize>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => String::from("null"),
    }
}

impl Report {
    pub fn json(&self) -> String {
        let boots: Vec<String> = self
            .boots
            .iter()
            .map(|b| {
                format!(
                    "{{\"n\":{},\"records\":{},\"firstMs\":{},\"lastMs\":{},\"timeSource\":\"{}\",\"uptime0Ms\":{},\"anchorsUsed\":{},\"anchorsRejected\":{},\"clockFastPpm\":{},\"csids\":[{}]}}",
                    b.n,
                    b.records,
                    b.first_ms,
                    b.last_ms,
                    if b.anchored { "anchor" } else { "none" },
                    b.uptime0_ms,
                    b.anchors_used,
                    b.anchors_rejected,
                    match b.clock_fast_ppm {
                        Some(ppm) => ppm.to_string(),
                        None => String::from("null"),
                    },
                    numbers(&b.csids)
                )
            })
            .collect();
        format!(
            "{{\"format\":\"{}\",\"found\":{},\"boardGood\":{},\"boardBad\":{},\"read\":{},\"failedCheck\":{},\"missing\":{},\"ignoredOldFormat\":{},\"complete\":{},\"longestReaderGapMs\":{},\"bytes\":{},\"csids\":[{}],\"boots\":[{}]}}",
            if self.raw { "raw" } else { "text" },
            or_null(self.found),
            or_null(self.board_good),
            or_null(self.board_bad),
            self.read,
            self.failed_check,
            self.missing,
            self.ignored_old_format,
            self.complete,
            self.longest_reader_gap_ms,
            self.bytes,
            numbers(&self.csids),
            boots.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
I TX 212361 c381: done ok=1 code=0 ms=465
record 0 boot 0 ms 2517
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36
record 1 boot 0 ms 2567
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36
record 2 boot 1 ms 900
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36
found 3 records, 3 good (0 time anchors), 0 bad, from 2 boots, log ends at address 0x100
";

    #[test]
    fn a_record_is_its_header_plus_the_line_under_it() {
        let d = parse(SAMPLE);
        assert_eq!(d.records.len(), 3);
        assert_eq!(d.found, Some(3));
        assert!(d.complete());
        assert_eq!(d.records[0].boot, 0);
        assert_eq!(d.records[0].ms, 2517);
        assert_eq!(d.records[0].hex.len(), PACKET_BYTES * 2);
        assert!(d.records[0].hex.starts_with("000102"));
        assert_eq!(d.boots(), vec![0, 1]);
    }

    #[test]
    fn a_dump_that_never_finished_is_not_taken() {
        let half = SAMPLE.lines().take(5).collect::<Vec<_>>().join("\n");
        let d = parse(&half);
        assert!(!d.complete());
        assert!(d.records.is_empty(), "an unfinished printing is not a dump");
    }

    #[test]
    fn opening_the_port_mid_printing_takes_the_last_whole_one() {
        // The tail of an older printing, then a complete one. This is the case that
        // matters: the board reprints on demand and the port opens whenever it opens.
        let tail = "record 99 boot 9 ms 1\n00 01 02\nfound 1 records, 1 good (0 time anchors), 0 bad, from 1 boots, log ends at 0x1\n";
        let d = parse(&format!("{}{}", tail, SAMPLE));
        assert_eq!(d.found, Some(3));
        assert_eq!(d.records.len(), 3);
        assert_eq!(d.boots(), vec![0, 1], "the older printing's boot 9 is not in it");
    }

    #[test]
    fn boots_with_no_gps_are_laid_end_to_end_from_1970() {
        let d = parse(SAMPLE);
        let (dated, basis) = date(&d);
        assert_eq!(basis, "none");
        assert_eq!(dated.len(), 3);
        // Boot 0 starts at zero and keeps its own spacing.
        assert_eq!(dated[0].0, 0);
        assert_eq!(dated[1].0, 50);
        // Boot 1 begins after boot 0 ends plus the gap, not on top of it.
        assert_eq!(dated[2].0, 50 + BOOT_GAP_MS);
    }

    #[test]
    fn an_anchor_dates_the_records_before_it_too() {
        // Exactly what the board prints, hex bits and all.
        let text = format!(
            "{}\n{}",
            "time 1 boot 0 ms 2567 epoch 1790000000 nano 0 bits 07 accns 25",
            SAMPLE
        );
        let d = parse(&text);
        assert_eq!(d.anchors.len(), 1);
        let (dated, basis) = date(&d);
        assert_eq!(basis, "anchor");
        // The record 50 ms before the anchor is dated 50 ms before it, not dropped.
        let first = dated.iter().find(|(_, r)| r.n == 0).unwrap().0;
        assert_eq!(first, 1790000000000 - 50);
    }

    #[test]
    fn the_log_says_which_boot_and_where_it_came_from() {
        let d = parse(SAMPLE);
        let log = to_log(&d, "ttyACM1");
        let lines: Vec<&str> = log.lines().collect();
        assert!(lines[0].starts_with("# descent-ground log v1"));
        assert!(log.contains("#DG,SRC,v1,kind=flash,from=ttyACM1,boot=0,packets=2,time_source=none"));
        assert!(log.contains("#DG,BOOT,v1,n=0"));
        assert!(log.contains("#DG,BOOT,v1,n=1"));
        assert_eq!(log.matches("PKT,55,").count(), 3);
        // Every line is the dashboard's three tab-separated fields.
        for l in lines.iter().skip(1) {
            assert_eq!(l.matches('\t').count(), 2, "not a log row: {}", l);
        }
    }

    #[test]
    fn a_negative_nanosecond_fraction_is_subtracted_not_dropped() {
        // nano is %ld on the board. Parsed unsigned it fails and silently becomes zero,
        // which is up to a second of error in the wrong direction.
        let text = format!(
            "{}\n{}",
            "time 1 boot 0 ms 2517 epoch 1790000000 nano -250000000 bits 07 accns 25",
            SAMPLE
        );
        let d = parse(&text);
        assert_eq!(d.anchors.len(), 1, "a negative fraction must not throw the line away");
        assert_eq!(d.anchors[0].unix_ms, 1790000000000 - 250, "quarter of a second earlier");
    }

    #[test]
    fn the_hex_bits_field_does_not_confuse_the_labels() {
        // bits is %02X, so it can hold A-F. Fields are found by label, so a letter in
        // there must not shift anything else.
        let text = format!(
            "{}\n{}",
            "time 9 boot 0 ms 2517 epoch 1790000000 nano 1000000 bits AF accns 25",
            SAMPLE
        );
        let d = parse(&text);
        assert_eq!(d.anchors.len(), 1);
        assert_eq!(d.anchors[0].boot, 0);
        assert_eq!(d.anchors[0].ms, 2517);
        assert_eq!(d.anchors[0].unix_ms, 1790000000001);
    }


    // A slot laid out the way FlashLog.cpp encodeRecord does it.
    fn slot(kind: u8, boot: u16, ms: u32, payload: &[u8]) -> [u8; SLOT_BYTES] {
        let mut b = [0u8; SLOT_BYTES];
        b[0] = kind;
        b[1..3].copy_from_slice(&boot.to_le_bytes());
        b[3..7].copy_from_slice(&ms.to_le_bytes());
        b[7..7 + payload.len()].copy_from_slice(payload);
        let crc = crc16(&b[..62]);
        b[62..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02X}", b)).collect()
    }

    fn raw_line(n: u64, slot: &[u8; SLOT_BYTES]) -> String {
        format!("raw {} {:06X} {}\r\n", n, n * 64, hex_of(slot))
    }

    fn payload_of(n: u64) -> Vec<u8> {
        (0..PACKET_BYTES as u8).map(|i| i.wrapping_add(n as u8)).collect()
    }

    fn packet(n: u64, boot: u16, ms: u32) -> String {
        raw_line(n, &slot(SLOT_PACKET, boot, ms, &payload_of(n)))
    }

    // A packet slot from the board with this id.
    fn slot_from(csid: u8, n: u64, boot: u16, ms: u32) -> [u8; SLOT_BYTES] {
        let mut payload = payload_of(n);
        payload[PACKET_CSID_AT] = csid;
        slot(SLOT_PACKET, boot, ms, &payload)
    }

    fn packet_from(csid: u8, n: u64, boot: u16, ms: u32) -> String {
        raw_line(n, &slot_from(csid, n, boot, ms))
    }

    // What a board left on the older dump firmware was still printing when it was reset:
    // the end of one record's bytes, three whole records, and one cut off by the reset.
    // Their indices are far past anything the dump that follows holds.
    fn old_tail() -> String {
        let bytes = |fill: u8| vec![format!("{:02X}", fill); PACKET_BYTES].join(" ");
        format!(
            " 00 00 8C 4E 00 00 0B 00 05 00 04 00 13 00 09 9C 6F 9D 3B\r\n\
record 3110 boot 6 ms 23151\r\n{}\r\n\
record 3111 boot 6 ms 23201\r\n{}\r\n\
record 3112 boot 6 ms 23252\r\n{}\r\n\
record 3113 boot 6 ms 23302\r\n\
00 00 00 00 E8 4E 00 00 03 00 08 00 FD 26\u{fffd}I BOOT 0 c0: start test=flashdump\r\n\
I FLASH 0 c0: start\r\n\
I FLASH 2 c0: init ok=1 id=0xEF4015\r\n",
            bytes(0xE1),
            bytes(0xE2),
            bytes(0xE3)
        )
    }

    #[test]
    fn a_printing_of_whole_slots_takes_nothing_from_old_format_lines_in_it() {
        let whole = [packet_from(13, 0, 2, 1000), time(1, 2, 3000, 1790441150, 0), packet_from(13, 2, 2, 5000), packet_from(13, 3, 3, 400), found(4, 4, 1, 2)].concat();
        let d = parse(&[old_tail(), whole.clone()].concat());
        assert!(d.raw);
        assert_eq!(d.records.len(), 3);
        assert!(d.records.iter().all(|r| r.addr.is_some()), "every record kept came up as a whole slot");
        assert_eq!(d.boots(), vec![2, 3], "boot 6 was the old printing's");
        assert_eq!(d.anchors.len(), 1);
        assert_eq!(d.ignored_old, 3);
        assert_eq!(d.missing, 0, "an index only the old lines carried is not one this dump is short of");

        let r = report(&d, 0, 0);
        assert_eq!((r.found, r.board_good, r.read), (Some(4), Some(4), 4));
        assert!(r.complete, "{}", r.json());
        assert_eq!(r.csids, vec![13]);
        assert!(r.json().contains("\"read\":4,\"failedCheck\":0,\"missing\":0,\"ignoredOldFormat\":3,\"complete\":true,"), "{}", r.json());
        // The log is the same as if the old lines had never been on the port.
        assert_eq!(to_log(&d, "dump.txt"), to_log(&parse(&whole), "dump.txt"));

        // An old anchor is left out the same way, and so are old lines that come after
        // the slots or in among them.
        let anchor = "time 3109 boot 6 ms 23000 epoch 1790000000 nano 0 bits 07 accns 25\r\n";
        let mixed = [
            anchor.to_string(),
            packet_from(13, 0, 2, 1000),
            old_tail(),
            packet_from(13, 1, 2, 1050),
            anchor.to_string(),
            found(2, 2, 0, 1),
        ]
        .concat();
        let d = parse(&mixed);
        assert_eq!(d.records.len(), 2);
        assert!(d.anchors.is_empty());
        assert_eq!(d.ignored_old, 5);
        assert!(report(&d, 0, 0).complete);
    }

    #[test]
    fn a_printing_with_no_whole_slots_reads_the_old_format_as_it_always_did() {
        let d = parse(SAMPLE);
        assert!(!d.raw);
        assert_eq!(d.records.len(), 3);
        assert_eq!(d.ignored_old, 0);
        assert!(report(&d, 0, 0).json().contains("\"missing\":0,\"ignoredOldFormat\":0,\"complete\":true,"));

        // Old anchors still date an old printing.
        let dated = format!("time 0 boot 0 ms 2000 epoch 1790000000 nano 0 bits 07 accns 25\n{}", SAMPLE);
        let d = parse(&dated);
        assert_eq!(d.anchors.len(), 1);
        assert_eq!(d.ignored_old, 0);

        // The format is settled a printing at a time. An old-format printing that
        // follows a whole printing of slots is read as what it is.
        let slots = [packet_from(13, 0, 2, 1000), found(1, 1, 0, 1)].concat();
        let d = parse(&[slots, SAMPLE.to_string()].concat());
        assert!(!d.raw);
        assert_eq!(d.records.len(), 3);
        assert_eq!(d.ignored_old, 0);
    }

    // Not a fixture: a check against a capture from a real pull, wherever one is kept.
    // DG_REAL_DUMP=/path/to/dump.txt cargo test -- a_real_dump --nocapture
    #[test]
    fn a_real_dump_reads_as_many_records_as_the_board_counted() {
        let Ok(path) = std::env::var("DG_REAL_DUMP") else { return };
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path, e));
        let text = String::from_utf8_lossy(&bytes);
        let r = report(&parse(&text), bytes.len(), 0);
        println!(
            "{}: found {:?} boardGood {:?} read {} failedCheck {} missing {} ignoredOldFormat {} complete {} csids {:?}",
            path, r.found, r.board_good, r.read, r.failed_check, r.missing, r.ignored_old_format, r.complete, r.csids
        );
        assert!(r.found.is_some(), "the board never got to its count");
        assert_eq!(r.found, r.board_good);
        assert_eq!(r.board_good, Some(r.read));
        assert!(r.complete);
    }

    #[test]
    fn the_report_says_which_board_the_records_came_from() {
        let text = [packet_from(13, 0, 6, 1000), packet_from(13, 1, 6, 1050), packet_from(13, 2, 6, 1100), found(3, 3, 0, 1)].concat();
        let r = report(&parse(&text), 0, 0);
        assert_eq!(r.csids, vec![13]);
        assert_eq!(r.boots.len(), 1);
        assert_eq!(r.boots[0].csids, vec![13]);
        assert!(r.json().contains("\"csids\":[13],\"boots\":[{"), "{}", r.json());
        assert!(r.json().ends_with("\"clockFastPpm\":null,\"csids\":[13]}]}"), "{}", r.json());
    }

    #[test]
    fn a_chip_with_boots_from_two_boards_names_both_and_which_boot_was_whose() {
        // Reflashed with a new id between two boots, and the log not wiped.
        let text = [
            packet_from(13, 0, 6, 1000),
            packet_from(13, 1, 6, 1050),
            packet_from(9, 2, 7, 400),
            packet_from(9, 3, 7, 450),
            found(4, 4, 0, 2),
        ]
        .concat();
        let r = report(&parse(&text), 0, 0);
        assert_eq!(r.csids, vec![9, 13], "ascending, whatever order they were written in");
        let of = |boot: u32| r.boots.iter().find(|b| b.n == boot).unwrap().csids.clone();
        assert_eq!(of(6), vec![13]);
        assert_eq!(of(7), vec![9]);

        // An id that changes inside one boot is shown as that, not settled to one.
        let text = [packet_from(13, 0, 6, 1000), packet_from(255, 1, 6, 1050), found(2, 2, 0, 1)].concat();
        let r = report(&parse(&text), 0, 0);
        assert_eq!(r.csids, vec![13, 255]);
        assert_eq!(r.boots[0].csids, vec![13, 255]);
    }

    #[test]
    fn a_record_that_failed_its_check_names_no_board() {
        let mut cut = slot_from(77, 1, 6, 1050);
        cut[20] ^= 0x01;
        let text = [packet_from(13, 0, 6, 1000), raw_line(1, &cut), found(2, 2, 0, 1)].concat();
        let d = parse(&text);
        assert_eq!(d.failed, 1);
        let r = report(&d, 0, 0);
        assert_eq!(r.csids, vec![13]);
        assert_eq!(r.boots[0].csids, vec![13]);
        // Nor does a time anchor, whatever its bytes happen to be.
        let text = [time(0, 6, 500, 1790441150, 0), found(1, 1, 1, 1)].concat();
        let r = report(&parse(&text), 0, 0);
        assert_eq!(r.csids, Vec::<u8>::new());
        assert!(r.json().contains("\"csids\":[],\"boots\":["), "{}", r.json());
    }

    fn time(n: u64, boot: u16, ms: u32, epoch: u32, nano: i32) -> String {
        let mut payload = Vec::new();
        payload.extend_from_slice(&epoch.to_le_bytes());
        payload.extend_from_slice(&nano.to_le_bytes());
        payload.push(0x07);
        payload.extend_from_slice(&25u32.to_le_bytes());
        raw_line(n, &slot(SLOT_TIME, boot, ms, &payload))
    }

    fn found(slots: usize, good: usize, anchors: usize, boots: usize) -> String {
        format!(
            "found {} records, {} good ({} time anchors), {} bad, from {} boots, log ends at address 0x{:06X}\r\n",
            slots,
            good,
            anchors,
            slots - good,
            boots,
            slots * 64
        )
    }

    #[test]
    fn the_slot_check_is_crc16_ccitt_false() {
        assert_eq!(crc16(b"123456789"), 0x29B1);   // the catalogue's check value for it
    }

    #[test]
    fn a_raw_line_is_one_whole_checked_slot() {
        let text = [packet(0, 3, 2517), packet(1, 3, 2567), found(2, 2, 0, 1)].concat();
        let d = parse(&text);
        assert!(d.raw);
        assert_eq!(d.found, Some(2));
        assert_eq!(d.good, Some(2));
        assert_eq!(d.bad, Some(0));
        assert_eq!(d.records.len(), 2);
        assert_eq!(
            d.records[0],
            Record { n: 0, addr: Some(0), boot: 3, ms: 2517, hex: hex_of(&payload_of(0)) }
        );
        assert_eq!(d.records[1].n, 1);
        assert_eq!(d.records[1].addr, Some(64));
        assert_eq!(d.records[1].ms, 2567);
        assert_eq!(d.records[1].hex, hex_of(&payload_of(1)));
        assert_eq!((d.failed, d.missing), (0, 0));
    }

    // Two lines built by hand from the slot layout in bench/flash_dump.ino and checked
    // against a parser written separately, so the byte order is not just this file
    // agreeing with itself.
    #[test]
    fn slots_the_flight_repos_parser_accepts_read_the_same_here() {
        let text = "\
raw 7 0001C0 A5060040E20100000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F202122232425262728292A2B2C2D2E2F30313233343536B644\r
raw 8 000200 A6060048E80100BEF6B76A804D19F107190000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007472\r
";
        let d = parse_what_arrived(text);
        assert_eq!(d.records.len(), 1);
        assert_eq!(d.records[0].n, 7);
        assert_eq!(d.records[0].addr, Some(0x1C0));
        assert_eq!(d.records[0].boot, 6);
        assert_eq!(d.records[0].ms, 123456);
        assert_eq!(d.records[0].hex, hex_of(&payload_of(0)));
        // epoch 1790441150, nano -250000000
        assert_eq!(d.anchors, vec![Anchor { boot: 6, ms: 125000, unix_ms: 1790441149750 }]);
    }

    #[test]
    fn a_slot_with_one_flipped_bit_is_left_out_and_counted() {
        let mut bytes = slot(SLOT_PACKET, 3, 2567, &payload_of(1));
        bytes[4] ^= 0x10;   // in the uptime, which the old format had no check on
        let text = [packet(0, 3, 2517), raw_line(1, &bytes), packet(2, 3, 2617), found(3, 3, 0, 1)].concat();
        let d = parse(&text);
        assert_eq!(d.records.iter().map(|r| r.n).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(d.failed, 1);
        assert_eq!(d.missing, 0, "its line did arrive");
        let r = report(&d, text.len(), 0);
        assert_eq!(r.read, 2);
        assert!(!r.complete, "the board counted three good ones");
    }

    #[test]
    fn a_line_that_is_not_a_whole_slot_is_counted_too() {
        let good = packet(1, 3, 2567);
        let short = &good.trim_end()[..good.trim_end().len() - 2];
        let not_hex = good.replacen("0A0B", "0AxB", 1);
        let unknown = raw_line(3, &slot(0x5A, 3, 2700, &payload_of(3)));
        let text = [packet(0, 3, 2517), format!("{}\r\n", short), not_hex, unknown, found(4, 3, 0, 1)].concat();
        let d = parse(&text);
        assert_eq!(d.records.len(), 1);
        // Index 1 twice over, and the type nobody knows.
        assert_eq!(d.failed, 2);
        assert_eq!(d.missing, 1, "nothing at all was seen of index 2");
    }

    #[test]
    fn an_index_that_never_arrived_is_counted_missing() {
        let text = [packet(0, 3, 2517), packet(1, 3, 2567), packet(3, 3, 2667), found(4, 4, 0, 1)].concat();
        let d = parse(&text);
        assert_eq!(d.records.len(), 3);
        assert_eq!((d.failed, d.missing), (0, 1));
        assert!(!report(&d, 0, 0).complete);

        // The board said five and the last two never came.
        let text = [packet(0, 3, 2517), packet(1, 3, 2567), packet(2, 3, 2617), found(5, 5, 0, 1)].concat();
        assert_eq!(parse(&text).missing, 2);
    }

    #[test]
    fn the_same_index_twice_keeps_the_first_good_one() {
        let mut bad = slot(SLOT_PACKET, 3, 2517, &payload_of(0));
        bad[20] ^= 0x01;
        let text = [
            raw_line(0, &bad),
            packet(0, 3, 2517),
            raw_line(0, &slot(SLOT_PACKET, 3, 9999, &payload_of(9))),
            packet(1, 3, 2567),
            found(2, 2, 0, 1),
        ]
        .concat();
        let d = parse(&text);
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[0].ms, 2517);
        assert_eq!(d.failed, 0, "index 0 was read in the end");
        assert!(report(&d, 0, 0).complete);
    }

    #[test]
    fn anchors_come_up_the_raw_path_and_date_the_boot() {
        let text = [
            packet(0, 6, 1000),
            time(1, 6, 3000, 1790441150, 250_000_000),
            packet(2, 6, 5000),
            found(3, 3, 1, 1),
        ]
        .concat();
        let d = parse(&text);
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.anchors, vec![Anchor { boot: 6, ms: 3000, unix_ms: 1790441150250 }]);
        let (dated, basis) = date(&d);
        assert_eq!(basis, "anchor");
        assert_eq!(dated[0].0, 1790441150250 - 2000);
        assert_eq!(dated[1].0, 1790441150250 + 2000);
        let c = clocks(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].uptime0_ms, 1790441150250 - 3000);
        assert_eq!((c[0].used, c[0].rejected), (1, 0));
        // The anchor is a slot like any other, so it counts towards what the board called good.
        assert!(report(&d, 0, 0).complete);
    }

    #[test]
    fn an_anchor_far_from_the_others_is_rejected_and_does_not_move_the_boot() {
        // The third lost a digit of its uptime on the way up: 22567 arrived as 2256. That makes
        // it the anchor with the smallest uptime, and about 20 s out.
        let text = format!(
            "{}\n{}\n{}\n{}",
            "time 3 boot 0 ms 2567 epoch 1790000000 nano 0 bits 07 accns 25",
            "time 4 boot 0 ms 12567 epoch 1790000010 nano 0 bits 07 accns 25",
            "time 5 boot 0 ms 2256 epoch 1790000020 nano 0 bits 07 accns 25",
            SAMPLE
        );
        let d = parse(&text);
        assert_eq!(d.anchors.len(), 3);
        let c = clocks(&d);
        assert_eq!(c[0].boot, 0);
        assert_eq!((c[0].used, c[0].rejected), (2, 1));
        assert_eq!(c[0].uptime0_ms, 1790000000000 - 2567);
        assert_eq!(c[0].anchor.as_ref().map(|a| a.ms), Some(2567));
        let (dated, _) = date(&d);
        assert_eq!(dated.iter().find(|(_, r)| r.n == 0).unwrap().0, 1790000000000 - 50);
        let log = to_log(&d, "ttyACM1");
        assert!(log.contains("anchor_uptime_ms=2567,anchor_unix_ms=1790000000000,"), "{}", log);
        assert!(log.contains(",anchors=2,anchors_rejected=1"), "{}", log);
    }

    #[test]
    fn two_anchors_that_disagree_date_nothing() {
        // No way to tell which of two is the damaged one, and 1970 is the honest answer.
        let text = format!(
            "{}\n{}\n{}",
            "time 3 boot 0 ms 2567 epoch 1790000000 nano 0 bits 07 accns 25",
            "time 4 boot 0 ms 1256 epoch 1790000010 nano 0 bits 07 accns 25",
            SAMPLE
        );
        let d = parse(&text);
        let c = clocks(&d);
        assert!(c[0].anchor.is_none());
        assert_eq!((c[0].used, c[0].rejected), (0, 2));
        assert_eq!(date(&d).0[0].0, 0);
        assert!(to_log(&d, "ttyACM1").contains("boot=0,packets=2,time_source=none"));
    }

    #[test]
    fn boots_that_overlap_in_time_still_get_one_header_each() {
        // Boot 1 is dated inside boot 0. One sort by time would switch boots three times.
        let text = [
            packet(0, 0, 1000),
            time(1, 0, 1000, 1790000000, 0),
            packet(2, 0, 9000),
            packet(3, 1, 1000),
            time(4, 1, 1000, 1790000004, 0),
            packet(5, 1, 2000),
            found(6, 6, 2, 2),
        ]
        .concat();
        let log = to_log(&parse(&text), "ttyACM1");
        assert_eq!(log.matches("#DG,SRC,").count(), 2, "{}", log);
        assert_eq!(log.matches("#DG,BOOT,").count(), 2, "{}", log);
        let rows: Vec<(&str, &str)> = log
            .lines()
            .filter(|l| l.contains("\tPKT,"))
            .map(|l| {
                let mut f = l.split('\t');
                (f.next().unwrap(), f.next().unwrap())
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("1790000000000", "flash-boot0"),
                ("1790000008000", "flash-boot0"),
                ("1790000004000", "flash-boot1"),
                ("1790000005000", "flash-boot1"),
            ]
        );
        for l in log.lines().skip(1) {
            assert_eq!(l.matches('\t').count(), 2, "not a log row: {}", l);
        }
    }

    #[test]
    fn a_boot_with_no_anchor_says_so_even_when_another_has_one() {
        let text = format!(
            "{}\n{}",
            "time 1 boot 0 ms 2567 epoch 1790000000 nano 0 bits 07 accns 25",
            SAMPLE
        );
        let log = to_log(&parse(&text), "ttyACM1");
        assert!(log.contains("boot=0,packets=2,time_source=anchor,"), "{}", log);
        assert!(log.contains("boot=1,packets=1,time_source=none,uptime0_ms=-900"), "{}", log);
        let none = log.lines().find(|l| l.contains("#DG,SRC") && l.contains("boot=1,")).unwrap();
        assert!(!none.contains("anchor_"), "{}", none);
    }

    #[test]
    fn the_source_line_carries_the_anchor_as_utc() {
        let text = [
            packet(0, 6, 1000),
            time(1, 6, 3000, 1790441150, 0),
            found(2, 2, 1, 1),
        ]
        .concat();
        let log = to_log(&parse(&text), "/dev/ttyACM1, the left one");
        let src = log.lines().find(|l| l.contains("#DG,SRC,")).unwrap();
        let said = src.split('\t').nth(2).unwrap();
        assert_eq!(
            said,
            "#DG,SRC,v1,kind=flash,from=/dev/ttyACM1_ the left one,boot=6,packets=1,time_source=anchor,\
uptime0_ms=1790441147000,anchor_uptime_ms=3000,anchor_unix_ms=1790441150000,\
anchor_utc=2026-09-26T16:45:50.000Z,anchors=1,anchors_rejected=0"
        );
        // The page splits this on commas, so every piece after the tag is one key=value.
        for piece in said.split(',').skip(3) {
            assert_eq!(piece.matches('=').count(), 1, "{}", piece);
        }
    }

    #[test]
    fn a_printing_cut_short_is_still_handed_over_when_asked() {
        let text = [packet(0, 3, 2517), packet(1, 3, 2567)].concat();
        assert!(parse(&text).records.is_empty());
        let d = parse_what_arrived(&text);
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.found, None);
        assert!(!report(&d, 0, 0).complete);
        // A finished printing earlier in the same text is still the one to take.
        let whole = [packet(0, 3, 2517), found(1, 1, 0, 1)].concat();
        let d = parse_what_arrived(&[whole, text].concat());
        assert_eq!(d.found, Some(1));
        assert_eq!(d.records.len(), 1);
    }

    #[test]
    fn the_report_says_what_was_read_against_what_the_board_counted() {
        let text = [
            packet(0, 6, 1000),
            time(1, 6, 3000, 1790441150, 0),
            packet(2, 6, 5000),
            packet(3, 7, 400),
            found(4, 4, 1, 2),
        ]
        .concat();
        assert_eq!(
            report(&parse(&text), 600, 12).json(),
            "{\"format\":\"raw\",\"found\":4,\"boardGood\":4,\"boardBad\":0,\"read\":4,\"failedCheck\":0,\
\"missing\":0,\"ignoredOldFormat\":0,\"complete\":true,\"longestReaderGapMs\":12,\"bytes\":600,\"csids\":[50,52,53],\"boots\":[\
{\"n\":6,\"records\":2,\"firstMs\":1000,\"lastMs\":5000,\"timeSource\":\"anchor\",\
\"uptime0Ms\":1790441147000,\"anchorsUsed\":1,\"anchorsRejected\":0,\"clockFastPpm\":null,\"csids\":[50,52]},\
{\"n\":7,\"records\":1,\"firstMs\":400,\"lastMs\":400,\"timeSource\":\"none\",\
\"uptime0Ms\":-400,\"anchorsUsed\":0,\"anchorsRejected\":0,\"clockFastPpm\":null,\"csids\":[53]}]}"
        );

        // The old format, and a board that never got to its count.
        let half = SAMPLE.lines().take(5).collect::<Vec<_>>().join("\n");
        let j = report(&parse_what_arrived(&half), 0, 0).json();
        assert!(j.starts_with("{\"format\":\"text\",\"found\":null,\"boardGood\":null,\"boardBad\":null,\"read\":2,"), "{}", j);
        assert!(j.contains("\"complete\":false"), "{}", j);

        // A slot the board itself called bad is not one we failed to read.
        let mut cut = slot(SLOT_PACKET, 6, 1050, &payload_of(1));
        cut[62] ^= 0xFF;
        let text = [packet(0, 6, 1000), raw_line(1, &cut), found(2, 1, 0, 1)].concat();
        let r = report(&parse(&text), 0, 0);
        assert!(r.complete);
        assert!(r.json().contains("\"boardBad\":1,\"read\":1,\"failedCheck\":1,\"missing\":0,"), "{}", r.json());
    }


    // What GPS says the time is when a board counting 0.75% fast says it has been up for ms.
    fn gps_while_running_fast(ms: u64) -> u64 {
        1_790_000_000_000u64 + ms * 10_000 / 10_075
    }

    // One boot, two hours, an anchor every 10 s with a record beside it and one between.
    fn two_hours_running_fast() -> String {
        let mut text = String::new();
        let mut n = 0u64;
        for step in 0..=720u64 {
            let ms = step * 10_000;
            let at = gps_while_running_fast(ms);
            text.push_str(&packet(n, 3, ms as u32));
            text.push_str(&time(n + 1, 3, ms as u32, (at / 1000) as u32, ((at % 1000) * 1_000_000) as i32));
            text.push_str(&packet(n + 2, 3, ms as u32 + 5_000));
            n += 3;
        }
        text.push_str(&found(n as usize, n as usize, 721, 1));
        text
    }

    #[test]
    fn the_report_says_how_far_the_boards_clock_runs_from_gps() {
        let text = two_hours_running_fast();
        let r = report(&parse(&text), text.len(), 0);
        let ppm = r.boots[0].clock_fast_ppm.expect("two hours of anchors is plenty to measure from");
        assert!((ppm - 7500).abs() <= 5, "{}", ppm);
        assert!(r.json().contains("\"clockFastPpm\":75"), "{}", r.json());
        // One anchor dates a boot and says nothing about how fast its clock runs.
        let one = format!("{}{}{}", packet(0, 1, 2517), time(1, 1, 3000, 1_790_000_000, 0), found(2, 2, 1, 1));
        let r = report(&parse(&one), one.len(), 0);
        assert_eq!(r.boots[0].clock_fast_ppm, None);
        assert!(r.json().contains("\"clockFastPpm\":null"), "{}", r.json());
    }

    #[test]
    fn a_clock_that_runs_fast_keeps_its_anchors_and_is_dated_from_the_ones_nearby() {
        // Two hours with the board counting 0.75% more milliseconds than GPS does. Anchors an
        // hour apart disagree about uptime 0 by 27 s, and every one of them is right.
        let (text, gps) = (two_hours_running_fast(), gps_while_running_fast);
        let d = parse(&text);
        let c = clocks(&d);
        assert_eq!((c[0].used, c[0].rejected), (721, 0));
        let (dated, _) = date(&d);
        // A record written with an anchor has that anchor's time, at both ends of the boot.
        assert_eq!(dated.first().unwrap().0, gps(0));
        let last = dated.iter().rev().find(|(_, r)| r.ms == 7_200_000).unwrap().0;
        assert_eq!(last, gps(7_200_000));
        // One halfway between two anchors lands halfway between their times.
        let mid = dated.iter().find(|(_, r)| r.ms == 3_605_000).unwrap().0;
        assert!((mid as i64 - gps(3_605_000) as i64).abs() <= 1, "{} against {}", mid, gps(3_605_000));
        // And time never runs backwards inside the boot.
        assert!(dated.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn a_row_carries_its_own_uptime_and_its_index_on_the_chip() {
        let text = format!("{}{}{}", packet(0, 2, 2517), packet(1, 2, 2567), found(2, 2, 0, 1));
        let log = to_log(&parse(&text), "ttyACM1");
        let rows: Vec<&str> = log.lines().filter(|l| l.contains("PKT,55,")).collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].ends_with(",,,,up=2517,n=0"), "{}", rows[0]);
        assert!(rows[1].ends_with(",,,,up=2567,n=1"), "{}", rows[1]);
    }
}
