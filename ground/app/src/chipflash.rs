// Saying which ChipSat is on the probe, and writing an image to one.
//
// A flash is a job, like a pull: it runs on its own thread, keeps what it has done where
// a page that was reloaded can ask for it, and one of them has the probe at a time. What
// it must not do is write the wrong board. The clip moves from board to board and the
// page still shows the last one it read, so the board is asked for its id again before
// anything is written, and once more when it is done.
use crate::chips;
use crate::json::esc;
use crate::settings::{self, Found};
use sha2::{Digest, Sha256};
use crate::stm32::{self, Doing, Said, Seen};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone)]
pub struct State {
    pub running: bool,
    pub what: &'static str,   // "flash", "read", or "" before the first job
    pub uid: String,          // the board a flash was meant for, or the one a read found
    pub pct: u32,
    pub note: String,
    pub steps: Vec<String>,
    pub error: Option<String>,
    pub saved: Option<String>,   // the image saved off the board, as /api/log takes it
    pub read: Option<String>,    // what a flash found when it read the board again, as /api/chip/read answers
    pub started_ms: u128,
    pub finished_ms: u128,
}

impl State {
    fn idle() -> State {
        State {
            running: false,
            what: "",
            uid: String::new(),
            pct: 0,
            note: String::new(),
            steps: Vec::new(),
            error: None,
            saved: None,
            read: None,
            started_ms: 0,
            finished_ms: 0,
        }
    }
}

pub type Shared = Arc<Mutex<State>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(State::idle()))
}

fn quoted(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("\"{}\"", esc(s)),
        None => String::from("null"),
    }
}

pub fn state_json(s: &State, now_ms: u128) -> String {
    let secs = if s.started_ms == 0 {
        0
    } else {
        let end = if s.running { now_ms } else { s.finished_ms };
        end.saturating_sub(s.started_ms) / 1000
    };
    let steps: Vec<String> = s.steps.iter().map(|l| format!("\"{}\"", esc(l))).collect();
    format!(
        "{{\"ok\":true,\"running\":{},\"what\":\"{}\",\"uid\":\"{}\",\"pct\":{},\"note\":\"{}\",\"steps\":[{}],\"error\":{},\"saved\":{},\"read\":{},\"seconds\":{}}}",
        s.running,
        s.what,
        esc(&s.uid),
        s.pct,
        esc(&s.note),
        steps.join(","),
        quoted(&s.error),
        quoted(&s.saved),
        s.read.as_deref().unwrap_or("null"),
        secs,
    )
}

fn begin(shared: &Shared, what: &'static str, uid: &str) {
    let mut s = shared.lock().unwrap();
    *s = State::idle();
    s.running = true;
    s.what = what;
    s.uid = uid.to_string();
    s.started_ms = crate::logfile::now_ms();
}

fn step(shared: &Shared, pct: u32, text: &str) {
    let mut s = shared.lock().unwrap();
    s.pct = s.pct.max(pct);
    s.note = text.to_string();
    s.steps.push(text.to_string());
}

fn finish(shared: &Shared, note: String, error: Option<String>) {
    let mut s = shared.lock().unwrap();
    s.running = false;
    s.finished_ms = crate::logfile::now_ms();
    if error.is_none() {
        s.pct = 100;
    }
    s.note = note;
    s.error = error;
}

// What the board steps report, kept where /api/chip/job finds it: a new kind of work is
// a new step, and more of the same only moves the bar. The bar is the whole job's and
// never goes back, so putting an image back after a failed write leaves it where it got to.
fn listener(shared: &Shared, writing: String) -> impl Fn(Doing, u64, u64) + '_ {
    move |doing, done, total| {
        let (words, from, to) = match doing {
            Doing::Saving => ("saving what is on the board", 5, 40),
            Doing::Erasing => ("erasing the flash", 40, 42),
            Doing::Writing => (writing.as_str(), 42, 70),
            Doing::Checking => ("checking what was written", 70, 90),
        };
        let mut s = shared.lock().unwrap();
        if s.steps.last().map(String::as_str) != Some(words) {
            s.steps.push(words.to_string());
        }
        s.pct = s.pct.max(from + (to - from) * stm32::pct_of(done, total) / 100);
        s.note = if total == 0 { words.to_string() } else { format!("{}, {} of {} kB", words, done / 1024, total / 1024) };
    }
}

// The list is a convenience and the board is not: a list that cannot be written is a
// line in the steps, and the job goes on.
fn remember(shared: &Shared, registry: &chips::Shared, uid: &str, firmware: &'static str) {
    let now_s = (crate::logfile::now_ms() / 1000) as u64;
    if let Err(e) = registry.lock().unwrap().saw(uid, firmware, now_s) {
        shared.lock().unwrap().steps.push(e);
    }
}

// What is done to the board, behind a trait so the order of it and what happens when a
// step fails can be run with no board on the desk.
pub trait Bench {
    fn look(&self) -> Result<Seen, String>;
    fn save(&self, dir: &Path, file: &str, said: Said) -> Result<PathBuf, String>;
    // Writes and reads back.
    fn write(&self, image: &[u8], said: Said) -> Result<(), String>;
    fn reset(&self) -> Result<(), String>;
}

pub struct StLink<'a> {
    pub probe: Option<&'a str>,
}

impl Bench for StLink<'_> {
    fn look(&self) -> Result<Seen, String> {
        stm32::look(self.probe)
    }
    fn save(&self, dir: &Path, file: &str, said: Said) -> Result<PathBuf, String> {
        stm32::save_firmware(self.probe, dir, file, said)
    }
    fn write(&self, image: &[u8], said: Said) -> Result<(), String> {
        stm32::write_firmware(self.probe, image, said)
    }
    fn reset(&self) -> Result<(), String> {
        stm32::reset_and_run(self.probe)
    }
}

// What a board's firmware can be told against: the images this build carries, and the
// dump images earlier builds carried.
pub struct Known {
    pub dump: Option<&'static [u8]>,
    // Length and sha256 of each earlier dump image. A board left on one of those is on
    // dump firmware, and calling it "unknown" reads as "flight software is on it".
    pub older_dumps: &'static [(usize, &'static str)],
    pub flight: Vec<FlightImage>,
    pub bench: Vec<(&'static str, &'static [u8])>,
    pub built: Vec<crate::localfw::Identity>,   // everything built on this laptop (localfw.rs)
    // tree source hash while the app uses the tree, a dump built from it counts as current
    pub source_now: Option<String>,
}

pub struct FlightImage {
    pub module: &'static str,
    pub bytes: &'static [u8],
    pub id: String,                // as /api/health gives it
    // Where the image keeps its settings block. Those 32 bytes are rewritten on a board
    // that has been given settings, so they are no part of telling which image it is.
    pub block_at: Option<usize>,
}

// flash_dump.bin as it was: before 2026-10-04, when it printed record and time lines, and
// before 2026-10-13, when it could not be interrupted while printing.
const OLDER_DUMPS: &[(usize, &str)] = &[
    (31224, "4a5f0ead87429a51d9c84757c8be07058b58ed14ac885e25b9f48ca3f8c531a6"),
    (31008, "25118bdfafab456c2bbe94260bfbdbb333318f676d95c2b0599648f6cf789d41"),
];

impl Known {
    pub fn carried() -> Known {
        Known {
            dump: crate::flash::CHIPSAT_DUMP,
            older_dumps: OLDER_DUMPS,
            flight: crate::flight::carried()
                .usable()
                .into_iter()
                .map(|i| FlightImage {
                    module: i.module,
                    bytes: i.bytes,
                    id: i.id().to_string(),
                    block_at: settings::locate(i.bytes).ok().flatten(),
                })
                .collect(),
            bench: bench_images(),
            built: Vec::new(),
            source_now: None,
        }
    }

    pub fn with(local: &crate::localfw::Local) -> Known {
        Known {
            built: local.registered(),
            source_now: local.origin().map(|o| o.source),
            ..Known::carried()
        }
    }
}

// leaves out a module the app was built without
pub fn bench_images() -> Vec<(&'static str, &'static [u8])> {
    [("le", crate::flash::CHIPSAT_BENCH_LE), ("hp", crate::flash::CHIPSAT_BENCH_HP)]
        .into_iter()
        .filter_map(|(m, b)| b.filter(|b| !b.is_empty()).map(|b| (m, b)))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Firmware {
    pub kind: &'static str,             // "dump", "flight", "bench" or "unknown"
    pub current: bool,                  // for dump: it is the image this build carries
    pub module: Option<&'static str>,   // for flight and bench: which module the image is built for
    pub id: Option<String>,
    pub built: Option<crate::localfw::Identity>,
}

impl Firmware {
    fn unknown() -> Firmware {
        Firmware { kind: "unknown", current: false, module: None, id: None, built: None }
    }

    fn built_json(&self) -> String {
        match &self.built {
            Some(b) => format!(",\"built\":{}", b.json((crate::logfile::now_ms() / 1000) as u64)),
            None => String::new(),
        }
    }

    // As /api/chip/read gives it under "firmware".
    pub fn json(&self) -> String {
        match (self.kind, self.module) {
            ("dump", _) => format!(
                "{{\"kind\":\"dump\",\"current\":{},\"id\":\"{}\"{}}}",
                self.current,
                esc(self.id.as_deref().unwrap_or("")),
                self.built_json()
            ),
            ("flight", Some(module)) | ("bench", Some(module)) => format!(
                "{{\"kind\":\"{}\",\"module\":\"{}\",\"id\":\"{}\"{}}}",
                self.kind,
                module,
                esc(self.id.as_deref().unwrap_or("")),
                self.built_json()
            ),
            _ => String::from("{\"kind\":\"unknown\"}"),
        }
    }

    // As the list of boards keeps it, see chips::firmware_words.
    pub fn word(&self) -> &'static str {
        match (self.kind, self.module) {
            ("dump", _) => "dump",
            ("flight", Some("le")) => "flight-le",
            ("flight", Some("hp")) => "flight-hp",
            ("bench", Some("le")) => "bench-le",
            ("bench", Some("hp")) => "bench-hp",
            _ => "unknown",
        }
    }
}

