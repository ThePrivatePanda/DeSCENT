// Fleet state as web/js/fleet.js keeps it: one entry per CSID, one per receiver.
// Every call takes the time t in ms, the receive time live and the log's own time in
// a replay, so both run the same code. Rows come out as JSON with fleet.js's names, so
// the page reads them as it read fleet.js's objects. Nothing here judges a value; the
// thresholds are the user's settings and the flags are the packet's own.
#![allow(dead_code)]   // the server starts using it in the next part of the 1.4 split

use std::collections::HashMap;

use crate::decode::{js_num, js_round, Decoded};
use crate::json::esc;
use crate::lines::{info_json, js_number, Event, Info, Kind};

// Every threshold the user can change in Settings (fleet.js DEFAULTS)
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub lost_min: f64,          // silent this long = lost (min, user rule)
    pub stale_factor: f64,      // stale after this many missed intervals
    pub stale_min_s: f64,       // and never sooner than this (s)
    pub dedupe_ms: f64,         // same bytes within this window = same over-the-air packet (ms)
    pub retain_min: f64,        // drop history older than this (min, 0 = keep everything)
    pub max_points: f64,        // history points kept per unit (f64::INFINITY keeps all, as a replay does)
    pub low_battery_v: f64,     // a lithium cell under load (V)
    pub weak_rssi_dbm: f64,     // best RSSI under this is weak RF (dBm)
    pub saturation_mps2: f64,   // BNO085 accelerometer range is ±8 g, about 78 m/s²
    // Not in DEFAULTS: fleet.js only splits when cfg.splitOnReset is not false
    pub split_on_reset: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            lost_min: 10.0,
            stale_factor: 3.0,
            stale_min_s: 5.0,
            dedupe_ms: 1500.0,
            retain_min: 0.0,
            max_points: 20000.0,
            low_battery_v: 3.5,
            weak_rssi_dbm: -115.0,
            saturation_mps2: 75.0,
            split_on_reset: true,
        }
    }
}

const STALE_DEFAULT_MS: f64 = 30000.0;   // before the interval is known
const WRAP_SLACK: f64 = 1000.0;          // counter 65xxx -> small = wrap, not reset
const SLACK: usize = 2000;               // points kept past the cap before one tidy-up
const INTERVAL_SAMPLES: usize = 15;

// The fields with a validity group, in packet order: key, index in Decoded::values,
// validity bit. As packet.js FIELDS; a test holds the indexes against decode.rs.
const GROUPED: [(&str, usize, usize); 21] = [
    ("lat", 0, 4),
    ("lon", 1, 4),
    ("gpsAlt", 2, 4),
    ("envAlt", 3, 6),
    ("ax", 4, 0),
    ("ay", 5, 0),
    ("az", 6, 0),
    ("gx", 7, 1),
    ("gy", 8, 1),
    ("gz", 9, 1),
    ("mx", 10, 2),
    ("my", 11, 2),
    ("mz", 12, 2),
    ("qi", 13, 3),
    ("qj", 14, 3),
    ("qk", 15, 3),
    ("qr", 16, 3),
    ("temp", 17, 6),
    ("pressure", 18, 6),
    ("humidity", 19, 6),
    ("battery", 22, 5),
];
const COUNTER: usize = 20;
const BATTERY: usize = 22;
const VALIDITY: usize = 23;
const ACCEL: [usize; 3] = [4, 5, 6];
const SOC: usize = 5;

// packet.js VALIDITY_BITS names for bits 0-6; bit 7 (fresh) is never an issue
const VALIDITY_NAMES: [&str; 7] =
    ["Linear acceleration", "Gyroscope", "Magnetometer", "Quaternion", "GPS", "State of charge", "Environment"];

// fleet.js SERIES: counter, rssi, snr, then every grouped field
fn series_names() -> impl Iterator<Item = &'static str> {
    ["counter", "rssi", "snr"].into_iter().chain(GROUPED.iter().map(|g| g.0))
}
const SERIES_LEN: usize = 3 + GROUPED.len();

fn median(a: &[f64]) -> f64 {
    if a.is_empty() {
        return f64::NAN;
    }
    let mut s = a.to_vec();
    s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let m = s.len() >> 1;
    if s.len() % 2 == 1 {
        s[m]
    } else {
        (s[m - 1] + s[m]) / 2.0
    }
}

