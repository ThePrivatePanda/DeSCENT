// The built-in SDR: what ground/sdr/sdr_rx.py does, inside the app, so that a release needs no
// Python. rtl_sdr runs at 1 MS/s tuned 250 kHz above 915.0 MHz, which keeps the dongle's DC notch
// and its spur at centre+190 kHz out of the channel. Here the channel is moved to 0 Hz, cut to
// 250 kS/s and written to one rxx_lora file_lora per spreading factor on its stdin, and what they
// decode is printed as the receiver lines of docs/sdr-receiver.md, exactly as sdr_rx.py prints them.
//
// rtl_sdr and file_lora are the bundle the release carries (ground/sdr/build-tools.sh, embedded by
// build.rs), unpacked into the data folder the first time the SDR is turned on. They are started
// directly, never through a shell. rtl_sdr gets a process group of its own so command.rs can end
// it alone; the decoders then see the end of their stdin and finish. If the app dies, rtl_sdr's
// next write fails and it exits, and the decoders follow it the same way.
use crate::json::{bool_field, int_field, num_field, str_field};
use nusb::MaybeFuture;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/sdr_bundle.rs"));

const SFS: [u32; 3] = [10, 11, 12];
const CHANNEL_HZ: u32 = 915_000_000;
const RATE: f64 = 1_000_000.0;
const CHUNK: usize = 1 << 18;   // bytes of u8 IQ, 131 ms
const TAPS: usize = 64;
const QUEUED: usize = 64;       // chunks waiting for one decoder, 8 s

// The RTL2832U sticks rtl-sdr 2.0.3 knows (known_devices in librtlsdr.c), by USB vendor and product.
const DONGLES: &[(u16, u16)] = &[
    (0x0bda, 0x2832), (0x0bda, 0x2838), (0x0413, 0x6680), (0x0413, 0x6f0f), (0x0458, 0x707f),
    (0x0ccd, 0x00a9), (0x0ccd, 0x00b3), (0x0ccd, 0x00b4), (0x0ccd, 0x00b5), (0x0ccd, 0x00b7),
    (0x0ccd, 0x00b8), (0x0ccd, 0x00b9), (0x0ccd, 0x00c0), (0x0ccd, 0x00c6), (0x0ccd, 0x00d3),
    (0x0ccd, 0x00d7), (0x0ccd, 0x00e0), (0x1554, 0x5020), (0x15f4, 0x0131), (0x15f4, 0x0133),
    (0x185b, 0x0620), (0x185b, 0x0650), (0x185b, 0x0680), (0x1b80, 0xd393), (0x1b80, 0xd394),
    (0x1b80, 0xd395), (0x1b80, 0xd397), (0x1b80, 0xd398), (0x1b80, 0xd39d), (0x1b80, 0xd3a4),
    (0x1b80, 0xd3a8), (0x1b80, 0xd3af), (0x1b80, 0xd3b0), (0x1d19, 0x1101), (0x1d19, 0x1102),
    (0x1d19, 0x1103), (0x1d19, 0x1104), (0x1f4d, 0xa803), (0x1f4d, 0xb803), (0x1f4d, 0xc803),
    (0x1f4d, 0xd286), (0x1f4d, 0xd803),
];

pub const NO_DONGLE: &str = "No RTL-SDR plugged in";
const ZADIG: &str = "Windows has no WinUSB driver on the RTL-SDR. Install it once with Zadig (zadig.akeo.ie): \
Options, List All Devices, pick \"Bulk-In, Interface (Interface 0)\", WinUSB, Replace Driver; then unplug the RTL-SDR and plug it back in";
const UDEV: &str = "This user may not open the RTL-SDR. Once, run: echo 'SUBSYSTEM==\"usb\", ATTRS{idVendor}==\"0bda\", \
ATTRS{idProduct}==\"2838\", MODE=\"0666\"' | sudo tee /etc/udev/rules.d/20-rtlsdr.rules; then unplug the RTL-SDR and plug it back in";
const BLACKLIST: &str = "The kernel's TV driver holds the RTL-SDR. Once, run: echo blacklist dvb_usb_rtl28xxu | \
sudo tee /etc/modprobe.d/blacklist-rtlsdr.conf; then sudo modprobe -r dvb_usb_rtl28xxu";
const HELD: &str = "The RTL-SDR is held by another program";
const GONE: &str = "The RTL-SDR went away (unplugged, or the USB hub dropped)";