// Whether the flash starts with this flight image, the settings block aside. Every other
// byte has to match: the two modules' images differ outside their blocks in one byte, the
// module the firmware falls back to, and that byte is what tells them apart on a board
// whose block has been rewritten.
fn is_flight(flash: &[u8], f: &FlightImage) -> bool {
    let n = f.bytes.len();
    if n == 0 || flash.len() < n {
        return false;
    }
    match f.block_at {
        Some(at) if at + settings::SIZE <= n => {
            flash[..at] == f.bytes[..at] && flash[at + settings::SIZE..n] == f.bytes[at + settings::SIZE..]
        }
        _ => flash[..n] == *f.bytes,
    }
}

// What the start of a board's flash is. Only ever one of the images in `known`, byte for byte
// apart from a flight image's settings. A flight image has no mark of its own, so one neither
// carried nor built here is "unknown", not "flight". Built here wins even if the app carries
// the same bytes
pub fn recognise(image: &[u8], known: &Known) -> Firmware {
    let carried = recognise_carried(image, known);
    // newest match, that's the build someone remembers
    let Some(b) = known.built.iter().rev().find(|b| b.matches(image)) else { return carried };
    let Some(p) = crate::localfw::part(&b.part) else { return carried };
    if carried.kind != "unknown" && carried.kind != p.kind {
        return carried;
    }
    let current = match p.kind {
        "dump" => carried.current || known.source_now.as_deref() == Some(b.origin.source.as_str()),
        "bench" => true,
        _ => false,
    };
    Firmware {
        kind: p.kind,
        current,
        module: p.module.or(carried.module),
        id: carried.id.clone().or_else(|| Some(b.id().to_string())),
        built: Some(b.clone()),
    }
}

fn recognise_carried(image: &[u8], known: &Known) -> Firmware {
    let sha256 = |bytes: &[u8]| -> String { Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect() };
    // An image is named by the start of its sha256, the way /api/health names the ones carried.
    let id = |hex: &str| Some(hex.chars().take(16).collect::<String>().to_ascii_lowercase());
    if let Some(carried) = known.dump.filter(|d| !d.is_empty() && image.starts_with(d)) {
        return Firmware { kind: "dump", current: true, id: id(&sha256(carried)), ..Firmware::unknown() };
    }
    // Earlier dump images are not carried, only their hashes, so the same number of
    // bytes off the board is hashed and held against each.
    let older = known
        .older_dumps
        .iter()
        .find(|(len, hash)| *len > 0 && image.len() >= *len && sha256(&image[..*len]).eq_ignore_ascii_case(hash));
    if let Some((_, hash)) = older {
        return Firmware { kind: "dump", current: false, id: id(hash), ..Firmware::unknown() };
    }
    // no settings block, so exact bytes
    if let Some((module, bytes)) = known.bench.iter().find(|(_, b)| !b.is_empty() && image.starts_with(b)) {
        return Firmware { kind: "bench", current: true, module: Some(*module), id: id(&sha256(bytes)), built: None };
    }
    match known.flight.iter().find(|f| is_flight(image, f)) {
        Some(f) => Firmware { kind: "flight", current: false, module: Some(f.module), id: Some(f.id.clone()), built: None },
        None => Firmware::unknown(),
    }
}

// Whether an image's name says it came off some other board. `ours` is an image picked
// from the data folder, where a name with no id in it means an older save and is worth a
// line; a file the operator chose from anywhere else is just a file.
pub fn came_off(name: &str, board: &str, force: bool, ours: bool) -> Result<Option<String>, String> {
    let on_probe = stm32::uid_tail(board);
    match stm32::uid_in_name(name) {
        Some(tail) if tail.eq_ignore_ascii_case(on_probe) => Ok(None),
        Some(tail) if force => Ok(Some(format!(
            "{} was saved off the board whose id ends {}, not this one ({}). Written anyway, because force was set",
            name, tail, on_probe
        ))),
        Some(tail) => Err(format!(
            "{} was saved off the board whose id ends {}, and this board's ends {}. One board's image is not written to another; force=1 overrides",
            name, tail, on_probe
        )),
        None if ours => Ok(Some(format!(
            "{} does not say which board it came off, so that was not checked",
            name
        ))),
        None => Ok(None),
    }
}

// Which board is on the probe and what is on it. Answers as /api/chip/read does.
pub fn read(shared: &Shared, registry: &chips::Shared, bench: &dyn Bench, known: &Known) -> String {
    begin(shared, "read", "");
    step(shared, 10, "reading the board on the probe");
    let seen = match bench.look() {
        Ok(seen) => seen,
        Err(e) => {
            finish(shared, String::from("failed"), Some(e.clone()));
            return format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e));
        }
    };
    shared.lock().unwrap().uid = seen.uid.clone();
    if seen.was_reset {
        step(shared, 90, "the board did not answer until it was reset, so it has been restarted");
    }
    let firmware = recognise(&seen.image, known);
    remember(shared, registry, &seen.uid, firmware.word());
    let (answer, trouble) = read_json(&seen, &firmware);
    if let Some(words) = &trouble {
        step(shared, 90, words);
    }
    finish(shared, format!("board {}, {} firmware", seen.uid, firmware.kind), None);
    answer
}

// What was found on a board, as /api/chip/read answers, and what is wrong with its
// settings block when something is.
fn read_json(seen: &Seen, firmware: &Firmware) -> (String, Option<String>) {
    // The dump image is smaller than a flight image, and writing it leaves the end of the
    // flight image where it was. A block found under it is not one the board is using.
    let settings = if firmware.kind == "dump" { Ok((Found::Nothing, None)) } else { settings::find_at(&seen.image) };
    let trouble = match &settings {
        Ok((Found::Layout(v), _)) => Some(format!(
            "the settings block on the board is layout {}, which this build does not know, so it was not read",
            v
        )),
        Ok((Found::Damaged, _)) => Some(String::from(settings::DAMAGED)),
        Ok(_) => None,
        Err(e) => Some(e.clone()),
    };
    let block_at = match &settings {
        Ok((_, Some(at))) => at.to_string(),
        _ => String::from("null"),
    };
    let settings = settings.map(|(found, _)| found.json()).unwrap_or_else(|_| String::from("null"));
    let json = format!(
        "{{\"ok\":true,\"uid\":\"{}\",\"flashBytes\":{},\"firmware\":{},\"settings\":{},\"settingsError\":{},\"settingsBlockAt\":{},\"probe\":{{\"serial\":\"{}\",\"kind\":\"{}\"}}}}",
        esc(&seen.uid),
        seen.flash_bytes,
        firmware.json(),
        settings,
        quoted(&trouble),
        block_at,
        esc(&seen.probe.serial),
        esc(&seen.probe.kind),
    );
    (json, trouble)
}

pub struct Job {
    pub shared: Shared,
    pub registry: chips::Shared,
    pub dir: PathBuf,
    pub probe: Option<String>,
    pub uid: String,
    pub backup: bool,
    pub image: Vec<u8>,
    pub label: String,          // what the steps call the image
    pub note: Option<String>,   // something about the image worth a line before it is written
    pub known: Known,
    // Set when the image is to be compiled for this board rather than carried. The build is a step
    // of the job and not part of the request, because it takes about 13 s: a request that blocked
    // for that long would look to whoever pressed the button like nothing had happened.
    pub build: Option<BuildPlan>,
}

// gets the image from the tree (localfw.rs), returns it plus where it came from in words
pub type Make = Box<dyn FnOnce(&dyn Fn(&str)) -> Result<(Vec<u8>, String), String> + Send>;

pub struct BuildPlan {
    pub make: Make,
    // None for the bench, it has no block
    pub asked: Option<settings::Asked>,
    pub words: String,   // what is being got, for the steps
}

// The caller holds the probe's gate, and hands it over so it is let go when the job ends.
pub fn start(job: Job, held: stm32::Held) {
    // Before the thread, so a page that asks straight after the POST sees it running.
    begin(&job.shared, "flash", &job.uid);
    let mut job = job;
    std::thread::spawn(move || {
        if let Some(plan) = job.build.take() {
            let shared = job.shared.clone();
            // Built for the module, then given the board's settings the same way the carried
            // image is: by rewriting the block. One writer, so what a damaged block falls
            // back to is the same whichever way the image came.
            let asked = plan.asked;
            let built = (plan.make)(&|said: &str| step(&shared, 2, said)).and_then(|(bytes, from)| match &asked {
                Some(asked) => {
                    let (at, _) = settings::writable(&bytes)?;
                    Ok((settings::patch(&bytes, at, asked)?, from))
                }
                None => Ok((bytes, from)),
            });
            match built {
                Ok((bytes, from)) => {
                    step(&job.shared, 5, &format!("{}, {} kB", from, bytes.len().div_ceil(1024)));
                    job.image = bytes;
                    job.label = plan.words;
                }
                Err(e) => {
                    finish(&job.shared, String::from("the image was not built, so nothing was written to the board"), Some(e));
                    drop(held);
                    return;
                }
            }
        }
        let bench = StLink { probe: job.probe.as_deref() };
        flash(&job, &bench);
        drop(held);
    });
}

