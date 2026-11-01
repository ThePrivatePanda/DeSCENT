// What a page hears over its WebSocket: every receiver line through lines.rs and
// fleet.rs, sent on as line, packet, unit and receiver messages. The live stream has one
// Model and each replay its own, so a replay cannot move a live row.
// Every message carries t, the time it is about (receive time live, the log's time in a
// replay), and the socket adds seq. A page that connects gets a snapshot first, ending
// with snapshot-end.
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use crate::decode::{bytes_to_hex, js_num, Decoded};
use crate::fleet::{Fleet, Receiver, Unit, Update};
use crate::json::{esc, num_field, str_field};
use crate::lines::{info_json, parse_line, Kind};
use crate::serial::PortInfo;
use crate::ws::{Broadcast, Socket};

// Lines kept per receiver for a page that connects later, the same as the page keeps
pub const LINES_KEPT: usize = 200;
pub const SETTINGS_FILE: &str = "descent-ground-model.json";
// The live fleet's graph cap. A replay keeps every point, as Open log did.
pub const LIVE_MAX_POINTS: f64 = 20000.0;

// The thresholds the page shows in its drawer and the app keeps for every viewer. The
// names are the contract's (SPLIT-1.4.md 4); fleet.js had lostMin, staleFactor,
// staleMinS, saturationMps2 and dedupeMs for the same things in other units.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSettings {
    pub lost_after_ms: f64,      // silent this long = lost (ms)
    pub stale_intervals: f64,    // stale after this many missed intervals
    pub stale_min_ms: f64,       // and never sooner than this (ms)
    pub low_battery_v: f64,      // a lithium cell under load (V)
    pub weak_rssi_dbm: f64,      // best RSSI under this is weak RF (dBm)
    pub accel_sat_ms2: f64,      // BNO085 range is ±8 g, about 78 m/s²
    pub dedupe_window_ms: f64,   // same bytes within this = one transmission (ms)
}

const SETTING_NAMES: [&str; 7] =
    ["lostAfterMs", "staleIntervals", "staleMinMs", "lowBatteryV", "weakRssiDbm", "accelSatMs2", "dedupeWindowMs"];

impl Default for ModelSettings {
    // fleet.js DEFAULTS
    fn default() -> ModelSettings {
        ModelSettings {
            lost_after_ms: 600000.0,
            stale_intervals: 3.0,
            stale_min_ms: 5000.0,
            low_battery_v: 3.5,
            weak_rssi_dbm: -115.0,
            accel_sat_ms2: 75.0,
            dedupe_window_ms: 1500.0,
        }
    }
}

impl ModelSettings {
    fn values(&self) -> [f64; 7] {
        [
            self.lost_after_ms,
            self.stale_intervals,
            self.stale_min_ms,
            self.low_battery_v,
            self.weak_rssi_dbm,
            self.accel_sat_ms2,
            self.dedupe_window_ms,
        ]
    }

    fn from_values(v: [f64; 7]) -> ModelSettings {
        ModelSettings {
            lost_after_ms: v[0],
            stale_intervals: v[1],
            stale_min_ms: v[2],
            low_battery_v: v[3],
            weak_rssi_dbm: v[4],
            accel_sat_ms2: v[5],
            dedupe_window_ms: v[6],
        }
    }

    // The fields without braces, so a message can put kind and t in front
    pub fn fields(&self) -> String {
        let v: Vec<String> = SETTING_NAMES.iter().zip(self.values()).map(|(k, x)| format!("\"{}\":{}", k, js_num(x))).collect();
        v.join(",")
    }

    pub fn json(&self) -> String {
        format!("{{{}}}", self.fields())
    }

    // These with whichever keys body has. Only numbers: a key that is there with
    // anything else refuses the lot. Not checked for sense, they are the user's to set.
    pub fn merged(&self, body: &str) -> Result<ModelSettings, String> {
        let mut v = self.values();
        for (i, k) in SETTING_NAMES.iter().enumerate() {
            if let Some(x) = num_field(body, k) {
                if !x.is_finite() {
                    return Err(format!("{} has to be a number", k));
                }
                v[i] = x;
            }
        }
        Ok(ModelSettings::from_values(v))
    }

