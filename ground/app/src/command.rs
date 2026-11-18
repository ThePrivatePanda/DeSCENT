// Receivers that are programs rather than USB ports: an SDR decoder named on a
// `receiver = <command line>` line in descent-ground.conf, or on its one `sdr = <command line>`
// line, started through the shell and read a line at a time off its stdout. The contract the
// program keeps is docs/sdr-receiver.md.
//
// A receiver = line's key is cmd:<n>, by the line's place in the file, so it moves when the file
// is edited; the sdr = line's is cmd:sdr. The id in its header is what names and the page know it
// by. Nothing is ever written to it. Its stdin is shut, and it is not in the hub's open ports,
// so neither the SF re-ask nor the name setting in serial.rs can reach it.
//
// The page can turn a program off and on, and start it again. Each runs in a process group of
// its own (on Windows, taskkill walks its tree) so that stopping it stops what it started as well:
// an SDR program holds the dongle through the rtl_sdr it runs, and one left behind keeps the next
// start from opening it. The sdr = program is the app's own: the page picks the gain it is started
// with (--gain), and that and whether it is on are kept in descent-ground-sdr.txt. Turning a
// receiver = line off lasts until the app is started again.
//
// With no sdr = line, cmd:sdr is the built-in SDR (sdr.rs) when the build carries one: off until
// the page first turns it on, kept like an sdr = program, and while no RTL-SDR is plugged in it
// says so and looks again every second rather than starting.
use crate::chips::{word, write_whole};
use crate::framing::Framer;
use crate::json::esc;
use crate::logfile::now_ms;
use crate::receivers::{self, Header};
use crate::sdr;
use crate::serial::{Event, PortInfo};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const SDR_KEY: &str = "cmd:sdr";
pub const FILE: &str = "descent-ground-sdr.txt";

// A program that keeps dying is started again after 1 s, then 2, 4 ... up to a minute, so a
// station that is off the network is not hammered with ssh logins. One that ran for a minute
// before it stopped starts again from the short end.
const FIRST_WAIT: Duration = Duration::from_secs(1);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

// What the page shows of a program's stderr. Enough for rtl_sdr's start-up chatter and the
// line saying why it stopped.
const SAID_LINES: usize = 50;

struct Seen {
    header: Option<Header>,
    header_line: Option<(u128, String)>,   // the last header as printed, and when
    status: String,          // open while it runs, stopped while it waits to be started again, off
    error: Option<String>,   // why it stopped, while it is stopped
    // why it last stopped, when, and whether the app stopped it; kept while it runs again
    last_exit: Option<(u128, String, bool)>,
    on: bool,
    gain: Option<String>,    // what the page picked for the sdr = program, passed as --gain
    gain_at: Option<u128>,
    pid: Option<u32>,
    started_ms: Option<u128>,
    starts: u32,
    next_ms: Option<u128>,   // when it is started again, while it waits
    stopping: Option<String>,   // why the app is ending this run, in words; the next start does not wait
    hurry: bool,             // asked to start again now, while it waits
    fatal: Option<String>,   // the code of the #DG,FATAL it printed, in this run or the last
    last_words: Option<String>,   // its last stderr line in the run that ended
    said: VecDeque<(u128, String)>,
}

struct Shared {
    seen: Mutex<Seen>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct Source {
    key: String,
    command: String,
    file: Option<PathBuf>,   // the sdr = program's settings; None for a receiver = line
    builtin: Option<PathBuf>,   // the data folder the built-in SDR unpacks into, for the built-in one
    shared: Arc<Shared>,
    tx: Sender<Event>,
}

// What may follow --gain on a line the shell reads: auto, or dB as digits with at most one point.
// Nothing else gets near the shell.
pub fn a_gain(text: &str) -> Result<String, String> {
    let t = text.trim();
    let (whole, fraction) = t.split_once('.').unwrap_or((t, "0"));
    let digits = |d: &str| !d.is_empty() && d.len() <= 3 && d.bytes().all(|b| b.is_ascii_digit());
    if t == "auto" || (digits(whole) && digits(fraction)) {
        Ok(t.to_string())
    } else {
        Err(format!("a gain is auto or a number of dB such as 20.7, and {:?} is not", t))
    }
}

const SETTINGS_HEADER: &str = "# The SDR on the sdr = line of descent-ground.conf, as the page left it. Written by DeSCENT Ground.\n\
# on yes|no\n\
# gain <dB>|auto, passed to the program as --gain. With no gain line it is started without one.\n";

// No file is the program as its conf line has it, on and with no --gain; or the built-in SDR, off.
fn load(path: &Path, on: bool) -> (bool, Option<String>) {
    let mut on = on;
    let mut gain = None;
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        match word(line) {
            Some(("on", v)) => on = v.trim() != "no",
            Some(("gain", v)) => gain = a_gain(v).ok(),
            _ => {}
        }
    }
    (on, gain)
}