// What is on the board when a flash ends.
#[derive(Clone, Copy)]
enum Board {
    Untouched,
    New,
    Back,
    NotBack,
    Unknown,   // the write failed and nothing had been saved to go back to
}

fn put_back(bench: &dyn Bench, image: &Path, said: Said) -> Result<(), String> {
    let bytes = std::fs::read(image).map_err(|e| format!("cannot read back {}: {}", image.display(), e))?;
    bench.write(&bytes, said)?;
    bench.reset()
}

// The shared state and the list of boards, which every step of a job reports to.
#[derive(Clone, Copy)]
struct Told<'a> {
    shared: &'a Shared,
    registry: &'a chips::Shared,
}

impl Job {
    fn shared(&self) -> Told<'_> {
        Told { shared: &self.shared, registry: &self.registry }
    }
}

// Writes an image and reads it back. When that fails, what was saved off the board
// first goes back on, and the error says whether it did and where the saved file is.
fn write_or_put_back(
    told: Told,
    bench: &dyn Bench,
    uid: &str,
    image: &[u8],
    label: &str,
    saved: Option<&Path>,
    board: &mut Board,
) -> Result<(), String> {
    let Told { shared, registry } = told;
    let Err(e) = bench.write(image, &listener(shared, format!("writing {}", label))) else { return Ok(()) };
    // A board with half an image on it is worse than one on its old image, so
    // what was saved goes back, the way a pull always puts it back. Written back,
    // not read back and accepted: a write cut short can leave a word that reads
    // correctly today and fails its ECC later, and on this chip that is an NMI
    // the firmware does not handle. Not verified on this hardware, so do not
    // trade the rewrite for a faster compare.
    Err(match saved {
        Some(path) => {
            step(shared, 0, "putting back what was on the board");
            match put_back(bench, path, &listener(shared, String::from("writing what was saved"))) {
                Ok(()) => {
                    *board = Board::Back;
                    format!("{}. What was on the board has been put back; it is also in {}", e, path.display())
                }
                Err(again) => {
                    *board = Board::NotBack;
                    remember(shared, registry, uid, "unknown");
                    format!(
                        "{}. What was on the board could NOT be put back ({}). It is in {} and has to be written back by hand",
                        e,
                        again,
                        path.display()
                    )
                }
            }
        }
        None => {
            *board = Board::Unknown;
            remember(shared, registry, uid, "unknown");
            format!("{}. Nothing was saved first, so the board may hold part of {} and nothing that runs", e, label)
        }
    })
}

// What a job that failed says last, by what it left on the board.
fn failed_note(board: Board) -> String {
    String::from(match board {
        Board::Untouched => "failed, nothing was written",
        Board::New => "failed after the image was written",
        Board::Back => "failed, what was on the board has been put back",
        Board::NotBack => "failed, and what was on the board was NOT put back",
        Board::Unknown => "failed, and the board may hold part of the image",
    })
}

// Whatever went wrong after the save, the operator is told where the saved image is.
fn with_saved(e: String, saved: &Option<PathBuf>) -> String {
    match saved {
        Some(path) if !e.contains(&path.display().to_string()) => {
            format!("{}. What was on the board before is in {}", e, path.display())
        }
        _ => e,
    }
}

// The block in what was written against the block read off the started board. An image
// with no block (dump, bench) has nothing to compare, which is the false.
fn settings_came_back(written: &[u8], on_board: &[u8]) -> Result<bool, String> {
    let Ok((Found::Block(wanted), _)) = settings::find_at(written) else { return Ok(false) };
    match settings::find_at(on_board) {
        Ok((Found::Block(there), _)) => match settings::differences(&wanted, &there).join(", ") {
            off if off.is_empty() => Ok(true),
            off => Err(format!("the image is written, but the board's settings differ from what was written: {}", off)),
        },
        _ => Err(String::from("the image is written, but its settings block could not be read back off the board")),
    }
}

