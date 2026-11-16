// Receivers that are programs rather than USB ports: an SDR decoder named on a
// `receiver = <command line>` line in descent-ground.conf, started through the shell and
// read a line at a time off its stdout. The contract the program keeps is
// docs/sdr-receiver.md.
//
// Its key is cmd:<n>, by the line's place in the file, so it moves when the file is edited;
// the id in its header is what names and the page know it by. Nothing is ever written to it.
// Its stdin is shut, and it is not in the hub's open ports, so neither the SF re-ask nor the
// name setting in serial.rs can reach it.
use crate::framing::Framer;
use crate::logfile::now_ms;
use crate::receivers::{self, Header};
use crate::serial::{Event, PortInfo};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// A program that keeps dying is started again after 1 s, then 2, 4 ... up to a minute, so a
// station that is off the network is not hammered with ssh logins. One that ran for a minute
// before it stopped starts again from the short end.
const FIRST_WAIT: Duration = Duration::from_secs(1);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

struct Seen {
    header: Option<Header>,
    status: String,          // open while it runs, stopped while it waits to be started again
    error: Option<String>,   // why it last stopped
}

pub struct Source {
    key: String,
    command: String,
    seen: Arc<Mutex<Seen>>,
}

impl Source {
    // As a port, so it is listed wherever the USB receivers are. port is the command line and
    // usb is empty: there is no adapter.
    pub fn port_info(&self, names: &receivers::Names) -> PortInfo {
        let seen = self.seen.lock().unwrap();
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
}

pub fn start(commands: &[String], tx: &Sender<Event>) -> Vec<Source> {
    commands
        .iter()
        .enumerate()
        .map(|(i, command)| {
            let key = format!("cmd:{}", i + 1);
            let seen = Arc::new(Mutex::new(Seen { header: None, status: String::from("stopped"), error: None }));
            std::thread::spawn({
                let (key, command, seen, tx) = (key.clone(), command.clone(), seen.clone(), tx.clone());
                move || keep_running(&key, &command, &seen, &tx)
            });
            Source { key, command: command.clone(), seen }
        })
        .collect()
}

fn keep_running(key: &str, command: &str, seen: &Mutex<Seen>, tx: &Sender<Event>) {
    let mut wait = Duration::ZERO;
    loop {
        let began = Instant::now();
        let spawned = shell(command).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let why = match spawned {
            Ok(mut child) => {
                println!("{}: started {}", key, command);
                set_status(seen, tx, "open", None);
                // Its complaints go to this app's own output, where an operator reads ours
                let err = child.stderr.take().unwrap();
                let err_key = key.to_string();
                std::thread::spawn(move || {
                    for line in BufReader::new(err).split(b'\n').map_while(Result::ok) {
                        println!("{}: {}", err_key, String::from_utf8_lossy(&line).trim_end());
                    }
                });
                pump(child.stdout.take().unwrap(), key, seen, tx);
                match child.wait() {
                    Ok(status) => format!("stopped, {}", status),
                    Err(e) => format!("stopped, and could not be asked why: {}", e),
                }
            }
            Err(e) => format!("could not be started: {}", e),
        };
        wait = next_wait(began.elapsed(), wait);
        println!("{}: {}; starting it again in {} s", key, why, wait.as_secs());
        set_status(seen, tx, "stopped", Some(why));
        std::thread::sleep(wait);
    }
}

fn next_wait(ran: Duration, last: Duration) -> Duration {
    if ran >= LONGEST_WAIT {
        FIRST_WAIT
    } else {
        (last * 2).clamp(FIRST_WAIT, LONGEST_WAIT)
    }
}

// The platform's shell, so a conf line can be `ssh station sdr_rx` or carry its own quoting
#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
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

fn set_status(seen: &Mutex<Seen>, tx: &Sender<Event>, status: &str, error: Option<String>) {
    {
        let mut s = seen.lock().unwrap();
        s.status = status.to_string();
        s.error = error;
    }
    let _ = tx.send(Event::Ports);
}

// Every whole line until the program's stdout ends, each into the same channel the USB ports
// feed. A header is kept, and the page told only when it differs from the last one.
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
            let news = receivers::header(&line).is_some_and(|h| {
                let mut s = seen.lock().unwrap();
                let news = s.header.as_ref() != Some(&h);
                s.header = Some(h);
                news
            });
            let _ = tx.send(Event::Line { key: key.to_string(), t_ms: t, text: line });
            if news {
                let _ = tx.send(Event::Ports);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const HEADER: &str = "#DG,RX,v1,id=5D4E01,f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,sfs=7+8+9+10+11+12,fw=0.1.0";

    fn seen() -> Mutex<Seen> {
        Mutex::new(Seen { header: None, status: String::from("open"), error: None })
    }

    fn drain(rx: &mpsc::Receiver<Event>) -> Vec<String> {
        rx.try_iter()
            .map(|e| match e {
                Event::Line { key, text, .. } => format!("{} {}", key, text),
                Event::Ports => String::from("ports"),
            })
            .collect()
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

    #[test]
    fn every_whole_line_goes_out_under_the_key_and_a_new_header_tells_the_page() {
        let (tx, rx) = mpsc::channel();
        let s = seen();
        let text = format!("{}\nPKT,2,00FF,,9.25,-12,sf=9\r\nHB,5000,1,0\n{}\nHB,10000,1,0\nPKT,2,00", HEADER, HEADER);
        pump(Dribble(text.into_bytes(), 7), "cmd:2", &s, &tx);
        assert_eq!(
            drain(&rx),
            [
                format!("cmd:2 {}", HEADER),
                String::from("ports"),
                String::from("cmd:2 PKT,2,00FF,,9.25,-12,sf=9"),
                String::from("cmd:2 HB,5000,1,0"),
                format!("cmd:2 {}", HEADER),
                String::from("cmd:2 HB,10000,1,0"),
            ],
            "the same header again is not news, and the half line at the end is dropped"
        );
        let h = s.lock().unwrap().header.clone().unwrap();
        assert_eq!((h.id.as_str(), h.kind.as_deref(), h.sf), ("5D4E01", Some("sdr"), None));
    }

    #[test]
    fn it_is_listed_by_its_id_and_kind_with_the_name_kept_on_this_computer() {
        let (tx, _rx) = mpsc::channel();
        let s = Source { key: String::from("cmd:1"), command: String::from("ssh station sdr_rx"), seen: Arc::new(seen()) };
        let names = receivers::Names::empty();
        let before = s.port_info(&names);
        assert_eq!((before.rx_id, before.rx_kind, before.status.as_str()), (None, None, "open"));

        pump(format!("{}\n", HEADER).as_bytes(), "cmd:1", &s.seen, &tx);
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

    #[cfg(unix)]
    #[test]
    fn a_real_program_is_read_and_started_again_when_it_stops() {
        let (tx, rx) = mpsc::channel();
        let sources = start(&[format!("printf '{}\\nHB,1,0,0\\n'; echo complaint >&2; exit 3", HEADER)], &tx);
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
    }
}
