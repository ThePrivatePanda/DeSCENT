// Receiver lines through the live path, checked against a replay of the log that path
// wrote. Bytes go into one Framer per receiver as a reader thread's reads would, each
// line goes through take_line as the main loop sends it, and what the live stream
// broadcast is read back off an in-memory socket. Replay is the reference: it is
// checked against fleet.js already.
// The real logs are only read when asked:
// DG_LIVE_LIST=<file of log paths> cargo test live_path::tests::real_logs -- --nocapture
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tiny_http::ReadWrite;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::framing::Framer;
use crate::json::str_field;
use crate::logfile::Log;
use crate::replay::Replay;
use crate::serial::PortInfo;
use crate::stream::{Model, ModelSettings, Stream, LIVE_MAX_POINTS};
use crate::ws::Socket;

// What a serial read hands over at most (serial.rs reads into 4096 bytes)
const READ: usize = 4096;

// Takes whatever is written and never has anything to read
#[derive(Clone)]
struct Wire(Arc<Mutex<Vec<u8>>>);

impl Read for Wire {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::WouldBlock.into())
    }
}

impl Write for Wire {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wire() -> (Wire, Socket) {
    let w = Wire(Arc::new(Mutex::new(Vec::new())));
    let rw: Box<dyn ReadWrite + Send> = Box::new(w.clone());
    (w.clone(), WebSocket::from_raw_socket(rw, Role::Server, None))
}

// Every message on the wire since last time, read as a page would, with seq taken off.
// seq has to go up by one from where the last read left it.
fn drain(w: &Wire, seq: &mut u64) -> Vec<String> {
    let bytes = std::mem::take(&mut *w.0.lock().unwrap());
    let mut ws = WebSocket::from_raw_socket(io::Cursor::new(bytes), Role::Client, None);
    let mut out = Vec::new();
    while let Ok(m) = ws.read() {
        if let Message::Text(t) = m {
            let t = t.as_str();
            let pre = format!("{{\"seq\":{},", *seq);
            assert!(t.starts_with(&pre), "expected seq {} on {}", *seq, t.chars().take(80).collect::<String>());
            *seq += 1;
            out.push(format!("{{{}", &t[pre.len()..]));
        }
    }
    out
}

// The live line message carries the 1.3 page's type key as well, and a live receiver
// the state of its port, which a replay has no ports to give (SPLIT-1.4.md 2)
fn unlegacy(m: String) -> String {
    let m = match m.strip_prefix("{\"type\":\"line\",") {
        Some(rest) => format!("{{{}", rest),
        None => m,
    };
    m.replacen(",\"state\":\"open\",\"format\":", ",\"state\":null,\"format\":", 1)
}

fn port(key: &str, status: &str) -> PortInfo {
    PortInfo {
        key: key.into(),
        port: format!("/dev/fake-{}", key),
        usb: "10c4:ea60".into(),
        status: status.into(),
        error: None,
        rx_id: None,
        rx_name: None,
        rx_fw: None,
        name: None,
    }
}

// One line as a receiver sent it: the bytes before its line ending
pub struct Sent {
    pub t: u128,
    pub rx: String,
    pub raw: Vec<u8>,
}

#[derive(Default)]
struct Counts {
    split: usize,      // lines that came in more than one read
    early: usize,      // lines begun before another receiver's line ended
    crlf: usize,
    closed: usize,     // ports closed mid-line and opened again
    runaway: usize,    // lines over the framer's limit, which must not surface
}

struct Live {
    msgs: Vec<String>,     // everything broadcast, in order, ports messages left out
    snapshot: Vec<String>, // what a page connecting at the end is sent
    log: PathBuf,          // the log take_line wrote
    counts: Counts,
}

// Feeds bytes in reads of at most READ, and returns the lines the framer gives up
fn reads(f: &mut Framer, b: &[u8]) -> Vec<String> {
    b.chunks(READ).flat_map(|c| f.push(c)).collect()
}

fn run_live(sent: &[Sent], data: &Path, disturb: bool) -> Live {
    let (log, warn) = Log::create(data);
    assert!(warn.is_none(), "{:?}", warn);
    let log_path = log.as_ref().unwrap().path().to_path_buf();
    let log = Mutex::new(log);
    let live = Stream::new(Model::new(ModelSettings::default(), LIVE_MAX_POINTS));
    let (w, socket) = wire();
    let mut seq = 0;
    live.connect(socket, 0.0);
    drain(&w, &mut seq);

    let mut framers: HashMap<String, Framer> = HashMap::new();
    let mut started: HashMap<usize, usize> = HashMap::new();   // line -> bytes already in its framer
    let mut msgs = Vec::new();
    let mut c = Counts::default();
    for (i, s) in sent.iter().enumerate() {
        // CRLF as Serial.println sends it, plain LF every other line
        let mut bytes = s.raw.clone();
        if i % 2 == 0 {
            bytes.extend_from_slice(b"\r\n");
            c.crlf += 1;
        } else {
            bytes.push(b'\n');
        }
        let f = framers.entry(s.rx.clone()).or_insert_with(Framer::new);
        let from = started.remove(&i).unwrap_or(0);
        let rest = &bytes[from..];
        // Somewhere inside the line, a different place each time
        let cut = if from == 0 { (i * 37) % rest.len() } else { 0 };
        let mut got = reads(f, &rest[..cut]);
        assert!(got.is_empty(), "line {} came out before its newline", i);
        got = reads(f, &rest[cut..]);
        if from > 0 || cut > 0 || rest.len() > READ {
            c.split += 1;
        }
        assert_eq!(got.len(), 1, "line {} gave {} lines", i, got.len());
        for text in got {
            assert_eq!(text, String::from_utf8_lossy(&s.raw).trim_end_matches('\r'), "line {}", i);
            crate::take_line(&log, &live, &s.rx, s.t, &text);
        }

        if disturb && i % 613 == 300 {
            // Longer than the framer keeps: dropped whole, and the next line still whole
            let mut junk = vec![b'x'; 9000];
            junk.extend_from_slice(b"\r\n");
            assert!(reads(f, &junk).is_empty());
            c.runaway += 1;
        }
        if disturb && i % 997 == 500 {
            // Unplugged mid-line: the reader thread ends with half a line in its framer
            // and the port comes back with a new thread and a new framer
            assert!(reads(f, b"PKT,55,00FF00").is_empty());
            framers.insert(s.rx.clone(), Framer::new());
            msgs.extend(drain(&w, &mut seq).into_iter().map(unlegacy));
            live.set_ports(&[port(&s.rx, "error")], s.t as f64);
            live.set_ports(&[port(&s.rx, "open")], s.t as f64);
            drain(&w, &mut seq);
            c.closed += 1;
        }

        // Half of the next line already in, if it is another receiver's
        if let Some(n) = sent.get(i + 1) {
            if n.rx != s.rx && n.raw.len() >= 2 {
                let half = n.raw.len() / 2;
                let nf = framers.entry(n.rx.clone()).or_insert_with(Framer::new);
                assert!(reads(nf, &n.raw[..half]).is_empty());
                started.insert(i + 1, half);
                c.early += 1;
            }
        }
    }
    let end = sent.last().map(|s| s.t as f64).unwrap_or(0.0);
    if disturb {
        // Live rows carry the port state; with no ports they read as a replay's do
        msgs.extend(drain(&w, &mut seq).into_iter().map(unlegacy));
        live.set_ports(&[], end);
        drain(&w, &mut seq);
    }
    live.tick(end);
    msgs.extend(drain(&w, &mut seq).into_iter().map(unlegacy));
    if let Some(l) = log.lock().unwrap().as_mut() {
        l.flush();
    }

    let (w2, socket2) = wire();
    live.connect(socket2, end);
    let snapshot = drain(&w2, &mut 0).into_iter().map(unlegacy).collect();
    Live { msgs, snapshot, log: log_path, counts: c }
}

struct Replayed {
    msgs: Vec<String>,
    snapshot: Vec<String>,
    lines: usize,
}

// The log opened as the page opens one, a page connected, then played from the start
// to the end in one tick
fn run_replay(log: &Path) -> Replayed {
    let bytes = std::fs::read(log).unwrap();
    let name = log.file_name().unwrap().to_string_lossy().into_owned();
    let r = Replay::new("1".into(), vec![(name, bytes)], ModelSettings::default(), 0.0).unwrap();
    let (w, socket) = wire();
    let mut seq = 0;
    r.connect(socket);
    let snapshot = drain(&w, &mut seq);
    r.seek(0);
    let reset = drain(&w, &mut seq);
    assert_eq!(str_field(&reset[0], "kind").as_deref(), Some("reset"));
    r.set_speed(1e15).unwrap();
    r.set_playing(true);
    r.step();
    let msgs = drain(&w, &mut seq);
    let state = r.state_json();
    assert!(state.contains(&format!("\"position\":{},\"lines\":{},", r.lines.len(), r.lines.len())), "{}", state);
    Replayed { msgs, snapshot, lines: r.lines.len() }
}

fn kind(m: &str) -> String {
    str_field(m, "kind").unwrap_or_default()
}

fn count(msgs: &[String], k: &str) -> usize {
    msgs.iter().filter(|m| kind(m) == k).count()
}

fn first_difference(a: &[String], b: &[String]) -> Option<String> {
    let i = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i))?;
    Some(format!("message {} of {} live, {} replay\nlive:   {:?}\nreplay: {:?}", i, a.len(), b.len(), a.get(i), b.get(i)))
}