fn flash(job: &Job, bench: &dyn Bench) {
    let Job { shared, registry, dir, uid, image, label, .. } = job;
    let mut saved: Option<PathBuf> = None;
    let mut board = Board::Untouched;

    let done = (|| -> Result<(), String> {
        step(shared, 1, "checking which board is on the probe");
        let seen = bench.look()?;
        remember(shared, registry, &seen.uid, recognise(&seen.image, &job.known).word());
        if &seen.uid != uid {
            return Err(format!(
                "the board on the probe is {} and this was meant for {}: the clip may have moved. Nothing was written.",
                seen.uid, uid
            ));
        }
        if seen.was_reset {
            step(shared, 2, "the board did not answer until it was reset");
        }
        // Before the save, not after it: the save is most of a minute, and an image that
        // will not fit is known not to fit now.
        stm32::fits(image.len(), seen.flash_bytes).map_err(|e| format!("{}. Nothing was written.", e))?;
        if let Some(note) = &job.note {
            step(shared, 2, note);
        }

        // saving the dump firmware would replace the newest real image, the only one kept
        let on_dump = recognise(&seen.image, &job.known).kind == "dump";
        if job.backup && on_dump {
            step(shared, 5, "the board is running the dump firmware, so nothing is saved off it");
        }
        if job.backup && !on_dump {
            step(shared, 5, "saving what is on the board");
            let name = registry.lock().unwrap().get(uid).map(|c| c.name.clone()).unwrap_or_default();
            let now = SystemTime::now();
            let place = crate::layout::folder(dir, crate::layout::IMAGES, now)?;
            let free = crate::layout::unused(&place.dir, &crate::layout::hhmm(now), &stm32::image_rest(uid, &name), ".bin")?;
            let file = free.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let path = bench.save(&place.dir, &file, &listener(shared, String::new()))?;
            shared.lock().unwrap().saved = Some(place.rel_of(&file));
            step(shared, 40, &format!("saved {}", path.display()));
            saved = Some(path);
        }

        write_or_put_back(job.shared(), bench, uid, image, label, saved.as_deref(), &mut board)?;
        board = Board::New;
        // From here the image is on the board and has been read back, whatever else
        // goes wrong, so that is what the list says even if the board cannot be asked.
        remember(shared, registry, uid, recognise(image, &job.known).word());

        step(shared, 92, "starting the board");
        bench.reset().map_err(|e| {
            format!("{} is written and checked, but the board would not start ({}). Power it off and on", label, e)
        })?;

        step(shared, 95, "reading the board again");
        let again = bench.look().map_err(|e| {
            format!("{} is written and checked, but the board did not answer afterwards ({})", label, e)
        })?;
        let now = recognise(&again.image, &job.known);
        remember(shared, registry, &again.uid, now.word());
        if &again.uid != uid {
            return Err(format!(
                "{} was written to board {}, and the board on the probe is now {}: the clip moved while it was being written",
                label, uid, again.uid
            ));
        }
        // The page shows this in place of asking for a read that has just been done.
        shared.lock().unwrap().read = Some(read_json(&again, &now).0);
        if settings_came_back(image, &again.image)? {
            step(shared, 98, "the settings on the board match what was written");
        }
        Ok(())
    })();

    // only the newest save per board is kept
    if let Some(path) = &saved {
        let gone = crate::layout::keep_only_latest(dir, path);
        if !gone.is_empty() {
            step(shared, 99, &format!("removed {} older saved image{} of this board", gone.len(), if gone.len() == 1 { "" } else { "s" }));
        }
    }
    let error = done.err().map(|e| with_saved(e, &saved));
    let note = match &error {
        None => format!("done, {} is on the board", label),
        Some(_) => failed_note(board),
    };
    finish(shared, note, error);
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: &str = "0038002A5553500920393256";
    const OTHER: &str = "0038002A5553500920393301";
    const OLD: &[u8] = b"what was on the board";
    const NEW: &[u8] = b"the new image";
    const DUMP: &[u8] = b"dump firmware";

    // A board that is not there. It writes down what was done to it, holds whatever was
    // last written, and can be told which step to fail.
    struct Desk {
        did: Mutex<Vec<String>>,
        named: Mutex<Vec<String>>,
        on_probe: &'static str,
        on_board: Mutex<Vec<u8>>,
        was_reset: bool,
        fail_look: Option<usize>,
        fail_save: bool,
        fail_write: Option<usize>,
        fail_reset: bool,
        // What a write that reports success leaves on the board, when that is not
        // what it was given.
        lands_as: Option<fn(&[u8]) -> Vec<u8>>,
    }

    fn desk() -> Desk {
        Desk {
            did: Mutex::new(Vec::new()),
            named: Mutex::new(Vec::new()),
            on_probe: UID,
            on_board: Mutex::new(OLD.to_vec()),
            was_reset: false,
            fail_look: None,
            fail_save: false,
            fail_write: None,
            fail_reset: false,
            lands_as: None,
        }
    }

    impl Desk {
        fn nth(&self, what: &str) -> usize {
            self.did.lock().unwrap().iter().filter(|d| d.starts_with(what)).count()
        }
    }

    impl Bench for Desk {
        fn look(&self) -> Result<Seen, String> {
            let nth = self.nth("look");
            self.did.lock().unwrap().push("look".into());
            if self.fail_look == Some(nth) {
                return Err(String::from("no answer from the board"));
            }
            Ok(Seen {
                uid: self.on_probe.to_string(),
                flash_bytes: 256 * 1024,
                image: self.on_board.lock().unwrap().clone(),
                probe: stm32::Probe { serial: "002A00".into(), kind: "STLink V3".into(), access: true },
                was_reset: self.was_reset,
            })
        }
        fn save(&self, dir: &Path, file: &str, said: Said) -> Result<PathBuf, String> {
            self.did.lock().unwrap().push("save".into());
            self.named.lock().unwrap().push(file.to_string());
            if self.fail_save {
                return Err(String::from("the saved firmware is short: 100 of 262144 bytes. Nothing was erased."));
            }
            said(Doing::Saving, 131_072, 262_144);
            said(Doing::Saving, 262_144, 262_144);
            let path = dir.join(file);
            std::fs::write(&path, &*self.on_board.lock().unwrap()).unwrap();
            Ok(path)
        }
        fn write(&self, image: &[u8], said: Said) -> Result<(), String> {
            let nth = self.nth("write");
            self.did.lock().unwrap().push(format!("write {}", String::from_utf8_lossy(image)));
            said(Doing::Erasing, 0, 0);
            said(Doing::Writing, 8_192, 16_384);
            if self.fail_write == Some(nth) {
                *self.on_board.lock().unwrap() = b"half of something".to_vec();
                return Err(String::from("the probe fell off"));
            }
            said(Doing::Writing, 16_384, 16_384);
            *self.on_board.lock().unwrap() = match self.lands_as {
                Some(spoil) => spoil(image),
                None => image.to_vec(),
            };
            said(Doing::Checking, 16_384, 16_384);
            Ok(())
        }
        fn reset(&self) -> Result<(), String> {
            self.did.lock().unwrap().push("reset".into());
            if self.fail_reset {
                return Err(String::from("could not reset the board: no answer"));
            }
            Ok(())
        }
    }

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-chipflash-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn known() -> chips::Shared {
        let mut r = chips::Registry::empty();
        r.saw(UID, "unknown", 1).unwrap();
        r.set_name(UID, "Flight spare 2").unwrap();
        Arc::new(Mutex::new(r))
    }

    // One flash at a desk with no board, and what it left behind.
    fn flashed(what: &str, desk: &Desk, backup: bool, image: &[u8], note: Option<&str>) -> (State, Vec<String>, chips::Shared, PathBuf) {
        let dir = scratch(what);
        let job = Job {
            shared: shared(),
            registry: known(),
            dir: dir.clone(),
            probe: None,
            uid: UID.into(),
            backup,
            image: image.to_vec(),
            label: "new.bin".into(),
            note: note.map(String::from),
            known: only_dump(),
            build: None,
        };
        begin(&job.shared, "flash", UID);
        flash(&job, desk);
        let state = job.shared.lock().unwrap().clone();
        let did = desk.did.lock().unwrap().clone();
        (state, did, job.registry.clone(), dir)
    }

    #[test]
    fn a_flash_checks_the_board_saves_it_writes_starts_it_and_checks_again() {
        let desk = desk();
        let (s, did, registry, dir) = flashed("whole", &desk, true, NEW, None);
        assert_eq!(did, vec!["look", "save", "write the new image", "reset", "look"]);
        assert_eq!(s.error, None);
        assert!(!s.running);
        assert_eq!(s.pct, 100);
        assert_eq!(s.what, "flash");
        assert_eq!(s.uid, UID);
        assert_eq!(s.note, "done, new.bin is on the board");

        // The saved image is named for the board it came off, and is what was on it.
        // relative, as /api/log takes it
        let name = s.saved.clone().unwrap();
        let day = crate::layout::day(SystemTime::now());
        assert!(name.starts_with(&format!("{}/images/", day)) && name.ends_with("-Flight-spare-2-20393256.bin"), "{}", name);
        assert_eq!(desk.named.lock().unwrap().clone(), vec![name.rsplit('/').next().unwrap().to_string()]);
        assert_eq!(std::fs::read(dir.join(&name)).unwrap(), OLD);

        assert_eq!(
            s.steps,
            vec![
                "checking which board is on the probe".to_string(),
                "saving what is on the board".to_string(),
                format!("saved {}", dir.join(&name).display()),
                "erasing the flash".to_string(),
                "writing new.bin".to_string(),
                "checking what was written".to_string(),
                "starting the board".to_string(),
                "reading the board again".to_string(),
            ]
        );
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("unknown"));
        assert!(registry.lock().unwrap().get(UID).unwrap().last_seen > 1);

        // The read at the end is kept for the page, in the words a read answers with.
        assert_eq!(
            s.read.as_deref(),
            Some(format!(
                "{{\"ok\":true,\"uid\":\"{}\",\"flashBytes\":262144,\"firmware\":{},\"settings\":null,\"settingsError\":null,\"settingsBlockAt\":null,\"probe\":{{\"serial\":\"002A00\",\"kind\":\"STLink V3\"}}}}",
                UID,
                recognise(NEW, &only_dump()).json()
            ))
            .as_deref()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flash_told_not_to_save_first_does_not() {
        let desk = desk();
        let (s, did, registry, dir) = flashed("nosave", &desk, false, DUMP, None);
        assert_eq!(did, vec!["look", "write dump firmware", "reset", "look"]);
        assert_eq!(s.error, None);
        assert_eq!(s.saved, None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // What the board says it is running afterwards is what the list remembers.
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("dump"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_board_on_the_probe_is_refused_with_both_ids_and_nothing_is_done_to_it() {
        let mut desk = desk();
        desk.on_probe = OTHER;
        let (s, did, registry, dir) = flashed("moved", &desk, true, NEW, None);
        assert_eq!(did, vec!["look"]);
        let e = s.error.unwrap();
        assert!(e.contains(UID) && e.contains(OTHER), "{}", e);
        assert!(e.contains("the clip may have moved"), "{}", e);
        assert_eq!(s.note, "failed, nothing was written");
        assert!(s.pct < 100);
        assert_eq!(s.read, None);
        // The board that was there has still been seen.
        assert!(registry.lock().unwrap().get(OTHER).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_too_big_for_the_board_is_refused_before_anything_is_saved_or_erased() {
        let desk = desk();
        let (s, did, _, dir) = flashed("big", &desk, true, &vec![0u8; 256 * 1024 + 1], None);
        assert_eq!(did, vec!["look"]);
        let e = s.error.unwrap();
        assert!(e.contains("262145") && e.contains("262144"), "{}", e);
        assert_eq!(s.note, "failed, nothing was written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_fails_stops_the_flash_with_the_board_as_it_was() {
        let mut desk = desk();
        desk.fail_save = true;
        let (s, did, _, dir) = flashed("nosaved", &desk, true, NEW, None);
        assert_eq!(did, vec!["look", "save"]);
        assert!(s.error.unwrap().contains("short"));
        assert_eq!(s.saved, None);
        assert_eq!(s.note, "failed, nothing was written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_fails_after_the_save_puts_the_saved_image_back() {
        let mut desk = desk();
        desk.fail_write = Some(0);
        let (s, did, registry, dir) = flashed("back", &desk, true, NEW, None);
        assert_eq!(did, vec!["look", "save", "write the new image", "write what was on the board", "reset"]);
        assert_eq!(desk.on_board.lock().unwrap().as_slice(), OLD);
        let path = dir.join(s.saved.clone().unwrap());
        let e = s.error.unwrap();
        assert!(e.starts_with("the probe fell off"), "{}", e);
        assert!(e.contains("has been put back"), "{}", e);
        assert!(e.contains(&path.display().to_string()), "{}", e);
        assert_eq!(s.note, "failed, what was on the board has been put back");
        assert!(s.steps.contains(&"putting back what was on the board".to_string()), "{:?}", s.steps);
        assert!(s.steps.contains(&"writing what was saved".to_string()), "{:?}", s.steps);
        assert!(!s.running);
        // It is on what it was on, so the list still says so.
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("unknown"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_put_back_that_fails_too_says_where_the_saved_image_is() {
        let mut desk = desk();
        desk.fail_write = Some(0);
        desk.fail_reset = true;
        let (s, did, _, dir) = flashed("notback", &desk, true, NEW, None);
        assert_eq!(did, vec!["look", "save", "write the new image", "write what was on the board", "reset"]);
        let path = dir.join(s.saved.clone().unwrap());
        let e = s.error.unwrap();
        assert!(e.contains("NOT be put back (could not reset the board: no answer)"), "{}", e);
        assert!(e.contains(&path.display().to_string()), "{}", e);
        assert!(e.contains("by hand"), "{}", e);
        assert_eq!(s.note, "failed, and what was on the board was NOT put back");
        assert_eq!(std::fs::read(&path).unwrap(), OLD);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_fails_with_nothing_saved_says_the_board_may_be_half_written() {
        let mut desk = desk();
        desk.fail_write = Some(0);
        let (s, did, registry, dir) = flashed("nothing", &desk, false, DUMP, None);
        assert_eq!(did, vec!["look", "write dump firmware"]);
        let e = s.error.unwrap();
        assert!(e.starts_with("the probe fell off"), "{}", e);
        assert!(e.contains("Nothing was saved first"), "{}", e);
        assert_eq!(s.note, "failed, and the board may hold part of the image");
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("unknown"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_that_will_not_start_after_a_good_write_keeps_the_new_image() {
        let mut desk = desk();
        desk.fail_reset = true;
        let (s, did, registry, dir) = flashed("noreset", &desk, true, DUMP, None);
        // Nothing is put back: what was asked for is on the board and has been checked.
        assert_eq!(did, vec!["look", "save", "write dump firmware", "reset"]);
        let path = dir.join(s.saved.clone().unwrap());
        let e = s.error.unwrap();
        assert!(e.contains("written and checked"), "{}", e);
        assert!(e.contains("could not reset the board: no answer"), "{}", e);
        assert!(e.contains(&path.display().to_string()), "{}", e);
        assert_eq!(s.note, "failed after the image was written");
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("dump"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_that_does_not_answer_afterwards_is_an_error_that_names_the_saved_image() {
        let mut desk = desk();
        desk.fail_look = Some(1);
        let (s, did, registry, dir) = flashed("gone", &desk, true, DUMP, None);
        assert_eq!(did, vec!["look", "save", "write dump firmware", "reset", "look"]);
        let path = dir.join(s.saved.clone().unwrap());
        let e = s.error.unwrap();
        assert!(e.contains("written and checked") && e.contains("no answer from the board"), "{}", e);
        assert!(e.contains(&path.display().to_string()), "{}", e);
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().firmware, Some("dump"));
        // Nothing was read, so the page is given nothing to show as read.
        assert_eq!(s.read, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flash_ends_with_the_settings_the_new_image_carries() {
        let desk = desk();
        let mut image = vec![0x42u8; 500];
        image.extend_from_slice(&block(7, 2));
        let (s, _, _, dir) = flashed("reread", &desk, false, &image, None);
        assert_eq!(s.error, None);
        let read = s.read.unwrap();
        assert!(read.starts_with(&format!("{{\"ok\":true,\"uid\":\"{}\",", UID)), "{}", read);
        assert!(read.contains("\"settings\":{\"layoutVersion\":1,\"csid\":7,\"module\":\"hp\",\"sf\":9,"), "{}", read);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_that_came_back_different_fail_the_job_and_the_page_gets_what_is_there() {
        let mut desk = desk();
        desk.lands_as = Some(|image| {
            let at = image.windows(settings::MAGIC.len()).position(|w| w == settings::MAGIC).unwrap();
            let mut back = image.to_vec();
            back[at..at + 32].copy_from_slice(&block(8, 2));
            back
        });
        let mut image = vec![0x42u8; 500];
        image.extend_from_slice(&block(7, 2));
        let (s, _, _, dir) = flashed("came-back", &desk, false, &image, None);
        let error = s.error.expect("a board with other settings is not a good write");
        assert!(error.contains("board id"), "{}", error);
        assert!(s.read.unwrap().contains("\"csid\":8"), "the read is the board's, not the image's");
        assert_eq!(s.note, "failed after the image was written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_that_came_back_the_same_are_said_to_match() {
        let desk = desk();
        let mut image = vec![0x42u8; 500];
        image.extend_from_slice(&block(7, 2));
        let (s, _, _, dir) = flashed("came-back-same", &desk, false, &image, None);
        assert_eq!(s.error, None);
        assert!(s.steps.iter().any(|l| l == "the settings on the board match what was written"), "{:?}", s.steps);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_about_the_image_is_a_step_before_anything_is_written() {
        let desk = desk();
        let note = "old.bin does not say which board it came off, so that was not checked";
        let (s, _, _, dir) = flashed("note", &desk, false, NEW, Some(note));
        let at = s.steps.iter().position(|l| l == note).expect("the note is a step");
        let erase = s.steps.iter().position(|l| l == "erasing the flash").unwrap();
        assert!(at < erase, "{:?}", s.steps);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_saved_off_another_board_is_refused_unless_forced() {
        let mine = "descent-image-20393256-Flight-spare-2-2026-09-21T14-13-20.bin";
        let theirs = "descent-image-20393301-board-2026-09-21T14-13-20.bin";
        assert_eq!(came_off(mine, UID, false, true), Ok(None));

        let e = came_off(theirs, UID, false, true).unwrap_err();
        assert!(e.contains("20393301") && e.contains("20393256"), "{}", e);
        assert!(e.contains("force=1"), "{}", e);
        // The same for a file the operator picked by hand: the name says the same thing.
        assert!(came_off(theirs, UID, false, false).is_err());

        let note = came_off(theirs, UID, true, true).unwrap().expect("forcing it is worth a line");
        assert!(note.contains("20393301") && note.contains("force"), "{}", note);

        // One a pull saved, inside its folder.
        let pulled = format!("descent-pull-2026-09-21T14-13-20-left/{}", theirs);
        assert!(came_off(&pulled, UID, false, true).is_err());
    }

    #[test]
    fn an_image_with_no_board_in_its_name_is_allowed_and_said_to_be_unchecked() {
        let old = "descent-ground-fsw-ttyACM1-2026-09-21T14-13-20.bin";
        let note = came_off(old, UID, false, true).unwrap().expect("an older save is worth a line");
        assert!(note.contains(old) && note.contains("not checked"), "{}", note);
        // A build somebody chose from their own folder is not a save of ours at all.
        assert_eq!(came_off("chipsat.bin", UID, false, false), Ok(None));
        assert_eq!(came_off("", UID, false, false), Ok(None));
    }

    // A build that carries the dump image and nothing else to tell a board by.
    fn only_dump() -> Known {
        Known { dump: Some(DUMP), older_dumps: &[], flight: Vec::new(), bench: Vec::new(), built: Vec::new(), source_now: None }
    }

    const OLD_DUMP: &[u8] = b"the dump firmware from the build before";
    const FLIGHT_LE: &[u8] = b"flight software built for the LE module";
    const FLIGHT_HP: &[u8] = b"flight software built for the HP module, a little longer";

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
    }

    // A build that also knows the dump image before this one, and carries both flight images.
    fn everything() -> Known {
        let older: &'static [(usize, &'static str)] =
            Box::leak(vec![(OLD_DUMP.len(), &*Box::leak(sha256(OLD_DUMP).into_boxed_str()))].into_boxed_slice());
        Known {
            dump: Some(DUMP),
            older_dumps: older,
            flight: vec![
                FlightImage { module: "le", bytes: FLIGHT_LE, id: "f84691175ddf2641".to_string(), block_at: None },
                FlightImage { module: "hp", bytes: FLIGHT_HP, id: "c2b044e470464918".to_string(), block_at: None },
            ],
            bench: vec![("le", BENCH_LE), ("hp", BENCH_HP)],
            built: Vec::new(),
            source_now: None,
        }
    }

    const BENCH_LE: &[u8] = b"the sensor bench built for the LE module";
    const BENCH_HP: &[u8] = b"the sensor bench built for the HP module";

    #[test]
    fn a_board_on_the_bench_firmware_is_said_to_be_and_by_module() {
        let known = everything();
        let mut flash = BENCH_HP.to_vec();
        flash.extend_from_slice(&[0xFF; 64]);
        let f = recognise(&flash, &known);
        assert_eq!((f.kind, f.module, f.word()), ("bench", Some("hp"), "bench-hp"));
        assert_eq!(f.json(), format!("{{\"kind\":\"bench\",\"module\":\"hp\",\"id\":\"{}\"}}", &sha256(BENCH_HP)[..16]));
        assert_eq!(recognise(BENCH_LE, &known).word(), "bench-le");
        assert_eq!(crate::chips::firmware_words("bench-le"), Some(("bench", Some("le"))));
        // truncated bench image
        assert_eq!(recognise(&BENCH_LE[..10], &known).kind, "unknown");
    }

    // A stand-in for a flight image that carries a block, as the real ones do: the same
    // bytes for both modules but for the block and for one byte far from it, which is
    // the module the firmware falls back to.
    const BLOCK_AT: usize = 1000;
    const FALLBACK_AT: usize = 100;

    fn built_block(module: u8) -> Vec<u8> {
        let mut b = settings::MAGIC.to_vec();
        b.extend_from_slice(&[settings::LAYOUT, 255, module, 7, 14, 1]);
        for fixed in [60_000u32, 10_000, 1_000, 60_000] {
            b.extend_from_slice(&fixed.to_le_bytes());
            b.extend_from_slice(&[0, 75, 50, 35, 20]);
            for ms in [10_000u32, 30_000, 60_000, 120_000, 180_000] {
                b.extend_from_slice(&ms.to_le_bytes());
            }
        }
        b.extend_from_slice(&[0; 20]);
        let crc = crate::dump::crc16(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(b.len(), settings::SIZE);
        b
    }

    // The same block after a tool has set the board id, as patch() writes it.
    fn tool_block(csid: u8, module: u8) -> Vec<u8> {
        let mut b = built_block(module);
        b[17] = csid;
        b[21] = 2;
        let crc = crate::dump::crc16(&b[..158]);
        b[158..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    fn flight_with_block(module: u8) -> &'static [u8] {
        let mut image: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        image[FALLBACK_AT] = module;
        image[BLOCK_AT..BLOCK_AT + settings::SIZE].copy_from_slice(&built_block(module));
        Box::leak(image.into_boxed_slice())
    }

    // A build that carries those two.
    fn with_blocks() -> Known {
        Known {
            dump: Some(DUMP),
            older_dumps: &[],
            flight: vec![
                FlightImage { module: "le", bytes: flight_with_block(1), id: "c4fb204d8f79e725".to_string(), block_at: Some(BLOCK_AT) },
                FlightImage { module: "hp", bytes: flight_with_block(2), id: "6935d29957db19b5".to_string(), block_at: Some(BLOCK_AT) },
            ],
            bench: Vec::new(),
            built: Vec::new(),
            source_now: None,
        }
    }

    fn on(desk: Desk, image: &[u8]) -> Desk {
        *desk.on_board.lock().unwrap() = image.to_vec();
        desk
    }

    fn built_here(part: &str, image: &[u8], block_at: Option<usize>, source: &str) -> crate::localfw::Identity {
        crate::localfw::Identity {
            part: part.to_string(),
            at: (crate::logfile::now_ms() / 1000) as u64,
            origin: crate::fswbuild::Origin {
                source: source.to_string(),
                commit: Some("abc1234def5678abc1234def5678abc1234def56".into()),
                dirty: Some(true),
                changed: vec!["V2_6_X_Code/src/config.h".into()],
            },
            sha256: sha256(image),
            masked: crate::localfw::masked(image, block_at),
            len: image.len(),
            block_at,
        }
    }
    const SRC: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    #[test]
    fn a_board_on_an_image_built_here_is_named_by_where_it_came_from() {
        let mut tree_le = flight_with_block(1).to_vec();
        tree_le[FALLBACK_AT + 1] ^= 0x5A;
        let mut known = with_blocks();
        assert_eq!(recognise(&in_flash(&tree_le), &known).kind, "unknown", "not carried, not built: unknown");
        known.built.push(built_here("flight-le", &tree_le, Some(BLOCK_AT), SRC));
        // different board id in the block
        let mut on_board = tree_le.clone();
        on_board[BLOCK_AT..BLOCK_AT + settings::SIZE].copy_from_slice(&tool_block(9, 1));
        let f = recognise(&in_flash(&on_board), &known);
        assert_eq!((f.kind, f.module, f.word()), ("flight", Some("le"), "flight-le"));
        let b = f.built.as_ref().expect("named by the registry");
        assert_eq!(b.origin.changed, vec!["V2_6_X_Code/src/config.h".to_string()]);
        assert_eq!(f.id.as_deref(), Some(&sha256(&tree_le)[..16]));
        let j = f.json();
        assert!(j.starts_with(&format!("{{\"kind\":\"flight\",\"module\":\"le\",\"id\":\"{}\",\"built\":{{\"part\":\"flight-le\"", &sha256(&tree_le)[..16])), "{}", j);
        assert!(j.contains("\"words\":\"flight LE built here ") && j.contains(" from abc1234 + config.h changed\""), "{}", j);
        let mut other = on_board.clone();
        other[0] ^= 1;
        assert_eq!(recognise(&other, &known).kind, "unknown");
    }

    #[test]
    fn the_carried_image_built_here_too_keeps_its_carried_name_and_says_where_it_was_built() {
        let mut known = with_blocks();
        let carried = flight_with_block(2);
        known.built.push(built_here("flight-hp", carried, Some(BLOCK_AT), SRC));
        let f = recognise(&in_flash(carried), &known);
        assert_eq!((f.kind, f.module, f.id.as_deref()), ("flight", Some("hp"), Some("6935d29957db19b5")));
        assert!(f.built.is_some());
        let mut wrong = with_blocks();
        wrong.built.push(built_here("bench-hp", carried, None, SRC));
        assert_eq!(recognise(carried, &wrong).kind, "flight");
        assert!(recognise(carried, &wrong).built.is_none());
    }

    #[test]
    fn a_dump_built_here_is_the_current_one_only_from_the_sources_in_use() {
        const TREE_DUMP: &[u8] = b"the dump firmware built from this computer's tree";
        let mut known = only_dump();
        known.built.push(built_here("flash_dump", TREE_DUMP, None, SRC));
        let f = recognise(&in_flash(TREE_DUMP), &known);
        assert_eq!((f.kind, f.current), ("dump", false));
        known.source_now = Some(SRC.to_string());
        let f = recognise(&in_flash(TREE_DUMP), &known);
        assert_eq!((f.kind, f.current, f.word()), ("dump", true, "dump"));
        assert!(f.json().contains("\"built\":{\"part\":\"flash_dump\",\"label\":\"dump firmware\""), "{}", f.json());
        known.built.push(built_here("bench-hp", BENCH_HP, None, SRC));
        assert_eq!(recognise(&in_flash(BENCH_HP), &known).word(), "bench-hp");
    }

    #[test]
    fn an_image_got_from_the_tree_is_a_step_of_the_flash_and_is_given_the_boards_settings() {
        let desk = on(desk(), OLD);
        let dir = scratch("tree");
        let image = flight_with_block(1).to_vec();
        let rate = |fixed_ms: u32| settings::Rate { fixed_ms, follow: false, percent: [75, 50, 35, 20], interval_ms: [10_000, 30_000, 60_000, 120_000, 180_000] };
        let asked = settings::Asked { csid: 9, sf: 7, dbm: 14, transmit: [rate(60_000), rate(1000)], log: [rate(10_000), rate(50)] };
        let make: Make = Box::new(move |say: &dyn Fn(&str)| {
            say("building the flight LE from the flight software in /tree");
            Ok((image, String::from("flight LE built here 14:02 from abc1234 + config.h changed")))
        });
        let mut job = Job {
            shared: shared(),
            registry: known(),
            dir: dir.clone(),
            probe: None,
            uid: UID.into(),
            backup: false,
            image: Vec::new(),
            label: "a flight image for board 9".into(),
            note: None,
            known: with_blocks(),
            build: Some(BuildPlan { make, asked: Some(asked), words: "a flight image for board 9".into() }),
        };
        // start() minus the thread
        begin(&job.shared, "flash", UID);
        let plan = job.build.take().unwrap();
        let shared2 = job.shared.clone();
        let (bytes, from) = (plan.make)(&|said: &str| step(&shared2, 2, said)).unwrap();
        let (at, _) = settings::writable(&bytes).unwrap();
        job.image = settings::patch(&bytes, at, plan.asked.as_ref().unwrap()).unwrap();
        step(&job.shared, 5, &format!("{}, {} kB", from, job.image.len().div_ceil(1024)));
        flash(&job, &desk);
        let s = job.shared.lock().unwrap().clone();
        assert_eq!(s.error, None, "{:?}", s.steps);
        let written = desk.on_board.lock().unwrap().clone();
        match settings::find(&written).unwrap() {
            Found::Block(set) => assert_eq!((set.csid, set.module), (9, "le")),
            other => panic!("the board's settings are in the block: {:?}", other),
        }
        assert!(s.steps.iter().any(|l| l == "flight LE built here 14:02 from abc1234 + config.h changed, 4 kB"), "{:?}", s.steps);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tree_still_at_an_older_layout_never_reaches_the_board() {
        // Stamping it would boot on compiled defaults and still read back as what was asked.
        let mut old = flight_with_block(1).to_vec();
        old[BLOCK_AT + 16] = 3;
        let crc = crate::dump::crc16(&old[BLOCK_AT..BLOCK_AT + 158]);
        old[BLOCK_AT + 158..BLOCK_AT + 160].copy_from_slice(&crc.to_le_bytes());
        let rate = |fixed_ms: u32| settings::Rate { fixed_ms, follow: false, percent: [75, 50, 35, 20], interval_ms: [10_000, 30_000, 60_000, 120_000, 180_000] };
        let asked = settings::Asked { csid: 9, sf: 7, dbm: 14, transmit: [rate(60_000), rate(1000)], log: [rate(10_000), rate(50)] };
        let make: Make = Box::new(move |_: &dyn Fn(&str)| Ok((old, String::from("flight LE built here"))));
        let state = shared();
        let dir = scratch("old-tree");
        let job = Job {
            shared: state.clone(),
            registry: known(),
            dir: dir.clone(),
            probe: None,
            uid: UID.into(),
            backup: false,
            image: Vec::new(),
            label: "a flight image for board 9".into(),
            note: None,
            known: with_blocks(),
            build: Some(BuildPlan { make, asked: Some(asked), words: "a flight image for board 9".into() }),
        };
        let gate = stm32::Gate::new();
        start(job, gate.take("the test").unwrap());
        // No Bench is given to this job: anything past the refusal would have to touch a probe.
        while state.lock().unwrap().running {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let s = state.lock().unwrap().clone();
        let e = s.error.unwrap();
        assert!(e.contains("layout 3 and this app writes layout 4"), "{}", e);
        assert!(s.note.contains("nothing was written"), "{}", s.note);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flight_image_is_still_itself_with_other_settings_in_its_block() {
        let known = with_blocks();
        for (module, n) in [("le", 1u8), ("hp", 2u8)] {
            let carried = flight_with_block(n);
            assert_eq!(recognise(carried, &known).module, Some(module));
            let mut given = carried.to_vec();
            given[BLOCK_AT..BLOCK_AT + settings::SIZE].copy_from_slice(&tool_block(12, n));
            assert_ne!(given, carried);
            let f = recognise(&in_flash(&given), &known);
            assert_eq!((f.kind, f.module), ("flight", Some(module)), "the {} image with other settings", module);
            // Even a block that names the other module does not make it the other image.
            let mut lying = given.clone();
            lying[BLOCK_AT + 18] = 3 - n;
            assert_eq!(recognise(&lying, &known).module, Some(module));
            // A byte changed anywhere else and it is not that image.
            let mut other = given.clone();
            other[BLOCK_AT + settings::SIZE] ^= 1;
            assert_eq!(recognise(&other, &known).kind, "unknown");
            let mut other = given;
            other[BLOCK_AT - 1] ^= 1;
            assert_eq!(recognise(&other, &known).kind, "unknown");
        }
    }

    #[test]
    fn the_two_flight_images_in_this_build_are_told_apart_with_their_blocks_left_out() {
        // They differ in the block and in one byte outside it, the module the firmware
        // falls back to. That one byte is what tells them apart once a board has been
        // given settings, so it had better be there.
        let known = Known::carried();
        let with_block: Vec<&FlightImage> = known.flight.iter().filter(|f| f.block_at.is_some()).collect();
        for f in &with_block {
            let at = f.block_at.unwrap();
            let mut given = f.bytes.to_vec();
            given[at + 17] = 12;   // not sealed: what the block holds is no part of this
            given[at + 30] ^= 0xFF;
            let seen = recognise(&given, &known);
            assert_eq!((seen.kind, seen.module, seen.id.as_deref()), ("flight", Some(f.module), Some(f.id.as_str())));
            for other in with_block.iter().filter(|o| o.module != f.module) {
                let outside = (0..f.bytes.len().min(other.bytes.len()))
                    .filter(|i| !(at..at + settings::SIZE).contains(i))
                    .filter(|i| f.bytes[*i] != other.bytes[*i])
                    .count();
                assert!(outside > 0, "the {} and {} images are the same outside their blocks", f.module, other.module);
            }
        }
    }

    #[test]
    fn a_block_whose_check_fails_is_reported_as_damaged_and_not_as_absent() {
        let mut image = flight_with_block(1).to_vec();
        image[BLOCK_AT + 19] ^= 0x04;
        let desk = on(desk(), &image);
        let state = shared();
        let answer = read(&state, &known(), &desk, &with_blocks());
        assert!(answer.starts_with("{\"ok\":true,"), "{}", answer);
        assert!(
            answer.contains("\"firmware\":{\"kind\":\"flight\",\"module\":\"le\",\"id\":\"c4fb204d8f79e725\"},\"settings\":null,\"settingsError\":\"the settings block is damaged; the board is running the values it was built with\",\"settingsBlockAt\":1000,"),
            "{}",
            answer
        );
        assert!(state.lock().unwrap().steps.iter().any(|l| l.contains("damaged")));

        // A whole one is given with where it is, and no block at all has no place.
        let answer = read(&shared(), &known(), &on(self::desk(), flight_with_block(2)), &with_blocks());
        assert!(answer.contains("\"setBy\":\"build\",\"txIntervalMs\":10000,\"transmit\":[{\"fixedMs\":60000,\"follow\":false,\"percent\":[75,50,35,20],\"intervalMs\":[10000,30000,60000,120000,180000]},"), "{}", answer);
        assert!(answer.contains("\"reservedClear\":true},\"settingsError\":null,\"settingsBlockAt\":1000,"), "{}", answer);
        let answer = read(&shared(), &known(), &self::desk(), &with_blocks());
        assert!(answer.contains("\"settings\":null,\"settingsError\":null,\"settingsBlockAt\":null,"), "{}", answer);
    }

    fn in_flash(image: &[u8]) -> Vec<u8> {
        let mut flash = image.to_vec();
        flash.extend_from_slice(&[0xFF; 64]);
        flash
    }

    #[test]
    fn firmware_is_the_dump_only_when_the_flash_starts_with_exactly_that_image() {
        let mut flash = in_flash(DUMP);
        assert_eq!(recognise(&flash, &only_dump()).kind, "dump");
        assert_eq!(recognise(DUMP, &only_dump()).kind, "dump");
        flash[3] ^= 1;
        assert_eq!(recognise(&flash, &only_dump()).kind, "unknown");
        assert_eq!(recognise(&DUMP[..DUMP.len() - 1], &only_dump()).kind, "unknown");
        assert_eq!(recognise(&[], &only_dump()).kind, "unknown");
        // A build with no dump image in it has nothing to compare against.
        let none = Known { dump: None, older_dumps: &[], flight: Vec::new(), bench: Vec::new(), built: Vec::new(), source_now: None };
        assert_eq!(recognise(DUMP, &none).kind, "unknown");
        let empty = Known { dump: Some(&[]), older_dumps: &[], flight: Vec::new(), bench: Vec::new(), built: Vec::new(), source_now: None };
        assert_eq!(recognise(DUMP, &empty).kind, "unknown");
    }

    #[test]
    fn the_dump_firmware_from_an_earlier_build_is_still_dump_firmware_and_said_to_be_old() {
        let now = recognise(&in_flash(DUMP), &everything());
        assert_eq!((now.kind, now.current), ("dump", true));
        assert_eq!(now.json(), format!("{{\"kind\":\"dump\",\"current\":true,\"id\":\"{}\"}}", &sha256(DUMP)[..16]));
        assert_eq!(now.word(), "dump");

        let old = recognise(&in_flash(OLD_DUMP), &everything());
        assert_eq!((old.kind, old.current), ("dump", false));
        assert_eq!(old.json(), format!("{{\"kind\":\"dump\",\"current\":false,\"id\":\"{}\"}}", &sha256(OLD_DUMP)[..16]));
        assert_eq!(old.word(), "dump");

        // One byte of it changed, or the end of it missing, and it is not that image.
        let mut flipped = in_flash(OLD_DUMP);
        flipped[5] ^= 0x01;
        assert_eq!(recognise(&flipped, &everything()).kind, "unknown");
        assert_eq!(recognise(&OLD_DUMP[..OLD_DUMP.len() - 1], &everything()).kind, "unknown");
        // And a build that has never heard of it says unknown, as before.
        assert_eq!(recognise(&in_flash(OLD_DUMP), &only_dump()).kind, "unknown");
    }

    #[test]
    fn a_flight_image_this_build_carries_is_recognised_with_its_module() {
        let le = recognise(&in_flash(FLIGHT_LE), &everything());
        assert_eq!(le, Firmware { kind: "flight", current: false, module: Some("le"), id: Some("f84691175ddf2641".into()), built: None });
        assert_eq!(le.json(), "{\"kind\":\"flight\",\"module\":\"le\",\"id\":\"f84691175ddf2641\"}");
        assert_eq!(le.word(), "flight-le");

        let hp = recognise(&in_flash(FLIGHT_HP), &everything());
        assert_eq!((hp.kind, hp.module, hp.id.as_deref()), ("flight", Some("hp"), Some("c2b044e470464918")));
        assert_eq!(hp.word(), "flight-hp");

        let mut flipped = in_flash(FLIGHT_HP);
        flipped[FLIGHT_HP.len() - 1] ^= 0x80;
        let no = recognise(&flipped, &everything());
        assert_eq!(no, Firmware { kind: "unknown", current: false, module: None, id: None, built: None });
        assert_eq!(no.json(), "{\"kind\":\"unknown\"}");
        assert_eq!(no.word(), "unknown");
        // Flight software this build does not carry has nothing to be told by.
        assert_eq!(recognise(&in_flash(FLIGHT_LE), &only_dump()).kind, "unknown");
    }

    #[test]
    fn the_older_dump_image_this_build_knows_is_the_one_committed_before_it() {
        let known = Known::carried();
        assert!(known.older_dumps.contains(&(31224, "4a5f0ead87429a51d9c84757c8be07058b58ed14ac885e25b9f48ca3f8c531a6")));
        // What the build carries is recognised as itself, each image as what it is.
        if let Some(dump) = known.dump {
            // Named as /api/health names it, so the sheet and the health line agree.
            let id = Some(crate::flash::CHIPSAT_DUMP_ID.to_string());
            assert_eq!(recognise(dump, &known), Firmware { kind: "dump", current: true, module: None, id, built: None });
        }
        for carried in &known.flight {
            let f = recognise(carried.bytes, &known);
            assert_eq!((f.kind, f.module, f.id.as_deref()), ("flight", Some(carried.module), Some(carried.id.as_str())));
        }
    }

    #[test]
    fn reading_a_board_on_flight_software_says_so_and_the_list_keeps_which_module() {
        let desk = desk();
        *desk.on_board.lock().unwrap() = in_flash(FLIGHT_LE);
        let registry = known();
        registry.lock().unwrap().set_module(UID, "hp").unwrap();
        let answer = read(&shared(), &registry, &desk, &everything());
        assert!(
            answer.contains("\"firmware\":{\"kind\":\"flight\",\"module\":\"le\",\"id\":\"f84691175ddf2641\"},\"settings\":null,"),
            "{}",
            answer
        );
        let r = registry.lock().unwrap();
        let chip = r.get(UID).unwrap();
        assert_eq!((chip.firmware, chip.firmware_module, chip.module), (Some("flight"), Some("le"), Some("hp")));
        drop(r);

        // The older dump firmware reads as dump firmware, and a block under it is not reported.
        *desk.on_board.lock().unwrap() = in_flash(OLD_DUMP);
        let answer = read(&shared(), &registry, &desk, &everything());
        assert!(
            answer.contains(&format!("\"firmware\":{{\"kind\":\"dump\",\"current\":false,\"id\":\"{}\"}},\"settings\":null,", &sha256(OLD_DUMP)[..16])),
            "{}",
            answer
        );
        let r = registry.lock().unwrap();
        assert_eq!((r.get(UID).unwrap().firmware, r.get(UID).unwrap().firmware_module), (Some("dump"), None));
    }

    #[test]
    fn reading_a_board_answers_with_what_it_is_and_remembers_it() {
        let desk = desk();
        *desk.on_board.lock().unwrap() = DUMP.to_vec();
        let state = shared();
        let registry = known();
        let answer = read(&state, &registry, &desk, &only_dump());
        assert_eq!(
            answer,
            format!(
                "{{\"ok\":true,\"uid\":\"{}\",\"flashBytes\":262144,\"firmware\":{{\"kind\":\"dump\",\"current\":true,\"id\":\"{}\"}},\"settings\":null,\"settingsError\":null,\"settingsBlockAt\":null,\"probe\":{{\"serial\":\"002A00\",\"kind\":\"STLink V3\"}}}}",
                UID,
                &sha256(DUMP)[..16]
            )
        );
        assert_eq!(desk.did.lock().unwrap().clone(), vec!["look"]);
        let r = registry.lock().unwrap();
        assert_eq!(r.get(UID).unwrap().firmware, Some("dump"));
        assert_eq!(r.get(UID).unwrap().name, "Flight spare 2", "reading a board does not rename it");
        assert!(r.get(UID).unwrap().last_seen > 1);

        let s = state.lock().unwrap().clone();
        assert!(!s.running);
        assert_eq!(s.what, "read");
        assert_eq!(s.uid, UID);
        assert_eq!(s.error, None);
        assert_eq!(s.pct, 100);
    }

    #[test]
    fn reading_a_board_nobody_has_seen_adds_it_to_the_list() {
        let mut desk = desk();
        desk.on_probe = OTHER;
        desk.was_reset = true;
        let state = shared();
        let registry = known();
        let answer = read(&state, &registry, &desk, &only_dump());
        assert!(answer.contains(&format!("\"uid\":\"{}\"", OTHER)) && answer.contains("\"kind\":\"unknown\""), "{}", answer);
        assert_eq!(registry.lock().unwrap().get(OTHER).unwrap().firmware, Some("unknown"));
        // Resetting a board to reach it is not nothing, so it is said.
        let steps = state.lock().unwrap().steps.clone();
        assert!(steps.iter().any(|l| l.contains("reset")), "{:?}", steps);
    }

    // A layout-1 block, as a board flashed before the battery table carries one.
    fn block(csid: u8, module: u8) -> Vec<u8> {
        let mut b = settings::MAGIC.to_vec();
        b.extend_from_slice(&[1, csid, module, 9, 14, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
        let crc = crate::dump::crc16(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    #[test]
    fn reading_a_board_reports_the_settings_block_its_image_carries() {
        let desk = desk();
        let mut image = vec![0x42u8; 500];
        image.extend_from_slice(&block(7, 2));
        *desk.on_board.lock().unwrap() = image.clone();
        let answer = read(&shared(), &known(), &desk, &only_dump());
        assert!(
            answer.contains("\"settings\":{\"layoutVersion\":1,\"csid\":7,\"module\":\"hp\",\"sf\":9,\"dbm\":14,\"setBy\":\"tool\",\"txIntervalMs\":0,\"transmit\":null,\"log\":null,\"olderSlots\":[],\"reservedClear\":true},\"settingsError\":null,"),
            "{}",
            answer
        );

        // Two of them is nothing to go by, and says so without failing the read.
        image.extend_from_slice(&block(8, 1));
        *desk.on_board.lock().unwrap() = image;
        let state = shared();
        let answer = read(&state, &known(), &desk, &only_dump());
        assert!(answer.starts_with("{\"ok\":true,"), "{}", answer);
        assert!(answer.contains("\"settings\":null,\"settingsError\":\"2 settings blocks"), "{}", answer);
        assert!(state.lock().unwrap().steps.iter().any(|l| l.contains("2 settings blocks")));
    }

    #[test]
    fn a_settings_block_in_a_newer_layout_is_said_to_be_there_and_not_read() {
        let desk = desk();
        let mut b = block(7, 2);
        b[16] = 5;
        let mut image = vec![0x42u8; 500];
        image.extend_from_slice(&b);
        *desk.on_board.lock().unwrap() = image;
        let state = shared();
        let answer = read(&state, &known(), &desk, &only_dump());
        assert!(answer.contains("\"settings\":null,\"settingsError\":\"the settings block on the board is layout 5"), "{}", answer);
        assert!(state.lock().unwrap().steps.iter().any(|l| l.contains("layout 5")));
    }

    #[test]
    fn a_block_left_behind_under_the_dump_firmware_is_not_reported() {
        // The dump image is smaller than a flight image, and writing it leaves the end of
        // the flight image where it was. A block in that is not one the board is using.
        let desk = desk();
        let mut image = DUMP.to_vec();
        image.extend_from_slice(&[0x42; 100]);
        image.extend_from_slice(&block(7, 2));
        *desk.on_board.lock().unwrap() = image;
        let answer = read(&shared(), &known(), &desk, &only_dump());
        assert!(answer.contains("\"kind\":\"dump\",\"current\":true,\"id\":\""), "{}", answer);
        assert!(answer.contains("\"},\"settings\":null,\"settingsError\":null,"), "{}", answer);
    }

    #[test]
    fn a_read_that_fails_says_why_and_leaves_the_list_alone() {
        let mut desk = desk();
        desk.fail_look = Some(0);
        let state = shared();
        let registry = known();
        let answer = read(&state, &registry, &desk, &only_dump());
        assert_eq!(answer, "{\"ok\":false,\"error\":\"no answer from the board\"}");
        assert_eq!(registry.lock().unwrap().get(UID).unwrap().last_seen, 1);
        let s = state.lock().unwrap().clone();
        assert!(!s.running);
        assert_eq!(s.error.as_deref(), Some("no answer from the board"));
    }

    #[test]
    fn the_job_state_a_reloaded_page_reads_back() {
        assert_eq!(
            state_json(&State::idle(), 5_000),
            "{\"ok\":true,\"running\":false,\"what\":\"\",\"uid\":\"\",\"pct\":0,\"note\":\"\",\"steps\":[],\"error\":null,\"saved\":null,\"read\":null,\"seconds\":0}"
        );

        let mut s = State::idle();
        s.running = true;
        s.what = "flash";
        s.uid = UID.into();
        s.pct = 42;
        s.note = "writing new.bin, 8 of 16 kB".into();
        s.steps = vec!["saving what is on the board".into(), "saved \"C:\\DeSCENT\\x.bin\"".into()];
        s.saved = Some("descent-fsw-20393256-board-2026-09-21T14-13-20.bin".into());
        s.started_ms = 1_000_000;
        assert_eq!(
            state_json(&s, 1_075_000),
            format!(
                "{{\"ok\":true,\"running\":true,\"what\":\"flash\",\"uid\":\"{}\",\"pct\":42,\"note\":\"writing new.bin, 8 of 16 kB\",\"steps\":[\"saving what is on the board\",\"saved \\\"C:\\\\DeSCENT\\\\x.bin\\\"\"],\"error\":null,\"saved\":\"descent-fsw-20393256-board-2026-09-21T14-13-20.bin\",\"read\":null,\"seconds\":75}}",
                UID
            )
        );

        // A finished job stops counting, and says what went wrong.
        s.running = false;
        s.finished_ms = 1_030_000;
        s.error = Some("the probe fell off".into());
        let j = state_json(&s, 9_999_999);
        assert!(j.contains("\"running\":false") && j.contains("\"error\":\"the probe fell off\"") && j.ends_with("\"seconds\":30}"), "{}", j);

        // What a flash read off the board at its end goes out as it is, not as a string.
        s.read = Some("{\"ok\":true,\"uid\":\"00\"}".into());
        assert!(state_json(&s, 9_999_999).contains("\"read\":{\"ok\":true,\"uid\":\"00\"},\"seconds\""));
    }

    #[test]
    fn progress_inside_a_step_moves_the_bar_without_adding_a_step_each_time() {
        let state = shared();
        begin(&state, "flash", UID);
        let said = listener(&state, String::from("writing new.bin"));
        said(Doing::Saving, 131_072, 262_144);
        let half = state.lock().unwrap().clone();
        assert_eq!(half.note, "saving what is on the board, 128 of 256 kB");
        said(Doing::Saving, 262_144, 262_144);
        said(Doing::Erasing, 0, 0);
        said(Doing::Writing, 8_192, 16_384);
        said(Doing::Writing, 16_384, 16_384);
        let s = state.lock().unwrap().clone();
        assert_eq!(s.steps, vec!["saving what is on the board", "erasing the flash", "writing new.bin"]);
        assert_eq!(s.note, "writing new.bin, 16 of 16 kB");
        assert!(half.pct > 0 && s.pct > half.pct && s.pct < 100, "{} then {}", half.pct, s.pct);
        // Putting an image back does not run the bar backwards.
        said(Doing::Erasing, 0, 0);
        assert!(state.lock().unwrap().pct >= s.pct);
    }

    #[test]
    fn a_board_on_the_dump_firmware_has_nothing_saved_and_older_images_of_a_board_go() {
        // on the dump firmware: nothing saved, newest real image stays
        let on_dump = desk();
        *on_dump.on_board.lock().unwrap() = DUMP.to_vec();
        let (s, did, _, dir) = flashed("ondump", &on_dump, true, NEW, None);
        assert!(!did.contains(&"save".to_string()), "{:?}", did);
        assert_eq!(s.saved, None);
        assert!(s.steps.iter().any(|x| x.contains("dump firmware, so nothing is saved")), "{:?}", s.steps);
        let _ = std::fs::remove_dir_all(&dir);

        // on flight software: saved, and the older image of the same board removed
        let on_flight = desk();
        let dir = scratch("keepnewest");
        std::fs::create_dir_all(dir.join("2026-10-01/images")).unwrap();
        let older = dir.join("2026-10-01/images/0900-board-20393256.bin");
        let other = dir.join("2026-10-01/images/0900-board-20393301.bin");
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&other, b"other board").unwrap();
        let job = Job {
            shared: shared(),
            registry: known(),
            dir: dir.clone(),
            probe: None,
            uid: UID.into(),
            backup: true,
            image: NEW.to_vec(),
            label: "new.bin".into(),
            note: None,
            known: only_dump(),
            build: None,
        };
        begin(&job.shared, "flash", UID);
        flash(&job, &on_flight);
        let s = job.shared.lock().unwrap().clone();
        assert_eq!(s.error, None);
        assert!(dir.join(s.saved.unwrap()).is_file());
        assert!(!older.exists(), "the older image of this board is gone");
        assert!(other.exists(), "another board's is not");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
