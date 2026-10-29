// Pulling a flight log off a ChipSat, entirely inside this program.
//
// Nothing outside the binary is needed: the debug probe is driven directly, the dump
// sketch is carried inside, and the log the board prints is parsed here. No toolchain,
// no scripts, no sibling checkout.
//
// The order matters and is not obvious. The serial port is opened and read BEFORE the
// board is touched, because the board talks continuously and a port nobody is reading
// overflows; and the whole printing is collected before anything is decided, because a
// port opened mid-printing catches the tail of an older dump. Which printing was the real
// one is settled afterwards, from the text.
//
// Each pull gets a folder of its own: dump.txt is what the board sent, byte for byte and
// whatever became of the pull, ground.log is that turned into a log the page opens,
// report.json sets what was read against what the board counted, and the flight software
// that was on the board is saved beside them.
use crate::json::esc;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

const BAUD: u32 = 115200;
const QUIET_MS: u128 = 8_000;      // silence after the last line before giving up on more
// From the reset to the end of the printing, and nothing else: saving and writing firmware
// are not on this clock. A full chip is 32768 slots at about 147 characters a line, 4.8 MB,
// and 115200 baud carries 11.5 kB a second, so it prints for about 7 minutes. Worked out,
// not timed on a full chip.
const READ_LIMIT_MS: u128 = 20 * 60 * 1000;

#[derive(Clone)]
pub struct Pull {
    pub running: bool,
    pub wipe: bool,       // clearing the chip rather than reading it
    pub keep_fsw: bool,
    pub board: String,
    pub started_ms: u128,
    pub finished_ms: u128,
    pub steps: Vec<String>,
    pub note: String,
    pub pct: u32,
    pub saved_fsw: Option<String>,
    pub log: Option<String>,      // as /api/log takes it: <folder>/ground.log
    pub dir: Option<String>,      // the pull's folder, in full
    pub raw: Option<String>,      // <folder>/dump.txt
    pub report: Option<String>,   // dump::Report as json
    pub error: Option<String>,
    pub restored: bool,
    // Stop pressed. Ends like a failed pull: what arrived is kept, the flight software goes back
    pub stop: bool,
}

impl Pull {
    fn idle() -> Self {
        Pull {
            running: false,
            wipe: false,
            keep_fsw: true,
            board: String::new(),
            started_ms: 0,
            finished_ms: 0,
            steps: Vec::new(),
            note: String::new(),
            pct: 0,
            saved_fsw: None,
            log: None,
            dir: None,
            raw: None,
            report: None,
            error: None,
            restored: false,
            stop: false,
        }
    }
}

pub type Shared = Arc<Mutex<Pull>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(Pull::idle()))
}

fn quoted(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("\"{}\"", esc(s)),
        None => String::from("null"),
    }
}

pub fn state_json(p: &Pull, now_ms: u128) -> String {
    let secs = if p.started_ms == 0 {
        0
    } else {
        let end = if p.running { now_ms } else { p.finished_ms };
        end.saturating_sub(p.started_ms) / 1000
    };
    let steps: Vec<String> = p.steps.iter().map(|l| format!("\"{}\"", esc(l))).collect();
    format!(
        "{{\"ok\":true,\"running\":{},\"wipe\":{},\"keepFsw\":{},\"board\":\"{}\",\"seconds\":{},\"pct\":{},\"note\":\"{}\",\"steps\":[{}],\"savedFsw\":{},\"log\":{},\"dir\":{},\"raw\":{},\"report\":{},\"restored\":{},\"stopping\":{},\"error\":{}}}",
        p.running,
        p.wipe,
        p.keep_fsw,
        esc(&p.board),
        secs,
        p.pct,
        esc(&p.note),
        steps.join(","),
        quoted(&p.saved_fsw),
        quoted(&p.log),
        quoted(&p.dir),
        quoted(&p.raw),
        // Already json, so it goes in as it is.
        p.report.as_deref().unwrap_or("null"),
        p.restored,
        p.running && p.stop,
        quoted(&p.error),
    )
}

fn say(shared: &Shared, broadcast: &crate::ws::Broadcast, pct: u32, note: &str) {
    {
        let mut p = shared.lock().unwrap();
        p.pct = pct;
        p.note = note.to_string();
        p.steps.push(note.to_string());
    }
    broadcast.send(&format!(
        "{{\"type\":\"pull\",\"pct\":{},\"note\":\"{}\"}}",
        pct,
        esc(note)
    ));
}

// Reads the console into one buffer while something else flashes the board. Bytes, not
// text: line noise is not valid UTF-8, and dump.txt has to be what arrived. The caller
// decides when to stop and, from what was heard, whether a whole dump arrived.
struct Listener {
    heard: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    last_ms: Arc<Mutex<u128>>,
    gap_ms: Arc<AtomicU64>,
    // Why the port stopped being read, if it did. Not a failure by itself: the pull still
    // waits out its limits, but a failure says this as well.
    lost: Arc<Mutex<Option<String>>>,
    // The same port for writing, when there is one: a clear talks to the dump firmware.
    writer: Mutex<Option<Box<dyn Write + Send>>>,
}

impl Listener {
    fn start(port_name: &str) -> Result<Listener, String> {
        let port = serialport::new(port_name, BAUD)
            .timeout(std::time::Duration::from_millis(200))
            .open()
            .map_err(|e| format!("{}: {}", port_name, e))?;
        let writer = port.try_clone().map_err(|e| format!("{}: cannot open it for writing: {}", port_name, e))?;
        let l = Listener::on(port);
        *l.writer.lock().unwrap() = Some(writer);
        Ok(l)
    }

    fn on<R: Read + Send + 'static>(mut port: R) -> Listener {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let last_ms = Arc::new(Mutex::new(crate::logfile::now_ms()));
        let gap_ms = Arc::new(AtomicU64::new(0));
        let lost = Arc::new(Mutex::new(None));
        let (h2, s2, l2, g2, lost2) = (heard.clone(), stop.clone(), last_ms.clone(), gap_ms.clone(), lost.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut back: Option<Instant> = None;
            while !s2.load(Ordering::Relaxed) {
                // From one read() coming back to the next one going in, this thread is not
                // listening, and the longest such stretch is kept. It is what a slot lost
                // on the way would be blamed on first: 115200 baud fills a small driver
                // buffer in tens of ms. A quiet line is time inside read() and not this.
                if let Some(back) = back {
                    g2.fetch_max(back.elapsed().as_millis() as u64, Ordering::Relaxed);
                }
                let got = port.read(&mut buf);
                back = Some(Instant::now());
                match got {
                    Ok(0) => {}
                    Ok(n) => {
                        h2.lock().unwrap().extend_from_slice(&buf[..n]);
                        *l2.lock().unwrap() = crate::logfile::now_ms();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    // unplugged, or the probe reset it out from under us
                    Err(e) => {
                        *lost2.lock().unwrap() = Some(e.to_string());
                        break;
                    }
                }
            }
        });
        Listener { heard, stop, last_ms, gap_ms, lost, writer: Mutex::new(None) }
    }

    // To the board. A port with no writer (a test's) takes the bytes and drops them.
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            w.write_all(bytes).and_then(|_| w.flush()).map_err(|e| format!("cannot write to the console: {}", e))?;
        }
        Ok(())
    }

    // Where the first line at or after `from` that begins with `text` ends. The flash driver
    // logs "I FLASH 416 c0: erase what=chip" while a chip is erasing, which contains "erase "
    // but is not the firmware's verdict; that is the line that begins with it, and comes later
    fn line_starting_after(&self, text: &str, from: usize) -> Option<usize> {
        let heard = self.heard.lock().unwrap();
        let mut at = from.min(heard.len());
        while at < heard.len() {
            let end = match heard[at..].iter().position(|b| *b == b'\n') {
                Some(n) => at + n + 1,
                None => return None,
            };
            if String::from_utf8_lossy(&heard[at..end]).trim_start().starts_with(text) {
                return Some(end);
            }
            at = end;
        }
        None
    }

    // Where a whole line holding `text` ends, looking at or after `from`. Whole, as for
    // the found line: what matters on it may be the part still arriving.
    fn line_with_after(&self, text: &str, from: usize) -> Option<usize> {
        let heard = self.heard.lock().unwrap();
        let mut at = from.min(heard.len());
        while at < heard.len() {
            let end = match heard[at..].iter().position(|b| *b == b'\n') {
                Some(n) => at + n + 1,
                None => return None,
            };
            if String::from_utf8_lossy(&heard[at..end]).contains(text) {
                return Some(end);
            }
            at = end;
        }
        None
    }

    // "I BOOT 0 c0: start ..." now, "I Boot 3 c0: start" on older firmware. Only the first few
    // lines, since a board boot-looping on this port prints one sooner or later too
    fn boot_line_after(&self, from: usize) -> Option<String> {
        let heard = self.heard.lock().unwrap();
        let text = String::from_utf8_lossy(&heard[from.min(heard.len())..]).to_string();
        let whole = match text.rfind('\n') {
            Some(end) => &text[..end],
            None => return None,
        };
        whole
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(BOOT_WITHIN_LINES)
            .find(|l| {
                let l = l.to_ascii_lowercase();
                l.contains(" boot ") && l.contains(": start")
            })
            .map(|l| l.trim().to_string())
    }

    fn last_ms(&self) -> u128 {
        *self.last_ms.lock().unwrap()
    }

    fn bytes_from(&self, from: usize) -> Vec<u8> {
        let heard = self.heard.lock().unwrap();
        heard[from.min(heard.len())..].to_vec()
    }

    fn len(&self) -> usize {
        self.heard.lock().unwrap().len()
    }

    fn found_line_after(&self, from: usize) -> (bool, usize) {
        whole_found_line(&self.heard.lock().unwrap(), from)
    }

    fn longest_gap_ms(&self) -> u64 {
        self.gap_ms.load(Ordering::Relaxed)
    }

    fn take(&self) -> Vec<u8> {
        self.heard.lock().unwrap().clone()
    }

    fn lost(&self) -> Option<String> {
        self.lost.lock().unwrap().clone()
    }

    fn done(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// Has to be the whole line, half of it reads as a smaller count. Returns where to look from
// next time, so only new bytes are scanned (the reader waits on this lock)
fn whole_found_line(bytes: &[u8], from: usize) -> (bool, usize) {
    let mut at = from.min(bytes.len());
    while let Some(len) = bytes[at..].iter().position(|b| *b == b'\n') {
        if bytes[at..at + len].trim_ascii_start().starts_with(b"found ") {
            return (true, at + len + 1);
        }
        at += len + 1;
    }
    (false, at)
}

// 1432-board9, or 1432-ttyACM1 with no name. A second one in the same minute is 1432-2-board9
fn folder_in(dir: &Path, hint: &str, at: SystemTime) -> Result<PathBuf, String> {
    crate::layout::unused(dir, &crate::layout::hhmm(at), &crate::layout::board_word(hint), "")
}

// What is on the board when a pull ends.
#[derive(Clone, Copy, PartialEq)]
enum Board {
    Untouched,   // it failed before the dump firmware was written
    Back,
    NotBack,     // an image was saved and could not be written back
    OnDump,      // nobody asked for the flight software to be kept
}

// Short of everything is still worth saying exactly, so the page can show how short.
fn tally(r: &crate::dump::Report) -> String {
    if r.complete {
        return String::from("done");
    }
    match r.board_good.or(r.found) {
        Some(n) => format!(
            "read {} of the {} records the board counted ({} never arrived, {} failed their check)",
            r.read, n, r.missing, r.failed_check
        ),
        None => format!("read {} records, and the board never got to its count", r.read),
    }
}

// Which board the records say they came from, for the last note of a pull. A pull is
// started against whatever is on the clip, and this is the only thing in it that says
// which board that was.
fn board_ids(ids: &[u8]) -> String {
    let said: Vec<String> = ids.iter().map(|n| n.to_string()).collect();
    match said.split_last() {
        None => String::new(),
        Some((only, [])) => format!(", board id {}", only),
        Some((last, rest)) => format!(", board ids {} and {}", rest.join(", "), last),
    }
}

fn closing_note(tally: Option<&str>, failed: bool, board: Board, folder: Option<&Path>) -> String {
    let mut note = match (failed, tally) {
        (false, Some(t)) => t.to_string(),
        _ => String::from("failed"),
    };
    note.push_str(match (board, failed) {
        (Board::Untouched, _) => "",
        (Board::Back, _) => ", flight software back on the board",
        (Board::NotBack, _) => ", and the flight software was NOT put back",
        (Board::OnDump, false) => ", the board is still running the dump firmware",
        // The write of the dump firmware may be what failed, so nothing is said about
        // what it is running.
        (Board::OnDump, true) => ", and the board does not have its flight software",
    });
    if let Some(f) = folder {
        note.push_str(if failed { "; what arrived is in " } else { "; saved in " });
        note.push_str(&f.display().to_string());
    }
    note
}

// prepends "firmware":{"dump":..,"onBoard":..} to report.json, null when unknown
fn with_firmware(report: &str, dump: &str, on_board: Option<&str>) -> String {
    let dump = if dump.is_empty() { "null" } else { dump };
    match report.strip_prefix('{') {
        Some(rest) => format!(
            "{{\"firmware\":{{\"dump\":{},\"onBoard\":{}}}{}{}",
            dump,
            on_board.unwrap_or("null"),
            if rest.trim_start().starts_with('}') { "" } else { "," },
            rest
        ),
        None => report.to_string(),
    }
}

// The board's last count, when it gave one. A wiped chip prints its count and no records.
fn board_count(text: &str) -> Option<u64> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("found "))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
}