    pub fn config(&self, max_points: f64) -> crate::fleet::Config {
        crate::fleet::Config {
            lost_min: self.lost_after_ms / 60000.0,
            stale_factor: self.stale_intervals,
            stale_min_s: self.stale_min_ms / 1000.0,
            dedupe_ms: self.dedupe_window_ms,
            retain_min: 0.0,
            max_points,
            low_battery_v: self.low_battery_v,
            weak_rssi_dbm: self.weak_rssi_dbm,
            saturation_mps2: self.accel_sat_ms2,
            split_on_reset: true,
        }
    }

    // Beside the program with the other files this computer keeps. A file that is not
    // there or does not read gives the defaults; a key it lacks keeps its default.
    pub fn load(dir: &Path) -> ModelSettings {
        match std::fs::read_to_string(dir.join(SETTINGS_FILE)) {
            Ok(text) => ModelSettings::default().merged(&text).unwrap_or_default(),
            Err(_) => ModelSettings::default(),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, self.json() + "\n").map_err(|e| format!("{}: {}", path.display(), e))
    }
}

pub fn line_msg(rx: &str, t: f64, text: &str) -> String {
    format!("{{\"kind\":\"line\",\"rx\":\"{}\",\"t\":{},\"text\":\"{}\"}}", esc(rx), js_num(t), esc(text))
}

fn values_json(d: &Decoded) -> String {
    // decode::to_json has them in packet.js's order; cut them out rather than list them twice
    let all = crate::decode::to_json(&Ok(d.clone()));
    let a = all.find("\"values\":").unwrap();
    let b = all.find(",\"saturated\":").unwrap();
    all[a..b].to_string()
}

fn opt_str(s: Option<&str>) -> String {
    s.map(|s| format!("\"{}\"", esc(s))).unwrap_or_else(|| "null".into())
}

fn opt_num(x: Option<f64>) -> String {
    x.map(js_num).unwrap_or_else(|| "null".into())
}

#[derive(Clone)]
pub struct Model {
    pub fleet: Fleet,
    pub settings: ModelSettings,
    max_points: f64,
    sent: HashMap<String, &'static str>,                   // label -> state last sent
    lines: Vec<(String, VecDeque<(f64, String)>)>,         // rx -> its last lines
    formats: HashMap<String, &'static str>,                // rx -> raw or csv, by what it printed
    ports: HashMap<String, (Option<String>, String)>,      // rx -> (name, port status), live only
}

impl Model {
    pub fn new(settings: ModelSettings, max_points: f64) -> Model {
        Model {
            fleet: Fleet::new(settings.config(max_points)),
            settings,
            max_points,
            sent: HashMap::new(),
            lines: Vec::new(),
            formats: HashMap::new(),
            ports: HashMap::new(),
        }
    }

    // A model with nothing heard and the same settings, for a replay starting again
    pub fn fresh(&self) -> Model {
        Model::new(self.settings.clone(), self.max_points)
    }

    fn keep_line(&mut self, rx: &str, t: f64, text: &str) {
        let at = match self.lines.iter().position(|(k, _)| k == rx) {
            Some(i) => i,
            None => {
                self.lines.push((rx.to_string(), VecDeque::new()));
                self.lines.len() - 1
            }
        };
        let l = &mut self.lines[at].1;
        l.push_back((t, text.to_string()));
        if l.len() > LINES_KEPT {
            l.pop_front();
        }
    }

    fn last_line(&self, rx: &str) -> Option<f64> {
        self.lines.iter().find(|(k, _)| k == rx).and_then(|(_, l)| l.back()).map(|(t, _)| *t)
    }

