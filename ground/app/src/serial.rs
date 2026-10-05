// One thread per open port.
//
// Nothing is opened until the operator says it is a receiver. That is not caution for
// its own sake: opening a port pulses DTR, which resets an ESP32, and serialport
// documents that Linux asserts DTR on open whatever you ask for, so there is no way
// to listen to an unknown board to find out what it is without possibly resetting it.
// A ChipSat and a T-Beam can carry the same USB bridge, so the chip cannot tell them
// apart either. The operator says so once and the board is remembered after that.
use crate::framing::Framer;
use crate::logfile::now_ms;
use crate::receivers::{self, Header};
use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BAUD: u32 = 115200;

// A board is remembered by its chip and serial number rather than by which port it
// landed on, so replugging it, or moving it to another socket, still counts as the
// same board. Plenty of CP2102s ship with the serial 0001, so approving one of those
// approves every board with that same chip and serial. The list says so.
pub fn board_key(vid: u16, pid: u16, serial: Option<&str>) -> String {
    format!("{:04x}:{:04x}:{}", vid, pid, serial.unwrap_or("-"))
}

// An ST-Link's USB serial is raw bytes rather than text and arrives as mojibake, which
// is useless for telling two identical probes apart on a bench. Anything that is not
// printable is shown as hex instead, which is the form OpenOCD and the flight tools
// use. Caveat: if the OS already replaced bad bytes on its way to us, this is the hex
// of what we were given, not of what the descriptor holds.
pub fn readable_serial(serial: &str) -> String {
    if serial.is_empty() || serial.chars().all(|c| c.is_ascii_graphic()) {
        return serial.to_string();
    }
    serial.bytes().map(|b| format!("{:02X}", b)).collect()
}

// Said out loud in the list so nobody has to recognise a hex id.
pub fn bridge_name(vid: u16, pid: u16) -> &'static str {
    match (vid, pid) {
        (0x10c4, 0xea60) => "CP2102",
        (0x1a86, 0x55d4) => "CH9102",
        (0x1a86, 0x7523) => "CH340",
        (0x303a, _) => "ESP32 native USB",
        (0x0483, _) => "ST-Link",
        _ => "USB serial",
    }
}

pub enum Event {
    Line { key: String, t_ms: u128, text: String },
    Ports,
}

// Every USB serial port on the machine, and what we are doing about it.
// state: "receiver" (open), "waiting" (never touched, needs a yes), "dismissed"
// (disconnected by hand this session), "ignored" (told to leave it alone for good).
#[derive(Clone)]
pub struct Board {
    pub port: String,
    pub usb: String,
    pub label: String,
    pub serial: String,
    pub key: String,
    pub state: String,
    pub name: String,   // what the operator calls it, empty until they say
}

#[derive(Clone)]
pub struct PortInfo {
    pub key: String,
    pub port: String,
    pub usb: String,
    pub status: String,
    pub error: Option<String>,
    // Which receiver this is, once its header line has been seen. The key above is only
    // the order the port was found in.
    pub rx_id: Option<String>,
    pub rx_name: Option<String>,   // what the board says it is called
    pub rx_fw: Option<String>,     // the firmware it says it runs, from 1.2.1 on
    pub name: Option<String>,      // what this computer calls that id
}

// The one thing that tells an unmarked board from its neighbour without opening it:
// the operator unplugs the one they mean and plugs it back in. A port that vanishes
// and then a port that appears carrying a key we just lost is that board. Ports are
// tracked rather than keys alone because two CP2102s can share a key, and a replug
// often lands on a different /dev/ttyUSBn.
const IDENTIFY_MS: u128 = 60_000;

pub struct Identify {
    started_ms: u128,
    last: Vec<(String, String)>,   // (board key, port) at the previous look
    gone: Vec<(String, String)>,   // pairs that have disappeared since the watch began
    found: Option<String>,
}

impl Identify {
    fn start(now_ms: u128, present: &[(String, String)]) -> Self {
        Identify { started_ms: now_ms, last: present.to_vec(), gone: Vec::new(), found: None }
    }

    fn step(&mut self, now_ms: u128, present: &[(String, String)]) {
        if self.found.is_some() || self.expired(now_ms) {
            return;
        }
        for (k, p) in &self.last {
            if !present.iter().any(|(_, q)| q == p) && !self.gone.iter().any(|(_, q)| q == p) {
                self.gone.push((k.clone(), p.clone()));
            }
        }
        for (k, p) in present {
            let back = !self.last.iter().any(|(_, q)| q == p)
                && self.gone.iter().any(|(j, _)| j == k);
            if back {
                self.found = Some(p.clone());
                break;
            }
        }
        self.last = present.to_vec();
    }

    fn expired(&self, now_ms: u128) -> bool {
        now_ms.saturating_sub(self.started_ms) >= IDENTIFY_MS
    }

    fn seconds_left(&self, now_ms: u128) -> u64 {
        let gone = now_ms.saturating_sub(self.started_ms);
        ((IDENTIFY_MS.saturating_sub(gone) + 999) / 1000) as u64
    }
}

// Keys are handed out in discovery order and stay with a device path, so rx1 is
// still rx1 after a replug and two receivers can never share a key.
pub struct Keys {
    map: HashMap<String, String>,
    next: usize,
}

impl Keys {
    pub fn new() -> Self {
        Keys { map: HashMap::new(), next: 1 }
    }