fn save(path: &Path, on: bool, gain: Option<&str>) -> Result<(), String> {
    let mut text = format!("{}on {}\n", SETTINGS_HEADER, if on { "yes" } else { "no" });
    if let Some(g) = gain {
        text += &format!("gain {}\n", g);
    }
    write_whole(path, &text)
}

pub fn start(commands: &[String], sdr: Option<&str>, dir: &Path, data: &Path, tx: &Sender<Event>) -> Vec<Source> {
    let mut all: Vec<Source> =
        commands.iter().enumerate().map(|(i, c)| Source::new(&format!("cmd:{}", i + 1), c, None, None, tx)).collect();
    if let Some(c) = sdr {
        all.push(Source::new(SDR_KEY, c, Some(dir.join(FILE)), None, tx));
    } else if sdr::carried() {
        let command = format!("built-in SDR: rtl_sdr and file_lora in {}", sdr::folder(data).display());
        all.push(Source::new(SDR_KEY, &command, Some(dir.join(FILE)), Some(data.to_path_buf()), tx));
    }
    for s in &all {
        std::thread::spawn({
            let s = s.clone();
            move || s.keep_running()
        });
    }
    all
}

impl Source {
    fn new(key: &str, command: &str, file: Option<PathBuf>, builtin: Option<PathBuf>, tx: &Sender<Event>) -> Source {
        let (on, gain) = file.as_deref().map_or((true, None), |f| load(f, builtin.is_none()));
        let seen = Seen {
            header: None,
            header_line: None,
            status: String::from(if on { "stopped" } else { "off" }),
            error: None,
            last_exit: None,
            on,
            gain,
            gain_at: None,
            pid: None,
            started_ms: None,
            starts: 0,
            next_ms: None,
            stopping: None,
            hurry: false,
            fatal: None,
            last_words: None,
            said: VecDeque::new(),
        };
        Source {
            key: key.to_string(),
            command: command.to_string(),
            file,
            builtin,
            shared: Arc::new(Shared { seen: Mutex::new(seen), wake: Condvar::new() }),
            tx: tx.clone(),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    // As a port, so it is listed wherever the USB receivers are. port is the command line and
    // usb is empty: there is no adapter.
    pub fn port_info(&self, names: &receivers::Names) -> PortInfo {
        let seen = self.shared.seen.lock().unwrap();
        let header = seen.header.as_ref();
        let rx_id = header.map(|h| h.id.clone());
        PortInfo {
            key: self.key.clone(),
            port: self.command.clone(),
            usb: String::new(),
            status: seen.status.clone(),
            error: seen.error.clone(),
            name: rx_id.as_deref().and_then(|id| names.get(id)).map(String::from),
            rx_name: header.and_then(|h| h.name.clone()).filter(|n| !n.is_empty()),
            rx_fw: header.and_then(|h| h.fw.clone()),
            rx_kind: header.and_then(|h| h.kind.clone()),
            rx_id,
            role: crate::serial::Role::Receiver,
            csid: None,
        }
    }

    fn told(&self) {
        let _ = self.tx.send(Event::Ports);
    }

    fn keep_running(&self) {
        let mut wait = Duration::ZERO;
        loop {
            let gain = {
                let mut s = self.shared.seen.lock().unwrap();
                while !s.on {
                    s = self.shared.wake.wait(s).unwrap();
                }
                s.hurry = false;
                s.gain.clone()
            };
            if self.builtin.is_some() && self.no_dongle() {
                continue;
            }
            let began = Instant::now();
            let why = match &self.builtin {
                Some(data) => self.run_builtin(data, gain.as_deref()),
                None => self.run_shell(&match (&self.file, gain) {
                    (Some(_), Some(g)) => format!("{} --gain {}", self.command, g),
                    _ => self.command.clone(),
                }),
            };
            let mut s = self.shared.seen.lock().unwrap();
            s.pid = None;
            if let Some(words) = s.stopping.take() {
                println!("{}: {} ({})", self.key, words, why);
                s.error = Some(words.clone());
                s.last_exit = Some((now_ms(), words, true));
                s.status = String::from(if s.on { "stopped" } else { "off" });
                wait = Duration::ZERO;
                drop(s);
                self.told();
                continue;
            }
            s.error = Some(why.clone());
            s.last_exit = Some((now_ms(), why.clone(), false));
            wait = next_wait(began.elapsed(), wait);
            println!("{}: {}; starting it again in {} s", self.key, why, wait.as_secs());
            s.status = String::from("stopped");
            s.next_ms = Some(now_ms() + wait.as_millis());
            drop(s);
            self.told();
            let deadline = Instant::now() + wait;
            let mut s = self.shared.seen.lock().unwrap();
            while s.on && !s.hurry {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                s = self.shared.wake.wait_timeout(s, left).unwrap().0;
            }
            if s.hurry {
                wait = Duration::ZERO;
            }
            s.next_ms = None;
            if !s.on {
                s.status = String::from("off");
                drop(s);
                self.told();
            }
        }
    }

    fn running_as(&self, pid: u32) {
        {
            let mut s = self.shared.seen.lock().unwrap();
            s.pid = Some(pid);
            s.started_ms = Some(now_ms());
            s.starts += 1;
            s.fatal = None;
            s.status = String::from("open");
            s.error = None;
        }
        self.told();
    }

    fn run_shell(&self, line: &str) -> String {
        match shell(line).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(mut child) => {
                println!("{}: started {}", self.key, line);
                self.running_as(child.id());
                let said = self.keep_stderr(child.stderr.take().unwrap());
                pump(child.stdout.take().unwrap(), &self.key, &self.shared.seen, &self.tx);
                let status = child.wait();
                self.shared.seen.lock().unwrap().last_words = said.join().unwrap_or(None);
                match status {
                    Ok(status) => format!("stopped, {}", status),
                    Err(e) => format!("stopped, and could not be asked why: {}", e),
                }
            }
            Err(e) => format!("could not be started: {}", e),
        }
    }

    // pid is rtl_sdr's, so ending it ends the run: the decoders finish on the end of their stdin.
    fn run_builtin(&self, data: &Path, gain: Option<&str>) -> String {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let ready = sdr::unpack(data).and_then(|tools| {
            let home = home.ok_or_else(|| String::from("neither HOME nor USERPROFILE is set, so there is nowhere to keep the SDR's id"))?;
            Ok((tools, sdr::receiver_id(&home)?))
        });
        let (tools, id) = match ready {
            Ok(t) => t,
            Err(e) => return format!("could not be started: {}", e),
        };
        let mut last = None;
        let ran = sdr::run(&tools, sdr::Input::Dongle { gain }, &id, |pid| {
            println!("{}: started rtl_sdr and file_lora from {}", self.key, tools.display());
            self.running_as(pid);
        }, &mut |o| match o {
            sdr::Out::Line(line) => heard(&self.key, now_ms(), line, &self.shared.seen, &self.tx),
            sdr::Out::Said(text) => {
                self.say(&text);
                last = Some(text);
            }
        });
        self.shared.seen.lock().unwrap().last_words = last;
        match ran {
            Ok(why) => format!("stopped: {}", why),
            Err(e) => format!("could not be started: {}", e),
        }
    }

    // True while no RTL-SDR is plugged in, after a second's wait or less, said on the page as a
    // stop with its FATAL code and on the console once.
    fn no_dongle(&self) -> bool {
        let absent = match sdr::dongle_present() {
            Ok(present) => !present,
            Err(e) => {
                self.say(&e);
                false
            }
        };
        if !absent {
            return false;
        }
        let mut s = self.shared.seen.lock().unwrap();
        if s.error.as_deref() != Some(sdr::NO_DONGLE) {
            println!("{}: {}; waiting for one", self.key, sdr::NO_DONGLE);
            s.error = Some(sdr::NO_DONGLE.to_string());
            s.fatal = Some(String::from("2"));
            s.last_words = Some(sdr::NO_DONGLE.to_string());
            s.status = String::from("stopped");
        }
        s.next_ms = Some(now_ms() + 1000);
        drop(s);
        self.told();
        let s = self.shared.seen.lock().unwrap();
        let mut s = self.shared.wake.wait_timeout_while(s, FIRST_WAIT, |s| s.on && !s.hurry).unwrap().0;
        s.next_ms = None;
        if !s.on {
            s.status = String::from("off");
            s.error = None;
        }
        true
    }

    fn say(&self, text: &str) {
        say(&self.key, &self.shared, text);
    }

    // Its complaints go to this app's own output, where an operator reads ours, and the last
    // of them to the page. The thread's answer is the last line of the run.
    fn keep_stderr(&self, err: impl Read + Send + 'static) -> std::thread::JoinHandle<Option<String>> {
        let (key, shared) = (self.key.clone(), self.shared.clone());
        std::thread::spawn(move || {
            let mut last = None;
            for line in BufReader::new(err).split(b'\n').map_while(Result::ok) {
                let text = String::from_utf8_lossy(&line).trim_end().to_string();
                if text.is_empty() {
                    continue;
                }
                say(&key, &shared, &text);
                last = Some(text);
            }
            last
        })
    }

    // The pid of a run to end, marked with why so the next start does not wait.
    fn running(s: &mut Seen, why: &str) -> Option<u32> {
        let pid = s.pid?;
        s.stopping = Some(why.to_string());
        Some(pid)
    }

    // Off stops it and keeps it stopped; on starts it at once. The answer is whether everything it
    // had started has gone, which it has when it was not running.
    pub fn turn(&self, on: bool) -> Result<bool, String> {
        let pid = {
            let mut s = self.shared.seen.lock().unwrap();
            if let Some(f) = &self.file {
                save(f, on, s.gain.as_deref())?;
            }
            s.on = on;
            s.hurry = on;
            if !on && s.pid.is_none() {
                s.status = String::from("off");
                s.next_ms = None;
            }
            if on { None } else { Self::running(&mut s, "Turned off from the page") }
        };
        self.shared.wake.notify_all();
        self.told();
        Ok(pid.is_none_or(end_tree))
    }

    // Now, and with the wait between starts back at its shortest.
    pub fn restart(&self) -> Result<bool, String> {
        self.restart_for("Restarted from the page")
    }

    fn restart_for(&self, why: &str) -> Result<bool, String> {
        let pid = {
            let mut s = self.shared.seen.lock().unwrap();
            if !s.on {
                return Err(format!("{} is off; turn it on instead", self.key));
            }
            s.hurry = true;
            Self::running(&mut s, why)
        };
        self.shared.wake.notify_all();
        Ok(pid.is_none_or(end_tree))
    }

    // Kept, then the program started again with it if it is on. Only the sdr = program: the
    // command on a receiver = line is the operator's, gain and all.
    pub fn set_gain(&self, gain: &str) -> Result<bool, String> {
        let Some(f) = &self.file else {
            return Err(format!("{} is started from a receiver = line; its gain is set on that line in descent-ground.conf", self.key));
        };
        let gain = a_gain(gain)?;
        let why = match gain.as_str() {
            "auto" => String::from("Restarted with automatic gain"),
            g => format!("Restarted with gain {} dB", g),
        };
        let on = {
            let mut s = self.shared.seen.lock().unwrap();
            save(f, s.on, Some(&gain))?;
            s.gain = Some(gain);
            s.gain_at = Some(now_ms());
            s.on
        };
        if on { self.restart_for(&why) } else { Ok(true) }
    }

    // On the way out of the app. Nothing is saved: it is on again at the next start.
    pub fn end(&self) {
        let pid = {
            let mut s = self.shared.seen.lock().unwrap();
            s.on = false;
            Self::running(&mut s, "Stopped as the app closed")
        };
        self.shared.wake.notify_all();
        if let Some(pid) = pid {
            end_tree(pid);
        }
    }

    // What the page shows of it. state: off, starting (running, no header yet), running,
    // stopping, or stopped (waiting until nextAt to start again). The header is the last one as
    // the program printed it, so its gain= and sfs= are read off it whatever the app makes of them.
    pub fn json(&self) -> String {
        let s = self.shared.seen.lock().unwrap();
        let heard = s.header_line.as_ref().is_some_and(|(t, _)| Some(*t) >= s.started_ms);
        let state = match (s.pid, s.on) {
            (Some(_), _) if s.stopping.is_some() => "stopping",
            (Some(_), _) if heard => "running",
            (Some(_), _) => "starting",
            (None, false) => "off",
            (None, true) if s.next_ms.is_some() => "stopped",
            (None, true) => "starting",
        };
        let text = |v: Option<&str>| v.map_or(String::from("null"), |v| format!("\"{}\"", esc(v)));
        let num = |v: Option<u128>| v.map_or(String::from("null"), |v| v.to_string());
        let header = match &s.header_line {
            Some((t, line)) => {
                let fields: Vec<String> = line
                    .split(',')
                    .skip(3)
                    .filter_map(|f| f.split_once('='))
                    .map(|(k, v)| format!("\"{}\":\"{}\"", esc(k), esc(v)))
                    .collect();
                format!("{{\"t\":{},\"fields\":{{{}}}}}", t, fields.join(","))
            }
            None => String::from("null"),
        };
        let said: Vec<String> = s.said.iter().map(|(t, l)| format!("{{\"t\":{},\"text\":\"{}\"}}", t, esc(l))).collect();
        format!(
            concat!(
                "{{\"key\":\"{}\",\"command\":\"{}\",\"sdrLine\":{},\"builtin\":{},\"on\":{},\"gain\":{},\"gainAt\":{},\"state\":\"{}\",",
                "\"startedAt\":{},\"nextAt\":{},\"starts\":{},\"lastExit\":{},\"lastExitAt\":{},\"lastExitByApp\":{},\"fatal\":{},\"lastWords\":{},\"header\":{},\"said\":[{}]}}"
            ),
            esc(&self.key),
            esc(&self.command),
            self.file.is_some(),
            self.builtin.is_some(),
            s.on,
            text(s.gain.as_deref()),
            num(s.gain_at),
            state,
            num(s.started_ms),
            num(s.next_ms),
            s.starts,
            text(s.last_exit.as_ref().map(|(_, why, _)| why.as_str())),
            num(s.last_exit.as_ref().map(|(t, _, _)| *t)),
            s.last_exit.as_ref().is_some_and(|(_, _, app)| *app),
            text(s.fatal.as_deref()),
            text(s.last_words.as_deref()),
            header,
            said.join(",")
        )
    }
}

fn next_wait(ran: Duration, last: Duration) -> Duration {
    if ran >= LONGEST_WAIT {
        FIRST_WAIT
    } else {
        (last * 2).clamp(FIRST_WAIT, LONGEST_WAIT)
    }
}

// The platform's shell, so a conf line can be `ssh station sdr_rx` or carry its own quoting.
// Its own process group, which end_tree signals whole.
#[cfg(unix)]
fn shell(command: &str) -> Command {
    use std::os::unix::process::CommandExt;
    let mut c = Command::new("sh");
    c.arg("-c").arg(command).process_group(0);
    c
}

// cmd.exe does not unquote the way Rust's argument escaping assumes, so the line goes to it as
// it was written. Not built or run on Windows yet.
#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    c.raw_arg("/C").raw_arg(command);
    c
}