    // One line off a receiver. Returns the messages it makes, in the order they go: the
    // line, the packet it held if any, the rows that changed, the receiver. With build
    // false the fleet takes it and nothing is written, for a replay seeking.
    // legacy gives the line message the 1.3 page's type key as well (ws::line_frame).
    pub fn line(&mut self, rx: &str, t: f64, text: &str, build: bool, legacy: bool) -> Vec<String> {
        // fleet.js files a line with no receiver under "rx"
        let key = if rx.is_empty() { "rx" } else { rx };
        self.keep_line(key, t, text);
        let ev = parse_line(text);
        match &ev.kind {
            Kind::Packet { source: "raw", .. } | Kind::Heartbeat { .. } | Kind::RxError { .. } | Kind::Device { kind: "rxinfo", .. } => {
                self.formats.insert(key.to_string(), "raw");
            }
            Kind::Packet { source: "csv", .. } => {
                self.formats.insert(key.to_string(), "csv");
            }
            _ => {}
        }
        let updates = self.fleet.ingest(&ev, rx, t);
        if !build {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.push(if legacy && t >= 0.0 && t.fract() == 0.0 {
            crate::ws::line_frame(key, t as u128, text)
        } else {
            line_msg(key, t, text)
        });
        if let Kind::Packet { source, bytes, decoded: Ok(d), rssi, snr, .. } = &ev.kind {
            let csid = d.csid();
            // A packet that failed its CRC is no unit's: its csid byte is as suspect as the rest
            let label = if d.crc_ok { format!("\"{}\"", csid) } else { "null".into() };
            out.push(format!(
                concat!(
                    "{{\"kind\":\"packet\",\"t\":{},\"csid\":{},\"label\":{},\"rx\":\"{}\",\"rssi\":{},\"snr\":{},",
                    "\"crcOk\":{},\"dup\":{},\"source\":\"{}\",\"hex\":\"{}\",{}}}"
                ),
                js_num(t),
                csid,
                label,
                esc(key),
                js_num(*rssi),
                js_num(*snr),
                d.crc_ok,
                self.fleet.last_result == "duplicate",
                source,
                bytes_to_hex(bytes),
                values_json(d)
            ));
        }
        for u in updates {
            if let Update::Unit(s) = u {
                self.mark_sent(&s);
                out.push(format!("{{\"t\":{},{}", js_num(t), &s[1..]));
            }
        }
        if let Some(m) = self.receiver_msg(key, t) {
            out.push(m);
        }
        out
    }

    fn mark_sent(&mut self, row: &str) {
        if let (Some(label), Some(state)) = (str_field(row, "label"), str_field(row, "state")) {
            let state = match state.as_str() {
                "LOST" => "LOST",
                "STALE" => "STALE",
                _ => "OK",
            };
            self.sent.insert(label, state);
        }
    }

    fn unit_msg(&self, u: &Unit, t: f64) -> String {
        format!("{{\"kind\":\"unit\",\"t\":{},{}}}", js_num(t), self.fleet.row(u, t).fields())
    }

    // The fleet.js receiver under the names the page reads (SPLIT-1.4.md 2): id is the
    // key packets and lines carry, boardId the id the board printed
    fn receiver_json(&self, r: &Receiver, t: f64) -> String {
        let info = |k: &str| r.info.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        let port = self.ports.get(&r.key);
        let name = port.and_then(|p| p.0.clone()).or_else(|| info("name").filter(|n| !n.is_empty()).map(String::from));
        let dups = r.packets.saturating_sub(r.unique + r.bad_crc);
        format!(
            concat!(
                "{{\"kind\":\"receiver\",\"t\":{},\"id\":\"{}\",\"boardId\":{},\"name\":{},\"sf\":{},\"state\":{},",
                "\"format\":{},\"info\":{},\"source\":{},\"firstT\":{},\"lastT\":{},\"lastHbT\":{},\"lastLine\":{},",
                "\"packets\":{},\"unique\":{},\"dups\":{},\"badCrc\":{},\"errors\":{},\"reportedOk\":{},\"reportedErrors\":{}}}"
            ),
            js_num(t),
            esc(&r.key),
            opt_str(r.id.as_deref()),
            opt_str(name.as_deref()),
            opt_num(r.sf()),
            opt_str(port.map(|p| p.1.as_str())),
            opt_str(self.formats.get(&r.key).copied()),
            info_json(&r.info),
            r.source.as_ref().map(info_json).unwrap_or_else(|| "null".into()),
            opt_num(r.first_t),
            opt_num(r.last_t),
            opt_num(r.last_hb_t),
            opt_num(self.last_line(&r.key)),
            r.packets,
            r.unique,
            dups,
            r.bad_crc,
            r.radio_errors,
            js_num(r.reported_ok),
            js_num(r.reported_errors)
        )
    }

    fn receiver_msg(&self, key: &str, t: f64) -> Option<String> {
        self.fleet.receivers(t).iter().find(|r| r.key == key).map(|r| self.receiver_json(r, t))
    }

    // Rows whose state has moved with time alone: stale and lost come from silence, so
    // nothing arrives to say so
    pub fn tick(&mut self, t: f64) -> Vec<String> {
        let mut out = Vec::new();
        let mut moved = Vec::new();
        for u in self.fleet.all_units() {
            let state = self.fleet.state(u, t).name();
            if self.sent.get(&u.label).copied() != Some(state) {
                moved.push((u.label.clone(), state));
                out.push(self.unit_msg(u, t));
            }
        }
        for (label, state) in moved {
            self.sent.insert(label, state);
        }
        out
    }

    pub fn settings_msg(&self, t: f64) -> String {
        format!("{{\"kind\":\"settings\",\"t\":{},{}}}", js_num(t), self.settings.fields())
    }

    // Everything a page needs to draw what it would have seen: the settings, every run,
    // every receiver, the last lines of each, then snapshot-end
    pub fn snapshot(&mut self, t: f64) -> Vec<String> {
        let mut out = vec![self.settings_msg(t)];
        let mut sent = HashMap::new();
        for u in self.fleet.all_units() {
            sent.insert(u.label.clone(), self.fleet.state(u, t).name());
            out.push(self.unit_msg(u, t));
        }
        self.sent = sent;
        for r in self.fleet.receivers(t) {
            out.push(self.receiver_json(r, t));
        }
        let mut lines: Vec<(f64, &str, &str)> =
            self.lines.iter().flat_map(|(rx, l)| l.iter().map(move |(lt, text)| (*lt, rx.as_str(), text.as_str()))).collect();
        lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        out.extend(lines.into_iter().map(|(lt, rx, text)| line_msg(rx, lt, text)));
        out.push(format!("{{\"kind\":\"snapshot-end\",\"t\":{}}}", js_num(t)));
        out
    }

    // New settings: the settings message, then every row, since state and issues are
    // worked out from them
    pub fn set_settings(&mut self, s: ModelSettings, t: f64) -> Vec<String> {
        self.fleet.set_config(s.config(self.max_points));
        self.settings = s;
        let mut out = vec![self.settings_msg(t)];
        let mut sent = HashMap::new();
        for u in self.fleet.all_units() {
            sent.insert(u.label.clone(), self.fleet.state(u, t).name());
            out.push(self.unit_msg(u, t));
        }
        self.sent = sent;
        out
    }

    // Names and port states off the port list; the receivers that changed go out again
    pub fn set_ports(&mut self, ports: &[PortInfo], t: f64) -> Vec<String> {
        let mut now = HashMap::new();
        for p in ports {
            let name = p.name.clone().or_else(|| p.rx_name.clone()).filter(|n| !n.is_empty());
            now.insert(p.key.clone(), (name, p.status.clone()));
        }
        let changed: Vec<String> = self
            .fleet
            .receivers(t)
            .iter()
            .filter(|r| now.get(&r.key) != self.ports.get(&r.key))
            .map(|r| r.key.clone())
            .collect();
        self.ports = now;
        changed.iter().filter_map(|k| self.receiver_msg(k, t)).collect()
    }

    pub fn history(&self, label: &str) -> Option<String> {
        self.fleet.history_flat(label)
    }

    pub fn packets(&self) -> u64 {
        self.fleet.total
    }

    #[cfg(test)]
    pub fn sent_state(&self, label: &str) -> Option<&'static str> {
        self.sent.get(label).copied()
    }
}