// Retired generations are lettered oldest first, so 64a came before 64b, and the
// plain number is always the one transmitting now. Past z: aa, ab.
pub fn gen_label(n: u32) -> String {
    let mut s = Vec::new();
    let mut n = n as u64 + 1;
    while n > 0 {
        n -= 1;
        s.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

// None is fleet.js's null; Some(None) is undefined, a #DG,BOOT line without n=
pub type Boot = Option<Option<String>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Reception {
    pub rx: String,
    pub rssi: f64,
    pub snr: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    pub t: Vec<f64>,             // s
    pub series: Vec<Vec<f64>>,   // in series_names() order, NaN where fleet.js has null
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RxSeries {
    pub t: Vec<f64>,   // s
    pub rssi: Vec<f64>,
    pub snr: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub csid: u8,
    pub boot: Boot,                  // which boot of the board this run is, when the dump says
    pub source_rx: Option<String>,   // the source this run came from, for a log off a chip
    pub from_chip: bool,             // read off flash, so counter-derived numbers do not apply
    pub label: String,               // '64' while live, '64a' once retired
    pub live: bool,
    pub first_t: Option<f64>,
    pub last_t: Option<f64>,
    pub packets: u64,
    pub missed: u64,
    pub resets: u64,
    pub repeats: u64,
    pub last_counter: Option<f64>,
    pub gaps: Vec<f64>,              // recent ms per counter step
    pub interval_ms: f64,
    pub latest: Option<Decoded>,
    pub latest_source: Option<&'static str>,
    pub receptions: Vec<Reception>,  // for the latest packet
    pub best_rssi: f64,
    pub best_snr: f64,
    pub history: History,
    pub rx_history: Vec<(String, RxSeries)>,   // in the order the receivers were first heard
    pub last_valid: Vec<(&'static str, f64)>,  // field -> last value seen while its bit was set
    pub saturated_count: u64,
    pub gen: Option<u32>,            // set when retired
}

fn new_unit(csid: u8) -> Unit {
    Unit {
        csid,
        boot: None,
        source_rx: None,
        from_chip: false,
        label: csid.to_string(),
        live: true,
        first_t: None,
        last_t: None,
        packets: 0,
        missed: 0,
        resets: 0,
        repeats: 0,
        last_counter: None,
        gaps: Vec::new(),
        interval_ms: f64::NAN,
        latest: None,
        latest_source: None,
        receptions: Vec::new(),
        best_rssi: f64::NAN,
        best_snr: f64::NAN,
        history: History { t: Vec::new(), series: vec![Vec::new(); SERIES_LEN] },
        rx_history: Vec::new(),
        last_valid: Vec::new(),
        saturated_count: 0,
        gen: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Receiver {
    pub key: String,
    pub id: Option<String>,
    pub info: Info,
    pub first_t: Option<f64>,
    pub last_t: Option<f64>,
    pub last_hb_t: Option<f64>,
    pub packets: u64,
    pub unique: u64,
    pub bad_crc: u64,
    pub radio_errors: u64,
    pub reported_ok: f64,
    pub reported_errors: f64,
    pub source: Option<Info>,   // not a radio: where its lines came from
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Lost,
    Stale,
    Ok,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Lost => "LOST",
            State::Stale => "STALE",
            State::Ok => "OK",
        }
    }
}

// One row of the fleet list: a unit as fleet.js rows() gives it, evaluated at a time
pub struct Row<'a> {
    pub unit: &'a Unit,
    pub state: State,
    pub age: f64,
    pub issues: Vec<String>,
    pub rx_percent: f64,
    pub stale_after_ms: f64,
    pub runs: usize,   // this run and the earlier ones of the board, as livecards.js counts
}

pub enum Update {
    Unit(String),       // the row's JSON, kind first
    Receiver(String),
}

impl Update {
    pub fn to_json(&self) -> &str {
        match self {
            Update::Unit(s) | Update::Receiver(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub heard: u64,
    pub active: u64,
    pub stale: u64,
    pub lost: u64,
    pub packets: u64,
    pub low_battery: u64,
    pub validity_issues: u64,
    pub weak_rf: u64,
    pub resets: u64,
    pub bad_crc: u64,
    pub saturated: u64,
}

#[derive(Debug, Clone)]
pub struct Fleet {
    pub cfg: Config,
    units: Vec<Unit>,                 // live, in the order first heard (a Map in fleet.js)
    retired: Vec<Unit>,               // label order of retirement
    receivers: Vec<Receiver>,
    sources: HashMap<String, Info>,   // rx -> where its lines came from, if not a radio
    generations: HashMap<u8, u32>,    // csid -> how many of its generations have ended
    current_boot: Boot,               // the boot a dump is currently replaying
    recent: HashMap<Vec<u8>, (f64, u8)>,   // bytes -> (t, csid)
    pub bad_crc: u64,
    pub bad_length: u64,
    pub total: u64,
    pub duplicates: u64,
    // Numbers as app.js adds them, Number(label), so a retired label adds a NaN that
    // hides nothing but still counts. SameValueZero, as a Set compares.
    pub hidden: Vec<f64>,
    pub last_retired: Option<String>,
    // What fleet.js ingest returned for the last line: new, duplicate, badcrc,
    // badlength, or the event kind
    pub last_result: &'static str,
}

fn same_value_zero(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

impl Fleet {
    pub fn new(cfg: Config) -> Fleet {
        Fleet {
            cfg,
            units: Vec::new(),
            retired: Vec::new(),
            receivers: Vec::new(),
            sources: HashMap::new(),
            generations: HashMap::new(),
            current_boot: None,
            recent: HashMap::new(),
            bad_crc: 0,
            bad_length: 0,
            total: 0,
            duplicates: 0,
            hidden: Vec::new(),
            last_retired: None,
            last_result: "",
        }
    }

    // Trims live units to the new cap on the spot; retired runs keep what they had, as
    // in fleet.js
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
        let cfg = self.cfg.clone();
        for u in &mut self.units {
            trim_history(&cfg, u, true);
        }
    }

    fn receiver_index(&mut self, rx: &str) -> usize {
        if let Some(i) = self.receivers.iter().position(|r| r.key == rx) {
            return i;
        }
        self.receivers.push(Receiver {
            key: rx.to_string(),
            id: None,
            info: Vec::new(),
            first_t: None,
            last_t: None,
            last_hb_t: None,
            packets: 0,
            unique: 0,
            bad_crc: 0,
            radio_errors: 0,
            reported_ok: f64::NAN,
            reported_errors: f64::NAN,
            source: None,
        });
        self.receivers.len() - 1
    }

    fn live_index(&self, csid: u8) -> Option<usize> {
        self.units.iter().position(|u| u.csid == csid)
    }

    // Feed one parsed line. Returns the rows it changed: a retired run if it ended one,
    // the unit the packet belongs to, and always the receiver.
    pub fn ingest(&mut self, ev: &Event, rx: &str, t: f64) -> Vec<Update> {
        let (result, unit, retired) = self.feed(ev, rx, t);
        self.last_result = result;
        let mut out = Vec::new();
        if let Some(label) = retired {
            if let Some(u) = self.retired.iter().find(|u| u.label == label) {
                out.push(Update::Unit(format!("{{\"kind\":\"unit\",{}}}", self.row(u, t).fields())));
            }
        }
        if let Some(i) = unit.and_then(|c| self.live_index(c)) {
            out.push(Update::Unit(format!("{{\"kind\":\"unit\",{}}}", self.row(&self.units[i], t).fields())));
        }
        let key = if rx.is_empty() { "rx" } else { rx };
        if let Some(r) = self.receivers.iter().find(|r| r.key == key) {
            out.push(Update::Receiver(format!("{{\"kind\":\"receiver\",{}}}", receiver_fields(r))));
        }
        out
    }

    // fleet.js ingest: (what it returns, the live unit touched, the label retired)
    fn feed(&mut self, ev: &Event, rx: &str, t: f64) -> (&'static str, Option<u8>, Option<String>) {
        let ri = self.receiver_index(if rx.is_empty() { "rx" } else { rx });
        let r = &mut self.receivers[ri];
        if r.first_t.is_none() {
            r.first_t = Some(t);
        }
        let (bytes, decoded, rssi, snr, source) = match &ev.kind {
            Kind::Device { kind: "rxinfo", info, .. } => {
                r.info = info.clone();
                if let Some((_, id)) = info.iter().find(|(k, _)| k == "id").filter(|(_, v)| !v.is_empty()) {
                    r.id = Some(id.clone());
                }
                r.last_t = Some(t);
                return ("rxinfo", None, None);
            }
            // Not a radio. A flash dump has no RSSI or SNR by nature, and a replay of one
            // must not be mistaken for live reception in a screenshot.
            Kind::Device { kind: "source", info, .. } => {
                r.source = Some(info.clone());
                self.sources.insert(rx.to_string(), info.clone());
                return ("source", None, None);
            }
            // Which boot the records after this one belong to. The counter cannot tell us.
            Kind::Device { kind: "boot", info, .. } => {
                self.current_boot = Some(info.iter().find(|(k, _)| k == "n").map(|(_, v)| v.clone()));
                return ("boot", None, None);
            }
            Kind::Device { kind, .. } => return (*kind, None, None),
            Kind::Heartbeat { ok, errors, .. } => {
                r.last_hb_t = Some(t);
                r.last_t = Some(t);
                r.reported_ok = *ok;
                r.reported_errors = *errors;
                return ("heartbeat", None, None);
            }
            Kind::RxError { .. } => {
                r.radio_errors += 1;
                r.last_t = Some(t);
                return ("rxerror", None, None);
            }
            Kind::Packet { decoded: Err(_), .. } => {
                r.last_t = Some(t);
                self.bad_length += 1;
                return ("badlength", None, None);
            }
            Kind::Packet { source, bytes, decoded: Ok(d), rssi, snr, .. } => (bytes, d, *rssi, *snr, *source),
            Kind::RxFatal { .. } => return ("rxfatal", None, None),
            Kind::CsvHeader => return ("csvheader", None, None),
            Kind::Empty => return ("empty", None, None),
            Kind::Other => return ("other", None, None),
        };
        r.last_t = Some(t);
        r.packets += 1;
        if !decoded.crc_ok {
            r.bad_crc += 1;
            self.bad_crc += 1;
            return ("badcrc", None, None);
        }

        let dedupe = self.cfg.dedupe_ms;
        self.recent.retain(|_, seen| !(t - seen.0 > dedupe));
        // Deduplication exists because two receivers hear one transmission. A chip log has
        // no receivers, and two records that happen to hold the same bytes are two real
        // samples, so dropping one loses data that was measured.
        let from_chip = self.sources.contains_key(rx);
        let seen = if from_chip { None } else { self.recent.get(bytes).copied() };
        if let Some((_, csid)) = seen {
            self.duplicates += 1;
            let i = self.live_index(csid);
            if let Some(i) = i {
                add_reception(&mut self.units[i], rx, rssi, snr, t);
            }
            return ("duplicate", i.map(|_| csid), None);
        }
        self.receivers[ri].unique += 1;
        self.total += 1;
        let mut d = decoded.clone();
        d.saturated = d.valid[0] && ACCEL.iter().any(|&k| d.values[k].abs() >= self.cfg.saturation_mps2);
        let csid = d.csid();
        let counter = d.values[COUNTER];
        self.recent.insert(bytes.clone(), (t, csid));
        // A log read off a chip is not a packet stream: the board logs at 20 Hz but the
        // counter moves once per transmission, and every boot in a dump is dated from the
        // same start, so the boots interleave. A chip log is not split on the counter;
        // a dump that names its boots is split one run per boot instead.
        let mut retired = None;
        let mut ui = self.live_index(csid);
        if let Some(i) = ui {
            let u = &self.units[i];
            let new_boot = from_chip && u.boot.is_some() && self.current_boot.is_some() && u.boot != self.current_boot;
            let reset = !from_chip && self.cfg.split_on_reset && is_reset(u, counter);
            if new_boot || reset {
                retired = self.retire(csid);
                self.last_retired = retired.clone();
                ui = None;
            }
        }
        let i = match ui {
            Some(i) => i,
            None => {
                self.units.push(new_unit(csid));
                self.units.len() - 1
            }
        };
        let cfg = self.cfg.clone();
        let u = &mut self.units[i];
        if from_chip {
            // Missed, restarts, repeats and the transmit interval are all read off the
            // counter, which in a chip log counts transmissions while the file holds every
            // sample. Nothing derived from it would be true, so none of it is claimed.
            u.source_rx = Some(rx.to_string());
            u.from_chip = true;
            if u.boot.is_none() {
                u.boot = self.current_boot.clone();
            }
            u.last_counter = Some(counter);
        } else {
            update_counter(u, counter, t);
        }
        u.packets += 1;
        if u.first_t.is_none() {
            u.first_t = Some(t);
        }
        u.last_t = Some(t);
        u.latest_source = Some(source);
        u.receptions.clear();
        u.best_rssi = f64::NAN;
        u.best_snr = f64::NAN;
        if d.saturated {
            u.saturated_count += 1;
        }
        for (key, at, bit) in GROUPED {
            if d.valid[bit] {
                match u.last_valid.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = d.values[at],
                    None => u.last_valid.push((key, d.values[at])),
                }
            }
        }
        push_history(&cfg, u, &d, t);
        u.latest = Some(d);
        add_reception(u, rx, rssi, snr, t);
        ("new", Some(csid), retired)
    }

    // Put the run that just ended aside under a letter and start a clean one, so a
    // reflashed board does not mix its old data into its new
    fn retire(&mut self, csid: u8) -> Option<String> {
        let i = self.live_index(csid)?;
        let mut u = self.units.remove(i);
        let n = self.generations.get(&csid).copied().unwrap_or(0);
        self.generations.insert(csid, n + 1);
        u.label = format!("{}{}", csid, gen_label(n));
        u.gen = Some(n);
        u.live = false;
        let label = u.label.clone();
        match self.retired.iter().position(|r| r.label == label) {
            Some(j) => self.retired[j] = u,
            None => self.retired.push(u),
        }
        Some(label)
    }

    // Every generation: the live runs, then the retired ones (fleet.js allUnits)
    pub fn all_units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().chain(self.retired.iter())
    }

    // Runs of one board that ended at a restart, oldest first
    pub fn earlier_runs(&self, csid: u8) -> Vec<&Unit> {
        let mut out: Vec<&Unit> = self.retired.iter().filter(|u| u.csid == csid).collect();
        out.sort_by_key(|u| u.gen);
        out
    }

    // fleet.js unit(label): Number(label) for a live one, else a retired label
    pub fn unit(&self, label: &str) -> Option<&Unit> {
        let n = js_number(label);
        self.units.iter().find(|u| u.csid as f64 == n).or_else(|| self.retired.iter().find(|u| u.label == label))
    }

    // Undo a split: the run goes back in front of the one that followed it. Returns the
    // label it went into.
    pub fn merge_run(&mut self, label: &str) -> Option<String> {
        let ai = self.retired.iter().position(|u| u.label == label)?;
        let (csid, gen) = (self.retired[ai].csid, self.retired[ai].gen);
        let later = self
            .retired
            .iter()
            .enumerate()
            .filter(|(_, u)| u.csid == csid && u.gen > gen)
            .min_by_key(|(_, u)| u.gen)
            .map(|(j, _)| (true, j))
            .or_else(|| self.live_index(csid).map(|j| (false, j)));
        let (in_retired, j) = later?;
        let a = self.retired[ai].clone();
        let into = if in_retired { &mut self.retired[j] } else { &mut self.units[j] };

        if let Some(at) = a.first_t {
            into.first_t = Some(into.first_t.map_or(at, |lt| at.min(lt)));
        }
        into.packets += a.packets;
        into.missed += a.missed;
        into.repeats += a.repeats;
        // The board did reboot, whatever the operator calls it, so the reset that split
        // the runs goes back into the count
        into.resets += a.resets + 1;
        into.saturated_count += a.saturated_count;

        into.history.t = [a.history.t.as_slice(), into.history.t.as_slice()].concat();
        for (k, s) in into.history.series.iter_mut().enumerate() {
            *s = [a.history.series[k].as_slice(), s.as_slice()].concat();
        }
        for (rx, h) in &a.rx_history {
            let at = match into.rx_history.iter().position(|(k, _)| k == rx) {
                Some(p) => p,
                None => {
                    into.rx_history.push((rx.clone(), RxSeries::default()));
                    into.rx_history.len() - 1
                }
            };
            let s = &mut into.rx_history[at].1;
            s.t = [h.t.as_slice(), s.t.as_slice()].concat();
            s.rssi = [h.rssi.as_slice(), s.rssi.as_slice()].concat();
            s.snr = [h.snr.as_slice(), s.snr.as_slice()].concat();
        }
        // The newer run's last trusted values win; the older one fills what it never saw
        for (k, v) in &a.last_valid {
            if !into.last_valid.iter().any(|(have, _)| have == k) {
                into.last_valid.push((*k, *v));
            }
        }
        let out = into.label.clone();
        self.retired.remove(ai);
        if self.earlier_runs(csid).is_empty() {
            self.generations.remove(&csid);
        }
        Some(out)
    }

    // Put every earlier run of a board back into the one transmitting now
    pub fn merge_all_runs(&mut self, csid: u8) -> usize {
        let labels: Vec<String> = self.earlier_runs(csid).iter().rev().map(|u| u.label.clone()).collect();
        labels.iter().filter(|l| self.merge_run(l).is_some()).count()
    }

    // A copy to put back later; replay scrubs backwards by restoring the nearest one.
    // The settings are not part of it, as in fleet.js.
    pub fn snapshot(&self) -> Fleet {
        self.clone()
    }

    pub fn restore(&mut self, state: &Fleet) {
        let (cfg, last_retired, last_result) = (self.cfg.clone(), self.last_retired.take(), self.last_result);
        *self = state.clone();
        self.cfg = cfg;
        self.last_retired = last_retired;
        self.last_result = last_result;
    }

    pub fn hide(&mut self, label: &str) {
        let n = js_number(label);
        if !self.hidden.iter().any(|h| same_value_zero(*h, n)) {
            self.hidden.push(n);
        }
    }

    fn is_hidden(&self, csid: u8) -> bool {
        self.hidden.iter().any(|h| *h == csid as f64)
    }

    // Clean slate, in pieces. Nothing here touches what is already on disk.
    pub fn clear_unit(&mut self, label: &str) -> bool {
        let Some(u) = self.unit(label) else { return false };
        let (csid, live) = (u.csid, u.live);
        if live {
            self.units.retain(|u| u.csid != csid);
        } else {
            self.retired.retain(|u| u.label != label);
        }
        self.recent.retain(|_, seen| seen.1 != csid);
        // fleet.js also deletes the label from hidden, which holds numbers, so that never
        // matches anything and nothing is done here either
        // With nothing of this board left, its lettering starts from a again
        if self.live_index(csid).is_none() && self.earlier_runs(csid).is_empty() {
            self.generations.remove(&csid);
        }
        true
    }

    pub fn clear_units(&mut self) {
        self.units.clear();
        self.retired.clear();
        self.generations.clear();
        self.recent.clear();
        self.hidden.clear();
        self.total = 0;
        self.duplicates = 0;
        self.bad_crc = 0;
        self.bad_length = 0;
    }

    // Zero a receiver's tallies without closing it: it keeps receiving
    pub fn clear_receiver(&mut self, rx: &str) -> bool {
        let Some(r) = self.receivers.iter_mut().find(|r| r.key == rx) else { return false };
        r.packets = 0;
        r.unique = 0;
        r.bad_crc = 0;
        r.radio_errors = 0;
        r.first_t = None;
        true
    }

    pub fn clear_receivers(&mut self) {
        for r in &mut self.receivers {
            r.packets = 0;
            r.unique = 0;
            r.bad_crc = 0;
            r.radio_errors = 0;
            r.first_t = None;
        }
    }

    pub fn stale_after_ms(&self, u: &Unit) -> f64 {
        if u.interval_ms.is_finite() {
            (self.cfg.stale_min_s * 1000.0).max(self.cfg.stale_factor * u.interval_ms)
        } else {
            STALE_DEFAULT_MS
        }
    }

    pub fn state(&self, u: &Unit, now: f64) -> State {
        let age = now - last_t(u);
        if age > self.cfg.lost_min * 60000.0 {
            State::Lost
        } else if age > self.stale_after_ms(u) {
            State::Stale
        } else {
            State::Ok
        }
    }

    pub fn issues(&self, u: &Unit) -> Vec<String> {
        let mut out = Vec::new();
        let Some(d) = &u.latest else { return out };
        if d.valid[SOC] && d.values[BATTERY] < self.cfg.low_battery_v {
            out.push("low battery".to_string());
        }
        for (bit, name) in VALIDITY_NAMES.iter().enumerate() {
            if !d.valid[bit] {
                out.push(format!("{} invalid", name));
            }
        }
        if u.best_rssi.is_finite() && u.best_rssi < self.cfg.weak_rssi_dbm {
            out.push("weak RF".to_string());
        }
        if d.saturated {
            out.push("accel saturated".to_string());
        }
        out
    }

    pub fn rx_percent(&self, u: &Unit) -> f64 {
        let expected = u.packets + u.missed;
        if expected > 0 {
            (100.0 * u.packets as f64) / expected as f64
        } else {
            f64::NAN
        }
    }

    pub fn row<'a>(&self, u: &'a Unit, t: f64) -> Row<'a> {
        Row {
            unit: u,
            state: self.state(u, t),
            age: t - last_t(u),
            issues: self.issues(u),
            rx_percent: self.rx_percent(u),
            stale_after_ms: self.stale_after_ms(u),
            runs: self.earlier_runs(u.csid).len() + 1,
        }
    }

    // The live units not hidden, by CSID, with stale and lost evaluated at t
    pub fn rows(&self, t: f64) -> Vec<Row<'_>> {
        self.rows_by(t, "")
    }

    // sort "attention" puts lost first, then stale, then the most issues, as the page's
    // fleet list does; anything else is by CSID
    pub fn rows_by(&self, t: f64, sort: &str) -> Vec<Row<'_>> {
        let mut rows: Vec<Row> = self.units.iter().filter(|u| !self.is_hidden(u.csid)).map(|u| self.row(u, t)).collect();
        rows.sort_by(|a, b| {
            let by_attention = if sort == "attention" {
                a.state.cmp(&b.state).then(b.issues.len().cmp(&a.issues.len()))
            } else {
                std::cmp::Ordering::Equal
            };
            by_attention.then(a.unit.csid.cmp(&b.unit.csid))
        });
        rows
    }

    // t is not used: fleet.js keeps no time-derived state for a receiver. It is in the
    // signature the server calls so one can be added without changing callers.
    pub fn receivers(&self, _t: f64) -> &[Receiver] {
        &self.receivers
    }

    pub fn summary(&self, now: f64) -> Summary {
        let mut s = Summary { packets: self.total, bad_crc: self.bad_crc, ..Summary::default() };
        for u in self.units.iter().filter(|u| !self.is_hidden(u.csid)) {
            s.heard += 1;
            match self.state(u, now) {
                State::Ok => s.active += 1,
                State::Stale => s.stale += 1,
                State::Lost => s.lost += 1,
            }
            let Some(d) = &u.latest else { continue };
            if d.valid[SOC] && d.values[BATTERY] < self.cfg.low_battery_v {
                s.low_battery += 1;
            }
            if (d.values[VALIDITY] as u32 & 0x7f) != 0x7f {
                s.validity_issues += 1;
            }
            if u.best_rssi.is_finite() && u.best_rssi < self.cfg.weak_rssi_dbm {
                s.weak_rf += 1;
            }
            if d.saturated {
                s.saturated += 1;
            }
            s.resets += u.resets;
        }
        s
    }

    // {"history":{t, counter, rssi, ...}, "rxHistory":{rx:{t, rssi, snr}}} for a label,
    // live ("64") or retired ("64a")
    pub fn history(&self, label: &str) -> Option<String> {
        let u = self.unit(label)?;
        let series: Vec<String> =
            series_names().zip(&u.history.series).map(|(k, s)| format!("\"{}\":{}", k, nums(s))).collect();
        let rx: Vec<String> = u
            .rx_history
            .iter()
            .map(|(k, h)| format!("\"{}\":{{\"t\":{},\"rssi\":{},\"snr\":{}}}", esc(k), nums(&h.t), nums(&h.rssi), nums(&h.snr)))
            .collect();
        Some(format!(
            "{{\"history\":{{\"t\":{},{}}},\"rxHistory\":{{{}}}}}",
            nums(&u.history.t),
            series.join(","),
            rx.join(",")
        ))
    }
}

