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
use std::io::Read;
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
}

impl Pull {
    fn idle() -> Self {
        Pull {
            running: false,
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
        "{{\"ok\":true,\"running\":{},\"keepFsw\":{},\"board\":\"{}\",\"seconds\":{},\"pct\":{},\"note\":\"{}\",\"steps\":[{}],\"savedFsw\":{},\"log\":{},\"dir\":{},\"raw\":{},\"report\":{},\"restored\":{},\"error\":{}}}",
        p.running,
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
}

impl Listener {
    fn start(port_name: &str) -> Result<Listener, String> {
        let port = serialport::new(port_name, BAUD)
            .timeout(std::time::Duration::from_millis(200))
            .open()
            .map_err(|e| format!("{}: {}", port_name, e))?;
        Ok(Listener::on(port))
    }

    fn on<R: Read + Send + 'static>(mut port: R) -> Listener {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let last_ms = Arc::new(Mutex::new(crate::logfile::now_ms()));
        let gap_ms = Arc::new(AtomicU64::new(0));
        let (h2, s2, l2, g2) = (heard.clone(), stop.clone(), last_ms.clone(), gap_ms.clone());
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
                    Err(_) => break,   // unplugged, or the probe reset it out from under us
                }
            }
        });
        Listener { heard, stop, last_ms, gap_ms }
    }

    fn last_ms(&self) -> u128 {
        *self.last_ms.lock().unwrap()
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

    fn done(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// Whether a line starting "found" has arrived whole at or after `from`, and where to look
// from next time, which is the start of the line still arriving. Whole matters: the first
// half of that line reads as a smaller count. Only the new bytes are looked at each time,
// because this runs with the buffer locked and the reader waits for it.
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

// descent-pull-2026-09-25T19-30-00-ttyACM1
fn folder_name(hint: &str, at: SystemTime) -> String {
    format!("descent-pull-{}-{}", crate::stm32::stamp(at), crate::stm32::tidy(hint))
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
    pub keep_fsw: bool,
    pub dump_image: &'static [u8],
    // The board this pull is meant for, when the page knows, and what the operator calls
    // it. With neither, the port is all there is to name things by.
    pub uid: Option<String>,
    pub name: Option<String>,
}

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
}

fn put_back(bench: &dyn Bench, image: &Path) -> Result<(), String> {
    let bytes = std::fs::read(image).map_err(|e| format!("cannot read back {}: {}", image.display(), e))?;
    bench.write(&bytes)?;
    bench.reset()
}

fn run(job: Job) {
    // Listening first. The board is talking already, and a port nobody reads fills up.
    say(&job.shared, &job.broadcast, 2, "listening to the board");
    match Listener::start(&job.port_name) {
        Ok(listener) => {
            let bench = StLink { probe: job.probe.as_deref(), broadcast: &job.broadcast };
            pull(&job, &bench, listener, &Limits { quiet_ms: QUIET_MS, read_ms: READ_LIMIT_MS });
        }
        Err(e) => finish(&job.shared, &job.broadcast, None, Some(e), Board::Untouched, None),
    }
}

fn pull(job: &Job, bench: &dyn Bench, listener: Listener, limits: &Limits) {
    let Job { shared, broadcast, dir, port_name, keep_fsw, dump_image, uid, name: board_name, .. } = job;
    let keep_fsw = *keep_fsw;

    let name = folder_name(board_name.as_deref().unwrap_or(port_name), SystemTime::now());
    // In full, because this is what the operator is shown to go and find it by.
    let folder = std::path::absolute(dir.join(&name)).unwrap_or_else(|_| dir.join(&name));
    if let Err(e) = std::fs::create_dir_all(&folder) {
        listener.done();
        let e = format!("could not make {}: {}", folder.display(), e);
        return finish(shared, broadcast, None, Some(e), Board::Untouched, None);
    }
    shared.lock().unwrap().dir = Some(folder.display().to_string());

    let mut saved: Option<PathBuf> = None;
    let mut wrote_dump = false;
    let mut reset_at = 0usize;   // how much the port had carried when the board was reset
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
        if keep_fsw {
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

        say(shared, broadcast, 30, "writing the dump firmware");
        wrote_dump = true;
        bench.write(dump_image)?;
        // Whatever the port carried before the reset is not this printing, even if it
        // holds a found line: a board already on the dump firmware has printed one.
        let mut at = listener.len();
        let before = at;
        reset_at = before;
        bench.reset()?;

        say(shared, broadcast, 45, "reading the log off the chip");
        let reset_ms = crate::logfile::now_ms();
        loop {
            let (whole, next) = listener.found_line_after(at);
            at = next;
            if whole {
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
    // getting it again means doing the whole pull again.
    let raw = folder.join("dump.txt");
    match std::fs::write(&raw, &heard) {
        Ok(()) => shared.lock().unwrap().raw = Some(format!("{}/dump.txt", name)),
        Err(e) => also(&mut error, format!("could not write {}: {}", raw.display(), e)),
    }

    let mut read_as: Option<String> = None;
    if error.is_none() {
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
            error = Some(format!(
                "nothing that reads as a log came off the board; the {} bytes it did send are in {}",
                heard.len(),
                raw.display()
            ));
        } else {
            say(shared, broadcast, 80, &format!("read {} records from {} boots", parsed.records.len(), parsed.boots().len()));
            let report = crate::dump::report(&parsed, heard.len(), listener.longest_gap_ms());
            let json = report.json();
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

    // Last, and whatever went wrong above: a board left on the dump firmware does not fly.
    let mut board = if wrote_dump { Board::OnDump } else { Board::Untouched };
    if let Some(image) = &saved {
        say(shared, broadcast, 90, "putting the flight software back");
        match put_back(bench, image) {
            Ok(()) => {
                shared.lock().unwrap().restored = true;
                board = Board::Back;
            }
            Err(e) => {
                board = Board::NotBack;
                also(
                    &mut error,
                    format!(
                        "the flight software was NOT put back ({}). The image that was on the board is {} and has to be written back by hand",
                        e,
                        image.display()
                    ),
                );
            }
        }
    }
    finish(shared, broadcast, read_as, error, board, Some(&folder));
}

fn finish(
    shared: &Shared,
    broadcast: &crate::ws::Broadcast,
    read_as: Option<String>,
    error: Option<String>,
    board: Board,
    folder: Option<&Path>,
) {
    let mut p = shared.lock().unwrap();
    p.running = false;
    p.finished_ms = crate::logfile::now_ms();
    if error.is_none() {
        p.pct = 100;
    }
    p.note = closing_note(read_as.as_deref(), error.is_some(), board, folder);
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
        assert_eq!(folder_name("/dev/ttyACM1", at), "descent-pull-2026-09-21T14-13-20-ttyACM1");
        assert_eq!(folder_name("COM3", at), "descent-pull-2026-09-21T14-13-20-COM3");
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
        let (listener, _ended) = played(steps);
        let desk = Desk { did: Mutex::new(Vec::new()), named: Mutex::new(Vec::new()), go: Mutex::new(Some(go)), fail_write };
        let job = Job {
            shared: shared(),
            broadcast: crate::ws::Broadcast::new(),
            dir: dir.clone(),
            port_name: "/dev/ttyACM1".into(),
            probe: None,
            keep_fsw,
            dump_image: DUMP,
            uid: None,
            name: None,
        };
        job.shared.lock().unwrap().running = true;
        job.shared.lock().unwrap().keep_fsw = keep_fsw;
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000 });
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        (state, did, dir)
    }

    const TWO: &str = "record 0 boot 0 ms 2517\r\n\
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36\r\n\
record 1 boot 0 ms 2567\r\n\
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2B 2C 2D 2E 2F 30 31 32 33 34 35 36\r\n";
    const FOUND_TWO: &str = "found 2 records, 2 good (0 time anchors), 0 bad, from 1 boots, log ends at address 0x000080\r\n";

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
        assert_eq!(folder.parent().unwrap(), dir);
        let name = folder.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("descent-pull-") && name.ends_with("-ttyACM1"), "{}", name);
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
            keep_fsw: true,
            dump_image: DUMP,
            uid: Some(uid.into()),
            name: name.map(String::from),
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000 });
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
        assert!(name.starts_with("descent-pull-") && name.ends_with("-Flight-spare-2"), "{}", name);
        assert_eq!(named.len(), 1);
        assert!(named[0].starts_with("descent-image-20393256-Flight-spare-2-") && named[0].ends_with(".bin"), "{}", named[0]);
        let _ = std::fs::remove_dir_all(&dir);

        // A board the list has no name for keeps the port in the folder's name, and is
        // still told apart by its id in the image's.
        let (p, _, named, dir) = pulled_for("unnamed", ON_THE_PROBE, None);
        let folder = PathBuf::from(p.dir.unwrap());
        assert!(folder.file_name().unwrap().to_string_lossy().ends_with("-ttyACM1"), "{}", folder.display());
        assert!(named[0].starts_with("descent-image-20393256-board-"), "{}", named[0]);
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
            keep_fsw: true,
            dump_image: DUMP,
            uid: None,
            name: None,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000 });
        assert!(!desk.did.lock().unwrap().contains(&"uid".to_string()));
        let named = desk.named.lock().unwrap().clone();
        assert!(named[0].starts_with("descent-ground-fsw-ttyACM1-") && named[0].ends_with(".bin"), "{}", named[0]);
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
            keep_fsw: false,
            dump_image: DUMP,
            uid: None,
            name: None,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 400, read_ms: 5_000 });
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
            keep_fsw: false,
            dump_image: DUMP,
            uid: None,
            name: None,
        };
        pull(&job, &desk, listener, &Limits { quiet_ms: 2_000, read_ms: 5_000 });
        let p = job.shared.lock().unwrap().clone();
        let log = std::fs::read_to_string(PathBuf::from(p.dir.unwrap()).join("ground.log")).unwrap();
        assert!(log.contains("flash-boot4"), "the printing after the reset is the one taken: {}", log);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