// A Model and the sockets that hear it. Messages are sent while the model is locked, so
// a snapshot and the messages after it cannot cross.
pub struct Stream {
    model: Mutex<Model>,
    pub out: Broadcast,
}

impl Stream {
    pub fn new(model: Model) -> Stream {
        Stream::with_out(model, Broadcast::new())
    }

    // Live shares its sockets with the ports and flash frames, which go out on their own
    pub fn with_out(model: Model, out: Broadcast) -> Stream {
        Stream { model: Mutex::new(model), out }
    }

    fn send(&self, msgs: &[String]) {
        for m in msgs {
            self.out.send_seq(m);
        }
    }

    pub fn line(&self, rx: &str, t: f64, text: &str, legacy: bool) {
        let mut m = self.model.lock().unwrap();
        let msgs = m.line(rx, t, text, true, legacy);
        self.send(&msgs);
    }

    pub fn tick(&self, t: f64) {
        let mut m = self.model.lock().unwrap();
        let msgs = m.tick(t);
        self.send(&msgs);
    }

    pub fn connect(&self, socket: Socket, t: f64) {
        let mut m = self.model.lock().unwrap();
        let first = m.snapshot(t);
        self.out.add_with(socket, &first);
    }

    // A replay that moved: reset, then the snapshot from where it is now
    pub fn resnap(&self, t: f64) {
        let mut m = self.model.lock().unwrap();
        let mut msgs = vec![format!("{{\"kind\":\"reset\",\"t\":{}}}", js_num(t))];
        msgs.extend(m.snapshot(t));
        self.send(&msgs);
    }