    pub fn key_for(&mut self, port: &str) -> String {
        if let Some(k) = self.map.get(port) {
            return k.clone();
        }
        let k = format!("rx{}", self.next);
        self.next += 1;
        self.map.insert(port.to_string(), k.clone());
        k
    }
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

// What a port has said about itself, and what has been done about it.
#[derive(Default)]
struct Heard {
    header: Option<Header>,
    // The id of the board on this port that has been sent the name kept for it and has
    // not reported a name since. A board can refuse one, and is then not asked again
    // with every header.
    asked: Option<String>,
}

struct Open {
    port: String,
    usb: String,
    writer: Option<Writer>,
    heard: Arc<Mutex<Heard>>,             // the reader thread writes this
    alive: Arc<Mutex<bool>>,
    status: Arc<Mutex<String>>,           // the reader thread owns this
    error: Arc<Mutex<Option<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

pub struct Hub {
    tx: Sender<Event>,
    keys: Keys,
    open: HashMap<String, Open>,   // by key
    flashing: HashSet<String>,     // device paths a flash owns
    complained: HashSet<String>,   // ports we have already moaned about
    approved: HashSet<String>,     // board keys the operator said are receivers
    ignored: HashSet<String>,      // board keys to leave alone for good
    dismissed: HashSet<String>,    // device paths disconnected by hand, until replug
    probing: HashSet<String>,      // device paths a five-second listen owns
    names: HashMap<String, String>,   // board key -> what the operator calls it
    identify: Option<Identify>,
    boards: Vec<Board>,            // every USB serial port and what we do about it
    store: Option<std::path::PathBuf>,
    // receiver id -> what this computer calls it. Shared with the reader threads, which
    // are the ones that see a board come back without its name.
    rx_names: Arc<Mutex<receivers::Names>>,
}

// Called on every line a receiver sends. Only its header line changes anything, and the
// answer is whether it did, so the page is told once and not every 30 s.
//
// A header is also where a board is seen to have lost its name. Flashing a receiver
// writes a whole-flash image at 0x0, and that image is blank across the partition the
// board keeps its settings in, so it comes back with none; and which board came back is
// not known until it says so here. A board that says it takes a name and has none is
// sent the one kept for its id. A board that reports a name is never sent one: its own
// stands, even where this computer calls it something else.
fn notice(line: &str, heard: &Mutex<Heard>, names: &Mutex<receivers::Names>, writer: Option<&Writer>) -> bool {
    let Some(header) = receivers::header(line) else { return false };
    let id = header.id.clone();
    let mut give_back = None;
    let news = {
        let mut was = heard.lock().unwrap();
        match header.name.as_deref() {
            Some("") if was.asked.as_deref() != Some(id.as_str()) => {
                give_back = names.lock().unwrap().get(&id).map(String::from);
                if give_back.is_some() {
                    was.asked = Some(id);
                }
            }
            Some("") | None => {}
            Some(_) => was.asked = None,
        }
        let news = was.header.as_ref() != Some(&header);
        was.header = Some(header);
        news
    };
    // Written with the lock let go: a port that is slow to take it must not hold up
    // whoever is asking what this port has said.
    if let Some(name) = give_back {
        let sent = writer
            .and_then(|w| w.lock().ok().map(|mut w| w.write_all(format!("#SET,name,{}\n", name).as_bytes()).and_then(|()| w.flush())))
            .map(|r| r.is_ok())
            .unwrap_or(false);
        if !sent {
            heard.lock().unwrap().asked = None;
        }
    }
    news
}

impl Hub {
    pub fn new(tx: Sender<Event>) -> Self {
        Hub {
            tx,
            keys: Keys::new(),
            open: HashMap::new(),
            flashing: HashSet::new(),
            complained: HashSet::new(),
            approved: HashSet::new(),
            ignored: HashSet::new(),
            dismissed: HashSet::new(),
            probing: HashSet::new(),
            names: HashMap::new(),
            identify: None,
            boards: Vec::new(),
            store: None,
            rx_names: Arc::new(Mutex::new(receivers::Names::empty())),
        }
    }