// Runs one set of lines both ways and says what differs. Returns the live run for
// the counts, and the lines of the log it wrote.
fn check(sent: &[Sent], what: &str, disturb: bool) -> (Live, Replayed, Vec<Sent>) {
    let data = std::env::temp_dir().join(format!("dg-live-{}-{}", std::process::id(), what));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let live = run_live(sent, &data, disturb);
    let replay = run_replay(&live.log);
    let written = sent_from_log(live.log.to_str().unwrap());
    let _ = std::fs::remove_dir_all(&data);

    assert_eq!(replay.lines, sent.len(), "{}: lines in the log", what);
    if let Some(d) = first_difference(&live.msgs, &replay.msgs) {
        panic!("{}: live and replay streams differ at {}", what, d);
    }
    let keep = |s: &[String]| -> Vec<String> {
        s.iter().filter(|m| matches!(kind(m).as_str(), "unit" | "receiver")).cloned().collect()
    };
    if let Some(d) = first_difference(&keep(&live.snapshot), &keep(&replay.snapshot)) {
        panic!("{}: final rows differ at {}", what, d);
    }
    // the whole snapshot, settings and kept lines too
    if let Some(d) = first_difference(&live.snapshot, &replay.snapshot) {
        panic!("{}: snapshots differ at {}", what, d);
    }
    (live, replay, written)
}