// Math.round(null) is 0, which is what fleet.js gets for a unit never heard
fn last_t(u: &Unit) -> f64 {
    u.last_t.unwrap_or(0.0)
}

// A board that rebooted. The counter starts again from 0, so a drop that is not a
// wrap, or a second 0 in a row, means the packets after it belong to a new run.
fn is_reset(u: &Unit, counter: f64) -> bool {
    u.last_counter.is_some_and(|last| is_reset_from(last, counter))
}

fn update_counter(u: &mut Unit, counter: f64, t: f64) {
    let last = u.last_counter.replace(counter);
    let Some(last) = last else { return };
    if is_reset_from(last, counter) {
        u.resets += 1;
        return;
    }
    if counter == last {
        u.repeats += 1;
        return;
    }
    let step = (counter - last + 65536.0) % 65536.0;
    u.missed += step as u64 - 1;
    if let Some(lt) = u.last_t.filter(|lt| t > *lt) {
        u.gaps.push((t - lt) / step);
        if u.gaps.len() > INTERVAL_SAMPLES {
            u.gaps.remove(0);
        }
        u.interval_ms = median(&u.gaps);
    }
}

fn is_reset_from(last: f64, counter: f64) -> bool {
    let wrapped = last > 65535.0 - WRAP_SLACK && counter < WRAP_SLACK;
    (counter < last && !wrapped) || (counter == 0.0 && last == 0.0)
}