    pub fn set_settings(&self, s: ModelSettings, t: f64) {
        let mut m = self.model.lock().unwrap();
        let msgs = m.set_settings(s, t);
        self.send(&msgs);
    }

    pub fn set_ports(&self, ports: &[PortInfo], t: f64) {
        let mut m = self.model.lock().unwrap();
        let msgs = m.set_ports(ports, t);
        self.send(&msgs);
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Model) -> R) -> R {
        f(&mut self.model.lock().unwrap())
    }
}

// A packet as fleet.rs tests build one; bad breaks its CRC
#[cfg(test)]
pub fn test_packet(csid: u8, counter: u16, bad: bool) -> String {
    let mut values = [0.0; 24];
    values[16] = 1.0;    // qr
    values[20] = counter as f64;
    values[21] = csid as f64;
    values[22] = 4.0;    // battery
    values[23] = 127.0;  // validity
    let mut b = crate::decode::encode(&values, 0.0);
    let crc = crate::dump::crc16(&b[..53]) ^ bad as u16;
    b[53..].copy_from_slice(&crc.to_le_bytes());
    format!("PKT,55,{},-80.0,9.5,10", bytes_to_hex(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_packet as packet_line;

    fn pkt(csid: u8, counter: u16) -> String {
        packet_line(csid, counter, false)
    }

    fn kinds(msgs: &[String]) -> Vec<String> {
        msgs.iter().map(|m| str_field(m, "kind").unwrap()).collect()
    }

    #[test]
    fn a_packet_line_is_a_line_a_packet_a_unit_and_its_receiver() {
        let mut m = Model::new(ModelSettings::default(), LIVE_MAX_POINTS);
        let msgs = m.line("rx1", 1000.0, &pkt(9, 1), true, false);
        assert_eq!(kinds(&msgs), ["line", "packet", "unit", "receiver"]);
        let p = &msgs[1];
        assert!(p.contains("\"csid\":9,\"label\":\"9\",\"rx\":\"rx1\",\"rssi\":-80,\"snr\":9.5,\"crcOk\":true,\"dup\":false"), "{}", p);
        assert!(p.contains("\"values\":{\"lat\":0,") && p.contains("\"valid\":{\"accel\":true,"), "{}", p);
        assert!(msgs[2].starts_with("{\"t\":1000,\"kind\":\"unit\",\"csid\":9,"), "{}", msgs[2]);
        assert!(msgs[2].contains("\"state\":\"OK\",") && msgs[2].contains("\"staleAfterMs\":30000"), "{}", msgs[2]);
        let r = &msgs[3];
        assert!(r.contains("\"id\":\"rx1\",\"boardId\":null") && r.contains("\"format\":\"raw\""), "{}", r);
        assert!(r.contains("\"lastLine\":1000,\"packets\":1,\"unique\":1,\"dups\":0,\"badCrc\":0,\"errors\":0"), "{}", r);

        // the same bytes off a second receiver inside the window
        let msgs = m.line("rx2", 1200.0, &pkt(9, 1), true, false);
        assert!(msgs[1].contains("\"dup\":true"), "{}", msgs[1]);
        assert!(msgs[3].contains("\"packets\":1,\"unique\":0,\"dups\":1"), "{}", msgs[3]);
    }

    #[test]
    fn a_bad_crc_packet_goes_out_with_no_label() {
        let mut m = Model::new(ModelSettings::default(), LIVE_MAX_POINTS);
        let bad = packet_line(9, 1, true);
        let msgs = m.line("rx1", 1000.0, &bad, true, false);
        assert_eq!(kinds(&msgs), ["line", "packet", "receiver"]);
        assert!(msgs[1].contains("\"label\":null") && msgs[1].contains("\"crcOk\":false"), "{}", msgs[1]);
        assert!(msgs[2].contains("\"badCrc\":1"), "{}", msgs[2]);
    }

    #[test]
    fn a_unit_goes_out_again_when_it_goes_stale_and_lost_and_not_between() {
        let mut m = Model::new(ModelSettings::default(), LIVE_MAX_POINTS);
        m.line("rx1", 0.0, &pkt(9, 1), true, false);
        assert_eq!(m.sent_state("9"), Some("OK"));
        assert!(m.tick(1000.0).is_empty());
        // 30 s before the interval is known
        assert!(m.tick(30000.0).is_empty());
        let stale = m.tick(30001.0);
        assert_eq!(stale.len(), 1);
        assert!(stale[0].contains("\"state\":\"STALE\""), "{}", stale[0]);
        assert!(m.tick(60000.0).is_empty());
        let lost = m.tick(600001.0);
        assert!(lost[0].contains("\"state\":\"LOST\""), "{}", lost[0]);
        assert!(m.tick(700000.0).is_empty());
    }

    #[test]
    fn settings_read_partial_and_refuse_what_is_not_a_number() {
        let s = ModelSettings::default().merged("{\"lostAfterMs\":120000}").unwrap();
        assert_eq!(s.lost_after_ms, 120000.0);
        assert_eq!(s.stale_min_ms, 5000.0);
        assert!(ModelSettings::default().merged("{\"weakRssiDbm\":\"low\"}").is_err());
        assert_eq!(
            ModelSettings::default().json(),
            "{\"lostAfterMs\":600000,\"staleIntervals\":3,\"staleMinMs\":5000,\"lowBatteryV\":3.5,\"weakRssiDbm\":-115,\"accelSatMs2\":75,\"dedupeWindowMs\":1500}"
        );
        let c = s.config(LIVE_MAX_POINTS);
        assert_eq!((c.lost_min, c.stale_min_s, c.max_points), (2.0, 5.0, 20000.0));
    }

    #[test]
    fn a_snapshot_is_settings_units_receivers_lines_then_the_end() {
        let mut m = Model::new(ModelSettings::default(), LIVE_MAX_POINTS);
        m.line("rx1", 0.0, "HB,1000,0,0", true, false);
        m.line("rx1", 10.0, &pkt(9, 1), true, false);
        m.line("rx2", 20.0, &pkt(13, 5), true, false);
        let s = m.snapshot(100.0);
        assert_eq!(kinds(&s), ["settings", "unit", "unit", "receiver", "receiver", "line", "line", "line", "snapshot-end"]);
        assert!(s[5].contains("\"text\":\"HB,1000,0,0\""), "{}", s[5]);
        // no type key in a snapshot line: the 1.3 page would take it as a new line
        assert!(!s[5].contains("\"type\""));
    }

    #[test]
    fn history_is_flat_with_the_receivers_beside() {
        let mut m = Model::new(ModelSettings::default(), LIVE_MAX_POINTS);
        m.line("rx1", 1000.0, &pkt(9, 1), true, false);
        let h = m.history("9").unwrap();
        assert!(h.starts_with("{\"t\":[1],\"counter\":[1],\"rssi\":[-80],"), "{}", h);
        assert!(h.ends_with(",\"rxHistory\":{\"rx1\":{\"t\":[1],\"rssi\":[-80],\"snr\":[9.5]}}}"), "{}", h);
        assert!(m.history("12").is_none());
    }
}