fn report(what: &str, live: &Live) {
    let c = &live.counts;
    println!(
        "{}: {} messages equal ({} line, {} packet, {} unit, {} receiver); final {} units, {} receivers equal; \
         {} split reads, {} begun early, {} CRLF, {} closed mid-line, {} runaway lines dropped",
        what,
        live.msgs.len(),
        count(&live.msgs, "line"),
        count(&live.msgs, "packet"),
        count(&live.msgs, "unit"),
        count(&live.msgs, "receiver"),
        count(&live.snapshot, "unit"),
        count(&live.snapshot, "receiver"),
        c.split,
        c.early,
        c.crlf,
        c.closed,
        c.runaway
    );
}

// The lines of a log as take_line would have been given them
fn sent_from_log(path: &str) -> Vec<Sent> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("# descent-ground log v1"), "{} is not an app log", path);
    text.split('\n')
        .skip(1)
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            let (t, rx, line) = (p.next()?, p.next()?, p.next()?);
            Some(Sent { t: t.parse().unwrap_or_else(|_| panic!("{}: time {:?}", path, t)), rx: rx.into(), raw: line.as_bytes().to_vec() })
        })
        .collect()
}

fn sent(t: u128, rx: &str, raw: &[u8]) -> Sent {
    Sent { t, rx: rx.into(), raw: raw.to_vec() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::test_packet;

    const T0: u128 = 1_790_000_000_000;

    // Two receivers hearing the same packets, with what a T-Beam port really gives:
    // a boot header, blank lines, boot noise that is not UTF-8, a bad CRC, a long line
    #[test]
    fn two_receivers_through_the_live_path_are_what_their_log_replays() {
        let mut s = vec![
            sent(T0, "rx1", b"#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=01,fw=1.2.1"),
            sent(T0 + 5, "rx2", &[0xff, 0xfe, b'o', b'k', 0]),
            sent(T0 + 6, "rx2", b""),
        ];
        let long = format!("#DG,DUMP,{}", "0123456789ABCDEF".repeat(400));   // 6409 bytes, two reads
        s.push(sent(T0 + 7, "rx2", long.as_bytes()));
        for n in 0..1500u16 {
            let t = T0 + 1000 + n as u128 * 500;
            let p = test_packet(9 + (n % 2) as u8 * 4, n / 2, n % 50 == 7);
            s.push(sent(t, "rx1", p.as_bytes()));
            if n % 3 != 0 {
                s.push(sent(t + 40, "rx2", p.as_bytes()));
            }
            if n % 20 == 0 {
                s.push(sent(t + 41, "rx1", format!("HB,{},{},0", n, n / 2).as_bytes()));
            }
        }
        let (live, _, _) = check(&s, "two", true);
        report("two receivers", &live);
        let c = &live.counts;
        assert!(c.split > 1000 && c.early > 500 && c.closed >= 2 && c.runaway >= 4);
    }

    // A receiver line can hold a tab or a lone CR mid-line. The log has no room for
    // either, so a replay of it reads spaces; the live stream has to as well.
    #[test]
    fn a_tab_or_a_lone_cr_inside_a_line_reads_the_same_live_and_replayed() {
        let s = vec![
            sent(T0, "rx1", b"HB,1000,0,0\tafter a tab"),
            sent(T0 + 1, "rx1", b"#boot\rnoise"),
            sent(T0 + 2, "rx1", format!("{}\tx", test_packet(9, 1, false)).as_bytes()),
        ];
        check(&s, "tabs", false);
    }

    #[test]
    fn real_logs() {
        let Ok(list) = std::env::var("DG_LIVE_LIST") else { return };
        let list = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("{}: {}", list, e));
        for path in list.lines().filter(|l| !l.is_empty()) {
            let s = sent_from_log(path);
            // a log of only its header line: replay won't open it, as the page never did
            if s.is_empty() {
                println!("{}: no lines, skipped", path);
                continue;
            }
            let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
            let (live, _, again) = check(&s, &name, true);
            // and the log take_line wrote is the one it was given, line for line
            assert_eq!(again.len(), s.len(), "{}", path);
            for (a, b) in again.iter().zip(&s) {
                assert!(a.t == b.t && a.rx == b.rx && a.raw == b.raw, "{}: {:?}", path, String::from_utf8_lossy(&b.raw));
            }
            report(path, &live);
        }
    }
}