    // What the operator decided last time. Kept beside the binary so a field laptop
    // stays no-click after the first run.
    pub fn remember_in(&mut self, dir: &std::path::Path) {
        let f = dir.join("descent-ground-boards.txt");
        if let Ok(text) = std::fs::read_to_string(&f) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                match line.split_once(' ') {
                    Some(("receiver", k)) => { self.approved.insert(k.trim().to_string()); }
                    Some(("ignore", k)) => { self.ignored.insert(k.trim().to_string()); }
                    // name <key> <whatever the operator typed>, spaces and all
                    Some(("name", rest)) => {
                        if let Some((k, n)) = rest.trim_start().split_once(' ') {
                            let n = n.trim();
                            if !n.is_empty() {
                                self.names.insert(k.to_string(), n.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.store = Some(f);
        *self.rx_names.lock().unwrap() = receivers::Names::load(dir);
    }

    fn save(&self) {
        let Some(f) = &self.store else { return };
        let mut text = String::from("# what DeSCENT Ground may open, and what each board is called.\n");
        text.push_str("# one per line: receiver <board>, ignore <board>, name <board> <what you call it>\n");
        let mut rows: Vec<String> = self
            .approved
            .iter()
            .map(|k| format!("receiver {}", k))
            .chain(self.ignored.iter().map(|k| format!("ignore {}", k)))
            .chain(self.names.iter().map(|(k, n)| format!("name {} {}", k, n)))
            .collect();
        rows.sort();
        for r in rows {
            text.push_str(&r);
            text.push('\n');
        }
        let _ = std::fs::write(f, text);
    }

    fn key_of_port(&self, port: &str) -> Option<String> {
        self.boards.iter().find(|b| b.port == port).map(|b| b.key.clone())
    }

    // Yes, this one is a receiver. Remembered as a board, so it still counts after a
    // replug or a move to another socket.
    pub fn approve(&mut self, port: &str) -> Result<(), String> {
        let key = self.key_of_port(port).ok_or_else(|| format!("{} is not plugged in", port))?;
        self.ignored.remove(&key);
        self.approved.insert(key);
        self.dismissed.remove(port);
        self.complained.remove(port);
        self.touch_board(port, Some("receiver"), None);
        self.save();
        let _ = self.tx.send(Event::Ports);
        Ok(())
    }

    // Not a receiver, and stop asking. Closes it if it happened to be open.
    pub fn ignore(&mut self, port: &str) -> Result<(), String> {
        let key = self.key_of_port(port).ok_or_else(|| format!("{} is not plugged in", port))?;
        self.approved.remove(&key);
        self.ignored.insert(key);
        if let Some(k) = self.open.iter().find(|(_, o)| o.port == port).map(|(k, _)| k.clone()) {
            self.close(&k);
        }
        self.touch_board(port, Some("ignored"), None);
        self.save();
        let _ = self.tx.send(Event::Ports);
        Ok(())
    }

    // Drops what this computer keeps about a board: whether to read it, and what it is
    // called here. Nothing is sent to the board, so a name its own firmware holds stays
    // on it. The port is shut and the board goes back to not touched, as one never
    // plugged in before. The name kept against a receiver's id can only be dropped while
    // the board is being read, since that is the only time the id is known.
    pub fn forget(&mut self, port: &str) -> Result<(), String> {
        let key = self.key_of_port(port).ok_or_else(|| format!("{} is not plugged in", port))?;
        if self.flashing.contains(port) {
            return Err(format!("{} is being flashed", port));
        }
        let reading = self
            .open
            .iter()
            .find(|(_, o)| o.port == port)
            .map(|(k, o)| (k.clone(), o.heard.lock().unwrap().header.as_ref().map(|h| h.id.clone())));
        if let Some((rx, id)) = reading {
            if let Some(id) = id {
                self.rx_names.lock().unwrap().set(&id, "")?;
            }
            self.close(&rx);
        }
        self.approved.remove(&key);
        self.ignored.remove(&key);
        self.names.remove(&key);
        self.dismissed.remove(port);
        self.complained.remove(port);
        self.touch_board(port, Some("waiting"), Some(""));
        self.save();
        let _ = self.tx.send(Event::Ports);
        Ok(())
    }

    // What the operator calls this board. Against the board, like the approvals, so a
    // replug or another socket keeps the name. An empty name clears it. The settings
    // file is one board per line, so a name cannot carry a newline.
    pub fn set_name(&mut self, port: &str, name: &str) -> Result<(), String> {
        let key = self.key_of_port(port).ok_or_else(|| format!("{} is not plugged in", port))?;
        if name.contains('\n') || name.contains('\r') {
            return Err(String::from("a name has to fit on one line"));
        }
        let name = name.trim();
        if name.is_empty() {
            self.names.remove(&key);
        } else {
            self.names.insert(key, name.to_string());
        }
        self.touch_board(port, None, Some(name));
        self.save();
        let _ = self.tx.send(Event::Ports);
        Ok(())
    }

    // Start watching for a replug. A second call restarts it and forgets the last answer.
    pub fn identify_start(&mut self) {
        let present: Vec<(String, String)> =
            self.boards.iter().map(|b| (b.key.clone(), b.port.clone())).collect();
        self.identify = Some(Identify::start(now_ms(), &present));
    }

    // The operator changed their mind. Whatever the watch had found goes with it.
    pub fn identify_stop(&mut self) {
        self.identify = None;
    }

    // What this computer calls the receiver with this id, wherever it is plugged in. If
    // it is being read now and its firmware takes a name, the board is told too, and the
    // answer is whether it was.
    pub fn set_receiver_name(&mut self, id: &str, name: &str) -> Result<bool, String> {
        let id = receivers::an_id(id)
            .ok_or_else(|| format!("a receiver's id is six hex characters, and {:?} is not", id))?;
        let name = receivers::a_name(name)?;
        self.rx_names.lock().unwrap().set(&id, &name)?;
        let _ = self.tx.send(Event::Ports);

        // Only a board whose last header had a name in it. Firmware from before names
        // has no such key to set, and what it makes of one has not been tried.
        let takes_one = self
            .open
            .iter()
            .find(|(_, o)| matches!(&o.heard.lock().unwrap().header, Some(h) if h.id == id && h.name.is_some()))
            .map(|(key, _)| key.clone());
        match takes_one {
            None => Ok(false),
            Some(key) => {
                self.send(&key, &format!("#SET,name,{}\n", name))
                    .map_err(|e| format!("the name is kept on this computer, but it could not be sent to the receiver ({})", e))?;
                // It has been told. If it turns the name down, its next header is not
                // a reason to tell it again.
                if let Some(o) = self.open.get(&key) {
                    o.heard.lock().unwrap().asked = Some(id);
                }
                Ok(true)
            }
        }
    }

    // watching, the port that came back, seconds left. An expired watch reads the same
    // as no watch at all; a found one stops the clock and keeps its answer until the
    // next start.
    pub fn identify_state(&self) -> (bool, Option<String>, u64) {
        let Some(w) = &self.identify else { return (false, None, 0) };
        let now = now_ms();
        if let Some(p) = &w.found {
            return (false, Some(p.clone()), 0);
        }
        if w.expired(now) {
            return (false, None, 0);
        }
        (true, None, w.seconds_left(now))
    }

    // A five-second listen is not ownership, but poll() and the flasher have to keep
    // off the port while it lasts, and anything already using it says no.
    pub fn begin_probe(&mut self, port: &str) -> Result<(), String> {
        if self.key_of_port(port).is_none() {
            return Err(format!("{} is not plugged in", port));
        }
        if self.probing.contains(port) {
            return Err(format!("{} is already being listened to", port));
        }
        if self.flashing.contains(port) {
            return Err(format!("{} is being flashed", port));
        }
        if self.open.values().any(|o| o.port == port) {
            return Err(format!("{} is open as a receiver — disconnect it there first", port));
        }
        self.probing.insert(port.to_string());
        Ok(())
    }

    pub fn end_probe(&mut self, port: &str) {
        self.probing.remove(port);
        let _ = self.tx.send(Event::Ports);
    }

    fn fingerprint(boards: &[Board]) -> String {
        boards.iter().map(|b| format!("{}|{}|{}", b.port, b.state, b.name)).collect()
    }

    // Answering a decision straight away, rather than leaving the page to show what it
    // showed before until the next scan two seconds later. That delay read as the first
    // click doing nothing, so people clicked twice.
    fn touch_board(&mut self, port: &str, state: Option<&str>, name: Option<&str>) {
        for b in self.boards.iter_mut() {
            if b.port == port {
                if let Some(s) = state { b.state = s.to_string(); }
                if let Some(n) = name { b.name = n.to_string(); }
            }
        }
    }

    pub fn boards(&self) -> Vec<Board> {
        self.boards.clone()
    }

    pub fn ports(&self) -> Vec<PortInfo> {
        let mut v: Vec<PortInfo> = self
            .open
            .iter()
            .map(|(key, o)| {
                let heard = o.heard.lock().ok().and_then(|h| h.header.clone());
                let rx_id = heard.as_ref().map(|h| h.id.clone());
                PortInfo {
                    key: key.clone(),
                    port: o.port.clone(),
                    usb: o.usb.clone(),
                    status: o.status.lock().map(|s| s.clone()).unwrap_or_else(|_| "error".into()),
                    error: o.error.lock().ok().and_then(|e| e.clone()),
                    name: rx_id.as_deref().and_then(|id| self.rx_names.lock().unwrap().get(id).map(String::from)),
                    rx_fw: heard.as_ref().and_then(|h| h.fw.clone()),
                    rx_name: heard.and_then(|h| h.name).filter(|n| !n.is_empty()),
                    rx_id,
                }
            })
            .collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    }

    pub fn send(&mut self, key: &str, text: &str) -> Result<(), String> {
        let o = self.open.get(key).ok_or_else(|| format!("{} is not open", key))?;
        let writer = o.writer.as_ref().ok_or_else(|| format!("{} has no port", key))?;
        let mut w = writer.lock().map_err(|_| format!("{} is busy", key))?;
        w.write_all(text.as_bytes()).map_err(|e| format!("{}: {}", key, e))?;
        w.flush().map_err(|e| format!("{}: {}", key, e))
    }

    // Disconnect by hand. The port stays shut until it is replugged or approved
    // again: reopening it two seconds later is what made the ✕ useless.
    pub fn dismiss(&mut self, key: &str) {
        if let Some(port) = self.open.get(key).map(|o| o.port.clone()) {
            self.dismissed.insert(port);
        }
        self.close(key);
    }

    pub fn close(&mut self, key: &str) {
        if let Some(mut o) = self.open.remove(key) {
            if let Ok(mut a) = o.alive.lock() {
                *a = false;
            }
            if let Some(h) = o.handle.take() {
                let _ = h.join();
            }
        }
        let _ = self.tx.send(Event::Ports);
    }

    // Hand a port over to the flasher: stop reading it, wait for the reader thread to
    // let go of the file descriptor, and keep poll() off it until the flash is done.
    // serialport takes an exclusive lock on the device, so espflash cannot open it
    // while our reader is still alive.
    pub fn release_for_flash(&mut self, port: &str) -> Result<(), String> {
        if self.flashing.contains(port) {
            return Err(format!("{} is already being flashed", port));
        }
        if self.probing.contains(port) {
            return Err(format!("{} is being listened to, so wait for that to finish", port));
        }
        self.flashing.insert(port.to_string());
        let key = self.open.iter().find(|(_, o)| o.port == port).map(|(k, _)| k.clone());
        if let Some(k) = key {
            self.close(&k);
        }
        let _ = self.tx.send(Event::Ports);
        Ok(())
    }

    // Give it back. poll() reopens it on its next pass, which also re-reads the
    // #DG,RX header from the firmware that is now on the board.
    pub fn take_back(&mut self, port: &str) {
        self.flashing.remove(port);
        self.complained.remove(port);
        let _ = self.tx.send(Event::Ports);
    }

    pub fn is_flashing(&self, port: &str) -> bool {
        self.flashing.contains(port)
    }

    // Called every 2 s. Opens anything new, forgets anything unplugged.
    pub fn poll(&mut self) {
        let found = serialport::available_ports().unwrap_or_default();
        let mut seen: Vec<String> = Vec::new();
        let mut boards: Vec<Board> = Vec::new();
        let mut present: HashSet<String> = HashSet::new();

        for p in found {
            let u = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => u.clone(),
                _ => continue,   // a built-in ttyS is never one of ours
            };
            present.insert(p.port_name.clone());
            let serial = readable_serial(&u.serial_number.clone().unwrap_or_default());
            let key = board_key(u.vid, u.pid, u.serial_number.as_deref());
            let usb = format!("{:04x}:{:04x}", u.vid, u.pid);
            let label = bridge_name(u.vid, u.pid).to_string();

            let approved = self.approved.contains(&key);
            let state = if self.ignored.contains(&key) {
                "ignored"
            } else if self.dismissed.contains(&p.port_name) {
                "dismissed"
            } else if approved {
                "receiver"
            } else {
                "waiting"
            };
            let name = self.names.get(&key).cloned().unwrap_or_default();
            boards.push(Board {
                port: p.port_name.clone(),
                usb: usb.clone(),
                label,
                serial,
                key,
                state: state.to_string(),
                name,
            });

            if state != "receiver"
                || self.flashing.contains(&p.port_name)
                || self.probing.contains(&p.port_name)
            {
                continue;
            }

            let rx = self.keys.key_for(&p.port_name);
            seen.push(rx.clone());
            if let Some(o) = self.open.get(&rx) {
                if o.status.lock().map(|s| s.as_str() == "open").unwrap_or(false) {
                    continue;
                }
                self.close(&rx);   // its reader died; fall through and open it again
            }
            match serialport::new(&p.port_name, BAUD).timeout(Duration::from_millis(200)).open() {
                Ok(port) => {
                    self.complained.remove(&p.port_name);
                    self.spawn(rx, p.port_name, usb, port);
                }
                Err(e) => {
                    let hint = if e.to_string().contains("ermission") {
                        format!("{} — add yourself to the dialout group: sudo usermod -aG dialout $USER, then log out and back in", e)
                    } else {
                        e.to_string()
                    };
                    // Once per port, not every two seconds forever.
                    if self.complained.insert(p.port_name.clone()) {
                        println!("{}: {}", p.port_name, hint);
                    }
                }
            }
        }

        // Unplugging a board clears a by-hand disconnect, so plugging it back in
        // brings it back the way anyone would expect.
        self.dismissed.retain(|p| present.contains(p));

        // The replug watch runs off this same pass, so nothing has to be opened for it.
        // A board unplugged and back inside one 2 s pass is missed, which is why the
        // page asks for a deliberate unplug rather than a tap.
        if let Some(w) = self.identify.as_mut() {
            let now = now_ms();
            let pairs: Vec<(String, String)> =
                boards.iter().map(|b| (b.key.clone(), b.port.clone())).collect();
            w.step(now, &pairs);
            if w.found.is_none() && w.expired(now) {
                self.identify = None;
            }
        }
        // Plugging or unplugging a board that was never opened changes this list and
        // nothing else, so without comparing it the page is not told and the operator
        // watches a stale list. It is the whole point of the identify flow.
        let before = Self::fingerprint(&self.boards);
        self.boards = boards;
        if Self::fingerprint(&self.boards) != before {
            let _ = self.tx.send(Event::Ports);
        }

        let gone: Vec<String> = self.open.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for k in gone {
            println!("{} unplugged", k);
            self.close(&k);
        }
    }

    fn spawn(
        &mut self,
        key: String,
        port_name: String,
        usb: String,
        port: Box<dyn serialport::SerialPort>,
    ) {
        let reader = match port.try_clone() {
            Ok(r) => r,
            Err(e) => {
                println!("{}: {}", port_name, e);
                return;
            }
        };
        let alive = Arc::new(Mutex::new(true));
        let status = Arc::new(Mutex::new(String::from("open")));
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let heard = Arc::new(Mutex::new(Heard::default()));
        let writer: Option<Writer> = Some(Arc::new(Mutex::new(Box::new(port))));

        let tx = self.tx.clone();
        let thread_key = key.clone();
        let thread_alive = alive.clone();
        let thread_status = status.clone();
        let thread_error = error.clone();
        let thread_heard = heard.clone();
        let thread_names = self.rx_names.clone();
        let thread_writer = writer.clone();
        let mut reader = reader;
        let handle = std::thread::spawn(move || {
            let mut framer = Framer::new();
            let mut buf = [0u8; 4096];
            loop {
                if !*thread_alive.lock().unwrap() {
                    return;
                }
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let t = now_ms();
                        for line in framer.push(&buf[..n]) {
                            let news = notice(&line, &thread_heard, &thread_names, thread_writer.as_ref());
                            let _ = tx.send(Event::Line {
                                key: thread_key.clone(),
                                t_ms: t,
                                text: line,
                            });
                            if news {
                                let _ = tx.send(Event::Ports);
                            }
                        }
                    }
                    Err(ref e) if e.kind() == ErrorKind::TimedOut => {}
                    Err(e) => {
                        // Unplugged, or yanked mid-write. Say so where poll() can see it,
                        // otherwise the entry sits in the map and is never reopened.
                        *thread_status.lock().unwrap() = "error".to_string();
                        *thread_error.lock().unwrap() = Some(e.to_string());
                        let _ = tx.send(Event::Ports);
                        return;
                    }
                }
            }
        });

        println!("{} on {} (USB {})", key, port_name, usb);
        self.open.insert(
            key.clone(),
            Open { port: port_name, usb, writer, heard, alive, status, error, handle: Some(handle) },
        );

        // Ask who this is straight away. Opening the port usually resets an ESP32
        // through the USB bridge's DTR line, so the boot header often arrives by
        // itself, but a board that does not reset would stay unidentified for 30 s.
        let _ = self.send(&key, "#GET\n");
        let _ = self.tx.send(Event::Ports);
    }

    #[cfg(test)]
    pub fn pretend_open(&mut self, key: &str, port: &str) {
        self.pretend_open_writing(key, port, None);
    }

    #[cfg(test)]
    fn pretend_open_writing(&mut self, key: &str, port: &str, writer: Option<Box<dyn Write + Send>>) {
        self.keys.map.insert(port.to_string(), key.to_string());
        self.open.insert(
            key.to_string(),
            Open {
                port: port.to_string(),
                usb: "10c4:ea60".into(),
                writer: writer.map(|w| Arc::new(Mutex::new(w))),
                heard: Arc::new(Mutex::new(Heard::default())),
                alive: Arc::new(Mutex::new(false)),
                status: Arc::new(Mutex::new("open".into())),
                error: Arc::new(Mutex::new(None)),
                handle: None,
            },
        );
    }
}

const PROBE_SECS: u64 = 5;
pub const PROBE_LINES: usize = 20;
pub const PROBE_CHARS: usize = 200;

// Listen to a board for five seconds and say nothing to it. Opening the port still
// pulses DTR, so an ESP32 on the other end probably reboots; that is the price of
// finding out what an unmarked board is, and it is less than sending #GET to
// something that might be a ChipSat. Returns every whole line and how many bytes
// arrived: a board can talk without ever finishing a line, and the shortening for the
// page happens after the lines have been read, not before, or a 35-field CSV row
// loses the fields that identify it.
pub fn probe(port: &str) -> Result<(Vec<String>, usize), String> {
    let mut p = serialport::new(port, BAUD)
        .timeout(Duration::from_millis(200))
        .open()
        .map_err(|e| format!("{}: {}", port, e))?;
    let mut framer = Framer::new();
    let mut buf = [0u8; 4096];
    let mut lines: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    let until = std::time::Instant::now() + Duration::from_secs(PROBE_SECS);
    while std::time::Instant::now() < until {
        match p.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                bytes += n;
                for line in framer.push(&buf[..n]) {
                    lines.push(line);
                }
            }
            Err(ref e) if e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(format!("{}: {}", port, e)),
        }
    }
    Ok((lines, bytes))
}

pub fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

// What a board sounds like, from what it said and nothing else. A receiver of ours
// announces itself with #DG,RX and then sends PKT and HB lines. The older CSV
// receiver writes a header row and then 35 fields a line. Bytes that match neither
// are still bytes, so they are not silence.
pub fn looks_like(lines: &[String], bytes: usize) -> &'static str {
    if bytes == 0 {
        return "silent";
    }
    let ours = lines.iter().any(|l| {
        l.starts_with("#DG,RX") || l.starts_with("PKT,") || l.starts_with("HB,")
    });
    if ours {
        return "descent-receiver";
    }
    let csv = lines
        .iter()
        .any(|l| l.starts_with("Latitude_deg,") || l.split(',').count() == 35);
    if csv {
        return "csv-receiver";
    }
    "something-else"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_is_remembered_by_its_chip_and_serial_not_its_port() {
        let a = board_key(0x10c4, 0xea60, Some("0001"));
        assert_eq!(a, "10c4:ea60:0001");
        // Same board on another socket is the same board.
        assert_eq!(a, board_key(0x10c4, 0xea60, Some("0001")));
        // A different chip is a different board, whatever port it lands on.
        assert_ne!(a, board_key(0x1a86, 0x55d4, Some("0001")));
        assert_eq!(board_key(0x1a86, 0x55d4, None), "1a86:55d4:-");
    }