fn add_reception(u: &mut Unit, rx: &str, rssi: f64, snr: f64, t: f64) {
    u.receptions.push(Reception { rx: rx.to_string(), rssi, snr });
    // NaN best: rssi <= NaN is false, so the first finite one goes in
    if rssi.is_finite() && !(rssi <= u.best_rssi) {
        u.best_rssi = rssi;
    }
    if snr.is_finite() && !(snr <= u.best_snr) {
        u.best_snr = snr;
    }
    let at = match u.rx_history.iter().position(|(k, _)| k == rx) {
        Some(p) => p,
        None => {
            u.rx_history.push((rx.to_string(), RxSeries::default()));
            u.rx_history.len() - 1
        }
    };
    let h = &mut u.rx_history[at].1;
    h.t.push(t / 1000.0);
    h.rssi.push(rssi);
    h.snr.push(snr);
    // Best-of-receivers values for the latest history point
    let lt = js_round(last_t(u)) / 1000.0;
    let hist = &mut u.history;
    if hist.t.last() == Some(&lt) {
        let i = hist.t.len() - 1;
        hist.series[1][i] = if u.best_rssi.is_finite() { u.best_rssi } else { f64::NAN };
        hist.series[2][i] = if u.best_snr.is_finite() { u.best_snr } else { f64::NAN };
    }
}