// rtl_sdr's last words, as the FATAL code and what to tell the operator. The first that matches wins.
const DONGLE_FAILURES: [(&str, u8, &str); 8] = [
    ("usb_open error -12", 3, ZADIG),
    ("Please fix the device permissions", 3, UDEV),
    ("Detaching kernel driver failed", 3, BLACKLIST),
    ("usb_claim_interface error", 3, HELD),
    ("No supported devices found", 2, NO_DONGLE),
    ("Failed to open rtlsdr device", 3, "The RTL-SDR could not be opened: held by another program, or no permission"),
    ("Library error", 4, GONE),
    ("samples lost, exiting", 4, GONE),
];

// What file_lora warns about once it has a LoRa header, as the spec's ERR codes
const DECODER_WARNINGS: [(&str, i32); 2] = [("header checksum invalid", -24), ("frame ended after", -1)];

pub enum Input<'a> {
    Dongle { gain: Option<&'a str> },
    File(&'a Path),   // a rtl_sdr recording made at 915.25 MHz, 1 MS/s, played in real time
}

pub enum Out {
    Line(String),   // what sdr_rx.py prints on stdout
    Said(String),   // and on stderr
}

pub fn carried() -> bool {
    !FILES.is_empty()
}

pub fn folder(data: &Path) -> PathBuf {
    data.join("sdr").join(BUNDLE_ID)
}

fn program(tools: &Path, name: &str) -> PathBuf {
    tools.join(format!("{}{}", name, std::env::consts::EXE_SUFFIX))
}

pub fn unpack(data: &Path) -> Result<PathBuf, String> {
    unpack_into(&data.join("sdr"), BUNDLE_ID, FILES)
}

// Once per bundle, written under another name and renamed, so a half-written folder is never taken
// for a whole one and one already there is used as it is. The other bundles there go.
fn unpack_into(parent: &Path, id: &str, files: &[(&str, bool, &[u8])]) -> Result<PathBuf, String> {
    let at = parent.join(id);
    if at.is_dir() {
        return Ok(at);
    }
    let part = parent.join(format!("{}.part{}", id, std::process::id()));
    let fail = |e: std::io::Error| format!("cannot unpack the SDR programs into {}: {}", parent.display(), e);
    for (name, exec, bytes) in files {
        let p = part.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(fail)?;
        std::fs::write(&p, bytes).map_err(fail)?;
        #[cfg(unix)]
        if *exec {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).map_err(fail)?;
        }
        #[cfg(not(unix))]
        let _ = exec;
    }
    std::fs::rename(&part, &at).map_err(fail)?;
    for e in std::fs::read_dir(parent).map_err(fail)?.flatten() {
        if e.path() != at {
            if let Err(err) = std::fs::remove_dir_all(e.path()) {
                println!("could not remove the older SDR programs in {}: {}", e.path().display(), err);
            }
        }
    }
    Ok(at)
}

pub fn dongle_present() -> Result<bool, String> {
    let mut all = nusb::list_devices().wait().map_err(|e| format!("cannot list the USB devices: {}", e))?;
    Ok(all.any(|d| DONGLES.contains(&(d.vendor_id(), d.product_id()))))
}

