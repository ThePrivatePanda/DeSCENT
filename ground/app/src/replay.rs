// A log opened for replay. Each has its own Model and its own sockets, so nothing done to
// it reaches the live fleet or the live stream. It plays as app.js played one: every
// 100 ms the clock goes on 100 ms times the speed and every line up to it is fed in.
// Opening one reads it to the end, as Open log did, so the page shows the whole log
// until it is asked to seek or play.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::decode::js_num;
use crate::json::esc;
use crate::lines::{log_events, merge_files};
use crate::stream::{Model, ModelSettings, Stream};
use crate::ws::Socket;

pub const TICK_MS: u64 = 100;
// A replay nobody is watching or asking about is let go after this
pub const IDLE: Duration = Duration::from_secs(30 * 60);
// Uploads bigger than this are refused. The largest log in ~/DeSCENT is a few MB.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;

pub struct Line {
    pub t: f64,
    pub rx: String,
    pub text: String,
}

struct Play {
    pos: usize,
    clock: f64,
    playing: bool,
    speed: f64,
    // Copies of the model along the way, so a seek back starts from the nearest one
    // instead of the start of the log (app.js checks)
    checks: Vec<(usize, Model)>,
    touched: Instant,
}

pub struct Replay {
    pub id: String,
    pub names: Vec<String>,
    files: Vec<Vec<u8>>,
    pub lines: Vec<Line>,
    every: usize,
    pub stream: Stream,
    play: Mutex<Play>,
}

// Midnight today, local: what the page dated a file with no times of its own from
pub fn local_midnight_ms() -> f64 {
    use chrono::{Local, TimeZone};
    let today = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    Local.from_local_datetime(&today).earliest().map(|d| d.timestamp_millis() as f64).unwrap_or(0.0)
}

// Every file's lines merged by time, as recorder.js mergeFiles did
pub fn read_lines(files: &[(String, Vec<u8>)], base_ms: f64) -> Vec<Line> {
    let texts: Vec<(String, String)> =
        files.iter().map(|(n, b)| (n.clone(), String::from_utf8_lossy(b).into_owned())).collect();
    let parsed = texts.iter().map(|(n, c)| log_events(c, n, base_ms)).collect();
    merge_files(parsed).into_iter().map(|l| Line { t: l.t, rx: l.rx, text: l.text.to_string() }).collect()
}

impl Replay {
    pub fn new(id: String, files: Vec<(String, Vec<u8>)>, settings: ModelSettings, base_ms: f64) -> Result<Replay, String> {
        let lines = read_lines(&files, base_ms);
        let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
        if lines.is_empty() {
            return Err(format!("no lines found in {}", names.join(", ")));
        }
        let every = 250.max(lines.len().div_ceil(12));
        let r = Replay {
            id,
            names,
            files: files.into_iter().map(|(_, b)| b).collect(),
            every,
            stream: Stream::new(Model::new(settings, f64::INFINITY)),
            play: Mutex::new(Play { pos: 0, clock: 0.0, playing: false, speed: 60.0, checks: Vec::new(), touched: Instant::now() }),
            lines,
        };
        {
            let mut p = r.play.lock().unwrap();
            r.feed_quietly(&mut p, r.lines.len());
        }
        Ok(r)
    }

    fn clock_at(&self, pos: usize) -> f64 {
        if pos > 0 { self.lines[pos - 1].t } else { self.lines[0].t }
    }

    fn feed_quietly(&self, p: &mut Play, to: usize) {
        self.stream.with(|m| {
            while p.pos < to {
                let l = &self.lines[p.pos];
                m.line(&l.rx, l.t, &l.text, false, false);
                p.pos += 1;
                if p.pos % self.every == 0 && !p.checks.iter().any(|c| c.0 == p.pos) {
                    p.checks.push((p.pos, m.clone()));
                }
            }
        });
        p.clock = self.clock_at(p.pos);
    }

    fn seek_locked(&self, p: &mut Play, to: usize) {
        let to = to.min(self.lines.len());
        let from = p.checks.iter().filter(|c| c.0 <= to).max_by_key(|c| c.0).map(|c| (c.0, c.1.clone()));
        // A copy keeps the settings it was taken under; the ones in force now win
        let now = self.stream.with(|m| m.settings.clone());
        let restore = |m: Model| {
            self.stream.with(|cur| {
                *cur = m;
                if cur.settings != now {
                    cur.set_settings(now.clone(), 0.0);
                }
            })
        };
        match from {
            Some((at, m)) if at > p.pos || to < p.pos => {
                restore(m);
                p.pos = at;
            }
            None if to < p.pos => {
                let m = self.stream.with(|cur| cur.fresh());
                restore(m);
                p.pos = 0;
            }
            _ => {}
        }
        self.feed_quietly(p, to);
        self.stream.resnap(p.clock);
    }

    pub fn touch(&self) {
        self.play.lock().unwrap().touched = Instant::now();
    }

    pub fn seek(&self, to: usize) {
        let mut p = self.play.lock().unwrap();
        p.touched = Instant::now();
        p.playing = false;
        self.seek_locked(&mut p, to);
    }