fn push_history(cfg: &Config, u: &mut Unit, d: &Decoded, t: f64) {
    let h = &mut u.history;
    h.t.push(js_round(t) / 1000.0);
    h.series[0].push(d.values[COUNTER]);
    h.series[1].push(f64::NAN);
    h.series[2].push(f64::NAN);
    // Every field with a validity group, battery included; null when invalid
    for (k, (_, at, bit)) in GROUPED.iter().enumerate() {
        h.series[3 + k].push(if d.valid[*bit] { d.values[*at] } else { f64::NAN });
    }
    trim_history(cfg, u, false);
}

// How many points to drop off the front: everything older than the retention time,
// which is honoured exactly since it decides what the charts show, and whatever is past
// the point cap. The cap is only a memory guard, so ingest lets it drift by SLACK points
// before one cut instead of moving twenty thousand points on every packet; exact cuts on
// the spot.
fn cut(cfg: &Config, t: &[f64], exact: bool) -> usize {
    let mut n = 0;
    if cfg.retain_min > 0.0 && !t.is_empty() {
        let oldest = t[t.len() - 1] - cfg.retain_min * 60.0;
        while n < t.len() && t[n] < oldest {
            n += 1;
        }
    }
    let over = (t.len() - n) as f64 - cfg.max_points;
    if over > 0.0 && (exact || over >= SLACK as f64) {
        n += over as usize;
    }
    n
}

fn trim_history(cfg: &Config, u: &mut Unit, exact: bool) {
    let n = cut(cfg, &u.history.t, exact);
    if n > 0 {
        u.history.t.drain(..n);
        for s in &mut u.history.series {
            s.drain(..n.min(s.len()));
        }
    }
    for (_, h) in &mut u.rx_history {
        let n = cut(cfg, &h.t, exact);
        if n > 0 {
            h.t.drain(..n);
            h.rssi.drain(..n);
            h.snr.drain(..n);
        }
    }
}

fn nums(a: &[f64]) -> String {
    let v: Vec<String> = a.iter().map(|x| js_num(*x)).collect();
    format!("[{}]", v.join(","))
}

fn opt_num(x: Option<f64>) -> String {
    x.map(js_num).unwrap_or_else(|| "null".into())
}

fn opt_str(s: Option<&str>) -> String {
    s.map(|s| format!("\"{}\"", esc(s))).unwrap_or_else(|| "null".into())
}