    #[test]
    fn a_serial_that_is_not_text_is_shown_as_hex() {
        assert_eq!(readable_serial("0001"), "0001");
        assert_eq!(readable_serial(""), "");
        // An ST-Link style descriptor: raw bytes, unreadable as text.
        assert_eq!(readable_serial("\u{1}\u{2}"), "0102");
        assert_ne!(readable_serial("2\u{fffd}o\u{6}"), "2\u{fffd}o\u{6}");
    }

    #[test]
    fn a_port_is_named_by_its_bridge_not_its_hex_id() {
        assert_eq!(bridge_name(0x1a86, 0x55d4), "CH9102");
        assert_eq!(bridge_name(0x0483, 0x3754), "ST-Link");
        assert_eq!(bridge_name(0x1234, 0x5678), "USB serial");
    }

    fn board_at(port: &str, key: &str, state: &str) -> Board {
        Board {
            port: port.into(),
            usb: "1a86:55d4".into(),
            label: "CH9102".into(),
            serial: "x".into(),
            key: key.into(),
            state: state.into(),
            name: String::new(),
        }
    }

    fn scratch(what: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn reloaded(dir: &std::path::Path) -> Hub {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.remember_in(dir);
        hub
    }

    #[test]
    fn a_name_with_spaces_survives_being_saved_and_read_back() {
        let dir = scratch("names");
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.remember_in(&dir);
        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting")];
        hub.approve("/dev/ttyACM1").unwrap();
        hub.set_name("/dev/ttyACM1", " Left T-Beam ").unwrap();

        let again = reloaded(&dir);
        assert_eq!(again.names.get("1a86:55d4:x").map(String::as_str), Some("Left T-Beam"));
        assert!(again.approved.contains("1a86:55d4:x"));   // the old lines still load

        // An empty name clears it, and nothing brings it back.
        hub.set_name("/dev/ttyACM1", "").unwrap();
        assert!(reloaded(&dir).names.is_empty());

        assert!(hub.set_name("/dev/ttyACM1", "two\nlines").unwrap_err().contains("one line"));
        assert!(hub.set_name("/dev/ttyUSB9", "nope").unwrap_err().contains("not plugged in"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_forgotten_board_is_dropped_from_both_files_and_is_not_read_any_more() {
        let dir = scratch("forget");
        let mut hub = reloaded(&dir);
        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting"), board_at("/dev/ttyACM2", "1a86:55d4:y", "waiting")];
        hub.approve("/dev/ttyACM1").unwrap();
        hub.set_name("/dev/ttyACM1", "Left T-Beam").unwrap();
        hub.ignore("/dev/ttyACM2").unwrap();
        // Being read, and it has said which receiver it is.
        hub.pretend_open("rx1", "/dev/ttyACM1");
        hub.set_receiver_name("A4C1F3", "01").unwrap();
        said(&hub, "rx1", "#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8");

        hub.forget("/dev/ttyACM1").unwrap();
        assert!(hub.open.is_empty(), "the port is shut");
        assert_eq!(hub.boards[0].state, "waiting");
        assert_eq!(hub.boards[0].name, "");
        let again = reloaded(&dir);
        assert!(!again.approved.contains("1a86:55d4:x") && again.names.is_empty());
        assert_eq!(again.rx_names.lock().unwrap().get("A4C1F3"), None);
        // The other board is as it was.
        assert!(again.ignored.contains("1a86:55d4:y"));

        // One left alone for good can be forgotten too, and is then asked about again.
        hub.forget("/dev/ttyACM2").unwrap();
        assert_eq!(hub.boards[1].state, "waiting");
        assert!(reloaded(&dir).ignored.is_empty());

        assert!(hub.forget("/dev/ttyUSB9").unwrap_err().contains("not plugged in"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_settings_file_from_before_names_still_loads() {
        let dir = scratch("oldfile");
        std::fs::write(
            dir.join("descent-ground-boards.txt"),
            "# what DeSCENT Ground may open. One board per line.\nreceiver 10c4:ea60:0001\nignore 1a86:7523:-\n",
        )
        .unwrap();
        let hub = reloaded(&dir);
        assert!(hub.approved.contains("10c4:ea60:0001"));
        assert!(hub.ignored.contains("1a86:7523:-"));
        assert!(hub.names.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, p)| (k.to_string(), p.to_string())).collect()
    }

    const T0: u128 = 1_790_000_000_000;

    #[test]
    fn the_board_that_goes_away_and_comes_back_is_the_one_the_operator_meant() {
        let mut w = Identify::start(T0, &pairs(&[("a", "/dev/ttyUSB0"), ("b", "/dev/ttyUSB1")]));
        w.step(T0 + 2_000, &pairs(&[("b", "/dev/ttyUSB1")]));
        assert_eq!(w.found, None);
        // Back on another socket, which is the usual way a replug comes back.
        w.step(T0 + 4_000, &pairs(&[("a", "/dev/ttyUSB2"), ("b", "/dev/ttyUSB1")]));
        assert_eq!(w.found.as_deref(), Some("/dev/ttyUSB2"));
        // A later board coming back does not overwrite the first answer.
        w.step(T0 + 6_000, &pairs(&[("a", "/dev/ttyUSB2")]));
        w.step(T0 + 8_000, &pairs(&[("a", "/dev/ttyUSB2"), ("b", "/dev/ttyUSB1")]));
        assert_eq!(w.found.as_deref(), Some("/dev/ttyUSB2"));
    }

    #[test]
    fn two_boards_with_the_same_serial_are_still_told_apart() {
        // Both sockets carry 10c4:ea60:0001, so only the port says which moved.
        let both = pairs(&[("k", "/dev/ttyUSB0"), ("k", "/dev/ttyUSB1")]);
        let mut w = Identify::start(T0, &both);
        w.step(T0 + 2_000, &pairs(&[("k", "/dev/ttyUSB0")]));
        assert_eq!(w.found, None);
        w.step(T0 + 4_000, &both);
        assert_eq!(w.found.as_deref(), Some("/dev/ttyUSB1"));
    }

    #[test]
    fn a_board_that_never_comes_back_is_no_answer_and_the_watch_runs_out() {
        let mut w = Identify::start(T0, &pairs(&[("a", "/dev/ttyUSB0")]));
        w.step(T0 + 2_000, &pairs(&[]));
        assert_eq!(w.found, None);
        assert_eq!(w.seconds_left(T0 + 2_000), 58);
        assert!(!w.expired(T0 + 59_000));
        w.step(T0 + 59_000, &pairs(&[]));
        assert_eq!(w.found, None);
        // After a minute the watch is over, and a replug then is nobody's answer.
        assert!(w.expired(T0 + 60_000));
        assert_eq!(w.seconds_left(T0 + 60_000), 0);
        w.step(T0 + 61_000, &pairs(&[("a", "/dev/ttyUSB0")]));
        assert_eq!(w.found, None);
    }

    #[test]
    fn a_watch_starts_from_what_is_plugged_in_and_a_second_one_forgets_the_first() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        assert_eq!(hub.identify_state(), (false, None, 0));
        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting")];
        hub.identify_start();
        let (watching, found, left) = hub.identify_state();
        assert!(watching);
        assert_eq!(found, None);
        assert_eq!(left, 60);

        hub.identify.as_mut().unwrap().found = Some("/dev/ttyACM1".into());
        assert_eq!(hub.identify_state(), (false, Some("/dev/ttyACM1".into()), 0));
        hub.identify_start();
        assert_eq!(hub.identify_state().1, None);
    }

    #[test]
    fn a_watch_that_is_stopped_is_over_at_once_with_nothing_found() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        // Nothing to stop is not an error.
        hub.identify_stop();
        assert_eq!(hub.identify_state(), (false, None, 0));

        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting")];
        hub.identify_start();
        assert!(hub.identify_state().0);
        hub.identify_stop();
        assert_eq!(hub.identify_state(), (false, None, 0));

        // An answer it already had goes too: the operator has stopped asking.
        hub.identify_start();
        hub.identify.as_mut().unwrap().found = Some("/dev/ttyACM1".into());
        hub.identify_stop();
        assert_eq!(hub.identify_state(), (false, None, 0));
        // And a watch can be started again afterwards.
        hub.identify_start();
        assert!(hub.identify_state().0);
    }

    const HEADER: &str = "#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8";

    // A line arriving on a port that is being read.
    fn said(hub: &Hub, key: &str, line: &str) -> bool {
        let o = &hub.open[key];
        notice(line, &o.heard, &hub.rx_names, o.writer.as_ref())
    }

    fn text(wire: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(wire.lock().unwrap().clone()).unwrap()
    }

    const UNNAMED: &str = "#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=,fw=1.2.1";
    const NAMED: &str = "#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=01,fw=1.2.1";

    // A hub that already has a name kept for the receiver A4C1F3, which is not plugged in.
    fn keeping(name: &str) -> Hub {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        assert_eq!(hub.set_receiver_name("A4C1F3", name), Ok(false));
        hub
    }

    #[test]
    fn a_board_that_comes_back_without_its_name_is_given_it_once() {
        // A flash writes over the part of the board's flash its settings are kept in, so
        // it reboots with no name and says so in its first header.
        let mut hub = keeping("01");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");
        // The same header 30 s later, from a board that turned the name down.
        said(&hub, "rx1", UNNAMED);
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n", "it is asked once, not every half minute");

        // It took the name, and is flashed again later without the port being closed.
        said(&hub, "rx1", NAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n#SET,name,01\n");
    }

    #[test]
    fn a_board_that_already_has_a_name_is_left_with_it() {
        let mut hub = keeping("01");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", NAMED);
        // Even one this computer does not call it by: the board's own name stands.
        said(&hub, "rx1", &NAMED.replace("name=01", "name=roof"));
        assert_eq!(text(&wire), "");
        assert_eq!(hub.ports()[0].rx_name.as_deref(), Some("roof"));
        assert_eq!(hub.ports()[0].name.as_deref(), Some("01"));
    }

    #[test]
    fn a_board_whose_header_has_no_name_in_it_is_not_sent_one() {
        let mut hub = keeping("01");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", HEADER);
        said(&hub, "rx1", &format!("{},fw=9.9.9", HEADER));
        assert_eq!(text(&wire), "");
    }

    #[test]
    fn a_board_this_computer_has_no_name_for_is_not_sent_one() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        // A name is kept, for another receiver.
        hub.set_receiver_name("00B2C4", "02").unwrap();
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "");

        // Named from the page, it is told then. If it turns that down, the header that
        // follows is not a reason to tell it again.
        assert_eq!(hub.set_receiver_name("A4C1F3", "01"), Ok(true));
        assert_eq!(text(&wire), "#SET,name,01\n");
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");
    }

    #[test]
    fn a_name_cleared_from_the_page_is_not_sent_back_to_the_board() {
        let mut hub = keeping("01");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", NAMED);
        assert_eq!(hub.set_receiver_name("A4C1F3", ""), Ok(true));
        assert_eq!(text(&wire), "#SET,name,\n");
        said(&hub, "rx1", UNNAMED);
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,\n");
    }

    #[test]
    fn a_port_that_is_opened_again_may_be_sent_the_name_again() {
        let mut hub = keeping("01");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", UNNAMED);
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");

        // Unplugged and back, or handed to a flash and taken back.
        hub.close("rx1");
        let wire = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");

        // Another board put on the same port is another board to ask.
        said(&hub, "rx1", "#DG,RX,v1,id=00B2C4,f=915.0,name=,fw=1.2.1");
        assert_eq!(text(&wire), "#SET,name,01\n", "nothing is kept for that one");
        hub.rx_names.lock().unwrap().set("00B2C4", "02").unwrap();
        said(&hub, "rx1", "#DG,RX,v1,id=00B2C4,f=915.0,name=,fw=1.2.1,sf=9");
        assert_eq!(text(&wire), "#SET,name,01\n#SET,name,02\n");
    }

    #[test]
    fn a_name_that_could_not_be_written_is_tried_again_with_the_next_header() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::TimedOut.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut hub = keeping("01");
        hub.pretend_open_writing("rx1", "/dev/ttyUSB0", Some(Box::new(Broken)));
        assert!(said(&hub, "rx1", UNNAMED), "the header is still news when the write fails");
        // The port has come good by the next header.
        let wire = Arc::new(Mutex::new(Vec::new()));
        *hub.open["rx1"].writer.as_ref().unwrap().lock().unwrap() = Box::new(Wire(wire.clone()));
        said(&hub, "rx1", UNNAMED);
        assert_eq!(text(&wire), "#SET,name,01\n");
    }

    // Something to write to in place of a port, that keeps what was written.
    struct Wire(Arc<Mutex<Vec<u8>>>);

    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn wired(hub: &mut Hub, key: &str, port: &str) -> Arc<Mutex<Vec<u8>>> {
        let sent = Arc::new(Mutex::new(Vec::new()));
        hub.pretend_open_writing(key, port, Some(Box::new(Wire(sent.clone()))));
        sent
    }

    #[test]
    fn a_header_line_tells_the_port_which_receiver_is_on_it() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        assert_eq!(hub.ports()[0].rx_id, None);

        assert!(!said(&hub, "rx1", "PKT,55,00FF,-50.0,13.75,-12"));
        assert_eq!(hub.ports()[0].rx_id, None);

        assert!(said(&hub, "rx1", HEADER));
        let p = &hub.ports()[0];
        assert_eq!(p.rx_id.as_deref(), Some("A4C1F3"));
        assert_eq!(p.rx_name, None);
        assert_eq!(p.name, None);
        assert_eq!(p.key, "rx1", "the key is still the order the port was found in");
        // The same header every 30 s is not news.
        assert!(!said(&hub, "rx1", HEADER));
        // Half a line after a reset does not unlearn it.
        assert!(!said(&hub, "rx1", "#DG,RX,v1,id=A4"));
        assert_eq!(hub.ports()[0].rx_id.as_deref(), Some("A4C1F3"));
    }

    #[test]
    fn a_header_with_a_name_says_what_the_board_calls_itself() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        assert!(said(&hub, "rx1", &format!("{},name=Left one", HEADER)));
        assert_eq!(hub.ports()[0].rx_name.as_deref(), Some("Left one"));
        // Firmware that takes a name and has none says so with an empty one.
        assert!(said(&hub, "rx1", &format!("{},name=", HEADER)));
        assert_eq!(hub.ports()[0].rx_name, None);
        assert_eq!(hub.ports()[0].rx_id.as_deref(), Some("A4C1F3"));
        // Another board on the same port, after a swap without a replug being seen.
        assert!(said(&hub, "rx1", "#DG,RX,v1,id=00B2C4,f=915.0,name=02"));
        assert_eq!(hub.ports()[0].rx_id.as_deref(), Some("00B2C4"));
        assert_eq!(hub.ports()[0].rx_name.as_deref(), Some("02"));
    }