pub struct Job {
    pub shared: Shared,
    pub broadcast: crate::ws::Broadcast,
    pub dir: PathBuf,
    pub port_name: String,
    pub probe: Option<String>,
    pub wipe: bool,   // erase the chip instead of reading it
    pub keep_fsw: bool,
    pub dump_image: Vec<u8>,
    pub dump_from: String,   // json for report.json, "" if unknown
    // runs first in the job thread, the dump may need a ~13 s build
    pub prepare: Option<Prepare>,
    // to name the saved flight image in the report
    pub known: Option<crate::chipflash::Known>,
    // without either, things are named after the port
    pub uid: Option<String>,
    pub name: Option<String>,
    pub check_console: bool,   // reset once and wait for the boot line before writing anything. Off for Pull anyway
}

// -> (dump bytes, dump_from, optional line for the steps)
pub type Prepare = Box<dyn FnOnce(&dyn Fn(&str)) -> Result<(Vec<u8>, String, Option<String>), String> + Send>;

// The caller holds the probe's gate, and hands it over so it is let go when the pull ends.
pub fn start(job: Job, held: crate::stm32::Held) -> Result<(), String> {
    {
        let p = job.shared.lock().map_err(|_| "pull state is wedged".to_string())?;
        if p.running {
            return Err(format!("already pulling from {}", p.board));
        }
    }
    {
        let mut p = job.shared.lock().unwrap();
        *p = Pull::idle();
        p.running = true;
        p.wipe = job.wipe;
        p.keep_fsw = job.keep_fsw;
        p.board = job.port_name.clone();
        p.started_ms = crate::logfile::now_ms();
    }
    std::thread::spawn(move || {
        run(job);
        drop(held);
    });
    Ok(())
}

const STOPPED: &str = "stopped by the operator";

// Only asks. The job stops within a second while waiting on the board, or after the current
// probe step
pub fn stop(shared: &Shared) -> Result<(), String> {
    let mut p = shared.lock().map_err(|_| "pull state is wedged".to_string())?;
    if !p.running {
        return Err(String::from("nothing is running to stop"));
    }
    if !p.stop {
        p.stop = true;
        p.steps.push(String::from("stop asked for, finishing the step in hand"));
    }
    Ok(())
}

fn stop_asked(shared: &Shared) -> bool {
    shared.lock().map(|p| p.stop).unwrap_or(false)
}

fn also(error: &mut Option<String>, more: String) {
    *error = Some(match error.take() {
        Some(first) => format!("{}. Also: {}", first, more),
        None => more,
    });
}

// The things done to the board. Behind a trait so the order they happen in, and what
// happens when one of them fails, can be run with no board on the desk.
trait Bench {
    fn uid(&self) -> Result<String, String>;
    fn save(&self, dir: &Path, file: &str) -> Result<PathBuf, String>;
    fn write(&self, image: &[u8]) -> Result<(), String>;
    fn reset(&self) -> Result<(), String>;
}

struct StLink<'a> {
    probe: Option<&'a str>,
    broadcast: &'a crate::ws::Broadcast,
}

impl Bench for StLink<'_> {
    fn uid(&self) -> Result<String, String> {
        crate::stm32::uid(self.probe)
    }
    fn save(&self, dir: &Path, file: &str) -> Result<PathBuf, String> {
        crate::stm32::save_firmware(self.probe, dir, file, &crate::stm32::to_page(self.broadcast))
    }
    fn write(&self, image: &[u8]) -> Result<(), String> {
        crate::stm32::write_firmware(self.probe, image, &crate::stm32::to_page(self.broadcast))
    }
    fn reset(&self) -> Result<(), String> {
        crate::stm32::reset_and_run(self.probe)
    }
}

struct Limits {
    quiet_ms: u128,
    read_ms: u128,
    // the dump firmware gives 5 s typical, 25 s max for the erase itself
    ask_ms: u128,
    erase_ms: u128,
    boot_ms: u128,   // reset to boot line
}

const BOOT_LIMIT_MS: u128 = 5_000;
const BOOT_WITHIN_LINES: usize = 4;
const ASK_LIMIT_MS: u128 = 30_000;
const ERASE_LIMIT_MS: u128 = 45_000;

fn put_back(bench: &dyn Bench, image: &Path) -> Result<(), String> {
    let bytes = std::fs::read(image).map_err(|e| format!("cannot read back {}: {}", image.display(), e))?;
    bench.write(&bytes)?;
    bench.reset()
}

fn run(mut job: Job) {
    if let Some(prepare) = job.prepare.take() {
        let (shared, broadcast) = (job.shared.clone(), job.broadcast.clone());
        match prepare(&|said: &str| say(&shared, &broadcast, 1, said)) {
            Ok((bytes, from, note)) => {
                job.dump_image = bytes;
                job.dump_from = from;
                if let Some(note) = note {
                    say(&job.shared, &job.broadcast, 1, &note);
                }
            }
            Err(e) => return finish(&job.shared, &job.broadcast, None, Some(e), Board::Untouched, None),
        }
    }
    // Listening first. The board is talking already, and a port nobody reads fills up.
    say(&job.shared, &job.broadcast, 2, "listening to the board");
    match Listener::start(&job.port_name) {
        Ok(listener) => {
            let bench = StLink { probe: job.probe.as_deref(), broadcast: &job.broadcast };
            pull(&job, &bench, listener, &Limits { quiet_ms: QUIET_MS, read_ms: READ_LIMIT_MS, ask_ms: ASK_LIMIT_MS, erase_ms: ERASE_LIMIT_MS, boot_ms: BOOT_LIMIT_MS });
        }
        Err(e) => finish(&job.shared, &job.broadcast, None, Some(e), Board::Untouched, None),
    }
}