impl Row<'_> {
    // The unit's own fields in fleet.js's order, history left out (Fleet::history has
    // it), then what rows() works out at the time asked
    pub fn fields(&self) -> String {
        let u = self.unit;
        let boot = match &u.boot {
            None => "\"boot\":null,".to_string(),
            Some(None) => String::new(),   // undefined, which JSON.stringify leaves out
            Some(Some(n)) => format!("\"boot\":\"{}\",", esc(n)),
        };
        let receptions: Vec<String> = u
            .receptions
            .iter()
            .map(|r| format!("{{\"rx\":\"{}\",\"rssi\":{},\"snr\":{}}}", esc(&r.rx), js_num(r.rssi), js_num(r.snr)))
            .collect();
        let last_valid: Vec<String> = u.last_valid.iter().map(|(k, v)| format!("\"{}\":{}", k, js_num(*v))).collect();
        let latest = match &u.latest {
            Some(d) => crate::decode::to_json(&Ok(d.clone())),
            None => "null".into(),
        };
        let gen = u.gen.map(|g| format!(",\"gen\":{}", g)).unwrap_or_default();
        let issues: Vec<String> = self.issues.iter().map(|i| format!("\"{}\"", esc(i))).collect();
        format!(
            concat!(
                "\"csid\":{},{}\"sourceRx\":{},\"fromChip\":{},\"label\":\"{}\",\"live\":{},\"firstT\":{},\"lastT\":{},",
                "\"packets\":{},\"missed\":{},\"resets\":{},\"repeats\":{},\"lastCounter\":{},\"gaps\":{},\"intervalMs\":{},",
                "\"latest\":{},\"latestSource\":{},\"receptions\":[{}],\"bestRssi\":{},\"bestSnr\":{},\"lastValid\":{{{}}},",
                "\"saturatedCount\":{}{},\"state\":\"{}\",\"age\":{},\"issues\":[{}],\"rxPercent\":{},\"staleAfterMs\":{},\"runs\":{}"
            ),
            u.csid,
            boot,
            opt_str(u.source_rx.as_deref()),
            u.from_chip,
            esc(&u.label),
            u.live,
            opt_num(u.first_t),
            opt_num(u.last_t),
            u.packets,
            u.missed,
            u.resets,
            u.repeats,
            opt_num(u.last_counter),
            nums(&u.gaps),
            js_num(u.interval_ms),
            latest,
            opt_str(u.latest_source),
            receptions.join(","),
            js_num(u.best_rssi),
            js_num(u.best_snr),
            last_valid.join(","),
            u.saturated_count,
            gen,
            self.state.name(),
            js_num(self.age),
            issues.join(","),
            js_num(self.rx_percent),
            js_num(self.stale_after_ms),
            self.runs
        )
    }

    pub fn to_json(&self) -> String {
        format!("{{{}}}", self.fields())
    }
}

fn receiver_fields(r: &Receiver) -> String {
    let source = r.source.as_ref().map(|s| format!(",\"source\":{}", info_json(s))).unwrap_or_default();
    format!(
        concat!(
            "\"key\":\"{}\",\"id\":{},\"info\":{},\"firstT\":{},\"lastT\":{},\"lastHbT\":{},\"packets\":{},\"unique\":{},",
            "\"badCrc\":{},\"radioErrors\":{},\"reportedOk\":{},\"reportedErrors\":{}{}"
        ),
        esc(&r.key),
        opt_str(r.id.as_deref()),
        info_json(&r.info),
        opt_num(r.first_t),
        opt_num(r.last_t),
        opt_num(r.last_hb_t),
        r.packets,
        r.unique,
        r.bad_crc,
        r.radio_errors,
        js_num(r.reported_ok),
        js_num(r.reported_errors),
        source
    )
}

impl Receiver {
    pub fn to_json(&self) -> String {
        format!("{{{}}}", receiver_fields(self))
    }
}