    #[test]
    fn a_header_with_a_firmware_version_is_remembered_for_its_port() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        hub.pretend_open("rx2", "/dev/ttyUSB1");
        let fw = |hub: &Hub, key: &str| hub.ports().into_iter().find(|p| p.key == key).unwrap().rx_fw;
        assert_eq!(fw(&hub, "rx1"), None, "a port that has not announced itself");

        assert!(said(&hub, "rx1", &format!("{},name=Left one,fw=1.2.1", HEADER)));
        assert!(said(&hub, "rx2", HEADER));
        assert_eq!(fw(&hub, "rx1").as_deref(), Some("1.2.1"));
        assert_eq!(fw(&hub, "rx2"), None, "firmware from before it said");

        // Reflashed where it sits: the next header is news, and the page is told.
        assert!(said(&hub, "rx1", &format!("{},name=Left one,fw=1.3.0", HEADER)));
        assert_eq!(fw(&hub, "rx1").as_deref(), Some("1.3.0"));
        // What followed fw= did not read as a version, so there is none to show.
        assert!(said(&hub, "rx1", &format!("{},name=Left one,fw=1.3.0PKT", HEADER)));
        assert_eq!(fw(&hub, "rx1"), None);
        assert_eq!(hub.ports().into_iter().find(|p| p.key == "rx1").unwrap().rx_name.as_deref(), Some("Left one"));
    }