fn pull(job: &Job, bench: &dyn Bench, listener: Listener, limits: &Limits) {
    let Job { shared, broadcast, dir, port_name, keep_fsw, dump_image, uid, name: board_name, wipe, check_console, dump_from, known, .. } = job;
    // a clear keeps the flight software only when asked, same as a pull
    let wipe = *wipe;
    let keep_fsw = *keep_fsw;

    let now = SystemTime::now();
    // A clear keeps nothing, the log was already pulled (decided 2026-10-21). Its flight
    // software copy waits in WORKING and is only kept if it can't be put back
    let (folder, name) = if wipe {
        for said in recover_working(dir) {
            say(shared, broadcast, 1, &said);
        }
        (dir.join(WORKING), String::new())
    } else {
        let place = match crate::layout::folder(dir, crate::layout::PULLS, now) {
            Ok(place) => place,
            Err(e) => {
                listener.done();
                return finish(shared, broadcast, None, Some(e), Board::Untouched, None);
            }
        };
        let folder = match folder_in(&place.dir, board_name.as_deref().unwrap_or(port_name), now) {
            Ok(folder) => folder,
            Err(e) => {
                listener.done();
                return finish(shared, broadcast, None, Some(e), Board::Untouched, None);
            }
        };
        // <day>/pulls/<folder>, as /api/log takes it
        let name = place.rel_of(&folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        (folder, name)
    };
    // In full, because this is what the operator is shown to go and find it by.
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    if let Err(e) = std::fs::create_dir_all(&folder) {
        listener.done();
        let e = format!("could not make {}: {}", folder.display(), e);
        return finish(shared, broadcast, None, Some(e), Board::Untouched, None);
    }
    if !wipe {
        shared.lock().unwrap().dir = Some(folder.display().to_string());
        // for the list, once the image beside it is gone
        if let Some(uid) = uid {
            let _ = std::fs::write(folder.join(crate::layout::BOARD_FILE), format!("{}\n", uid));
        }
    }

    let mut saved: Option<PathBuf> = None;
    let mut wrote_dump = false;
    let mut reset_at = 0usize;   // how much the port had carried when the board was reset
    let mut stopped = false;     // stopped by the operator once the board was printing
    let read = (|| -> Result<(), String> {
        // The page read a board a while ago and the clip has been in somebody's hand
        // since, so the board is asked again before it is saved or written.
        if let Some(meant) = uid {
            say(shared, broadcast, 3, "checking which board is on the probe");
            let on = bench.uid()?;
            if &on != meant {
                return Err(format!(
                    "the board on the probe is {} and this pull was meant for {}: the clip may have moved. Nothing was written.",
                    on, meant
                ));
            }
        }
        // Make sure the port is this board's console. Pulls have read the wrong port before
        // (2026-10-16, 2026-10-17) and it only showed after flashing the board twice
        let mut on_dump = false;
        if *check_console {
            say(shared, broadcast, 4, &format!("checking that {} is this board's console", port_name));
            let from = listener.len();
            bench.reset()?;
            let reset_ms = crate::logfile::now_ms();
            loop {
                if let Some(line) = listener.boot_line_after(from) {
                    on_dump = line.contains("test=flashdump");
                    break;
                }
                if stop_asked(shared) {
                    return Err(String::from(STOPPED));
                }
                if crate::logfile::now_ms().saturating_sub(reset_ms) > limits.boot_ms {
                    return Err(format!(
                        "{} is not this board's console: nothing on it said it had started when the board was reset. Nothing was written. \
Pick the board's console port, or Pull anyway if the software on the board prints nothing",
                        port_name
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        // otherwise the dump firmware would be saved and reported as the flight software
        if on_dump {
            say(shared, broadcast, 5, "the board is already running the dump firmware, so there is no flight software on it to save");
        }
        if keep_fsw && !on_dump {
            say(shared, broadcast, 5, "saving the flight software that is on the board");
            let file = match uid {
                Some(uid) => crate::stm32::image_name(uid, board_name.as_deref().unwrap_or(""), SystemTime::now()),
                None => crate::stm32::backup_name(port_name, SystemTime::now()),
            };
            let path = bench.save(&folder, &file)?;
            shared.lock().unwrap().saved_fsw =
                path.file_name().map(|n| n.to_string_lossy().to_string());
            saved = Some(path);
        }
        // last point where the board still has its own software
        if stop_asked(shared) {
            return Err(String::from(STOPPED));
        }

        if dump_image.is_empty() {
            return Err(String::from("there was no dump firmware to write"));
        }
        say(shared, broadcast, 30, "writing the dump firmware");
        wrote_dump = true;
        bench.write(dump_image)?;
        // Whatever the port carried before the reset is not this printing, even if it
        // holds a found line: a board already on the dump firmware has printed one.
        let mut at = listener.len();
        let before = at;
        reset_at = before;
        bench.reset()?;

        if wipe {
            // E until the firmware asks whether to go ahead: it reads the console every 32
            // records while printing, and a byte sent while it was still booting is lost.
            // Then Y, once, and its own word on how the erase went.
            say(shared, broadcast, 45, "asking the dump firmware to erase the chip");
            let reset_ms = crate::logfile::now_ms();
            let asked = loop {
                if stop_asked(shared) {
                    return Err(String::from(STOPPED));
                }
                listener.send(b"E")?;
                std::thread::sleep(std::time::Duration::from_millis(400));
                if let Some(end) = listener.line_with_after("send Y to go ahead", at) {
                    break end;
                }
                if crate::logfile::now_ms().saturating_sub(reset_ms) > limits.ask_ms {
                    return Err(String::from("the dump firmware never asked whether to erase: is the console port the board's?"));
                }
            };
            // no stopping after the Y, resetting mid erase leaves the chip in an unknown state
            say(shared, broadcast, 60, "erasing the chip, about 5 s");
            listener.send(b"Y")?;
            let sent_ms = crate::logfile::now_ms();
            let said = loop {
                if let Some(end) = listener.line_starting_after("erase ", asked) {
                    let heard = listener.heard.lock().unwrap();
                    let line = String::from_utf8_lossy(&heard[asked..end]);
                    break line.lines().find(|l| l.trim_start().starts_with("erase ")).unwrap_or("").trim().to_string();
                }
                if crate::logfile::now_ms().saturating_sub(sent_ms) > limits.erase_ms {
                    return Err(String::from("the dump firmware did not say whether the erase finished"));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            };
            if !said.starts_with("erase ok") {
                return Err(format!("the chip was not erased: the board said {:?}", said.trim()));
            }
            shared.lock().unwrap().note = format!(
                "{}, {}",
                said.trim(),
                if saved.is_some() { "flight software going back on" } else { "the board stays on the dump firmware" }
            );
            return Ok(());
        }

        say(shared, broadcast, 45, "reading the log off the chip");
        let reset_ms = crate::logfile::now_ms();
        // written as it arrives, so a pull cut short still leaves what it got
        let raw = folder.join("dump.txt");
        let mut on_disk = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&raw).ok();
        if on_disk.is_some() {
            shared.lock().unwrap().raw = Some(format!("{}/dump.txt", name));
        }
        let mut written = 0usize;
        loop {
            if let Some(f) = on_disk.as_mut() {
                let fresh = listener.bytes_from(written);
                if !fresh.is_empty() {
                    use std::io::Write;
                    if f.write_all(&fresh).and_then(|_| f.flush()).is_ok() {
                        written += fresh.len();
                    } else {
                        on_disk = None;   // the final write below says so if it fails too
                    }
                }
            }
            let (whole, next) = listener.found_line_after(at);
            at = next;
            if whole {
                break;
            }
            if stop_asked(shared) {
                stopped = true;
                break;
            }
            let now = crate::logfile::now_ms();
            // The board was halted while it was written, so the line has been quiet for
            // longer than this already. Quiet only counts from the reset.
            if now.saturating_sub(listener.last_ms().max(reset_ms)) > limits.quiet_ms {
                break;
            }
            if now.saturating_sub(reset_ms) > limits.read_ms {
                break;
            }
            // A full chip prints for about seven minutes. The board only says how much it
            // holds at the end, so there is no fraction to show, only that it is arriving.
            shared.lock().unwrap().note =
                format!("reading the log off the chip, {} kB so far", at.saturating_sub(before) / 1024);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        Ok(())
    })();
    listener.done();
    let heard = listener.take();
    let mut error = read.err();

    // Before anything is made of it. Whatever happens next, this is the evidence, and
    // getting it again means doing the whole pull again. A clear read nothing worth it.
    let raw = folder.join("dump.txt");
    if !wipe {
        match std::fs::write(&raw, &heard) {
            Ok(()) => shared.lock().unwrap().raw = Some(format!("{}/dump.txt", name)),
            Err(e) => also(&mut error, format!("could not write {}: {}", raw.display(), e)),
        }
    }

    let mut read_as: Option<String> = None;
    if wipe && error.is_none() {
        read_as = Some(String::from("flash cleared"));
    } else if error.is_none() {
        // Only what arrived after the reset is read. What the port carried before it is
        // by definition not this printing: a board left on an older dump firmware is
        // still printing its log when the pull starts, and its records pair up and read
        // as well as the real ones. They stay in dump.txt, which is the whole capture.
        let text = String::from_utf8_lossy(&heard[reset_at.min(heard.len())..]);
        let parsed = crate::dump::parse_what_arrived(&text);
        if parsed.records.is_empty() && board_count(&text) == Some(0) {
            error = Some(format!(
                "the log on the chip is empty: the board counted 0 records. What it printed is in {}",
                raw.display()
            ));
        } else if parsed.records.is_empty() {
            // The board's own word on its chip, when it gave one: "init ok=0 id=0xFFFFFF
            // reason=noack" is a chip that did not answer, and that is the whole story
            let said = text
                .lines()
                .find(|l| l.contains("FLASH") && l.contains("init"))
                .map(|l| format!(" The board said: {}.", l.trim()))
                .unwrap_or_default();
            error = Some(format!(
                "nothing that reads as a log came off the board; the {} bytes it did send are in {}.{}",
                heard.len(),
                raw.display(),
                said
            ));
        } else {
            say(shared, broadcast, 80, &format!("read {} records from {} boots", parsed.records.len(), parsed.boots().len()));
            let report = crate::dump::report(&parsed, heard.len(), listener.longest_gap_ms());
            // so a log can be matched to its build after the tree moves on
            let on_board = match (saved.as_deref(), known) {
                (Some(path), Some(known)) => std::fs::read(path).ok().map(|b| crate::chipflash::recognise(&b, known).json()),
                _ => None,
            };
            let json = with_firmware(&report.json(), dump_from, on_board.as_deref());
            let log = folder.join("ground.log");
            match std::fs::write(&log, crate::dump::to_log(&parsed, port_name)) {
                Ok(()) => shared.lock().unwrap().log = Some(format!("{}/ground.log", name)),
                Err(e) => also(&mut error, format!("could not write {}: {}", log.display(), e)),
            }
            if let Err(e) = std::fs::write(folder.join("report.json"), &json) {
                also(&mut error, format!("could not write report.json: {}", e));
            }
            shared.lock().unwrap().report = Some(json);
            read_as = Some(format!("{}{}", tally(&report), board_ids(&report.csids)));
        }
    }

    // Only on a failure. A stop is the operator's own doing, and a whole log means the
    // port held out for as long as it mattered.
    if !stopped && error.as_deref().is_some_and(|e| !e.starts_with(STOPPED)) {
        if let Some(why) = listener.lost() {
            also(&mut error, format!("{} stopped answering during the pull ({}), it may have been unplugged", port_name, why));
        }
    }

    // stop goes first, otherwise an early stop reads as an empty printing
    if stopped {
        error = Some(match error.take() {
            Some(more) => format!("{}. Also: {}", STOPPED, more),
            None => String::from(STOPPED),
        });
    }

    // Last, and whatever went wrong above: a board left on the dump firmware does not fly.
    let mut board = if wrote_dump { Board::OnDump } else { Board::Untouched };
    if let Some(image) = saved.as_ref().filter(|_| wrote_dump) {
        say(shared, broadcast, 90, "putting the flight software back");
        match put_back(bench, image) {
            Ok(()) => {
                shared.lock().unwrap().restored = true;
                board = Board::Back;
                // Only the newest save per board is kept. With the console check off the
                // "flight software" saved might be the dump firmware, which isn't kept
                if is_dump(image) {
                    let _ = std::fs::remove_file(image);
                    let mut p = shared.lock().unwrap();
                    p.saved_fsw = None;
                    p.restored = false;
                    board = Board::OnDump;
                } else if !wipe {
                    crate::layout::keep_only_latest(dir, image);
                }
            }
            // only the dump firmware failed to go back on, nothing lost
            Err(_) if is_dump(image) => {
                let _ = std::fs::remove_file(image);
                shared.lock().unwrap().saved_fsw = None;
                board = Board::OnDump;
            }
            Err(e) => {
                board = Board::NotBack;
                // only time a clear keeps it, the ChipSats screen puts it back from there
                let kept = if wipe { kept_from_clear(dir, image, now) } else { Ok(image.clone()) };
                let kept = kept.unwrap_or_else(|why| {
                    also(&mut error, why);
                    image.clone()
                });
                if wipe {
                    shared.lock().unwrap().saved_fsw = kept.file_name().map(|n| n.to_string_lossy().to_string());
                }
                also(
                    &mut error,
                    format!(
                        "the flight software was NOT put back ({}). The image that was on the board is {} and has to be written back by hand",
                        e,
                        kept.display()
                    ),
                );
            }
        }
    }
    if wipe && board != Board::NotBack {
        if let Some(image) = saved.as_ref() {
            let _ = std::fs::remove_file(image);
            shared.lock().unwrap().saved_fsw = None;
        }
    }
    finish(shared, broadcast, read_as, error, board, (!wipe).then_some(folder.as_path()));
}

// Inside the data folder so moving out is a rename. Nothing lists it
const WORKING: &str = ".clearing";

fn kept_from_clear(data: &Path, image: &Path, at: SystemTime) -> Result<PathBuf, String> {
    let place = crate::layout::folder(data, crate::layout::IMAGES, at)?;
    let file = image.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = file.strip_suffix(".bin").unwrap_or(&file).to_string();
    let (hhmm, rest) = match stem.split_once('-') {
        Some((t, rest)) if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) => (t.to_string(), rest.to_string()),
        _ => (crate::layout::hhmm(at), stem.clone()),
    };
    let to = crate::layout::unused(&place.dir, &hhmm, &rest, ".bin")?;
    std::fs::rename(image, &to).map_err(|e| format!("could not move {} to {}: {}", image.display(), to.display(), e))?;
    // an older save goes beside the newer ones instead of replacing them
    let newer = crate::stm32::uid_in_name(&file).is_some_and(|uid| {
        crate::layout::saved_images(data).iter().any(|p| {
            p != &to
                && p.file_name().and_then(|n| crate::stm32::uid_in_name(&n.to_string_lossy())).as_deref() == Some(uid.as_str())
                && modified(p) > Some(at)
        })
    });
    if !newer {
        crate::layout::keep_only_latest(data, &to);
    }
    Ok(to)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn is_dump(image: &Path) -> bool {
    std::fs::read(image).map(|b| crate::layout::looks_like_dump(&b)).unwrap_or(false)
}

// Flight software left in WORKING by a clear that the app quit in the middle of
fn recover_working(data: &Path) -> Vec<String> {
    let mut said = Vec::new();
    let work = data.join(WORKING);
    for entry in std::fs::read_dir(&work).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "bin") || !path.is_file() {
            continue;
        }
        if is_dump(&path) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let at = modified(&path).unwrap_or_else(SystemTime::now);
        said.push(match kept_from_clear(data, &path, at) {
            Ok(to) => format!("an earlier clear left {} without putting it back; it is now {}", path.display(), to.display()),
            Err(e) => format!("an earlier clear left {}, and it could not be filed: {}", path.display(), e),
        });
    }
    said
}

fn finish(
    shared: &Shared,
    broadcast: &crate::ws::Broadcast,
    read_as: Option<String>,
    error: Option<String>,
    board: Board,
    folder: Option<&Path>,
) {
    // a stop isn't shown as a failure
    let stopped = error.as_deref().is_some_and(|e| e.starts_with(STOPPED));
    let read_as = match (stopped, read_as) {
        (true, Some(t)) => Some(format!("{}, {}", STOPPED, t)),
        (true, None) => Some(String::from(STOPPED)),
        (false, r) => r,
    };
    let mut p = shared.lock().unwrap();
    p.running = false;
    p.finished_ms = crate::logfile::now_ms();
    if error.is_none() {
        p.pct = 100;
    }
    p.note = closing_note(read_as.as_deref(), error.is_some() && !stopped, board, folder);
    p.error = error;
    let frame = format!(
        "{{\"type\":\"pull\",\"pct\":{},\"note\":\"{}\",\"done\":true}}",
        p.pct,
        esc(&p.note)
    );
    drop(p);
    broadcast.send(&frame);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_a_reloaded_page_reads_back() {
        let mut p = Pull::idle();
        p.running = true;
        p.board = "/dev/ttyACM1".into();
        p.started_ms = 1_000_000;
        p.pct = 45;
        p.note = "reading the log off the chip".into();
        p.steps.push("saving the flight software \"first\"".into());
        let j = state_json(&p, 1_075_000);
        assert!(j.contains("\"running\":true"));
        assert!(j.contains("\"seconds\":75"), "{}", j);
        assert!(j.contains("\"keepFsw\":true"));
        assert!(j.contains("\"restored\":false"));
        assert!(j.contains("\\\"first\\\""), "a quote must not break the json: {}", j);
    }

    #[test]
    fn a_finished_pull_stops_counting() {
        let mut p = Pull::idle();
        p.started_ms = 1_000_000;
        p.finished_ms = 1_030_000;
        assert!(state_json(&p, 9_999_999).contains("\"seconds\":30"));
    }

    #[test]
    fn the_state_says_where_the_pull_went_and_what_it_read() {
        let mut p = Pull::idle();
        assert!(state_json(&p, 0).contains("\"log\":null,\"dir\":null,\"raw\":null,\"report\":null,"));
        p.dir = Some("/home/op/DeSCENT/descent-pull-2026-09-21T14-13-20-ttyACM1".into());
        p.log = Some("descent-pull-2026-09-21T14-13-20-ttyACM1/ground.log".into());
        p.raw = Some("descent-pull-2026-09-21T14-13-20-ttyACM1/dump.txt".into());
        p.report = Some("{\"format\":\"raw\",\"read\":3}".into());
        let j = state_json(&p, 0);
        assert!(j.contains("\"log\":\"descent-pull-2026-09-21T14-13-20-ttyACM1/ground.log\""), "{}", j);
        assert!(j.contains("\"dir\":\"/home/op/DeSCENT/descent-pull-2026-09-21T14-13-20-ttyACM1\""), "{}", j);
        assert!(j.contains("\"raw\":\"descent-pull-2026-09-21T14-13-20-ttyACM1/dump.txt\""), "{}", j);
        // An object, not a string with an object in it.
        assert!(j.contains("\"report\":{\"format\":\"raw\",\"read\":3},"), "{}", j);
    }

    #[test]
    fn a_pull_folder_is_named_for_the_second_and_the_board() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
        let hm = crate::layout::hhmm(at);
        let dir = std::env::temp_dir().join(format!("dg-pull-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(folder_in(&dir, "/dev/ttyACM1", at).unwrap(), dir.join(format!("{}-ttyACM1", hm)));
        assert_eq!(folder_in(&dir, "COM3", at).unwrap(), dir.join(format!("{}-COM3", hm)));
        std::fs::create_dir(dir.join(format!("{}-board9", hm))).unwrap();
        assert_eq!(folder_in(&dir, "9", at).unwrap(), dir.join(format!("{}-2-board9", hm)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_printing_is_over_when_its_found_line_has_ended() {
        let old = b"raw 1 000040 AA\r\nfound 2 records, 2 good\r\nsend D to print the log again\r\n";
        // What was printed before the reset does not count.
        assert_eq!(whole_found_line(old, old.len()), (false, old.len()));
        assert!(whole_found_line(old, 0).0);

        let mut text = old.to_vec();
        let reset = text.len();
        text.extend_from_slice(b"I Boot 3 c0: start test=flashdump\r\nraw 0 000000 AA\r\nfound 3276");
        // Half a found line would be read as a smaller count, so it waits for the line end,
        // and picks up again from the start of that line.
        let (whole, next) = whole_found_line(&text, reset);
        assert!(!whole);
        assert_eq!(&text[next..], b"found 3276");
        text.extend_from_slice(b"8 records, 32768 good\r\n");
        assert!(whole_found_line(&text, next).0);
        // A line that only mentions the word is not one.
        assert!(!whole_found_line(b"I LOG 12 c1: found nothing\r\n", 0).0);
    }

    #[test]
    fn the_board_ids_are_said_as_a_person_would_say_them() {
        assert_eq!(board_ids(&[13]), ", board id 13");
        assert_eq!(board_ids(&[9, 13]), ", board ids 9 and 13");
        assert_eq!(board_ids(&[9, 13, 255]), ", board ids 9, 13 and 255");
        assert_eq!(board_ids(&[]), "");
    }

    #[test]
    fn the_last_note_says_what_was_read_and_what_is_on_the_board() {
        let folder = std::path::Path::new("/home/op/DeSCENT/descent-pull-x");
        assert_eq!(
            closing_note(Some("done"), false, Board::Back, Some(folder)),
            "done, flight software back on the board; saved in /home/op/DeSCENT/descent-pull-x"
        );
        assert_eq!(
            closing_note(Some("done"), false, Board::OnDump, Some(folder)),
            "done, the board is still running the dump firmware; saved in /home/op/DeSCENT/descent-pull-x"
        );
        assert_eq!(
            closing_note(Some("done"), true, Board::NotBack, Some(folder)),
            "failed, and the flight software was NOT put back; what arrived is in /home/op/DeSCENT/descent-pull-x"
        );
        assert_eq!(
            closing_note(None, true, Board::Back, Some(folder)),
            "failed, flight software back on the board; what arrived is in /home/op/DeSCENT/descent-pull-x"
        );
        assert_eq!(closing_note(None, true, Board::Untouched, None), "failed");

        let text = "record 0 boot 0 ms 2517\n\
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36\n";
        let cut = crate::dump::report(&crate::dump::parse_what_arrived(text), 0, 0);
        assert_eq!(tally(&cut), "read 1 records, and the board never got to its count");
        let short = format!("{}found 3 records, 3 good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x0000C0\n", text);
        let short = crate::dump::report(&crate::dump::parse(&short), 0, 0);
        assert_eq!(
            tally(&short),
            "read 1 of the 3 records the board counted (2 never arrived, 0 failed their check)"
        );
        assert_eq!(
            closing_note(Some(&tally(&short)), false, Board::Back, Some(folder)),
            "read 1 of the 3 records the board counted (2 never arrived, 0 failed their check), \
flight software back on the board; saved in /home/op/DeSCENT/descent-pull-x"
        );
        let whole = format!("{}found 1 records, 1 good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x000040\n", text);
        assert_eq!(tally(&crate::dump::report(&crate::dump::parse(&whole), 0, 0)), "done");
    }

    // A port that plays back what it is given, one read at a time, and then drops out.
    struct Script {
        steps: std::vec::IntoIter<Step>,
        over: std::sync::mpsc::Sender<()>,
    }

    enum Step {
        Bytes(Vec<u8>),
        Silence(u64),
        Wait(std::sync::mpsc::Receiver<()>),
        Gone(&'static str),   // the port fails the way a pulled cable does
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.steps.next() {
                Some(Step::Bytes(b)) => {
                    buf[..b.len()].copy_from_slice(&b);
                    Ok(b.len())
                }
                Some(Step::Silence(ms)) => {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    Err(std::io::ErrorKind::TimedOut.into())
                }
                Some(Step::Wait(go)) => {
                    let _ = go.recv();
                    Ok(0)
                }
                Some(Step::Gone(why)) => Err(std::io::Error::other(why)),
                None => {
                    let _ = self.over.send(());
                    Err(std::io::ErrorKind::BrokenPipe.into())
                }
            }
        }
    }

    fn played(steps: Vec<Step>) -> (Listener, std::sync::mpsc::Receiver<()>) {
        let (over, ended) = std::sync::mpsc::channel();
        (Listener::on(Script { steps: steps.into_iter(), over }), ended)
    }

    #[test]
    fn what_the_port_sent_is_kept_byte_for_byte() {
        // Line noise is not valid UTF-8, and one character can straddle two reads.
        let (l, ended) = played(vec![Step::Bytes(b"raw 0 \xFF\xC3".to_vec()), Step::Bytes(b"\xA9 00\r\n".to_vec())]);
        ended.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(l.take(), b"raw 0 \xFF\xC3\xA9 00\r\n");
    }

    #[test]
    fn the_reader_gap_is_its_own_time_and_not_a_quiet_line() {
        // A line with nothing on it is time spent inside read().
        let (l, ended) = played(vec![Step::Silence(300), Step::Silence(300)]);
        ended.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(l.longest_gap_ms() < 250, "{}", l.longest_gap_ms());

        // Somebody else holding the buffer is not.
        let (go, wait) = std::sync::mpsc::channel();
        let (l, ended) = played(vec![Step::Wait(wait), Step::Bytes(b"raw".to_vec()), Step::Silence(1)]);
        {
            let _held = l.heard.lock().unwrap();
            go.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        ended.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(l.longest_gap_ms() >= 200, "{}", l.longest_gap_ms());
    }

    // A board that is not there. It writes down what was done to it, starts "printing"
    // when it is reset, and can be told which write to fail.
    struct Desk {
        did: Mutex<Vec<String>>,
        named: Mutex<Vec<String>>,
        go: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        fail_write: Option<usize>,
    }

    const ON_THE_PROBE: &str = "0038002A5553500920393256";

    const FLIGHT: &[u8] = b"flight software";
    const DUMP: &[u8] = b"dump firmware";

    impl Bench for Desk {
        fn uid(&self) -> Result<String, String> {
            self.did.lock().unwrap().push("uid".into());
            Ok(ON_THE_PROBE.to_string())
        }
        fn save(&self, dir: &Path, file: &str) -> Result<PathBuf, String> {
            self.did.lock().unwrap().push("save".into());
            self.named.lock().unwrap().push(file.to_string());
            let path = dir.join("fsw.bin");
            std::fs::write(&path, FLIGHT).unwrap();
            Ok(path)
        }
        fn write(&self, image: &[u8]) -> Result<(), String> {
            let mut did = self.did.lock().unwrap();
            let nth = did.iter().filter(|d| d.starts_with("write")).count();
            did.push(format!("write {}", String::from_utf8_lossy(image)));
            drop(did);
            // The board is halted and silent for as long as a write takes, which on the
            // bench is much longer than the quiet limit. Here it is just longer.
            std::thread::sleep(std::time::Duration::from_millis(500));
            if self.fail_write == Some(nth) {
                return Err(String::from("the probe fell off"));
            }
            Ok(())
        }
        fn reset(&self) -> Result<(), String> {
            self.did.lock().unwrap().push("reset".into());
            if let Some(go) = self.go.lock().unwrap().take() {
                let _ = go.send(());
            }
            Ok(())
        }
    }

    // One pull at a desk with no board: what the board prints once it is reset, whether to
    // keep the flight software, and which write goes wrong. Quiet is cut to 400 ms so a
    // printing that stops does not hold the test for 8 s.
    fn pulled(what: &str, printed: &[u8], keep_fsw: bool, fail_write: Option<usize>) -> (Pull, Vec<String>, PathBuf) {
        pulled_ending(what, printed, Vec::new(), keep_fsw, fail_write)
    }

    // The same, with what the port does once the printing is over.
    fn pulled_ending(what: &str, printed: &[u8], then: Vec<Step>, keep_fsw: bool, fail_write: Option<usize>) -> (Pull, Vec<String>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dg-pull-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let mut steps = vec![
            Step::Bytes(b"I TX 1 c1: flight software talking\r\n".to_vec()),
            Step::Wait(wait),
            Step::Silence(100),   // booting
        ];
        steps.extend(printed.chunks(1000).map(|c| Step::Bytes(c.to_vec())));
        steps.extend(then);
        let (listener, _ended) = played(steps);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: what.starts_with("wipe"),
            keep_fsw,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        job.shared.lock().unwrap().running = true;
        job.shared.lock().unwrap().keep_fsw = keep_fsw;
        job.shared.lock().unwrap().wipe = job.wipe;
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        (state, did, dir)
    }

    #[test]
    fn a_clear_puts_the_flight_software_back_only_when_asked_to_keep_it() {
        // The board boots, starts printing, hears the E, asks, hears the Y, erases.
        let said = b"I Boot 3 c0: start test=flashdump\r\nraw 0 000000 AA\r\n\r\nstopped printing: E received\r\n\
erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\n\
erasing, 5 s typical, 25 s at most\r\nerase ok after 5012 ms\r\nsend D to print the log again, E to erase the whole chip\r\n";
        // unticked: dump firmware, reset, erase over the console. Nothing read off, nothing put back
        let (p, did, dir) = pulled("wipe", said, false, None);
        assert_eq!(did, vec!["write dump firmware", "reset"]);
        assert_eq!(p.error, None);
        assert!(!p.restored && p.wipe);
        assert!(p.steps.iter().all(|s| !s.contains("saving") && !s.contains("putting the flight software back")), "{:?}", p.steps);
        assert!(p.steps.iter().any(|s| s == "erasing the chip, about 5 s"), "{:?}", p.steps);
        assert_eq!(p.note, "flash cleared, the board is still running the dump firmware");
        assert_eq!((p.dir, p.log, p.raw, p.saved_fsw), (None, None, None, None));
        assert_eq!(kept_in(&dir), Vec::<String>::new(), "a clear keeps nothing");
        let _ = std::fs::remove_dir_all(&dir);

        let (p, did, dir) = pulled("wipe-kept", said, true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert_eq!(p.error, None);
        assert!(p.restored && p.wipe);
        assert_eq!(p.note, "flash cleared, flight software back on the board");
        assert_eq!((p.dir, p.log, p.raw, p.saved_fsw), (None, None, None, None));
        assert_eq!(kept_in(&dir), Vec::<String>::new(), "the copy in the working folder goes once it is back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // working folder included
    fn kept_in(dir: &Path) -> Vec<String> {
        fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.push(p.strip_prefix(base).unwrap().display().to_string());
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn what_a_clear_could_not_put_back_becomes_the_boards_newest_image() {
        let dir = std::env::temp_dir().join(format!("dg-pull-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let day = crate::layout::day(SystemTime::now());
        for (d, f) in [("2026-10-17/images", "1454-board9-0027003A.bin"), ("2026-10-17/images", "1513-board12-002C002E.bin"), (WORKING, "1432-board9-0027003A.bin")] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join(f), FLIGHT).unwrap();
        }
        // same name already there from earlier in the minute
        std::fs::create_dir_all(dir.join(&day).join("images")).unwrap();
        std::fs::write(dir.join(&day).join("images/1432-board9-0027003A.bin"), FLIGHT).unwrap();
        let at = SystemTime::now() + std::time::Duration::from_millis(10);
        let to = kept_from_clear(&dir, &dir.join(WORKING).join("1432-board9-0027003A.bin"), at).unwrap();
        assert_eq!(to, dir.join(&day).join("images/1432-2-board9-0027003A.bin"));
        assert_eq!(
            kept_in(&dir),
            vec!["2026-10-17/images/1513-board12-002C002E.bin".to_string(), format!("{}/images/1432-2-board9-0027003A.bin", day)],
            "board 9's older images go, board 12's stays"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_left_behind_is_filed_on_its_own_day_and_does_not_replace_a_newer_save() {
        let dir = std::env::temp_dir().join(format!("dg-pull-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(WORKING)).unwrap();
        let old = SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
        let left = dir.join(WORKING).join("0910-board9-0027003A.bin");
        std::fs::write(&left, FLIGHT).unwrap();
        std::fs::File::options().write(true).open(&left).unwrap().set_modified(old).unwrap();
        std::fs::write(dir.join(WORKING).join("0911-board12-002C002E.bin"), DUMP_WITH_NAME).unwrap();
        // board 9 was pulled since, so that save is newer
        let today = crate::layout::day(SystemTime::now());
        std::fs::create_dir_all(dir.join(&today).join("images")).unwrap();
        std::fs::write(dir.join(&today).join("images/1100-board9-0027003A.bin"), FLIGHT).unwrap();

        let said = recover_working(&dir);
        assert_eq!(said.len(), 1, "{:?}", said);
        let then = crate::layout::day(old);
        let mut want = vec![format!("{}/images/0910-board9-0027003A.bin", then), format!("{}/images/1100-board9-0027003A.bin", today)];
        want.sort();
        assert_eq!(kept_in(&dir), want, "both of board 9's kept, board 12's dump firmware gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const DUMP_WITH_NAME: &[u8] = b"\x00\x01 test=flashdump \x02";

    #[test]
    fn a_clear_anyway_on_a_board_already_on_the_dump_firmware_does_not_say_it_put_flight_software_back() {
        // console check off, so the dump firmware gets saved as if it were flight software
        struct OnDump(Desk);
        impl Bench for OnDump {
            fn uid(&self) -> Result<String, String> { self.0.uid() }
            fn save(&self, dir: &Path, file: &str) -> Result<PathBuf, String> {
                let path = self.0.save(dir, file)?;
                std::fs::write(&path, DUMP_WITH_NAME).unwrap();
                Ok(path)
            }
            fn write(&self, image: &[u8]) -> Result<(), String> { self.0.write(image) }
            fn reset(&self) -> Result<(), String> { self.0.reset() }
        }
        let dir = std::env::temp_dir().join(format!("dg-pull-wipe-ondump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let said = b"I Boot 3 c0: start test=flashdump\r\nstopped printing: E received\r\n\
erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\n\
erase ok after 5012 ms\r\n";
        let (go, wait) = std::sync::mpsc::channel();
        let mut steps = vec![Step::Bytes(b"I TX 1 c1: talking\r\n".to_vec()), Step::Wait(wait), Step::Silence(100)];
        steps.push(Step::Bytes(said.to_vec()));
        let (listener, _ended) = played(steps);
        let desk = OnDump(Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None });
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: true,
            keep_fsw: false,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        job.shared.lock().unwrap().running = true;
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let p = job.shared.lock().unwrap().clone();
        assert_eq!(p.error, None, "{:?}", p.error);
        assert!(!p.restored);
        assert_eq!(p.note, "flash cleared, the board is still running the dump firmware");
        assert_eq!(kept_in(&dir), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_that_cannot_put_the_flight_software_back_keeps_it_with_the_boards_images() {
        let said = b"I Boot 3 c0: start test=flashdump\r\nstopped printing: E received\r\n\
erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\n\
erase ok after 5012 ms\r\n";
        let (p, did, dir) = pulled("wipe-notback", said, true, Some(1));
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software"]);
        assert!(!p.restored);
        let kept = kept_in(&dir);
        assert_eq!(kept.len(), 1, "{:?}", kept);
        assert!(kept[0].contains("/images/") && !kept[0].contains(WORKING), "{:?}", kept);
        let e = p.error.unwrap();
        assert!(e.contains("NOT put back") && e.contains(&kept[0]), "{}", e);
        assert!(p.saved_fsw.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flash_drivers_own_erase_log_line_is_not_the_verdict() {
        // Board 9, 2026-10-17: the driver logs "erase what=chip" while erasing, and the app took
        // that for the firmware's word, read nothing off it, and reset the board mid-erase
        let said = b"I BOOT 0 c0: start test=flashdump\r\nstopped printing: E received\r\n\
erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\n\
erasing, 5 s typical, 25 s at most\r\nI FLASH 416 c0: erase what=chip\r\nerase ok after 5012 ms\r\nsend D to print the log again, E to erase the whole chip\r\n";
        let (p, _, dir) = pulled("wipe-logged", said, true, None);
        assert_eq!(p.error, None, "{:?}", p.error);
        assert!(p.restored && p.wipe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_the_firmware_does_not_confirm_is_a_failure_with_the_board_put_back() {
        let said = b"I Boot 3 c0: start test=flashdump\r\nstopped printing: E received\r\n\
erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\nerase Timeout after 25000 ms\r\n";
        let (p, did, dir) = pulled("wipe-failed", said, true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        let e = p.error.unwrap();
        assert!(e.contains("was not erased") && e.contains("erase Timeout"), "{}", e);
        assert!(p.restored);
        let _ = std::fs::remove_dir_all(&dir);

        // Never asked: the console is not this board's.
        let (p, did, dir) = pulled("wipe-silent", b"I TX 1 c1: something else talking\r\n", true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.error.unwrap().contains("never asked"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TWO: &str = "record 0 boot 0 ms 2517\r\n\
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36\r\n\
record 1 boot 0 ms 2567\r\n\
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36\r\n";
    const FOUND_TWO: &str = "found 2 records, 2 good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x000080\r\n";

    #[test]
    fn the_report_starts_with_which_firmware_read_it_and_which_was_on_the_board() {
        assert_eq!(with_firmware("{\"read\":2}", "{\"id\":\"ab\"}", None), "{\"firmware\":{\"dump\":{\"id\":\"ab\"},\"onBoard\":null},\"read\":2}");
        assert_eq!(with_firmware("{}", "", Some("{\"kind\":\"flight\"}")), "{\"firmware\":{\"dump\":null,\"onBoard\":{\"kind\":\"flight\"}}}");
        assert_eq!(with_firmware("not json", "", None), "not json");
    }

    #[test]
    fn a_pull_report_records_the_dump_used_and_the_flight_image_built_here_that_was_on_the_board() {
        let dir = std::env::temp_dir().join(format!("dg-pull-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let mut steps = vec![Step::Bytes(b"I TX 1 c1: flight software talking\r\n".to_vec()), Step::Wait(wait), Step::Silence(100)];
        steps.extend(printed.as_bytes().chunks(1000).map(|c| Step::Bytes(c.to_vec())));
        let (listener, _ended) = played(steps);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let sha = |b: &[u8]| -> String { use sha2::{Digest, Sha256}; Sha256::digest(b).iter().map(|x| format!("{:02x}", x)).collect() };
        let mut known = crate::chipflash::Known::carried();
        known.built = vec![crate::localfw::Identity {
            part: "flight-le".into(),
            at: (crate::logfile::now_ms() / 1000) as u64,
            origin: crate::fswbuild::Origin { source: sha(b"sources"), commit: Some("abc1234ffff".into()), dirty: Some(true), changed: vec!["V2_6_X_Code/src/config.h".into()] },
            sha256: sha(FLIGHT),
            masked: sha(FLIGHT),
            len: FLIGHT.len(),
            block_at: None,
        }];
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: false,
            keep_fsw: true,
            dump_image: DUMP.to_vec(),
            dump_from: String::from("{\"part\":\"flash_dump\",\"words\":\"dump firmware built here 14:02 from abc1234\"}"),
            prepare: None,
            known: Some(known),
            uid: None,
            name: None,
            check_console: false,
        };
        job.shared.lock().unwrap().running = true;
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let p = job.shared.lock().unwrap().clone();
        assert_eq!(p.error, None);
        let report = std::fs::read_to_string(PathBuf::from(p.dir.unwrap()).join("report.json")).unwrap();
        assert!(report.starts_with("{\"firmware\":{\"dump\":{\"part\":\"flash_dump\",\"words\":\"dump firmware built here 14:02 from abc1234\"},\"onBoard\":{\"kind\":\"flight\",\"module\":\"le\""), "{}", report);
        assert!(report.contains("\"words\":\"flight LE built here ") && report.contains("from abc1234 + config.h changed"), "{}", report);
        assert!(report.contains("\"read\":2,"), "the rest of the report is still there: {}", report);
        assert_eq!(p.report.as_deref(), Some(report.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dump_firmware_that_cannot_be_got_stops_the_pull_before_the_board_is_touched() {
        let mut job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: std::env::temp_dir(),
            port_name: "/dev/nonexistent-dg-port".into(),
            probe: None,
            wipe: false,
            keep_fsw: true,
            dump_image: Vec::new(),
            dump_from: String::new(),
            prepare: Some(Box::new(|say: &dyn Fn(&str)| {
                say("building the dump firmware from the flight software in /tree");
                Err(String::from("the dump firmware from this computer's flight software did not build, and this app carries none. error: x"))
            })),
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        job.shared.lock().unwrap().running = true;
        let shared = job.shared.clone();
        job.wipe = false;
        run(job);
        let p = shared.lock().unwrap().clone();
        assert!(p.error.as_deref().unwrap().contains("did not build"), "{:?}", p.error);
        assert_eq!(p.steps[0], "building the dump firmware from the flight software in /tree");
        assert!(!p.steps.iter().any(|s| s.contains("listening")), "{:?}", p.steps);
    }

    #[test]
    fn a_pull_leaves_one_folder_and_the_flight_software_back_on() {
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let (p, did, dir) = pulled("whole", printed.as_bytes(), true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert_eq!(p.error, None);
        assert!(!p.running);
        assert!(p.restored);
        assert_eq!(p.pct, 100);

        let folder = PathBuf::from(p.dir.clone().unwrap());
        assert!(folder.is_absolute());
        let day = crate::layout::day(SystemTime::now());
        assert_eq!(folder.parent().unwrap(), dir.join(&day).join("pulls"));
        let leaf = folder.file_name().unwrap().to_string_lossy().to_string();
        assert!(leaf.ends_with("-ttyACM1") && leaf.len() == "1432-ttyACM1".len(), "{}", leaf);
        // relative to the data folder, as /api/log takes it
        let name = format!("{}/pulls/{}", day, leaf);
        // Every byte of TWO's packets is its own offset, so the id at 50 reads 50.
        assert_eq!(p.note, format!("done, board id 50, flight software back on the board; saved in {}", folder.display()));
        assert_eq!(p.log, Some(format!("{}/ground.log", name)));
        assert_eq!(p.raw, Some(format!("{}/dump.txt", name)));
        assert_eq!(p.saved_fsw.as_deref(), Some("fsw.bin"));

        // Everything the port carried, the chatter from before the reset included.
        let raw = std::fs::read(folder.join("dump.txt")).unwrap();
        assert_eq!(raw, format!("I TX 1 c1: flight software talking\r\n{}", printed).as_bytes());
        let log = std::fs::read_to_string(folder.join("ground.log")).unwrap();
        assert_eq!(log.matches("PKT,55,").count(), 2);
        let report = std::fs::read_to_string(folder.join("report.json")).unwrap();
        assert_eq!(p.report.as_deref(), Some(report.as_str()));
        assert!(report.contains("\"read\":2,") && report.contains("\"complete\":true"), "{}", report);
        assert!(report.contains(&format!("\"bytes\":{},", raw.len())), "{}", report);
        assert!(report.contains("\"csids\":[50],\"boots\":["), "{}", report);
        assert_eq!(std::fs::read(folder.join("fsw.bin")).unwrap(), FLIGHT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_printing_that_stops_short_is_kept_and_the_board_is_still_put_back() {
        let (p, did, dir) = pulled("short", TWO.as_bytes(), true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert_eq!(p.error, None, "part of a log is not a failure");
        assert!(p.restored);
        assert!(
            p.note.starts_with("read 2 records, and the board never got to its count, board id 50, flight software back on the board; saved in "),
            "{}",
            p.note
        );
        let folder = PathBuf::from(p.dir.unwrap());
        assert!(folder.join("dump.txt").is_file());
        assert!(folder.join("ground.log").is_file());
        assert!(p.report.unwrap().contains("\"found\":null,"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_chip_is_said_to_be_empty_and_the_board_is_still_put_back() {
        // A wiped chip before a flight is an ordinary thing to pull, and "nothing that
        // reads as a log" would send someone looking for a fault that is not there.
        let empty = b"found 0 records, 0 good (0 time anchors), 0 bad, from 0 boots, log ends at address 0x000000\r\n";
        let (p, did, dir) = pulled("empty", empty, true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.restored);
        let e = p.error.unwrap();
        assert!(e.contains("the log on the chip is empty"), "{}", e);
        assert!(!e.contains("nothing that reads as a log"), "{}", e);
        assert_eq!(p.log, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chip_that_did_not_answer_is_quoted_in_the_failure() {
        // Board 000F0033 on 2026-10-16: the dump firmware printed its init line and nothing
        // else, and the failure sent the operator hunting for a port problem instead
        let said = b"I BOOT 0 c0: start test=flashdump\r\nI FLASH 0 c0: start\r\nI FLASH 2 c0: init ok=0 id=0xFFFFFF reason=noack\r\n";
        let (p, _, dir) = pulled("noack", said, true, None);
        let e = p.error.unwrap();
        assert!(e.contains("nothing that reads as a log"), "{}", e);
        assert!(e.contains("The board said: I FLASH 2 c0: init ok=0 id=0xFFFFFF reason=noack."), "{}", e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_that_prints_nothing_is_a_failure_and_is_still_put_back() {
        let (p, did, dir) = pulled("nothing", b"I Boot 3 c0: start test=flashdump\r\n", true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.restored);
        let folder = PathBuf::from(p.dir.unwrap());
        let e = p.error.unwrap();
        assert!(e.contains("nothing that reads as a log"), "{}", e);
        assert!(e.contains(&folder.join("dump.txt").display().to_string()), "{}", e);
        assert_eq!(p.note, format!("failed, flight software back on the board; what arrived is in {}", folder.display()));
        assert_eq!(p.log, None);
        // What it did say is still on disk.
        let raw = std::fs::read_to_string(folder.join("dump.txt")).unwrap();
        assert!(raw.ends_with("start test=flashdump\r\n"), "{}", raw);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_console_that_goes_away_mid_dump_is_named_in_the_failure() {
        // cut off partway through the first record
        let said = b"I BOOT 0 c0: start test=flashdump\r\nraw 0 0000";
        let (p, did, dir) = pulled_ending("gone", said, vec![Step::Gone("No such device (os error 19)")], true, None);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.restored);
        let e = p.error.unwrap();
        assert!(e.starts_with("nothing that reads as a log"), "{}", e);
        assert!(e.contains("/dev/ttyACM1 stopped answering"), "{}", e);
        assert!(e.contains("No such device (os error 19)"), "{}", e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dump_firmware_that_will_not_write_still_gets_the_flight_software_put_back() {
        let (p, did, dir) = pulled("nowrite", b"", true, Some(0));
        assert_eq!(did, vec!["save", "write dump firmware", "write flight software", "reset"]);
        assert!(p.restored);
        assert_eq!(p.error.as_deref(), Some("the probe fell off"));
        assert!(p.note.starts_with("failed, flight software back on the board; "), "{}", p.note);
        assert!(PathBuf::from(p.dir.unwrap()).join("dump.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restore_that_fails_says_where_the_saved_image_is() {
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let (p, did, dir) = pulled("norestore", printed.as_bytes(), true, Some(1));
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software"]);
        assert!(!p.restored);
        let folder = PathBuf::from(p.dir.unwrap());
        let e = p.error.unwrap();
        assert!(e.contains("NOT put back (the probe fell off)"), "{}", e);
        assert!(e.contains(&folder.join("fsw.bin").display().to_string()), "{}", e);
        assert!(p.note.starts_with("failed, and the flight software was NOT put back; "), "{}", p.note);
        // The log was read before that went wrong, and is still there to open.
        assert!(p.log.is_some());
        assert!(folder.join("ground.log").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pull_that_was_told_not_to_keep_the_flight_software_leaves_the_dump_firmware_on() {
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let (p, did, dir) = pulled("nokeep", printed.as_bytes(), false, None);
        assert_eq!(did, vec!["write dump firmware", "reset"]);
        assert!(!p.restored);
        assert_eq!(p.saved_fsw, None);
        assert_eq!(p.error, None);
        assert!(p.note.starts_with("done, board id 50, the board is still running the dump firmware; saved in "), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A pull that was told which board it is for, and what the list calls that board.
    fn pulled_for(what: &str, uid: &str, name: Option<&str>) -> (Pull, Vec<String>, Vec<String>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dg-pull-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let (listener, _ended) = played(vec![Step::Wait(wait), Step::Bytes(printed.into_bytes())]);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: false,
            keep_fsw: true,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: Some(uid.into()),
            name: name.map(String::from),
            check_console: false,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        let named = desk.named.lock().unwrap().clone();
        (state, did, named, dir)
    }

    #[test]
    fn a_pull_meant_for_one_board_stops_before_touching_another() {
        let meant = "0038002A5553500920393301";
        let (p, did, _, dir) = pulled_for("moved", meant, Some("Right one"));
        assert_eq!(did, vec!["uid"], "nothing is saved or written on a board nobody asked about");
        let e = p.error.unwrap();
        assert!(e.contains(meant) && e.contains(ON_THE_PROBE), "{}", e);
        assert!(e.contains("the clip may have moved"), "{}", e);
        assert_eq!(p.saved_fsw, None);
        assert!(!p.restored);
        assert!(p.note.starts_with("failed; what arrived is in "), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pull_that_knows_the_board_names_its_folder_and_image_after_it() {
        let (p, did, named, dir) = pulled_for("known", ON_THE_PROBE, Some("Flight spare 2"));
        assert_eq!(did, vec!["uid", "save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert_eq!(p.error, None);
        let folder = PathBuf::from(p.dir.unwrap());
        let name = folder.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.ends_with("-Flight-spare-2"), "{}", name);
        assert_eq!(named.len(), 1);
        assert!(named[0].ends_with("-Flight-spare-2-20393256.bin"), "{}", named[0]);
        let _ = std::fs::remove_dir_all(&dir);

        // A board the list has no name for keeps the port in the folder's name, and is
        // still told apart by its id in the image's.
        let (p, _, named, dir) = pulled_for("unnamed", ON_THE_PROBE, None);
        let folder = PathBuf::from(p.dir.unwrap());
        assert!(folder.file_name().unwrap().to_string_lossy().ends_with("-ttyACM1"), "{}", folder.display());
        assert!(named[0].ends_with("-board-20393256.bin"), "{}", named[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pull_with_no_board_given_asks_nothing_and_names_things_after_the_port() {
        let dir = std::env::temp_dir().join(format!("dg-pull-asbefore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let printed = format!("{}{}", TWO, FOUND_TWO);
        let (listener, _ended) = played(vec![Step::Wait(wait), Step::Bytes(printed.into_bytes())]);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: false,
            keep_fsw: true,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        assert!(!desk.did.lock().unwrap().contains(&"uid".to_string()));
        let named = desk.named.lock().unwrap().clone();
        assert!(named[0].ends_with("-ttyACM1.bin") && crate::stm32::uid_in_name(&named[0]).is_none(), "{}", named[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // One pull where the port was already carrying something when the board was reset.
    fn pulled_after(what: &str, before: &[u8], printed: &[u8]) -> (Pull, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dg-pull-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let mut steps = vec![Step::Bytes(before.to_vec()), Step::Wait(wait), Step::Silence(100)];
        steps.extend(printed.chunks(1000).map(|c| Step::Bytes(c.to_vec())));
        let (listener, _ended) = played(steps);
        // Until the old printing is in the buffer there is nothing for the reset to follow.
        while listener.len() < before.len() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: false,
            keep_fsw: false,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let state = job.shared.lock().unwrap().clone();
        (state, dir)
    }

    // A packet slot as flash_dump prints one, every payload byte the same.
    fn raw_packet(n: u64, boot: u16, ms: u32, fill: u8) -> String {
        let mut b = [fill; 64];
        b[0] = 0xA5;
        b[1..3].copy_from_slice(&boot.to_le_bytes());
        b[3..7].copy_from_slice(&ms.to_le_bytes());
        let crc = crate::dump::crc16(&b[..62]);
        b[62..].copy_from_slice(&crc.to_le_bytes());
        let hex: String = b.iter().map(|v| format!("{:02X}", v)).collect();
        format!("raw {} {:06X} {}\r\n", n, n * 64, hex)
    }

    // What a board left on the older dump firmware was printing when it was halted to be
    // written: whole records, and one cut off part way through its bytes.
    fn old_printing_tail() -> String {
        let bytes = vec!["EE"; 55].join(" ");
        format!(
            " 00 00 8C 4E 13 00 09 9C 6F 9D 3B\r\n\
record 3110 boot 6 ms 23151\r\n{b}\r\n\
record 3111 boot 6 ms 23201\r\n{b}\r\n\
record 3112 boot 6 ms 23252\r\n{b}\r\n\
record 3113 boot 6 ms 23302\r\n\
00 00 00 00 E8 4E 00 00 03 00 08 00 FD 26",
            b = bytes
        )
    }

    #[test]
    fn records_the_port_carried_before_the_reset_are_kept_as_evidence_and_not_as_this_dump() {
        let before = old_printing_tail();
        let after = [
            "\u{fffd}I BOOT 0 c0: start test=flashdump\r\nI FLASH 2 c0: init ok=1 id=0xEF4015\r\n".to_string(),
            raw_packet(0, 2, 1000, 0x0D),
            raw_packet(1, 2, 1050, 0x0D),
            "found 2 records, 2 good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x000080\r\n".to_string(),
        ]
        .concat();
        let (p, dir) = pulled_after("oldtail", before.as_bytes(), after.as_bytes());
        assert_eq!(p.error, None);
        let folder = PathBuf::from(p.dir.unwrap());

        // Everything the port carried is still in dump.txt, bytes as they came.
        let raw = std::fs::read(folder.join("dump.txt")).unwrap();
        assert_eq!(raw, [before.as_bytes(), after.as_bytes()].concat());

        let log = std::fs::read_to_string(folder.join("ground.log")).unwrap();
        assert_eq!(log.matches("PKT,55,").count(), 2, "{}", log);
        assert!(!log.contains("EEEEEEEE"), "an old record got into the log: {}", log);
        assert!(!log.contains("flash-boot6"), "{}", log);

        let report = p.report.unwrap();
        assert!(report.contains("\"found\":2,\"boardGood\":2,\"boardBad\":0,\"read\":2,"), "{}", report);
        assert!(report.contains("\"complete\":true"), "{}", report);
        // Never read, which is not the same as read and set aside.
        assert!(report.contains("\"ignoredOldFormat\":0,"), "{}", report);
        assert!(report.contains("\"csids\":[13],"), "{}", report);
        assert!(p.note.starts_with("done, board id 13, the board is still running the dump firmware; saved in "), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_came_before_the_reset_is_left_out_whichever_format_the_dump_is_in() {
        // The same tail in front of a dump in the older format, where nothing about the
        // lines themselves says they are not part of it. Only the reset does.
        let before = old_printing_tail();
        let after = format!("\r\nI Boot 3 c0: start test=flashdump\r\n{}{}", TWO, FOUND_TWO);
        let (p, dir) = pulled_after("oldtailtext", before.as_bytes(), after.as_bytes());
        assert_eq!(p.error, None);
        let report = p.report.unwrap();
        assert!(report.contains("\"found\":2,\"boardGood\":2,\"boardBad\":0,\"read\":2,"), "{}", report);
        assert!(report.contains("\"complete\":true"), "{}", report);
        let log = std::fs::read_to_string(PathBuf::from(p.dir.unwrap()).join("ground.log")).unwrap();
        assert_eq!(log.matches("PKT,55,").count(), 2, "{}", log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_found_line_from_before_the_reset_does_not_end_the_read() {
        // The board was already on the dump firmware and had printed a whole log before
        // it was reset. The read has to wait for the printing that follows the reset.
        let dir = std::env::temp_dir().join(format!("dg-pull-before-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let old = format!("{}found 2 records, 2 good\r\n", TWO);
        let new = format!("{}{}", TWO.replace("boot 0", "boot 4"), FOUND_TWO);
        let (listener, _ended) = played(vec![
            Step::Bytes(old.into_bytes()),
            Step::Wait(wait),
            Step::Silence(500),
            Step::Bytes(new.into_bytes()),
        ]);
        // Until the old printing is in the buffer there is nothing for the reset to follow.
        while listener.len() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: false,
            keep_fsw: false,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 2_000, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let p = job.shared.lock().unwrap().clone();
        let log = std::fs::read_to_string(PathBuf::from(p.dir.unwrap()).join("ground.log")).unwrap();
        assert!(log.contains("flash-boot4"), "the printing after the reset is the one taken: {}", log);
        let _ = std::fs::remove_dir_all(&dir);
    }
    // `stop_when` runs on its own thread and stops the job when it returns. Quiet limit is
    // long so only the stop can end the read early
    fn pulled_stopping(
        what: &str,
        printed: Vec<Step>,
        keep_fsw: bool,
        stop_when: Option<Box<dyn FnOnce(&Shared) + Send>>,
    ) -> (Pull, Vec<String>, PathBuf, std::time::Duration) {
        let dir = std::env::temp_dir().join(format!("dg-pull-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let mut steps = vec![Step::Wait(wait), Step::Silence(100)];
        steps.extend(printed);
        let (listener, _ended) = played(steps);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            wipe: what.starts_with("wipe"),
            keep_fsw,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: false,
        };
        job.shared.lock().unwrap().running = true;
        // None: stop already asked for when the job starts
        let watched = job.shared.clone();
        let stopper = match stop_when {
            None => {
                stop(&watched).unwrap();
                None
            }
            Some(when) => Some(std::thread::spawn(move || {
                when(&watched);
                stop(&watched).unwrap();
            })),
        };
        let began = Instant::now();
        pull(&job, &desk, listener, &Limits { quiet_ms: 10_000, read_ms: 20_000, ask_ms: 10_000, erase_ms: 10_000, boot_ms: 600 });
        let took = began.elapsed();
        if let Some(t) = stopper {
            t.join().unwrap();
        }
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        (state, did, dir, took)
    }

    fn until(shared: &Shared, what: impl Fn(&Pull) -> bool) {
        while !what(&shared.lock().unwrap()) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_pull_stopped_mid_read_keeps_what_arrived_and_puts_the_flight_software_back() {
        // two records, then slow printing for longer than the test runs
        let mut printed = vec![Step::Bytes(TWO.as_bytes().to_vec())];
        printed.extend((0..100).map(|_| Step::Silence(200)));
        let (p, did, dir, took) = pulled_stopping("stopread", printed, true, Some(Box::new(|s: &Shared| {
            until(s, |p| p.raw.is_some());
            std::thread::sleep(std::time::Duration::from_millis(800));
        })));
        assert!(took < std::time::Duration::from_secs(5), "the stop ended it, not the quiet limit: {:?}", took);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.restored);
        assert!(!p.running);
        assert_eq!(p.error.as_deref(), Some("stopped by the operator"));
        let folder = PathBuf::from(p.dir.clone().unwrap());
        assert!(
            p.note.starts_with("stopped by the operator, read 2 records, and the board never got to its count, board id 50, flight software back on the board; saved in "),
            "{}",
            p.note
        );
        assert!(std::fs::read_to_string(folder.join("dump.txt")).unwrap().contains(TWO));
        let log = std::fs::read_to_string(folder.join("ground.log")).unwrap();
        assert_eq!(log.matches("PKT,55,").count(), 2, "what arrived is read as well as kept");
        assert!(p.steps.iter().any(|s| s.starts_with("stop asked for")), "{:?}", p.steps);
        assert!(state_json(&p, 0).contains("\"stopping\":false"), "a finished job is not stopping");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stop_before_the_dump_firmware_leaves_the_board_as_it_was() {
        // asked while the flight software is being saved, so the board is never written
        let (p, did, dir, _) = pulled_stopping("stopearly", Vec::new(), true, None);
        assert_eq!(did, vec!["save"], "nothing written, so nothing to put back");
        assert!(!p.restored);
        assert_eq!(p.error.as_deref(), Some("stopped by the operator"));
        assert!(p.note.starts_with("stopped by the operator; saved in "), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_stopped_while_waiting_to_be_asked_puts_the_flight_software_back_unerased() {
        let printed = (0..100).map(|_| Step::Silence(200)).collect();
        let (p, did, dir, took) = pulled_stopping("wipe-stopped", printed, true, Some(Box::new(|s: &Shared| {
            until(s, |p| p.note.starts_with("asking the dump firmware"));
            std::thread::sleep(std::time::Duration::from_millis(500));
        })));
        assert!(took < std::time::Duration::from_secs(5), "{:?}", took);
        assert_eq!(did, vec!["save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert!(p.restored);
        assert_eq!(p.error.as_deref(), Some("stopped by the operator"));
        assert_eq!(p.note, "stopped by the operator, flight software back on the board");
        assert_eq!(kept_in(&dir), Vec::<String>::new(), "a stopped clear keeps nothing either");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn there_is_nothing_to_stop_when_nothing_runs() {
        let s = shared();
        assert!(stop(&s).is_err());
        s.lock().unwrap().running = true;
        stop(&s).unwrap();
        stop(&s).unwrap();
        let p = s.lock().unwrap().clone();
        assert_eq!(p.steps.len(), 1, "a second press says nothing new");
        assert!(state_json(&p, 0).contains("\"stopping\":true"));
    }
    // `answers` is what the port says when the check resets the board, None for nothing
    fn pulled_checking(what: &str, answers: Option<&str>) -> (Pull, Vec<String>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dg-pull-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let mut steps = vec![Step::Bytes(b"I TX 1 c1: flight software talking\r\n".to_vec()), Step::Wait(wait)];
        if let Some(said) = answers {
            // the write between the two resets takes about 500 ms here
            steps.push(Step::Bytes(said.as_bytes().to_vec()));
            steps.push(Step::Silence(1500));
            steps.push(Step::Bytes(format!("I BOOT 0 c0: start test=flashdump\r\n{}{}", TWO, FOUND_TWO).into_bytes()));
        } else {
            steps.extend((0..20).map(|_| Step::Silence(100)));
        }
        let (listener, _ended) = played(steps);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write: None };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyUSB0".into(),
            probe: None,
            wipe: false,
            keep_fsw: true,
            dump_image: DUMP.to_vec(),
            dump_from: String::new(),
            prepare: None,
            known: None,
            uid: None,
            name: None,
            check_console: true,
        };
        job.shared.lock().unwrap().running = true;
        // longer than the gap between the check's boot line and the dump's printing
        pull(&job, &desk, listener, &Limits { quiet_ms: 2_000, read_ms: 5_000, ask_ms: 3_000, erase_ms: 3_000, boot_ms: 600 });
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        (state, did, dir)
    }

    #[test]
    fn a_console_that_says_the_board_started_is_pulled() {
        let (p, did, dir) = pulled_checking("console-ok", Some("\u{fffd}I BOOT 0 c0: start log=4\r\nI GPS 3 c0: start\r\n"));
        assert_eq!(did, vec!["reset", "save", "write dump firmware", "reset", "write flight software", "reset"]);
        assert_eq!(p.error, None, "{:?}", p.note);
        assert!(p.restored);
        assert!(p.steps.iter().any(|s| s == "checking that /dev/ttyUSB0 is this board's console"), "{:?}", p.steps);
        let log = std::fs::read_to_string(PathBuf::from(p.dir.unwrap()).join("ground.log")).unwrap();
        assert_eq!(log.matches("PKT,55,").count(), 2, "the boot line from the check is not part of the dump");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_console_that_says_nothing_is_refused_before_anything_is_written() {
        let (p, did, dir) = pulled_checking("console-silent", None);
        assert_eq!(did, vec!["reset"], "found out before the minute the save takes, and nothing written");
        let e = p.error.unwrap();
        assert!(e.starts_with("/dev/ttyUSB0 is not this board's console") && e.contains("Nothing was written"), "{}", e);
        assert!(!p.restored, "nothing was written, so nothing is put back");
        assert_eq!(p.saved_fsw, None);
        assert!(p.note.starts_with("failed; what arrived is in "), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_boot_line_that_comes_after_other_talk_is_some_other_boards() {
        // a board boot-looping on this port prints a boot line sooner or later too
        let chatter = "I TX 1 c1: send\r\nI TX 2 c1: send\r\nI TX 3 c1: send\r\nI TX 4 c1: send\r\nI TX 5 c1: send\r\nI BOOT 0 c0: start log=4\r\n";
        let (p, did, dir) = pulled_checking("console-other", Some(chatter));
        assert_eq!(did, vec!["reset"]);
        assert!(p.error.unwrap().contains("is not this board's console"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_already_on_the_dump_firmware_is_not_saved_as_its_flight_software() {
        // an earlier pull left every board like this on 2026-10-18
        let (p, did, dir) = pulled_checking("console-ondump", Some("I BOOT 0 c0: start test=flashdump\r\nI FLASH 0 c0: start\r\n"));
        assert_eq!(did, vec!["reset", "write dump firmware", "reset"], "nothing saved, so nothing put back as if it were flight software");
        assert_eq!(p.error, None);
        assert_eq!(p.saved_fsw, None);
        assert!(!p.restored);
        assert!(p.steps.iter().any(|s| s.contains("already running the dump firmware")), "{:?}", p.steps);
        assert!(p.note.contains("the board is still running the dump firmware"), "{}", p.note);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