impl Summary {
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"heard\":{},\"active\":{},\"stale\":{},\"lost\":{},\"packets\":{},\"lowBattery\":{},",
                "\"validityIssues\":{},\"weakRf\":{},\"resets\":{},\"badCrc\":{},\"saturated\":{}}}"
            ),
            self.heard,
            self.active,
            self.stale,
            self.lost,
            self.packets,
            self.low_battery,
            self.validity_issues,
            self.weak_rf,
            self.resets,
            self.bad_crc,
            self.saturated
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::{log_events, merge_files, parse_line};

    // A packet with a good CRC, built as packet.js encode builds one: everything 0 but
    // these, battery 3.0 V
    fn pkt(csid: u8, counter: u16, rssi: &str, snr: &str, validity: u8, ax: f64) -> String {
        let mut v = [0.0; 24];
        v[BATTERY] = 3.0;
        v[21] = csid as f64;
        v[COUNTER] = counter as f64;
        v[VALIDITY] = validity as f64;
        v[4] = ax;
        let mut b = crate::decode::encode(&v, 0.0);
        let crc = crate::dump::crc16(&b[..53]);
        b[53..].copy_from_slice(&crc.to_le_bytes());
        format!("PKT,55,{},{},{},0", crate::decode::bytes_to_hex(&b), rssi, snr)
    }

    fn feed(f: &mut Fleet, rx: &str, line: &str, t: f64) -> &'static str {
        f.ingest(&parse_line(line), rx, t);
        f.last_result
    }

    fn labels(f: &Fleet) -> Vec<&str> {
        f.all_units().map(|u| u.label.as_str()).collect()
    }

    // Expected values in these tests are what fleet.js printed for the same lines and
    // times (node, the same packets built with packet.js encode), not worked out by hand.
    #[test]
    fn test_packets_are_the_ones_packet_js_builds() {
        assert_eq!(
            pkt(5, 65530, "-80", "9", 127, 0.0),
            "PKT,55,000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000FAFF05327F2698,-80,9,0"
        );
    }

    #[test]
    fn grouped_fields_sit_where_decode_puts_them() {
        let mut d = crate::decode::decode(&[0; 55]).unwrap();
        for (i, v) in d.values.iter_mut().enumerate() {
            *v = i as f64;
        }
        for (key, at, _) in GROUPED {
            assert_eq!(d.value(key), Some(at as f64), "{}", key);
        }
        assert_eq!(d.value("counter"), Some(COUNTER as f64));
        assert_eq!(d.value("battery"), Some(BATTERY as f64));
        assert_eq!(d.value("validity"), Some(VALIDITY as f64));
    }

    #[test]
    fn labels_go_a_to_z_then_aa() {
        let got: Vec<String> = [0, 1, 25, 26, 27, 701, 702].iter().map(|n| gen_label(*n)).collect();
        assert_eq!(got, ["a", "b", "z", "aa", "ab", "zz", "aaa"]);
    }

    #[test]
    fn a_wrap_is_one_run_and_a_drop_or_second_zero_is_a_new_one() {
        let counters = [65530, 65535, 3, 10, 2, 0, 0];
        let mut f = Fleet::new(Config::default());
        for (i, c) in counters.iter().enumerate() {
            assert_eq!(feed(&mut f, "rx1", &pkt(5, *c, "-80", "9", 127, 0.0), 2000.0 * i as f64), "new");
        }
        assert_eq!(f.last_retired.as_deref(), Some("5c"));
        assert_eq!(labels(&f), ["5", "5a", "5b", "5c"]);
        let a = f.unit("5a").unwrap();
        assert_eq!((a.packets, a.missed, a.gen), (4, 13, Some(0)));
        assert_eq!(a.gaps, [400.0, 500.0, 285.7142857142857]);
        assert_eq!(a.interval_ms, 400.0);
        assert_eq!(a.last_counter, Some(10.0));
        assert_eq!(f.rx_percent(a), 23.529411764705884);
        for l in ["5", "5b", "5c"] {
            let u = f.unit(l).unwrap();
            assert_eq!((u.packets, u.missed, f.rx_percent(u)), (1, 0, 100.0), "{}", l);
            assert!(u.interval_ms.is_nan());
        }

        // Not split: the same counters count as resets on one run
        let mut g = Fleet::new(Config { split_on_reset: false, ..Config::default() });
        for (i, c) in counters.iter().enumerate() {
            feed(&mut g, "rx1", &pkt(5, *c, "-80", "9", 127, 0.0), 2000.0 * i as f64);
        }
        assert_eq!(labels(&g), ["5"]);
        let u = g.unit("5").unwrap();
        assert_eq!((u.packets, u.missed, u.resets, u.repeats), (7, 13, 3, 0));
        assert_eq!(u.interval_ms, 400.0);

        // A second identical 0 inside the dedupe window is the same packet, not a reset
        let mut h = Fleet::new(Config::default());
        for (i, c) in counters.iter().enumerate() {
            feed(&mut h, "rx1", &pkt(5, *c, "-80", "9", 127, 0.0), 1000.0 * i as f64);
        }
        assert_eq!(h.last_result, "duplicate");
        assert_eq!(h.last_retired.as_deref(), Some("5b"));
        assert_eq!(h.unit("5").unwrap().receptions.len(), 2);
    }

    #[test]
    fn the_same_bytes_are_one_packet_up_to_dedupe_ms_and_a_new_one_after() {
        let mut f = Fleet::new(Config::default());
        let p = pkt(7, 1, "-90", "5", 127, 0.0);
        let got = [
            feed(&mut f, "rx1", &p, 0.0),
            feed(&mut f, "rx2", &pkt(7, 1, "-70", "3", 127, 0.0), 1500.0),
            feed(&mut f, "rx3", &p, 3001.0),
        ];
        assert_eq!(got, ["new", "duplicate", "new"]);
        assert_eq!((f.duplicates, f.total), (1, 2));
        let u = f.unit("7").unwrap();
        assert_eq!((u.packets, u.repeats), (2, 1));
        assert_eq!(u.receptions, [Reception { rx: "rx3".into(), rssi: -90.0, snr: 5.0 }]);
        // The first point keeps the best of both receivers
        assert_eq!(u.history.series[1], [-70.0, -90.0]);
        let rx: Vec<(&str, &[f64])> = u.rx_history.iter().map(|(k, h)| (k.as_str(), h.t.as_slice())).collect();
        assert_eq!(rx, [("rx1", &[0.0][..]), ("rx2", &[1.5][..]), ("rx3", &[3.001][..])]);
    }

    #[test]
    fn interval_is_the_median_of_the_last_15_gaps_per_step() {
        let gaps = [1000, 1100, 900, 2000, 1000, 1050, 950, 3000, 1000, 1000, 1200, 800, 1000, 1000, 1010, 990];
        let steps = [1, 1, 1, 2, 1, 1, 1, 3, 1, 1, 1, 1, 1, 1, 1, 1];
        let mut f = Fleet::new(Config::default());
        let (mut t, mut c) = (0.0, 0);
        feed(&mut f, "rx1", &pkt(9, c, "-80", "9", 127, 0.0), t);
        for (g, s) in gaps.iter().zip(steps) {
            t += *g as f64;
            c += s;
            feed(&mut f, "rx1", &pkt(9, c, "-80", "9", 127, 0.0), t);
        }
        assert_eq!(t, 19000.0);
        let u = f.unit("9").unwrap();
        assert_eq!((u.packets, u.missed), (17, 3));
        assert_eq!(u.gaps, [1100.0, 900.0, 1000.0, 1000.0, 1050.0, 950.0, 1000.0, 1000.0, 1000.0, 1200.0, 800.0, 1000.0, 1000.0, 1010.0, 990.0]);
        assert_eq!(u.interval_ms, 1000.0);
        assert_eq!(f.rx_percent(u), 85.0);
        assert_eq!(f.stale_after_ms(u), 5000.0);

        // Two gaps, an even count: the mean of the middle two
        let mut g = Fleet::new(Config::default());
        for (c, t) in [(0, 0.0), (1, 1000.0), (2, 1500.0)] {
            feed(&mut g, "rx1", &pkt(9, c, "-80", "9", 127, 0.0), t);
        }
        assert_eq!(g.unit("9").unwrap().interval_ms, 750.0);
    }

    #[test]
    fn stale_and_lost_start_just_past_their_thresholds() {
        let mut f = Fleet::new(Config::default());
        for (c, t) in [(0, 0.0), (1, 1000.0)] {
            feed(&mut f, "rx1", &pkt(9, c, "-80", "9", 127, 0.0), t);
        }
        let u = f.unit("9").unwrap();
        // 3 x 1000 ms is under the 5 s floor
        let got: Vec<&str> = [0.0, 3000.0, 3001.0, 5000.0, 5001.0, 600000.0, 600001.0]
            .iter()
            .map(|dt| f.state(u, 1000.0 + dt).name())
            .collect();
        assert_eq!(got, ["OK", "OK", "OK", "OK", "STALE", "STALE", "LOST"]);
        // Interval not known yet: 30 s
        let mut v = new_unit(1);
        v.last_t = Some(0.0);
        let got: Vec<&str> = [29999.0, 30000.0, 30001.0].iter().map(|t| f.state(&v, *t).name()).collect();
        assert_eq!(got, ["OK", "OK", "STALE"]);
    }

    #[test]
    fn a_chip_log_is_not_deduplicated_and_splits_on_its_boots() {
        let mut f = Fleet::new(Config::default());
        let p = pkt(9, 4, "", "", 0x3f, 80.0);
        let got = [
            feed(&mut f, "flash-boot0", "#DG,SRC,v1,kind=flash,boot=0", 0.0),
            feed(&mut f, "flash-boot0", "#DG,BOOT,v1,n=0", 0.0),
            feed(&mut f, "flash-boot0", &p, 10.0),
            feed(&mut f, "flash-boot0", &p, 20.0),
            feed(&mut f, "flash-boot0", "#DG,BOOT,v1,n=1", 30.0),
            feed(&mut f, "flash-boot0", &p, 40.0),
        ];
        assert_eq!(got, ["source", "boot", "new", "new", "boot", "new"]);
        assert_eq!(labels(&f), ["9", "9a"]);
        let (u, a) = (f.unit("9").unwrap(), f.unit("9a").unwrap());
        assert_eq!((u.packets, u.boot.clone()), (1, Some(Some("1".to_string()))));
        assert_eq!((a.packets, a.boot.clone(), a.gen), (2, Some(Some("0".to_string())), Some(0)));
        assert!(u.from_chip && u.best_rssi.is_nan());
        assert_eq!(f.issues(u), ["low battery", "Environment invalid", "accel saturated"]);
        assert_eq!(
            f.summary(40.0).to_json(),
            r#"{"heard":1,"active":1,"stale":0,"lost":0,"packets":3,"lowBattery":1,"validityIssues":1,"weakRf":0,"resets":0,"badCrc":0,"saturated":1}"#
        );
    }

    #[test]
    fn issues_come_in_fleet_js_order() {
        let mut f = Fleet::new(Config::default());
        feed(&mut f, "rx1", &pkt(3, 1, "-120", "-5", 0x20, 0.0), 0.0);
        let u = f.unit("3").unwrap();
        assert_eq!(
            f.issues(u),
            [
                "low battery",
                "Linear acceleration invalid",
                "Gyroscope invalid",
                "Magnetometer invalid",
                "Quaternion invalid",
                "GPS invalid",
                "Environment invalid",
                "weak RF"
            ]
        );
        assert_eq!(
            f.summary(0.0).to_json(),
            r#"{"heard":1,"active":1,"stale":0,"lost":0,"packets":1,"lowBattery":1,"validityIssues":1,"weakRf":1,"resets":0,"badCrc":0,"saturated":0}"#
        );
    }

    #[test]
    fn a_merged_run_goes_back_in_front_and_counts_its_reset() {
        let mut f = Fleet::new(Config::default());
        for (i, c) in [5, 6, 0, 1].iter().enumerate() {
            let rssi = (-80 - i as i32).to_string();
            feed(&mut f, if i < 2 { "rx1" } else { "rx2" }, &pkt(5, *c, &rssi, "9", 127, 0.0), 1000.0 * i as f64);
        }
        assert_eq!(f.merge_run("5a").as_deref(), Some("5"));
        let u = f.unit("5").unwrap();
        assert_eq!((u.packets, u.missed, u.resets), (4, 0, 1));
        assert_eq!(u.gaps, [1000.0]);
        assert_eq!(u.history.t, [0.0, 1.0, 2.0, 3.0]);
        let rx: Vec<&str> = u.rx_history.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(rx, ["rx2", "rx1"]);
        assert!(f.earlier_runs(5).is_empty());
    }

    #[test]
    fn ingest_says_which_rows_changed() {
        let mut f = Fleet::new(Config::default());
        let kinds = |u: &[Update]| u.iter().map(|u| u.to_json().split(',').next().unwrap().to_string()).collect::<Vec<_>>();
        let u = f.ingest(&parse_line(&pkt(5, 3, "-80", "9", 127, 0.0)), "rx1", 0.0);
        assert_eq!(kinds(&u), ["{\"kind\":\"unit\"", "{\"kind\":\"receiver\""]);
        assert!(u[0].to_json().starts_with("{\"kind\":\"unit\",\"csid\":5,\"boot\":null,"));
        let u = f.ingest(&parse_line(&pkt(5, 1, "-80", "9", 127, 0.0)), "rx1", 2000.0);
        assert_eq!(kinds(&u).len(), 3);
        assert!(u[0].to_json().contains("\"label\":\"5a\",\"live\":false"));
        let u = f.ingest(&parse_line("HB,9965,0,0"), "rx1", 3000.0);
        assert_eq!(kinds(&u), ["{\"kind\":\"receiver\""]);
        assert_eq!(f.last_result, "heartbeat");
    }

    // Not a fixture: the Rust half of the check against every real log, which
    // ground/tools/golden_fleet.sh runs and compares with fleet.js line by line.
    // DG_GOLDEN_LIST=<file of paths> DG_GOLDEN_FLEET_OUT=<out.jsonl> cargo test fleet::tests::golden
    #[test]
    fn golden() {
        use std::io::Write;
        let Ok(list) = std::env::var("DG_GOLDEN_LIST") else { return };
        let Ok(out) = std::env::var("DG_GOLDEN_FLEET_OUT") else { return };
        let list = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("{}: {}", list, e));
        let mut w = std::io::BufWriter::new(std::fs::File::create(&out).unwrap_or_else(|e| panic!("{}: {}", out, e)));
        const BASE_MS: f64 = 1793232000000.0;   // as golden_fleet.js
        let configs = [
            ("replay", Config { max_points: f64::INFINITY, retain_min: 0.0, ..Config::default() }),
            ("capped", Config { max_points: 2500.0, retain_min: 10.0, ..Config::default() }),
        ];
        for path in list.lines().filter(|l| !l.is_empty()) {
            let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
            let content = String::from_utf8_lossy(&bytes);
            let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy();
            let events = merge_files(vec![log_events(&content, &name, BASE_MS)]);
            if events.is_empty() {
                continue;
            }
            let n = events.len();
            let mut at: Vec<usize> = (1..=4).map(|k| n * k / 4).collect();
            at.dedup();
            at.retain(|x| *x > 0);
            for (cname, cfg) in &configs {
                writeln!(w, "{{\"file\":\"{}\",\"cfg\":\"{}\",\"events\":{}}}", esc(path), cname, n).unwrap();
                let mut f = Fleet::new(cfg.clone());
                let mut pos = 0;
                for (k, stop) in at.iter().enumerate() {
                    while pos < *stop {
                        let e = &events[pos];
                        pos += 1;
                        f.ingest(&parse_line(e.text), &e.rx, e.t);
                    }
                    if *cname == "capped" && k == 1 {
                        f.set_config(cfg.clone());
                    }
                    let t = events[pos - 1].t;
                    writeln!(w, "{{\"at\":{},\"t\":{}}}", pos, js_num(t)).unwrap();
                    for r in f.rows(t) {
                        writeln!(w, "{{\"row\":{}}}", r.to_json()).unwrap();
                    }
                    let attention: Vec<String> = f.rows_by(t, "attention").iter().map(|r| format!("\"{}\"", esc(&r.unit.label))).collect();
                    writeln!(w, "{{\"attention\":[{}]}}", attention.join(",")).unwrap();
                    for u in &f.retired {
                        writeln!(w, "{{\"retired\":{}}}", f.row(u, t).to_json()).unwrap();
                    }
                    for r in f.receivers(t) {
                        writeln!(w, "{{\"receiver\":{}}}", r.to_json()).unwrap();
                    }
                    writeln!(w, "{{\"summary\":{}}}", f.summary(t).to_json()).unwrap();
                    writeln!(
                        w,
                        "{{\"totals\":{{\"total\":{},\"duplicates\":{},\"badCrc\":{},\"badLength\":{},\"lastRetired\":{}}}}}",
                        f.total,
                        f.duplicates,
                        f.bad_crc,
                        f.bad_length,
                        opt_str(f.last_retired.as_deref())
                    )
                    .unwrap();
                }
                for u in f.all_units() {
                    let h = f.history(&u.label).unwrap();
                    writeln!(w, "{{\"label\":\"{}\",{}", esc(&u.label), &h[1..]).unwrap();
                }
            }
        }
        w.flush().unwrap();
    }
}