    #[test]
    fn whether_a_board_is_sent_its_name_does_not_go_by_its_firmware_version() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        let versioned = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        let named = wired(&mut hub, "rx2", "/dev/ttyUSB1");
        // A version and no name= in the header: nothing says it takes a name.
        said(&hub, "rx1", &format!("{},fw=9.9.9", HEADER));
        // A name= and no version, which is what 1.2.0 printed.
        said(&hub, "rx2", "#DG,RX,v1,id=00B2C4,f=915.0,name=");

        assert_eq!(hub.set_receiver_name("A4C1F3", "01"), Ok(false));
        assert!(versioned.lock().unwrap().is_empty());
        assert_eq!(hub.set_receiver_name("00B2C4", "02"), Ok(true));
        assert_eq!(named.lock().unwrap().as_slice(), b"#SET,name,02\n");
    }

    #[test]
    fn a_receivers_name_is_kept_against_its_id_whichever_port_it_is_on() {
        let dir = scratch("rxnames");
        let mut hub = reloaded(&dir);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", HEADER);
        // Whatever case the id comes back in, it is the same receiver.
        assert_eq!(hub.set_receiver_name("a4c1f3", " the \"left\" one "), Ok(false));
        assert_eq!(hub.ports()[0].name.as_deref(), Some("the \"left\" one"));
        // One that is not plugged in can be named too.
        assert_eq!(hub.set_receiver_name("00B2C4", "02"), Ok(false));

        // Another day, the other socket, the other key.
        let mut again = reloaded(&dir);
        again.pretend_open("rx2", "/dev/ttyUSB3");
        assert_eq!(again.ports()[0].name, None, "nothing is known about a port until its header arrives");
        said(&again, "rx2", HEADER);
        assert_eq!(again.ports()[0].name.as_deref(), Some("the \"left\" one"));
        assert_eq!(again.ports()[0].key, "rx2");

        assert_eq!(again.set_receiver_name("A4C1F3", ""), Ok(false));
        assert_eq!(again.ports()[0].name, None);
        assert_eq!(reloaded(&dir).rx_names.lock().unwrap().get("A4C1F3"), None);
        assert_eq!(reloaded(&dir).rx_names.lock().unwrap().get("00B2C4"), Some("02"));

        // The serial side's own file is written out whole from what it knows, and these
        // names are not in what it knows.
        hub.boards = vec![board_at("/dev/ttyUSB0", "10c4:ea60:0001", "waiting")];
        hub.approve("/dev/ttyUSB0").unwrap();
        assert_eq!(reloaded(&dir).rx_names.lock().unwrap().get("00B2C4"), Some("02"));
        assert!(!std::fs::read_to_string(dir.join("descent-ground-boards.txt")).unwrap().contains("00B2C4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_id_or_a_name_with_a_comma_is_refused_and_nothing_is_kept() {
        let dir = scratch("rxrefused");
        let mut hub = reloaded(&dir);
        let sent = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        said(&hub, "rx1", &format!("{},name=", HEADER));

        for id in ["", "rx1", "A4C1F", "A4C1F3A", "A4C1FZ", "/dev/ttyUSB0"] {
            let e = hub.set_receiver_name(id, "01").unwrap_err();
            assert!(e.contains("six hex"), "{:?}: {}", id, e);
        }
        let e = hub.set_receiver_name("A4C1F3", "left, roof").unwrap_err();
        assert!(e.contains("comma"), "{}", e);
        assert!(hub.set_receiver_name("A4C1F3", "left\troof").is_err());

        assert_eq!(hub.ports()[0].name, None);
        assert!(sent.lock().unwrap().is_empty(), "nothing goes to the board for a name that was refused");
        assert!(!dir.join(receivers::FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_name_goes_to_the_board_only_when_its_header_said_it_takes_one() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        let old = wired(&mut hub, "rx1", "/dev/ttyUSB0");
        let new = wired(&mut hub, "rx2", "/dev/ttyUSB1");
        said(&hub, "rx1", HEADER);
        said(&hub, "rx2", "#DG,RX,v1,id=00B2C4,f=915.0,name=");

        // Firmware from before names: kept here, and the board is left alone.
        assert_eq!(hub.set_receiver_name("A4C1F3", "01"), Ok(false));
        assert!(old.lock().unwrap().is_empty());
        assert!(new.lock().unwrap().is_empty());

        assert_eq!(hub.set_receiver_name("00b2c4", "Left one"), Ok(true));
        assert_eq!(new.lock().unwrap().as_slice(), b"#SET,name,Left one\n");
        assert!(old.lock().unwrap().is_empty(), "the other receiver is not told");
        assert_eq!(hub.ports().iter().find(|p| p.key == "rx2").unwrap().name.as_deref(), Some("Left one"));

        // One that is not being read has nobody to tell.
        assert_eq!(hub.set_receiver_name("123456", "03"), Ok(false));
        assert_eq!(new.lock().unwrap().len(), "#SET,name,Left one\n".len());

        // Clearing the name clears it on the board too.
        new.lock().unwrap().clear();
        assert_eq!(hub.set_receiver_name("00B2C4", ""), Ok(true));
        assert_eq!(new.lock().unwrap().as_slice(), b"#SET,name,\n");

        // Once the firmware on that port has changed, so has the answer. Its first header
        // has no name in it, so it is given the one kept for it without being asked.
        said(&hub, "rx1", &format!("{},name=", HEADER));
        assert_eq!(old.lock().unwrap().as_slice(), b"#SET,name,01\n");
        old.lock().unwrap().clear();
        assert_eq!(hub.set_receiver_name("A4C1F3", "roof"), Ok(true));
        assert_eq!(old.lock().unwrap().as_slice(), b"#SET,name,roof\n");
    }

    #[test]
    fn a_name_that_could_not_be_sent_is_still_kept_and_the_error_says_so() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");   // open, with nothing to write to
        said(&hub, "rx1", &format!("{},name=", HEADER));
        let e = hub.set_receiver_name("A4C1F3", "01").unwrap_err();
        assert!(e.contains("kept on this computer") && e.contains("rx1"), "{}", e);
        assert_eq!(hub.ports()[0].name.as_deref(), Some("01"));
    }

    #[test]
    fn what_a_board_is_is_judged_on_what_it_said() {
        assert_eq!(looks_like(&[], 0), "silent");
        assert_eq!(looks_like(&["#DG,RX,1,sf9".into()], 12), "descent-receiver");
        assert_eq!(looks_like(&["PKT,55,00FF,-50.0,13.75,-12".into()], 28), "descent-receiver");
        assert_eq!(looks_like(&["HB,1,2,3".into()], 8), "descent-receiver");
        assert_eq!(looks_like(&["Latitude_deg,Longitude_deg,Alt_m".into()], 32), "csv-receiver");
        let row = (0..35).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let n = row.len();
        assert_eq!(looks_like(&[row], n), "csv-receiver");
        // A real CSV row is longer than the 200 characters the page is shown, so the
        // counting has to happen before anything is shortened.
        let wide = (0..35).map(|i| format!("{}.000000", i)).collect::<Vec<_>>().join(",");
        assert!(wide.len() > PROBE_CHARS);
        let n = wide.len();
        assert_eq!(looks_like(&[wide.clone()], n), "csv-receiver");
        assert_eq!(looks_like(&[clip(&wide, PROBE_CHARS)], n), "something-else");
        assert_eq!(clip("abcdef", 3), "abc");
        assert_eq!(clip("ab", 8), "ab");
        assert_eq!(looks_like(&["ets Jun  8 2016 00:22:57".into()], 24), "something-else");
        // Bytes that never finished a line are not silence.
        assert_eq!(looks_like(&[], 17), "something-else");
    }

    #[test]
    fn a_five_second_listen_keeps_everything_else_off_the_port() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting")];
        hub.begin_probe("/dev/ttyACM1").unwrap();
        assert!(hub.begin_probe("/dev/ttyACM1").unwrap_err().contains("already being listened to"));
        assert!(hub.release_for_flash("/dev/ttyACM1").unwrap_err().contains("listened to"));
        hub.end_probe("/dev/ttyACM1");
        hub.begin_probe("/dev/ttyACM1").unwrap();
        hub.end_probe("/dev/ttyACM1");

        // An open receiver is not something to listen in on, and neither is a flash.
        hub.pretend_open("rx1", "/dev/ttyACM1");
        assert!(hub.begin_probe("/dev/ttyACM1").unwrap_err().contains("open as a receiver"));
        hub.close("rx1");
        hub.release_for_flash("/dev/ttyACM1").unwrap();
        assert!(hub.begin_probe("/dev/ttyACM1").unwrap_err().contains("being flashed"));
        assert!(hub.begin_probe("/dev/ttyUSB9").unwrap_err().contains("not plugged in"));
    }

    #[test]
    fn nothing_is_a_receiver_until_it_is_said_to_be() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.boards = vec![board_at("/dev/ttyACM1", "1a86:55d4:x", "waiting")];
        assert!(!hub.approved.contains("1a86:55d4:x"));

        hub.approve("/dev/ttyACM1").unwrap();
        assert!(hub.approved.contains("1a86:55d4:x"));

        hub.ignore("/dev/ttyACM1").unwrap();
        assert!(!hub.approved.contains("1a86:55d4:x"));
        assert!(hub.ignored.contains("1a86:55d4:x"));

        // A board that is not plugged in cannot be decided about.
        assert!(hub.approve("/dev/ttyUSB9").unwrap_err().contains("not plugged in"));
    }

    #[test]
    fn a_by_hand_disconnect_is_not_undone_two_seconds_later() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        hub.dismiss("rx1");
        assert!(hub.dismissed.contains("/dev/ttyUSB0"));
        // poll() skips a dismissed port, and only a replug clears it.
        hub.dismissed.retain(|p| p == "/dev/ttyUSB0");
        assert!(hub.dismissed.contains("/dev/ttyUSB0"));
    }

    #[test]
    fn each_port_keeps_its_own_key_so_two_receivers_never_merge() {
        let mut keys = Keys::new();
        let a = keys.key_for("/dev/ttyUSB0");
        let b = keys.key_for("/dev/ttyUSB1");
        assert_eq!(a, "rx1");
        assert_eq!(b, "rx2");
        assert_eq!(keys.key_for("/dev/ttyUSB0"), "rx1");   // a replug keeps its key
        assert_ne!(a, b);
    }

    #[test]
    fn writing_to_a_port_that_is_gone_is_an_error_not_a_panic() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = Hub::new(tx);
        let err = hub.send("rx9", "#GET\n").unwrap_err();
        assert!(err.contains("rx9"));
    }
}