    pub fn set_playing(&self, on: bool) {
        let mut p = self.play.lock().unwrap();
        p.touched = Instant::now();
        if on && p.pos >= self.lines.len() {
            self.seek_locked(&mut p, 0);
        }
        p.playing = on;
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), String> {
        if !(speed.is_finite() && speed > 0.0) {
            return Err("speed has to be a number above 0".into());
        }
        let mut p = self.play.lock().unwrap();
        p.touched = Instant::now();
        p.speed = speed;
        Ok(())
    }

    // One tick of a playing replay: the lines up to the new clock, sent as they go, then
    // whatever went stale or lost by that clock
    pub fn step(&self) {
        let mut p = self.play.lock().unwrap();
        if !p.playing {
            return;
        }
        p.clock += TICK_MS as f64 * p.speed;
        while p.pos < self.lines.len() && self.lines[p.pos].t <= p.clock {
            let l = &self.lines[p.pos];
            self.stream.line(&l.rx, l.t, &l.text, false);
            p.pos += 1;
            if p.pos % self.every == 0 && !p.checks.iter().any(|c| c.0 == p.pos) {
                let (pos, m) = (p.pos, self.stream.with(|m| m.clone()));
                p.checks.push((pos, m));
            }
        }
        // app.js left the clock where the last tick put it, up to 100 ms times the speed
        // past the log, and judged stale and lost there. At the end it stops at the last line.
        if p.pos >= self.lines.len() {
            p.playing = false;
            p.clock = p.clock.min(self.clock_at(p.pos));
        }
        let clock = p.clock;
        self.stream.tick(clock);
    }

    pub fn connect(&self, socket: Socket) {
        let p = self.play.lock().unwrap();
        self.stream.connect(socket, p.clock);
    }

    pub fn state_json(&self) -> String {
        let mut p = self.play.lock().unwrap();
        p.touched = Instant::now();
        let packets = self.stream.with(|m| m.packets());
        let names: Vec<String> = self.names.iter().map(|n| format!("\"{}\"", esc(n))).collect();
        format!(
            "{{\"id\":\"{}\",\"position\":{},\"lines\":{},\"packets\":{},\"from\":{},\"t\":{},\"playing\":{},\"speed\":{},\"names\":[{}]}}",
            esc(&self.id),
            p.pos,
            self.lines.len(),
            packets,
            js_num(self.lines[0].t),
            js_num(p.clock),
            p.playing,
            js_num(p.speed),
            names.join(",")
        )
    }

    pub fn set_settings(&self, s: ModelSettings) {
        let p = self.play.lock().unwrap();
        self.stream.set_settings(s, p.clock);
    }

    pub fn csv(&self) -> String {
        crate::csv::to_csv(self.lines.iter().map(|l| (l.t, l.rx.as_str(), l.text.as_str())))
    }

    // The file as it came for one file; several are merged into one log in the app's own
    // format, which opens again as the same replay
    pub fn log(&self) -> Vec<u8> {
        if self.files.len() == 1 {
            return self.files[0].clone();
        }
        let mut out = String::from("# descent-ground log v1 merged from ");
        out.push_str(&self.names.join(", "));
        out.push('\n');
        for l in &self.lines {
            out.push_str(&format!("{}\t{}\t{}\n", js_num(l.t), l.rx, l.text));
        }
        out.into_bytes()
    }

    fn idle(&self) -> bool {
        let p = self.play.lock().unwrap();
        self.stream.out.count() == 0 && p.touched.elapsed() > IDLE
    }

    fn close(&self) {
        self.stream.out.close_all();
    }
}

pub struct Replays {
    open: Mutex<HashMap<String, Arc<Replay>>>,
    next: AtomicU64,
}

pub type Shared = Arc<Replays>;

impl Replays {
    // With the thread that plays them and lets idle ones go
    pub fn start() -> Shared {
        let r = Arc::new(Replays { open: Mutex::new(HashMap::new()), next: AtomicU64::new(1) });
        let run = r.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(TICK_MS));
            run.tick();
        });
        r
    }

    #[cfg(test)]
    pub fn idle_only() -> Shared {
        Arc::new(Replays { open: Mutex::new(HashMap::new()), next: AtomicU64::new(1) })
    }

    pub fn tick(&self) {
        let all: Vec<Arc<Replay>> = self.open.lock().unwrap().values().cloned().collect();
        for r in all {
            if r.idle() {
                self.close(&r.id);
            } else {
                r.step();
            }
        }
    }

    pub fn open(&self, files: Vec<(String, Vec<u8>)>, settings: ModelSettings, base_ms: f64) -> Result<Arc<Replay>, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed).to_string();
        let r = Arc::new(Replay::new(id.clone(), files, settings, base_ms)?);
        self.open.lock().unwrap().insert(id, r.clone());
        Ok(r)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Replay>> {
        self.open.lock().unwrap().get(id).cloned()
    }

    pub fn close(&self, id: &str) -> bool {
        let r = self.open.lock().unwrap().remove(id);
        if let Some(r) = &r {
            r.close();
        }
        r.is_some()
    }

    pub fn all(&self) -> Vec<Arc<Replay>> {
        self.open.lock().unwrap().values().cloned().collect()
    }
}