// TERM to the whole group, then KILL to whatever is left after 3 s. True once nothing in the
// group is left. kill(1) rather than a libc call, which the app has no other use for.
#[cfg(unix)]
fn end_tree(pid: u32) -> bool {
    let group = format!("-{}", pid);
    let signal = |sig: &str| {
        Command::new("kill")
            .args([sig, "--", group.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let gone = |tries: u32| {
        (0..tries).any(|_| {
            std::thread::sleep(Duration::from_millis(100));
            !signal("-0")
        })
    };
    signal("-TERM");
    gone(30) || {
        signal("-KILL");
        gone(20)
    }
}

// /T ends every process the shell started, by parent; /F because a console program has no
// window to be asked to close. Not built or run on Windows yet.
#[cfg(windows)]
fn end_tree(pid: u32) -> bool {
    Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn say(key: &str, shared: &Shared, text: &str) {
    println!("{}: {}", key, text);
    let mut s = shared.seen.lock().unwrap();
    if s.said.len() == SAID_LINES {
        s.said.pop_front();
    }
    s.said.push_back((now_ms(), text.to_string()));
}

// Every whole line until the program's stdout ends, each into the same channel the USB ports
// feed.
fn pump(mut out: impl Read, key: &str, seen: &Mutex<Seen>, tx: &Sender<Event>) {
    let mut framer = Framer::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let t = now_ms();
        for line in framer.push(&buf[..n]) {
            heard(key, t, line, seen, tx);
        }
    }
}

// A header is kept, and the page told only when it differs from the last one.
fn heard(key: &str, t: u128, line: String, seen: &Mutex<Seen>, tx: &Sender<Event>) {
    let news = match receivers::header(&line) {
        Some(h) => {
            let mut s = seen.lock().unwrap();
            let news = s.header.as_ref() != Some(&h);
            s.header = Some(h);
            s.header_line = Some((t, line.clone()));
            news
        }
        None => {
            if let Some(code) = line.strip_prefix("#DG,FATAL,") {
                seen.lock().unwrap().fatal = Some(code.trim_end().to_string());
            }
            false
        }
    };
    let _ = tx.send(Event::Line { key: key.to_string(), t_ms: t, text: line });
    if news {
        let _ = tx.send(Event::Ports);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const HEADER: &str = "#DG,RX,v1,id=5D4E01,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=7+8+9+10+11+12,fw=0.1.0";

    fn source(key: &str, command: &str, file: Option<PathBuf>) -> (Source, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (Source::new(key, command, file, None, &tx), rx)
    }

    fn drain(rx: &mpsc::Receiver<Event>) -> Vec<String> {
        rx.try_iter()
            .map(|e| match e {
                Event::Line { key, text, .. } => format!("{} {}", key, text),
                Event::Ports => String::from("ports"),
            })
            .collect()
    }

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-command-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Read in pieces that cut lines anywhere, as a pipe hands them over
    struct Dribble(Vec<u8>, usize);

    impl Read for Dribble {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.len().min(self.1).min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            Ok(n)
        }
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "waited 10 s for {}", what);
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn state(s: &Source) -> String {
        let j = s.json();
        let at = j.find("\"state\":\"").unwrap() + 9;
        j[at..at + j[at..].find('"').unwrap()].to_string()
    }

    #[cfg(unix)]
    fn left(marker: &str) -> usize {
        let out = Command::new("pgrep").args(["-f", marker]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().count()
    }

    #[test]
    fn every_whole_line_goes_out_under_the_key_and_a_new_header_tells_the_page() {
        let (s, _) = source("cmd:2", "", None);
        let (tx, rx) = mpsc::channel();
        let text = format!("{}\nPKT,2,00FF,,9.25,-12,sf=9\r\nHB,5000,1,0\n{}\nHB,10000,1,0\n#DG,FATAL,2\nPKT,2,00", HEADER, HEADER);
        pump(Dribble(text.into_bytes(), 7), "cmd:2", &s.shared.seen, &tx);
        assert_eq!(
            drain(&rx),
            [
                format!("cmd:2 {}", HEADER),
                String::from("ports"),
                String::from("cmd:2 PKT,2,00FF,,9.25,-12,sf=9"),
                String::from("cmd:2 HB,5000,1,0"),
                format!("cmd:2 {}", HEADER),
                String::from("cmd:2 HB,10000,1,0"),
                String::from("cmd:2 #DG,FATAL,2"),
            ],
            "the same header again is not news, and the half line at the end is dropped"
        );
        let seen = s.shared.seen.lock().unwrap();
        let h = seen.header.clone().unwrap();
        assert_eq!((h.id.as_str(), h.kind.as_deref(), h.sf), ("5D4E01", Some("sdr"), None));
        assert_eq!(seen.fatal.as_deref(), Some("2"));
        drop(seen);
        let j = s.json();
        assert!(j.contains("\"fields\":{\"id\":\"5D4E01\",\"f\":\"915.0\""), "{}", j);
        assert!(j.contains("\"sfs\":\"7+8+9+10+11+12\",\"fw\":\"0.1.0\"}"), "{}", j);
    }

    #[test]
    fn it_is_listed_by_its_id_and_kind_with_the_name_kept_on_this_computer() {
        let (tx, _rx) = mpsc::channel();
        let (s, _) = source("cmd:1", "ssh station sdr_rx", None);
        s.shared.seen.lock().unwrap().status = String::from("open");
        let names = receivers::Names::empty();
        let before = s.port_info(&names);
        assert_eq!((before.rx_id, before.rx_kind, before.status.as_str()), (None, None, "open"));

        pump(format!("{}\n", HEADER).as_bytes(), "cmd:1", &s.shared.seen, &tx);
        let mut names = receivers::Names::empty();
        names.set("5D4E01", "roof SDR").unwrap();
        let p = s.port_info(&names);
        assert_eq!(p.rx_id.as_deref(), Some("5D4E01"));
        assert_eq!(p.rx_kind.as_deref(), Some("sdr"));
        assert_eq!(p.rx_fw.as_deref(), Some("0.1.0"));
        assert_eq!(p.name.as_deref(), Some("roof SDR"));
        assert_eq!(p.rx_name, None);
        assert_eq!((p.port.as_str(), p.usb.as_str()), ("ssh station sdr_rx", ""));
    }

    #[test]
    fn a_program_that_keeps_stopping_waits_longer_each_time_up_to_a_minute() {
        let quick = Duration::from_millis(300);
        let mut waits = Vec::new();
        let mut w = Duration::ZERO;
        for _ in 0..8 {
            w = next_wait(quick, w);
            waits.push(w.as_secs());
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(next_wait(Duration::from_secs(90), Duration::from_secs(60)), FIRST_WAIT);
    }

    #[test]
    fn a_gain_is_auto_or_plain_db_and_nothing_the_shell_would_read() {
        for yes in ["auto", "0", "0.9", "20.7", "49.6", " 28.0 "] {
            assert!(a_gain(yes).is_ok(), "{}", yes);
        }
        for no in ["", "-1", "20.", ".9", "1e2", "20.7; rm -rf ~", "$(reboot)", "20 7", "AUTO", "1234"] {
            assert!(a_gain(no).is_err(), "{}", no);
        }
    }

    #[test]
    fn the_sdr_settings_survive_being_saved_and_read_back() {
        let dir = scratch("settings");
        let path = dir.join(FILE);
        assert_eq!(load(&path, true), (true, None), "no file is on with no gain");
        save(&path, false, Some("20.7")).unwrap();
        assert_eq!(load(&path, true), (false, Some(String::from("20.7"))));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("on no\ngain 20.7\n"), "{}", text);
        save(&path, true, Some("auto")).unwrap();
        assert_eq!(load(&path, true), (true, Some(String::from("auto"))));
        // a gain somebody typed that would not be safe on the command line is not used
        std::fs::write(&path, "on yes\ngain 20; reboot\n").unwrap();
        assert_eq!(load(&path, true), (true, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_program_is_read_and_started_again_when_it_stops() {
        let (tx, rx) = mpsc::channel();
        let dir = scratch("again");
        let sources = start(&[format!("printf '{}\\nHB,1,0,0\\n'; echo complaint >&2; exit 3", HEADER)], None, &dir, &dir, &tx);
        let mut lines = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while lines.len() < 4 && Instant::now() < deadline {
            if let Ok(Event::Line { key, text, .. }) = rx.recv_timeout(Duration::from_millis(200)) {
                lines.push(format!("{} {}", key, text));
            }
        }
        // twice: the second pair is the restart a second after the first exit
        assert_eq!(lines, [format!("cmd:1 {}", HEADER), "cmd:1 HB,1,0,0".into(), format!("cmd:1 {}", HEADER), "cmd:1 HB,1,0,0".into()]);
        let p = sources[0].port_info(&receivers::Names::empty());
        assert_eq!(p.rx_id.as_deref(), Some("5D4E01"));
        wait_for("its stderr", || sources[0].json().contains("\"text\":\"complaint\""));
        let j = sources[0].json();
        assert!(j.contains("\"lastExit\":\"stopped, exit status: 3\""), "{}", j);
        assert!(j.contains("\"lastExitByApp\":false"), "{}", j);
        assert!(j.contains("\"lastWords\":\"complaint\""), "{}", j);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Off has to take the whole tree with it: the SDR program's rtl_sdr holds the dongle.
    #[cfg(unix)]
    #[test]
    fn off_stops_everything_the_program_started_and_on_starts_it_again() {
        let dir = scratch("off");
        let (tx, _rx) = mpsc::channel();
        let all = start(&[], Some("sleep 3171 & sh -c 'sleep 3172'; echo done"), &dir, &dir, &tx);
        let sdr = &all[0];
        assert_eq!(sdr.key(), SDR_KEY);
        wait_for("both sleeps", || left("sleep 317[12]") >= 2);
        assert_eq!(state(sdr), "starting", "no header yet");

        assert_eq!(sdr.turn(false), Ok(true));
        assert_eq!(left("sleep 317[12]"), 0, "a child or grandchild was left running");
        wait_for("off", || state(sdr) == "off");
        assert!(sdr.json().contains("\"lastExit\":\"Turned off from the page\""), "{}", sdr.json());
        assert_eq!(load(&dir.join(FILE), true), (false, None));
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(left("sleep 317[12]"), 0, "off must not be started again");

        assert_eq!(sdr.turn(true), Ok(true));
        wait_for("the sleeps again", || left("sleep 317[12]") >= 2);
        assert!(sdr.json().contains("\"starts\":2"), "{}", sdr.json());
        sdr.end();
        assert_eq!(left("sleep 317[12]"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn restart_ends_the_run_and_starts_at_once_and_a_gain_goes_on_the_command_line() {
        let dir = scratch("gain");
        let (tx, rx) = mpsc::channel();
        // what is appended lands on the last command, here a function that says its arguments
        let all = start(&[], Some(&format!("f() {{ echo \"args $*\" >&2; exec sleep 3173; }}; echo '{}'; f", HEADER)), &dir, &dir, &tx);
        let sdr = &all[0];
        wait_for("running", || state(sdr) == "running");
        wait_for("one sleep", || left("sleep 3173") == 1);

        assert_eq!(sdr.set_gain("20.7; reboot"), Err(String::from("a gain is auto or a number of dB such as 20.7, and \"20.7; reboot\" is not")));
        let began = Instant::now();
        assert_eq!(sdr.set_gain("20.7"), Ok(true));
        wait_for("the restart with the gain", || sdr.json().contains("\"starts\":2") && sdr.json().contains("\"text\":\"args --gain 20.7\""));
        assert!(began.elapsed() < Duration::from_secs(4), "a restart from the page does not wait out the backoff");
        wait_for("one sleep again", || left("sleep 3173") == 1);
        assert_eq!(load(&dir.join(FILE), true), (true, Some(String::from("20.7"))));
        assert!(sdr.json().contains("\"gain\":\"20.7\""), "{}", sdr.json());
        assert!(sdr.json().contains("\"lastExit\":\"Restarted with gain 20.7 dB\""), "{}", sdr.json());
        assert!(sdr.json().contains("\"lastExitByApp\":true"), "{}", sdr.json());

        assert_eq!(sdr.restart(), Ok(true));
        wait_for("a third start", || sdr.json().contains("\"starts\":3"));
        sdr.end();
        assert_eq!(left("sleep 3173"), 0);
        assert!(drain(&rx).iter().filter(|l| l.starts_with("cmd:sdr #DG,RX")).count() >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_receiver_line_has_no_gain_to_set_and_is_not_restarted_while_off() {
        let dir = scratch("line");
        let (tx, _rx) = mpsc::channel();
        let all = start(&[String::from("exit 1")], None, &dir, &dir, &tx);
        assert!(all[0].set_gain("20.7").unwrap_err().contains("receiver = line"));
        all[0].turn(false).unwrap();
        assert!(all[0].restart().unwrap_err().contains("is off"));
        assert!(!dir.join(FILE).exists(), "a receiver = line's on and off is not kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