// Whether Windows says interface 0 of the dongle has a driver other than WinUSB, which is what
// rtl_sdr needs there. Only asked once rtl_sdr has failed to open it, so a driver this cannot
// tell apart from a wrong one (libusbK) never stops a dongle that works.
#[cfg(windows)]
fn without_winusb() -> bool {
    let Ok(mut all) = nusb::list_devices().wait() else { return false };
    let Some(d) = all.find(|d| DONGLES.contains(&(d.vendor_id(), d.product_id()))) else { return false };
    match d.open().wait() {
        Ok(dev) => dev.claim_interface(0).wait().is_err_and(|e| {
            let e = e.to_string();
            e.contains("incompatible driver") || e.contains("could not determine driver")
        }),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn without_winusb() -> bool {
    false
}

// The id in the header, kept in the file sdr_rx.py keeps it in, so the same computer's SDR keeps
// its name whichever of the two runs it.
pub fn receiver_id(home: &Path) -> Result<String, String> {
    let path = home.join(".config").join("descent").join("sdr_rx_id");
    if let Ok(text) = std::fs::read_to_string(&path) {
        return Ok(text.trim().to_string());
    }
    let seed = format!("{:?} {}", std::time::SystemTime::now(), std::process::id());
    let id: String = Sha256::digest(seed.as_bytes()).iter().take(3).map(|b| format!("{:02X}", b)).collect();
    std::fs::create_dir_all(path.parent().unwrap())
        .and_then(|()| std::fs::write(&path, format!("{}\n", id)))
        .map_err(|e| format!("cannot keep the SDR's id in {}: {}", path.display(), e))?;
    Ok(id)
}

fn lowpass(cutoff: f64) -> [f32; TAPS] {
    // as numpy's sinc and hamming have it, in doubles, then summed to one
    let m = TAPS as f64;
    let h: Vec<f64> = (0..TAPS)
        .map(|i| {
            let x = std::f64::consts::PI * (2.0 * cutoff / RATE * (i as f64 - (m - 1.0) / 2.0));
            let window = 0.54 + 0.46 * (std::f64::consts::PI * (2.0 * i as f64 + 1.0 - m) / (m - 1.0)).cos();
            x.sin() / x * window
        })
        .collect();
    let sum: f64 = h.iter().sum();
    let mut taps = [0f32; TAPS];
    for (t, v) in taps.iter_mut().zip(&h) {
        *t = (v / sum) as f32;
    }
    taps
}

pub struct FrontEnd {
    n: u64,
    history: Vec<[f32; 2]>,   // the last TAPS-1 samples, then the chunk being filtered
    taps: [f32; TAPS],
}

impl FrontEnd {
    pub fn new() -> FrontEnd {
        FrontEnd { n: 0, history: vec![[0.0; 2]; TAPS - 1], taps: lowpass(75e3) }
    }

    // u8 IQ in, the channel at 250 kS/s out as the little-endian float pairs file_lora reads, with
    // how many bytes sat on the ADC's rail and the median power of the output.
    pub fn push(&mut self, raw: &[u8]) -> (Vec<u8>, usize, f32) {
        let clipped = raw.iter().filter(|&&b| b == 0 || b == 255).count();
        let count = raw.len() / 2;
        // The channel sits at -RATE/4, so moving it to 0 Hz is a multiply by j**n.
        for (k, iq) in raw.chunks_exact(2).enumerate() {
            let (i, q) = ((iq[0] as f32 - 127.5) / 127.5, (iq[1] as f32 - 127.5) / 127.5);
            self.history.push(match (self.n + k as u64) % 4 {
                0 => [i, q],
                1 => [-q, i],
                2 => [-i, -q],
                _ => [q, -i],
            });
        }
        // every fourth sample of the whole stream, counted from the first
        let first = ((4 - self.n % 4) % 4) as usize;
        let mut out = Vec::with_capacity(count / 4 * 8 + 8);
        let mut power = Vec::with_capacity(count / 4 + 1);
        for k in (first..count).step_by(4) {
            let (mut re, mut im) = (0f32, 0f32);
            for (j, s) in self.history[k..k + TAPS].iter().enumerate() {
                let t = self.taps[TAPS - 1 - j];
                re += s[0] * t;
                im += s[1] * t;
            }
            out.extend_from_slice(&re.to_le_bytes());
            out.extend_from_slice(&im.to_le_bytes());
            power.push(re.hypot(im).powi(2));
        }
        self.n += count as u64;
        self.history.drain(..count);
        (out, clipped, median(&mut power))
    }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    let mid = v.len() / 2;
    if v.len() % 2 == 1 { v[mid] } else { (v[mid - 1] + v[mid]) / 2.0 }
}

#[derive(Default)]
struct Counts {
    received: u64,   // samples
    dropped: u64,    // samples a decoder's queue had no room for
    late: u64,
    first: Option<Instant>,
    clipped: u64,
    bytes: u64,
    floors: Vec<f32>,
}

enum Heard {
    Frame(u32, String),
    Warning(u32, i32),
    Ended,             // a decoder's stdout closed
    Said(String),      // a line rtl_sdr wrote on stderr
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().take_while(|&c| c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// A frame file_lora printed, as the line sdr_rx.py makes of it
fn frame_line(sf: u32, frame: &str) -> Option<String> {
    let data: String = decode_base64(&str_field(frame, "packet")?)?.iter().map(|b| format!("{:02X}", b)).collect();
    let snr = format!("{:.2}", num_field(frame, "snr_db")?);
    let crc_present = bool_field(frame, "crc_present")?;
    if crc_present && bool_field(frame, "crc_valid") == Some(true) {
        let cfo = num_field(frame, "cfo_hz")?.round_ties_even();
        return Some(format!("PKT,{},{},,{},{},sf={}", int_field(frame, "len")?, data, snr, cfo, sf));
    }
    Some(format!("ERR,{},,{},sf={},hex={}", if crc_present { -7 } else { -1 }, snr, sf, data))
}

fn level_fields(c: &mut Counts) -> String {
    let clip = if c.bytes > 0 { 100.0 * c.clipped as f64 / c.bytes as f64 } else { 0.0 };
    // A packet outlasts a chunk, so the quietest chunk stands for the floor. A board sending
    // back to back for the whole 5 s raises it to that board's level.
    let floor = c.floors.iter().copied().reduce(f32::min).map(|f| 10.0 * (f as f64).log10());
    c.clipped = 0;
    c.bytes = 0;
    c.floors.clear();
    // sat=1 is this program's own flag: at least one sample sat on the ADC's rail since the last HB
    format!(",clip={:.2},sat={}", clip, (clip > 0.0) as u8) + &floor.map_or(String::new(), |f| format!(",floor={:.1}", f))
}

// Reads until buf is full or the stream ends, so every chunk but the last is whole and IQ pairs
// never split across two.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    n
}

fn spawn_decoder(tools: &Path, sf: u32, heard: &mpsc::Sender<Heard>) -> Result<(Child, SyncSender<Arc<Vec<u8>>>), String> {
    let path = program(tools, "file_lora");
    let mut child = Command::new(&path)
        .args(["-s", &sf.to_string(), "-o", "2", "-w", "0x12", "--report-bad", "-q", "1"])
        .args(["--center-freq", &CHANNEL_HZ.to_string(), "--bw", "125e3", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {}", path.display(), e))?;
    let (out, err, mut stdin) = (child.stdout.take().unwrap(), child.stderr.take().unwrap(), child.stdin.take().unwrap());
    let tx = heard.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(Heard::Frame(sf, line));
        }
        let _ = tx.send(Heard::Ended);
    });
    let tx = heard.clone();
    std::thread::spawn(move || {
        for raw in BufReader::new(err).split(b'\n').map_while(Result::ok) {
            let line = String::from_utf8_lossy(&raw);
            for (text, code) in DECODER_WARNINGS {
                if line.contains(text) {
                    let _ = tx.send(Heard::Warning(sf, code));
                }
            }
        }
    });
    // One writer each, so a decoder that falls behind loses its own samples and holds up no other.
    let (queue, chunks) = mpsc::sync_channel::<Arc<Vec<u8>>>(QUEUED);
    std::thread::spawn(move || {
        for chunk in chunks {
            if stdin.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    Ok((child, queue))
}

fn spawn_dongle(tools: &Path, gain: Option<&str>, heard: &mpsc::Sender<Heard>) -> Result<Child, String> {
    let path = program(tools, "rtl_sdr");
    let mut cmd = Command::new(&path);
    cmd.args(["-f", &(CHANNEL_HZ + 250_000).to_string(), "-s", &(RATE as u32).to_string()]);
    if let Some(g) = gain.filter(|g| *g != "auto") {
        cmd.args(["-g", g]);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {}", path.display(), e))?;
    let err = child.stderr.take().unwrap();
    let tx = heard.clone();
    std::thread::spawn(move || {
        for raw in BufReader::new(err).split(b'\n').map_while(Result::ok) {
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            if !line.is_empty() {
                let _ = tx.send(Heard::Said(line));
            }
        }
    });
    Ok(child)
}

// Until the samples end: rtl_sdr stops or is stopped, or the file has been played. started is
// told rtl_sdr's process id once it runs, so it can be ended from outside. The answer is why it
// ended, in the words the page shows; Err when it could not start.
pub fn run(tools: &Path, input: Input, id: &str, started: impl FnOnce(u32), out: &mut dyn FnMut(Out)) -> Result<String, String> {
    let gain = match &input {
        Input::Dongle { gain } => gain.unwrap_or("auto"),
        Input::File(_) => "auto",
    };
    let header = format!(
        "#DG,RX,v1,id={},f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=10+11+12,gain={},fw={}",
        id,
        gain,
        env!("CARGO_PKG_VERSION")
    );
    let began = Instant::now();
    let (tx, heard) = mpsc::channel();
    let mut children = Vec::new();
    let mut queues = Vec::new();
    for sf in SFS {
        match spawn_decoder(tools, sf, &tx) {
            Ok((child, queue)) => {
                children.push(child);
                queues.push(queue);
            }
            Err(e) => {
                for c in &mut children {
                    let _ = c.kill();
                }
                return Err(e);
            }
        }
    }
    let live = matches!(input, Input::Dongle { .. });
    let (mut reader, rtl): (Box<dyn Read + Send>, Option<Child>) = match input {
        Input::Dongle { gain } => match spawn_dongle(tools, gain, &tx) {
            Ok(mut child) => {
                started(child.id());
                (Box::new(child.stdout.take().unwrap()), Some(child))
            }
            Err(e) => {
                drop(queues);
                return Err(e);
            }
        },
        Input::File(path) => match std::fs::File::open(path) {
            Ok(f) => (Box::new(f), None),
            Err(e) => {
                drop(queues);
                return Err(format!("cannot read {}: {}", path.display(), e));
            }
        },
    };
    drop(tx);

    let counts = Arc::new(Mutex::new(Counts::default()));
    std::thread::spawn({
        let counts = counts.clone();
        move || {
            let mut front = FrontEnd::new();
            let mut raw = vec![0u8; CHUNK];
            loop {
                let n = fill(&mut reader, &mut raw) & !1;
                if n == 0 {
                    break;
                }
                let (iq, clipped, floor) = front.push(&raw[..n]);
                let iq = Arc::new(iq);
                let mut lost = false;
                for q in &queues {
                    if live {
                        lost |= matches!(q.try_send(iq.clone()), Err(TrySendError::Full(_)));
                    } else {
                        let _ = q.send(iq.clone());
                    }
                }
                let received = {
                    let mut c = counts.lock().unwrap();
                    c.first.get_or_insert_with(Instant::now);
                    c.received += n as u64 / 2;
                    if lost {
                        c.dropped += n as u64 / 2;
                    }
                    c.clipped += clipped as u64;
                    c.bytes += n as u64;
                    c.floors.push(floor);
                    c.received
                };
                // paced to real time, so the HB, the header and the app see what a dongle would give them
                if !live {
                    let ahead = received as f64 / RATE - began.elapsed().as_secs_f64();
                    if ahead > 0.0 {
                        std::thread::sleep(Duration::from_secs_f64(ahead));
                    }
                }
            }
        }
    });

    let (mut ok, mut errors) = (0u32, 0u32);
    let mut fatal: Option<(u8, &str)> = None;
    let mut open = SFS.len();
    let mut next_header = began;
    let mut next_hb = began;
    let heartbeat = |ok: u32, errors: u32, out: &mut dyn FnMut(Out)| {
        let mut c = counts.lock().unwrap();
        if let (true, Some(first)) = (live, c.first) {
            // rtl_sdr loses samples without saying so; count what the clock says should
            // have come, less half a second of buffering, and never let the count go down
            let late = first.elapsed().as_secs_f64() * RATE - c.received as f64 - RATE / 2.0;
            c.late = c.late.max(late.max(0.0) as u64);
        }
        let ovf = c.dropped + if live { c.late } else { 0 };
        out(Out::Line(format!("HB,{},{},{},ovf={},drop=0{}", began.elapsed().as_millis(), ok, errors, ovf, level_fields(&mut c))));
    };
    while open > 0 {
        let now = Instant::now();
        if now >= next_header {
            out(Out::Line(header.clone()));
            next_header = now + Duration::from_secs(30);
        }
        if now >= next_hb {
            heartbeat(ok, errors, out);
            next_hb = now + Duration::from_secs(5);
        }
        let line = match heard.recv_timeout(Duration::from_millis(500)) {
            Ok(Heard::Frame(sf, frame)) => match frame_line(sf, &frame) {
                Some(line) => line,
                None => {
                    out(Out::Said(format!("file_lora at SF{} printed what is not a frame: {}", sf, frame)));
                    continue;
                }
            },
            Ok(Heard::Warning(sf, code)) => format!("ERR,{},,,sf={}", code, sf),
            Ok(Heard::Ended) => {
                open -= 1;
                continue;
            }
            Ok(Heard::Said(text)) => {
                if fatal.is_none() {
                    fatal = DONGLE_FAILURES.iter().find(|(t, _, _)| text.contains(t)).map(|&(_, code, why)| (code, why));
                }
                out(Out::Said(format!("rtl_sdr: {}", text)));
                continue;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if line.starts_with("PKT") {
            ok += 1;
        } else {
            errors += 1;
        }
        out(Out::Line(line));
    }
    heartbeat(ok, errors, out);
    for mut c in children {
        let _ = c.wait();
    }
    let Some(mut rtl) = rtl else {
        return Ok(String::from("played to the end"));
    };
    let _ = rtl.wait();
    // what rtl_sdr said on its way out can still be in the channel
    while let Ok(Heard::Said(text)) = heard.try_recv() {
        if fatal.is_none() {
            fatal = DONGLE_FAILURES.iter().find(|(t, _, _)| text.contains(t)).map(|&(_, code, why)| (code, why));
        }
        out(Out::Said(format!("rtl_sdr: {}", text)));
    }
    let (code, why) = match fatal {
        Some((3, _)) if without_winusb() => (3, ZADIG),
        Some(f) => f,
        None => (1, "rtl_sdr stopped without saying why"),
    };
    out(Out::Line(format!("#DG,FATAL,{}", code)));
    out(Out::Said(why.to_string()));
    Ok(why.to_string())
}

pub const HELP: &str = "\
Usage: descent-ground sdr [--gain DB|auto] [--cu8 FILE] [--tools DIR]

Runs the built-in SDR on its own and prints its receiver lines (docs/sdr-receiver.md) on stdout,
what rtl_sdr says on stderr. --cu8 plays a rtl_sdr recording made at 915.25 MHz, 1 MS/s, in real
time instead of reading the dongle. --tools is a folder holding rtl_sdr and file_lora; without it
the app's own are unpacked beside the program, as when the page turns the SDR on.
";

pub fn cli(args: &[String]) -> i32 {
    let (mut gain, mut cu8, mut tools) = (None, None, None);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| format!("{} wants a value after it", a));
        let got = match a.as_str() {
            "--gain" => value().and_then(|v| crate::command::a_gain(&v)).map(|v| gain = Some(v)),
            "--cu8" => value().map(|v| cu8 = Some(PathBuf::from(v))),
            "--tools" => value().map(|v| tools = Some(PathBuf::from(v))),
            "-h" | "--help" => {
                print!("{}", HELP);
                return 0;
            }
            _ => Err(format!("{} is not an option of descent-ground sdr", a)),
        };
        if let Err(e) = got {
            eprintln!("{}", e);
            return 2;
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let Some(home) = home else {
        eprintln!("neither HOME nor USERPROFILE is set, so there is nowhere to keep the SDR's id");
        return 1;
    };
    let tools = match tools {
        Some(t) => t,
        None if carried() => {
            let exe = std::env::current_exe().unwrap();
            match unpack(&crate::opts::data_dir(exe.parent().unwrap(), Some(&home))) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{}", e);
                    return 1;
                }
            }
        }
        None => {
            eprintln!("this build carries no SDR programs; name a folder holding rtl_sdr and file_lora with --tools");
            return 2;
        }
    };
    let id = match receiver_id(&home) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    let input = match &cu8 {
        Some(path) => Input::File(path),
        None => Input::Dongle { gain: gain.as_deref() },
    };
    let mut print = |o: Out| match o {
        Out::Line(l) => {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{}", l).and_then(|()| stdout.flush());
        }
        Out::Said(t) => eprintln!("{}", t),
    };
    match run(&tools, input, &id, |_| {}, &mut print) {
        Ok(_) if cu8.is_some() => 0,
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

    fn tone(cycles_per_sample: f64, n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|k| {
                let p = 2.0 * std::f64::consts::PI * cycles_per_sample * k as f64;
                [(127.5 + 100.0 * p.cos()).round() as u8, (127.5 + 100.0 * p.sin()).round() as u8]
            })
            .collect()
    }

    fn power(out: &[u8]) -> f64 {
        let v: Vec<f32> = out.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        v.chunks_exact(2).skip(32).map(|p| (p[0] * p[0] + p[1] * p[1]) as f64).sum::<f64>() / (v.len() / 2 - 32) as f64
    }

    // The channel is 250 kHz below where the dongle is tuned: a tone there comes through, one at
    // the tuned centre does not.
    #[test]
    fn the_channel_comes_through_and_the_rest_is_cut() {
        let channel = power(&FrontEnd::new().push(&tone(-0.25 + 10e3 / RATE, 40_000)).0);
        let centre = power(&FrontEnd::new().push(&tone(0.0, 40_000)).0);
        assert!(channel > 0.5, "{}", channel);
        assert!(centre < channel * 1e-4, "{} against {}", centre, channel);
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_samples_are_chunked() {
        let raw = tone(-0.25 + 31e3 / RATE, 10_001);
        let (whole, clipped, _) = FrontEnd::new().push(&raw);
        let mut parts = FrontEnd::new();
        let mut pieces = Vec::new();
        for piece in [&raw[..2 * 7], &raw[2 * 7..2 * 5003], &raw[2 * 5003..]] {
            pieces.extend(parts.push(piece).0);
        }
        assert_eq!(whole.len(), (10_001 + 3) / 4 * 8);
        assert_eq!(whole, pieces);
        assert_eq!(clipped, 0);
    }

    #[test]
    fn the_filter_sums_to_one_and_is_symmetric() {
        let t = lowpass(75e3);
        assert!((t.iter().map(|&v| v as f64).sum::<f64>() - 1.0).abs() < 1e-6);
        assert!((0..TAPS).all(|i| (t[i] - t[TAPS - 1 - i]).abs() < 1e-7));
    }

    #[test]
    fn frames_become_the_lines_sdr_rx_prints() {
        let good = r#"{"packet":"AAEC/w==","len":4,"snr_db":8.4,"crc_present":true,"crc_valid":true,"cfo_hz":-702.5}"#;
        assert_eq!(frame_line(12, good).unwrap(), "PKT,4,000102FF,,8.40,-702,sf=12");
        let bad = r#"{"packet":"AAEC","len":3,"snr_db":-4.0,"crc_present":true,"crc_valid":false,"cfo_hz":1.5}"#;
        assert_eq!(frame_line(10, bad).unwrap(), "ERR,-7,,-4.00,sf=10,hex=000102");
        let none = r#"{"packet":"AA==","len":1,"snr_db":1.0,"crc_present":false,"crc_valid":null,"cfo_hz":0.0}"#;
        assert_eq!(frame_line(11, none).unwrap(), "ERR,-1,,1.00,sf=11,hex=00");
        assert_eq!(frame_line(11, "decoder says hello"), None);
    }

    #[test]
    fn a_bundle_unpacks_once_and_replaces_the_one_before() {
        let parent = std::env::temp_dir().join(format!("dg-sdr-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let old = unpack_into(&parent, "aaaa", &[("rtl_sdr", true, b"old")]).unwrap();
        let files: &[(&str, bool, &[u8])] = &[("rtl_sdr", true, b"new"), ("THIRD_PARTY/SOURCES.txt", false, b"where")];
        let new = unpack_into(&parent, "bbbb", files).unwrap();
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("THIRD_PARTY").join("SOURCES.txt")).unwrap(), b"where");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(new.join("rtl_sdr")).unwrap().permissions().mode() & 0o111, 0o111);
        }
        std::fs::write(new.join("rtl_sdr"), b"kept").unwrap();
        unpack_into(&parent, "bbbb", files).unwrap();
        assert_eq!(std::fs::read(new.join("rtl_sdr")).unwrap(), b"kept", "a folder already there is used as it is");
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
        std::fs::remove_dir_all(&parent).unwrap();
    }
}
