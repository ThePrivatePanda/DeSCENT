// The dashboard, and the few calls it makes back. Bound to 127.0.0.1 only: this is
// the operator's own laptop, not a service.
use crate::json::{esc, int_field};
use crate::logfile::Log;
use crate::serial::{Board, Hub, PortInfo};
use crate::ws::{Broadcast, Socket};
use crate::{assets, flash, logfile, urlfile};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server};
use tungstenite::protocol::Role;

// Fixed, not ephemeral: the dashboard keeps its settings in localStorage and its
// crash autosave in IndexedDB, and the browser keys both to the origin. A new port
// every launch would hand the operator a blank dashboard and no recovered session.
pub const FIRST_PORT: u16 = 8765;
pub const LAST_PORT: u16 = 8774;

// rxId is the id a receiver prints in its header line, null until one has arrived.
// rxName is what the board says it is called, rxFw the firmware it says it runs (1.2.1
// was the first to say), and the last name is what this computer calls that id. A board already has a "name", the one kept against its USB serial, so
// in "boards" the name kept against the id is "rxLocalName".
pub fn ports_json(ports: &[PortInfo], boards: &[Board], log: Option<&str>) -> String {
    let or_null = |v: Option<&String>| match v {
        Some(s) => format!("\"{}\"", esc(s)),
        None => String::from("null"),
    };
    let items: Vec<String> = ports
        .iter()
        .map(|p| {
            format!(
                "{{\"key\":\"{}\",\"port\":\"{}\",\"usb\":\"{}\",\"status\":\"{}\",\"error\":{},\"rxId\":{},\"rxName\":{},\"rxFw\":{},\"name\":{}}}",
                esc(&p.key),
                esc(&p.port),
                esc(&p.usb),
                esc(&p.status),
                or_null(p.error.as_ref()),
                or_null(p.rx_id.as_ref()),
                or_null(p.rx_name.as_ref()),
                or_null(p.rx_fw.as_ref()),
                or_null(p.name.as_ref()),
            )
        })
        .collect();
    let rest: Vec<String> = boards
        .iter()
        .map(|b| {
            let read = ports.iter().find(|p| p.port == b.port);
            format!(
                "{{\"port\":\"{}\",\"usb\":\"{}\",\"label\":\"{}\",\"serial\":\"{}\",\"state\":\"{}\",\"name\":\"{}\",\"rxId\":{},\"rxName\":{},\"rxFw\":{},\"rxLocalName\":{}}}",
                esc(&b.port),
                esc(&b.usb),
                esc(&b.label),
                esc(&b.serial),
                esc(&b.state),
                esc(&b.name),
                or_null(read.and_then(|p| p.rx_id.as_ref())),
                or_null(read.and_then(|p| p.rx_name.as_ref())),
                or_null(read.and_then(|p| p.rx_fw.as_ref())),
                or_null(read.and_then(|p| p.name.as_ref())),
            )
        })
        .collect();
    format!(
        "{{\"ports\":[{}],\"boards\":[{}],\"log\":{}}}",
        items.join(","),
        rest.join(","),
        match log {
            Some(l) => format!("\"{}\"", esc(l)),
            None => String::from("null"),
        }
    )
}

// What a launcher polls to know we are up. Built from numbers rather than read off a
// running server, so it can be checked without one.
// Whether the program file has been written since this copy of it started. Every build
// says the same version, and building again does nothing to a copy that is already
// running, so without this an operator can test a fix for an hour on a build from before
// it. A file that has gone, or cannot be asked, is not known to be newer.
pub fn newer_on_disk(started: std::time::SystemTime, modified: Option<std::time::SystemTime>) -> bool {
    modified.map(|m| m > started).unwrap_or(false)
}

pub fn health_json(version: &str, port: u16, log: Option<&str>, boards: usize, receivers: usize, built: u64, newer: bool, fsw: bool) -> String {
    // The dump firmware has no version of its own and does change, so the build says
    // which copy it is carrying rather than leaving a stale one to be guessed at. The id
    // is the first 16 characters of the image's sha256, which the flight repo publishes
    // in full. The flight images are listed the same way, each with the module it is for.
    let flight = crate::flight::carried();
    format!(
        "{{\"ok\":true,\"version\":\"{}\",\"built\":{},\"newerOnDisk\":{},\"port\":{},\"dumpFirmware\":\"{}\",\"flightImages\":{},\"flightBuild\":{},\"fswTree\":{},\"log\":{},\"boards\":{},\"receivers\":{}}}",
        esc(version),
        built,
        newer,
        port,
        esc(crate::flash::CHIPSAT_DUMP_ID),
        flight.images_json(),
        flight.build_json(),
        fsw,
        match log {
            Some(l) => format!("\"{}\"", esc(l)),
            None => String::from("null"),
        },
        boards,
        receivers
    )
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

// Try our ports in order. A bind that fails on a port where our own /api/ports
// answers means a second double-click, which the caller turns into "open the
// browser at the one already running".
pub enum Bound {
    Ours(Server, u16),
    AlreadyRunning(u16),
}

// A port that was asked for is the only one tried: a script that said 8900 and quietly
// got 8765 has no way to notice. Our own copy already on it is still worth reporting as
// that rather than as a failure.
//
// host is 127.0.0.1 unless --host says otherwise, and it should stay that way on a laptop:
// this app flashes boards and writes files, so anything that can reach it can do both
// without asking. The flag exists for a machine in a rack with the boards plugged into it,
// where the person using it is somewhere else.
pub fn bind(host: &str, want: Option<u16>) -> Result<Bound, String> {
    if let Some(port) = want {
        return match Server::http((host, port)) {
            Ok(s) => Ok(Bound::Ours(s, port)),
            Err(e) => {
                if ours_is_listening(port) {
                    Ok(Bound::AlreadyRunning(port))
                } else {
                    Err(format!("port {} is taken: {}", port, e))
                }
            }
        };
    }
    let mut last = String::new();
    for port in FIRST_PORT..=LAST_PORT {
        match Server::http((host, port)) {
            Ok(s) => return Ok(Bound::Ours(s, port)),
            Err(e) => {
                last = e.to_string();
                if ours_is_listening(port) {
                    return Ok(Bound::AlreadyRunning(port));
                }
            }
        }
    }
    Err(last)
}

// One request, one answer, from outside the running app. HTTP/1.0 so the other end
// closes and the read ends.
fn ask(port: u16, request: &str) -> Option<String> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    s.write_all(request.as_bytes()).ok()?;
    let mut buf = String::new();
    let _ = s.take(2048).read_to_string(&mut buf);
    Some(buf)
}

fn ours_is_listening(port: u16) -> bool {
    match ask(port, "GET /api/ports HTTP/1.0\r\n\r\n") {
        Some(buf) => buf.contains("\"ports\""),
        None => false,
    }
}

fn health_answers(port: u16) -> bool {
    match ask(port, "GET /api/health HTTP/1.0\r\n\r\n") {
        Some(buf) => buf.contains("\"receivers\":"),
        None => false,
    }
}

// Where the running copy is, for --stop. The url file names the port when one was
// written, and our ports are walked when it was not. Either way the port has to answer
// /api/health first: a kill leaves the file behind, and an older build answers
// /api/ports without knowing how to stop.
pub fn running_port(dir: &Path, want: Option<u16>) -> Option<u16> {
    if let Some(port) = want {
        return if health_answers(port) { Some(port) } else { None };
    }
    if let Some(port) = urlfile::port(dir) {
        if health_answers(port) {
            return Some(port);
        }
    }
    (FIRST_PORT..=LAST_PORT).find(|p| health_answers(*p))
}

// Ask, then wait for the port to go quiet. "It said yes" is not "it is gone", and a
// script that starts another copy straight after needs the port free.
pub fn quit(port: u16) -> Result<(), String> {
    let answer = ask(port, "POST /api/quit HTTP/1.0\r\nContent-Length: 0\r\n\r\n")
        .ok_or_else(|| format!("port {} stopped answering before it replied", port))?;
    if !answer.contains("\"ok\":true") {
        return Err(format!("port {} would not take the request", port));
    }
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_err() {
            return Ok(());
        }
    }
    Err(format!("port {} still answers three seconds after agreeing to stop", port))
}

fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Every log in the data folder, so they can be opened or saved from the page. Someone who
// pulled a chip should not have to go looking in a directory for what they just read. A
// pull keeps its log in a folder of its own, and is listed by the name /api/log takes.
//
// Firmware read off a ChipSat is listed with them as kind "image", so it can be found and
// put back from the page too. Its "uid" is the board it came off, and is left out when the
// name does not say. A name only carries the end of the id, so the whole id is given when
// exactly one board this laptop has read ends that way, and the end alone when not. A
// pull is given the board of the image saved beside its log.
pub fn logs_json(dir: &Path, chips: &crate::chips::Registry) -> String {
    let mut rows: Vec<(u64, String, u64, &str, Option<String>)> = Vec::new();
    let mut add = |name: String, meta: Option<std::fs::Metadata>, kind: &'static str, board: Option<String>| {
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let when = meta.as_ref().map(modified_secs).unwrap_or(0);
        let board = board.map(|tail| chips.whole_uid(&tail).unwrap_or(tail));
        rows.push((when, name, size, kind, board));
    };
    // board from board.txt, else the image in the folder, else (older pulls) a list name in the folder name
    let pull_folder = |name: &str, path: &Path, add: &mut dyn FnMut(String, Option<std::fs::Metadata>, &'static str, Option<String>)| {
        let mut board = crate::layout::board_in(path).map(|uid| crate::stm32::uid_tail(&uid).to_string());
        for inside in std::fs::read_dir(path).into_iter().flatten().flatten() {
            let file = format!("{}/{}", name, inside.file_name().to_string_lossy());
            if is_image(&file) {
                let off = crate::stm32::uid_in_name(&file);
                board = board.or(off.clone());
                add(file, inside.metadata().ok(), "image", off);
            }
        }
        let leaf = name.rsplit('/').next().unwrap_or("");
        let board = board.or_else(|| {
            crate::layout::board_of_folder(leaf).and_then(|w| chips.named(w)).map(|uid| crate::stm32::uid_tail(&uid).to_string())
        });
        // a pull that read nothing has no log but may still have saved an image
        if let Ok(meta) = std::fs::metadata(path.join("ground.log")) {
            add(format!("{}/ground.log", name), Some(meta), "pull", board);
        }
    };
    let entries = |d: &Path| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().to_string(), e));
    for (name, e) in entries(dir) {
        if crate::layout::is_day(&name) {
            // day folder (layout.rs). Clears have no log to open
            for (file, f) in entries(&e.path().join(crate::layout::LOGS)) {
                if file.ends_with(".log") {
                    add(format!("{}/{}/{}", name, crate::layout::LOGS, file), f.metadata().ok(), "session", None);
                }
            }
            // a clear's image can still be a board's newest
            for kind in [crate::layout::PULLS, crate::layout::CLEARS] {
                for (folder, f) in entries(&e.path().join(kind)) {
                    pull_folder(&format!("{}/{}/{}", name, kind, folder), &f.path(), &mut add);
                }
            }
            for (file, f) in entries(&e.path().join(crate::layout::IMAGES)) {
                let rel = format!("{}/{}/{}", name, crate::layout::IMAGES, file);
                if is_image(&rel) {
                    let off = crate::stm32::uid_in_name(&rel);
                    add(rel, f.metadata().ok(), "image", off);
                }
            }
        } else if name.ends_with(".log") {
            add(name, e.metadata().ok(), "session", None);
        } else if name.starts_with(PULL_FOLDER) {
            pull_folder(&name, &e.path(), &mut add);
        } else if is_image(&name) {
            let off = crate::stm32::uid_in_name(&name);
            add(name, e.metadata().ok(), "image", off);
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));   // newest first: it is the one just written
    let items: Vec<String> = rows
        .iter()
        .map(|(when, name, size, kind, board)| {
            let uid = match board {
                Some(uid) => format!(",\"uid\":\"{}\"", esc(uid)),
                None => String::new(),
            };
            format!(
                "{{\"name\":\"{}\",\"bytes\":{},\"when\":{},\"kind\":\"{}\"{}}}",
                esc(name),
                size,
                when,
                kind,
                uid
            )
        })
        .collect();
    format!("{{\"logs\":[{}]}}", items.join(","))
}

const PULL_FOLDER: &str = "descent-pull-";

// Bytes, because a name can hold anything once it is decoded and has to be whole UTF-8
// before it is looked at.
fn percent_decoded(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = (*b.get(i + 1)? as char).to_digit(16)?;
            let lo = (*b.get(i + 2)? as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// What follows the ? in a url, as the pairs it holds. A flash sends its image as the
// body, so everything else it has to say comes this way. None when an escape in it does
// not decode: half a file name is not a name to go looking for.
fn query(url: &str) -> Option<Vec<(String, String)>> {
    let Some((_, text)) = url.split_once('?') else { return Some(Vec::new()) };
    let mut pairs = Vec::new();
    for part in text.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        // A form writes a space as +. Turned back before the escapes, so a + that was
        // sent as %2B is still one.
        pairs.push((percent_decoded(&key.replace('+', " "))?, percent_decoded(&value.replace('+', " "))?));
    }
    Some(pairs)
}

fn asked<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

// The file a /api/log/<name> request means, relative to the data folder, or None to
// refuse it. Decoded before it is checked, or %2e%2e walks straight past the check
fn log_name(asked: &str) -> Option<String> {
    let name = percent_decoded(asked.trim_start_matches('/'))?;
    if data_file(&name) { Some(name) } else { None }
}

fn data_file(name: &str) -> bool {
    // A colon is a drive (D:x replaces the base when joined) or an NTFS stream on Windows.
    if name.contains("..") || name.contains('\\') || name.contains(':') || name.chars().any(|c| c.is_control()) {
        return false;
    }
    use crate::layout::{CLEARS, IMAGES, LOGS, PULLS};
    let parts: Vec<&str> = name.split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    match parts.as_slice() {
        [_] => true,
        [folder, _] => folder.starts_with(PULL_FOLDER),
        [day, kind, _] => crate::layout::is_day(day) && (*kind == LOGS || *kind == IMAGES),
        [day, kind, _, _] => crate::layout::is_day(day) && (*kind == PULLS || *kind == CLEARS),
        _ => false,
    }
}

// A firmware image read off a ChipSat, by the name /api/logs gives it: one a flash saved
// straight into the data folder, under its newer name or its older one, or the one a
// pull kept in its folder. A receiver's backup is a .bin in the same folder and is not
// one of these; it is an ESP32 image and must never be offered for a ChipSat.
fn is_image(name: &str) -> bool {
    if !name.ends_with(".bin") {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    match parts.as_slice() {
        [file] => file.starts_with(crate::stm32::IMAGE_PREFIX) || file.starts_with(crate::stm32::OLDER_IMAGE_PREFIX),
        [folder, _] => folder.starts_with(PULL_FOLDER),
        [day, kind, _] => crate::layout::is_day(day) && *kind == crate::layout::IMAGES,
        [day, kind, _, _] => crate::layout::is_day(day) && (*kind == crate::layout::PULLS || *kind == crate::layout::CLEARS),
        _ => false,
    }
}

pub fn probes_json(probes: &[crate::stm32::Probe]) -> String {
    let items: Vec<String> = probes
        .iter()
        .map(|p| format!("{{\"serial\":\"{}\",\"kind\":\"{}\",\"access\":{}}}", esc(&p.serial), esc(&p.kind), p.access))
        .collect();
    format!("{{\"probes\":[{}]}}", items.join(","))
}

// The name a browser saves it under. Every pull's log is called ground.log, so the folder
// goes into the name, and it goes into a header, so only plain characters are let through.
fn save_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' => '-',
            '"' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect()
}

// A pull's folder also holds the firmware image that was on the board, and a browser
// handed that as text will try to show it.
fn content_type(name: &str) -> &'static str {
    if name.ends_with(".bin") {
        "application/octet-stream"
    } else if name.ends_with(".json") {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    }
}

pub struct Ctx {
    pub hub: Arc<Mutex<Hub>>,
    pub broadcast: Broadcast,
    pub log: Arc<Mutex<Option<Log>>>,
    pub dir: PathBuf,    // beside the program: settings, the url file, board names, tiles
    pub data: PathBuf,   // what the program records: session logs, pulls, firmware read off a board
    pub port: u16,
    pub pull: crate::pull::Shared,
    pub chips: crate::chips::Shared,
    pub chip: crate::chipflash::Shared,   // the last read or flash of a ChipSat
    pub gate: crate::stm32::Gate,
    pub exe: Option<PathBuf>,             // the file this process was started from
    pub started: std::time::SystemTime,
    pub local: crate::localfw::Shared,    // fsw tree on this laptop, if any
}

// The one way out, whoever asked. The log is flushed before the url file goes, because
// a file somebody else is holding open cannot be removed on Windows and that must not
// cost us the tail of the log.
pub fn shutdown(ctx: &Ctx, why: &str) -> ! {
    if let Ok(mut l) = ctx.log.lock() {
        if let Some(l) = l.as_mut() {
            l.flush();
        }
    }
    if let Some(w) = urlfile::remove(&ctx.dir) {
        println!("{}", w);
    }
    println!("{}", why);
    std::process::exit(0);
}

pub fn serve(server: Server, ctx: Arc<Ctx>) {
    for request in server.incoming_requests() {
        let ctx = ctx.clone();
        std::thread::spawn(move || handle(request, ctx));
    }
}

fn is_websocket(request: &Request) -> Option<String> {
    let mut upgrade = false;
    let mut key = None;
    for h in request.headers() {
        let f = h.field.as_str().as_str().to_ascii_lowercase();
        if f == "upgrade" && h.value.as_str().to_ascii_lowercase().contains("websocket") {
            upgrade = true;
        }
        if f == "sec-websocket-key" {
            key = Some(h.value.as_str().to_string());
        }
    }
    if upgrade { key } else { None }
}

fn answer(request: Request, text: String) {
    let _ = request.respond(
        Response::from_string(text).with_header(header("Content-Type", "application/json")),
    );
}

// The drone's flight controller is on the same USB hub as the ChipSat consoles and gets picked
// by mistake. Whatever was asked of it as a console is refused by name, before anything opens it
fn flight_controller(ctx: &Ctx, port: &str) -> Option<String> {
    let is = ctx.hub.lock().unwrap().boards().iter().any(|b| b.port == port && crate::serial::is_flight_controller(&b.usb));
    is.then(|| format!("{} is the drone's flight controller (ArduPilot), not a ChipSat console: pick the board's own serial port", port))
}

fn failed(e: &str) -> String {
    format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(e))
}

// For the calls that change a board or what is remembered about one. A link, or a
// browser fetching ahead, asks with GET and must not be able to flash anything.
fn post_only(request: Request) -> Option<Request> {
    if request.method().as_str() == "POST" {
        return Some(request);
    }
    let _ = request.respond(
        Response::from_string("{\"ok\":false,\"error\":\"POST only\"}")
            .with_status_code(405)
            .with_header(header("Content-Type", "application/json")),
    );
    None
}

// Twice the largest flash this chip family has. Enough to read a wrong file whole and
// say what is wrong with it, and not enough to fill memory with one.
const MAX_UPLOAD: u64 = 512 * 1024;

// The settings an image is to be given, when the request has them. They go together: an
// image with some of them changed and the rest as built is nobody's intent. Per phase,
// in the block's order (waiting, rising, falling, landed): fixed0..3 in ms, follow0..3 as
// 0 or 1, soc0..3 as four thresholds and socms0..3 as five intervals, comma separated,
// log0..3 as the flash log gap in ms, and logblind the gap with no altitude source.
fn settings_asked(q: &[(String, String)]) -> Result<Option<crate::radio::Request>, String> {
    let mut keys: Vec<(String, String)> = vec![
        ("csid".into(), "board id".into()),
        ("sf".into(), "spreading factor".into()),
        ("dbm".into(), "transmit power".into()),
    ];
    for (p, name) in crate::settings::PHASE_NAMES.iter().enumerate() {
        keys.push((format!("fixed{}", p), format!("{} gap", name)));
        keys.push((format!("follow{}", p), format!("{} battery flag", name)));
        keys.push((format!("soc{}", p), format!("{} battery thresholds", name)));
        keys.push((format!("socms{}", p), format!("{} battery intervals", name)));
        keys.push((format!("log{}", p), format!("{} log gap", name)));
    }
    keys.push(("logblind".into(), "log gap with no altitude source".into()));
    if keys.iter().all(|(key, _)| asked(q, key).is_none()) {
        return Ok(None);
    }
    // A page loaded before the log gaps existed sends everything but them. That is a stale
    // tab, not a half-filled form, and the fix is a reload, so say that and not "no gap"
    let log_keys = keys.iter().filter(|(key, _)| key.starts_with("log"));
    if log_keys.clone().count() > 0 && log_keys.clone().all(|(key, _)| asked(q, key).is_none()) {
        return Err(String::from(
            "the page in this tab is older than the app and sent no flash log gaps: reload the page (Ctrl+Shift+R) and try again",
        ));
    }
    let text = |key: &str, words: &str| asked(q, key).map(str::to_string).ok_or_else(|| format!("no {} given", words));
    let number = |key: &str, words: &str| -> Result<i64, String> {
        let t = text(key, words)?;
        t.trim().parse::<i64>().map_err(|_| format!("the {} has to be a whole number, and {:?} is not", words, t.trim()))
    };
    let list = |key: &str, words: &str| -> Result<Vec<i64>, String> {
        text(key, words)?
            .split(',')
            .map(|v| v.trim().parse::<i64>().map_err(|_| format!("the {} have to be whole numbers, and {:?} is not", words, v.trim())))
            .collect()
    };
    let flag = |key: &str, words: &str| -> Result<bool, String> {
        match text(key, words)?.trim() {
            "0" => Ok(false),
            "1" => Ok(true),
            other => Err(format!("the {} is 0 or 1, and {:?} is neither", words, other)),
        }
    };
    let (csid, sf, dbm) = (number("csid", "board id")?, number("sf", "spreading factor")?, number("dbm", "transmit power")?);
    let mut rates = Vec::new();
    for (p, name) in crate::settings::PHASE_NAMES.iter().enumerate() {
        rates.push(crate::radio::RateRequest {
            fixed_ms: number(&format!("fixed{}", p), &format!("{} gap", name))?,
            follow: flag(&format!("follow{}", p), &format!("{} battery flag", name))?,
            percent: list(&format!("soc{}", p), &format!("{} battery thresholds", name))?,
            interval_ms: list(&format!("socms{}", p), &format!("{} battery intervals", name))?,
        });
    }
    let mut logs = Vec::new();
    for (p, name) in crate::settings::PHASE_NAMES.iter().enumerate() {
        logs.push(number(&format!("log{}", p), &format!("{} log gap", name))?);
    }
    let log_blind_ms = number("logblind", "log gap with no altitude source")?;
    Ok(Some(crate::radio::Request { csid, sf, dbm, rates, logs, log_blind_ms }))
}

// "set for board 9, SF12 at 15 dBm; waiting every 60 s, rising every 10 s, falling back to
// back, landed by battery: ...; log every 1 s, 0.05 s, 0.05 s, 1 s, blind 0.2 s".
// If the slots pair up as the page writes them (waiting = landed, rising = falling) it's
// "ground every 60 s, air every 1 s; log ground every 10 s, air every 0.05 s, blind 0.2 s"
fn settings_words(asked: &crate::settings::Asked) -> String {
    let secs = |ms: u32| format!("{} s", ms as f64 / 1000.0);
    let (r, l) = (&asked.rates, &asked.logs);
    if r[0] == r[3] && r[1] == r[2] && l[0] == l[3] && l[1] == l[2] {
        return format!(
            "set for board {}, SF{} at {} dBm; ground {}, air {}; log ground every {}, air every {}, blind {}",
            asked.csid,
            asked.sf,
            asked.dbm,
            r[0].words(),
            r[1].words(),
            secs(l[0]),
            secs(l[1]),
            secs(asked.log_blind_ms)
        );
    }
    let phases: Vec<String> = crate::settings::PHASE_NAMES
        .iter()
        .zip(asked.rates.iter())
        .map(|(name, r)| format!("{} {}", name, r.words()))
        .collect();
    let logs: Vec<String> = asked.logs.iter().map(|ms| secs(*ms)).collect();
    format!(
        "set for board {}, SF{} at {} dBm; {}; log every {}, blind {}",
        asked.csid,
        asked.sf,
        asked.dbm,
        phases.join(", "),
        logs.join(", "),
        secs(asked.log_blind_ms)
    )
}

// A carried flight image with a board's settings written into its block, and the words
// for them. This is the way a board is configured when there is no flight software tree
// to build from. Checked against the radio table first, because a block the firmware
// turns down does not fail: the board flies on what the image was built with.
fn set_for(image: &[u8], module: &str, radio: &crate::radio::Radio, wanted: &crate::radio::Request) -> Result<(Vec<u8>, String), String> {
    let asked = radio.check(wanted, module)?;
    let (at, _) = crate::settings::writable(image)?;
    let patched = crate::settings::patch(image, at, &asked)?;
    Ok((patched, settings_words(&asked)))
}

// The image a flash was asked to write, what to call it in the steps, and anything about
// it worth a line there. Everything that can be refused without the probe is refused
// here, so the page hears it in the answer and not from a job that has already started.
fn chip_image(
    q: &[(String, String)],
    body: Vec<u8>,
    uid: &str,
    ctx: &Ctx,
) -> Result<(Vec<u8>, String, Option<String>), String> {
    let force = asked(q, "force") == Some("1");
    let (image, label, note) = match (asked(q, "embedded"), asked(q, "saved")) {
        (Some(_), Some(_)) => return Err(String::from("embedded and saved were both given, and a flash writes one image")),
        (Some("flight"), None) => {
            let module = ctx.chips.lock().unwrap().get(uid).and_then(|c| c.module);
            let flight = crate::flight::carried();
            let image = flight.choose(module)?;
            let label = format!("the {} flight image {}", image.module.to_uppercase(), image.id());
            match settings_asked(q)? {
                None => (image.bytes.to_vec(), label, None),
                Some(wanted) => {
                    let (bytes, set) = set_for(image.bytes, image.module, &crate::radio::carried()?, &wanted)?;
                    (bytes, format!("{} {}", label, set), None)
                }
            }
        }
        // sensor bench: no settings block, sends as board 254 so it can't pass for a flight board
        (Some("bench"), None) => {
            let stored = ctx.chips.lock().unwrap().get(uid).and_then(|c| c.module);
            let module = asked(q, "module").and_then(crate::chips::module_word).or(stored).ok_or_else(|| {
                String::from("no radio module given and none remembered for this board: read it first, or say le or hp")
            })?;
            let bytes = crate::chipflash::bench_images()
                .into_iter()
                .find(|(m, _)| *m == module)
                .map(|(_, b)| b)
                .ok_or_else(|| format!("this build carries no {} bench firmware: fsw/tools/emit_firmware.sh makes it", module.to_uppercase()))?;
            (bytes.to_vec(), format!("the {} sensor bench firmware", module.to_uppercase()), None)
        }
        (Some(other), None) => return Err(format!("this build carries no image called {:?}", other)),
        (None, Some(name)) => {
            if !data_file(name) || !is_image(name) {
                return Err(format!("{:?} is not a saved image in the data folder", name));
            }
            let note = crate::chipflash::came_off(name, uid, force, true)?;
            let bytes = std::fs::read(ctx.data.join(name)).map_err(|e| format!("cannot read {}: {}", name, e))?;
            (bytes, name.to_string(), note)
        }
        (None, None) => {
            if body.len() as u64 > MAX_UPLOAD {
                return Err(format!("that file is over {} kB, which is more than any ChipSat holds", MAX_UPLOAD / 1024));
            }
            let name: String = asked(q, "name")
                .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(""))
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            let note = crate::chipflash::came_off(&name, uid, force, false)?;
            let label = if name.is_empty() { String::from("the image that was sent") } else { name };
            (body, label, note)
        }
    };
    if image.is_empty() {
        return Err(String::from("there is no image to write: send the file as the request body, or name a saved or embedded one"));
    }
    Ok((image, label, note))
}

fn from_tree(ctx: &Ctx, part: &'static str) -> crate::chipflash::Make {
    let local = ctx.local.clone();
    Box::new(move |say: &dyn Fn(&str)| {
        let (bytes, id) = local.get_or_refuse(part, say)?;
        Ok((bytes, id.words((crate::logfile::now_ms() / 1000) as u64)))
    })
}

fn chip_flash(url: &str, body: Vec<u8>, ctx: &Ctx) -> Result<(), String> {
    let q = query(url).ok_or_else(|| String::from("the request's query string does not read"))?;
    let uid = asked(&q, "uid").ok_or_else(|| String::from("no uid in the request, so there is no saying which board this is for"))?;
    let uid = crate::stm32::a_uid(uid).ok_or_else(|| format!("{:?} is not a board id, which is 24 hex characters", uid))?;
    // off by default, same as receivers (decided 2026-10-21)
    let backup = asked(&q, "backup") == Some("1");
    let probe = asked(&q, "probe").filter(|p| !p.is_empty()).map(String::from);
    // The compile runs in the job, so only what can be checked without it is checked here.
    // With a tree, flight and bench images come from it too (localfw.rs)
    let embedded = asked(&q, "embedded");
    let on = ctx.local.tree().is_some();
    let (image, label, note, build) = if embedded == Some("built") || (on && embedded == Some("flight")) {
        ctx.local.tree().ok_or_else(|| {
            String::from(
                "no flight software tree beside this program. It is the folder holding build.sh and \
                 V2_6_X_Code, usually fsw/ in the same repository, or set CHIPSAT_FSW to it",
            )
        })?;
        let stored = ctx.chips.lock().unwrap().get(&uid).and_then(|c| c.module);
        let module = asked(&q, "module")
            .and_then(crate::chips::module_word)
            .or(stored)
            .ok_or_else(|| String::from("no radio module given and none remembered for this board"))?;
        // checked against the tree's own radio table before any build
        let wanted = settings_asked(&q)?.ok_or_else(|| String::from("no settings given to build with"))?;
        let (radio, radio_note) = ctx.local.radio(true)?;
        let asked = radio.check(&wanted, module)?;
        let words = format!("a flight image for board {} on the {} module, {}", asked.csid, module.to_uppercase(), settings_words(&asked).trim_start_matches(&format!("set for board {}, ", asked.csid)));
        let part = crate::localfw::part_for("flight", module).ok_or_else(|| format!("{:?} is not a radio module", module))?;
        let label = words.clone();
        (Vec::new(), label, radio_note, Some(crate::chipflash::BuildPlan { make: from_tree(ctx, part.name), asked: Some(asked), words }))
    } else if on && embedded == Some("bench") {
        let stored = ctx.chips.lock().unwrap().get(&uid).and_then(|c| c.module);
        let module = asked(&q, "module").and_then(crate::chips::module_word).or(stored).ok_or_else(|| {
            String::from("no radio module given and none remembered for this board: read it first, or say le or hp")
        })?;
        let part = crate::localfw::part_for("bench", module).ok_or_else(|| format!("{:?} is not a radio module", module))?;
        let words = format!("the {} sensor bench firmware", module.to_uppercase());
        (Vec::new(), words.clone(), None, Some(crate::chipflash::BuildPlan { make: from_tree(ctx, part.name), asked: None, words }))
    } else {
        let (i, l, n) = chip_image(&q, body, &uid, ctx)?;
        (i, l, n, None)
    };

    let held = ctx.gate.take(&format!("flashing board {}", uid))?;
    println!(
        "flashing {} onto board {}{}",
        label,
        uid,
        if backup { ", saving what is on it first" } else { ", NOT saving what is on it" }
    );
    crate::chipflash::start(
        crate::chipflash::Job {
            shared: ctx.chip.clone(),
            registry: ctx.chips.clone(),
            dir: ctx.data.clone(),
            probe,
            uid,
            backup,
            image,
            label,
            note,
            known: crate::chipflash::Known::with(&ctx.local),
            build,
        },
        held,
    );
    Ok(())
}

fn handle(mut request: Request, ctx: Arc<Ctx>) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();

    if path == "/api/stream" {
        if let Some(key) = is_websocket(&request) {
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            let response = Response::empty(101)
                .with_header(header("Upgrade", "websocket"))
                .with_header(header("Connection", "Upgrade"))
                .with_header(header("Sec-WebSocket-Accept", &accept));
            let stream = request.upgrade("websocket", response);
            let socket: Socket = tungstenite::WebSocket::from_raw_socket(stream, Role::Server, None);
            ctx.broadcast.add(socket);
            return;
        }
        let _ = request.respond(Response::from_string("not a websocket request").with_status_code(400));
        return;
    }

    // What a launcher polls until the app is up, and the one place it can read the
    // version it is talking to.
    if path == "/api/health" {
        let (boards, receivers) = match ctx.hub.lock() {
            Ok(h) => (h.boards().len(), h.ports().len()),
            Err(_) => (0, 0),
        };
        let log = ctx
            .log
            .lock()
            .ok()
            .and_then(|l| l.as_ref().map(|l| l.path().display().to_string()));
        let modified = ctx.exe.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        let body = health_json(
            env!("CARGO_PKG_VERSION"),
            ctx.port,
            log.as_deref(),
            boards,
            receivers,
            crate::opts::built(),
            newer_on_disk(ctx.started, modified),
            // decides build-and-flash vs patching the carried image
            ctx.local.tree().is_some(),
        );
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // How a launcher stops us without keeping a pid. Only reachable from this machine,
    // because the listener is bound to 127.0.0.1.
    if path == "/api/quit" {
        if request.method().as_str() != "POST" {
            let _ = request.respond(
                Response::from_string("{\"ok\":false,\"error\":\"POST only\"}")
                    .with_status_code(405)
                    .with_header(header("Content-Type", "application/json")),
            );
            return;
        }
        let _ = request.respond(
            Response::from_string("{\"ok\":true}").with_header(header("Content-Type", "application/json")),
        );
        // respond() has written the answer, but the other end still has to read it off
        // the socket before we close everything by leaving.
        std::thread::sleep(Duration::from_millis(100));
        shutdown(&ctx, "stopping: asked to over /api/quit");
    }

    if path == "/api/ports" {
        let (ports, boards) = match ctx.hub.lock() {
            Ok(h) => (h.ports(), h.boards()),
            Err(_) => (Vec::new(), Vec::new()),
        };
        // Asked of the log itself rather than remembered, so a rotation shows up here.
        let log = ctx
            .log
            .lock()
            .ok()
            .and_then(|l| l.as_ref().map(|l| l.path().display().to_string()));
        let body = ports_json(&ports, &boards, log.as_deref());
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // What this computer calls a receiver, against the id it prints and not the port it
    // is on today. A receiver whose firmware takes a name is told it as well, and
    // "onBoard" says whether that happened.
    if path == "/api/receiver/name" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let done = match (crate::json::str_field(&body, "id"), crate::json::text_field(&body, "name")) {
            (None, _) => Err(String::from("no id in the request")),
            (_, None) => Err(String::from("no name in the request")),
            (Some(id), Some(name)) => ctx.hub.lock().unwrap().set_receiver_name(&id, &name),
        };
        let text = match done {
            Ok(on_board) => format!("{{\"ok\":true,\"onBoard\":{}}}", on_board),
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    if let Some(rest) = path.strip_prefix("/api/receiver/") {
        let mut it = rest.splitn(2, '/');
        let key = it.next().unwrap_or("").to_string();
        let action = it.next().unwrap_or("");
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let result = match action {
            "get" => ctx.hub.lock().unwrap().send(&key, "#GET\n"),
            "close" => {
                ctx.hub.lock().unwrap().dismiss(&key);
                Ok(())
            }
            "config" => match int_field(&body, "sf") {
                Some(sf) if (6..=12).contains(&sf) => {
                    ctx.hub.lock().unwrap().send(&key, &format!("#SET,sf,{}\n", sf))
                }
                Some(sf) => Err(format!("spreading factor {} is not between 6 and 12", sf)),
                None => Err(String::from("no spreading factor in the request")),
            },
            other => Err(format!("no such action: {}", other)),
        };
        let (code, text) = match result {
            Ok(()) => (200, String::from("{\"ok\":true}")),
            Err(e) => (400, format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e))),
        };
        let _ = request.respond(
            Response::from_string(text)
                .with_status_code(code)
                .with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // The operator saying what a board is. Nothing is opened before this, because
    // opening a port pulses DTR and can reset the board on the other end.
    if path == "/api/board/receiver" || path == "/api/board/ignore" {
        let receiver = path.ends_with("receiver");
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let text = match crate::json::str_field(&body, "port") {
            Some(p) => {
                let mut hub = ctx.hub.lock().unwrap();
                let r = if receiver { hub.approve(&p) } else { hub.ignore(&p) };
                match r {
                    Ok(()) => {
                        println!("{} is {}", p, if receiver { "a receiver" } else { "left alone" });
                        String::from("{\"ok\":true}")
                    }
                    Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
                }
            }
            None => String::from("{\"ok\":false,\"error\":\"no port in the request\"}"),
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // Forgetting a board is forgetting what this computer keeps about it. Nothing is sent
    // to the board.
    if path == "/api/board/forget" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let done = match crate::json::str_field(&body, "port") {
            None => Err(String::from("no port in the request")),
            Some(p) => ctx.hub.lock().unwrap().forget(&p).map(|()| println!("{} is forgotten", p)),
        };
        let text = match done {
            Ok(()) => String::from("{\"ok\":true}"),
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    // A name the operator can read, against the board and not the port. An empty name
    // clears it.
    if path == "/api/board/name" {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let text = match (crate::json::str_field(&body, "port"), crate::json::str_field(&body, "name")) {
            (Some(p), Some(name)) => {
                match ctx.hub.lock().unwrap().set_name(&p, &name) {
                    Ok(()) => {
                        println!("{} is {}", p, if name.trim().is_empty() { "nameless again" } else { name.trim() });
                        String::from("{\"ok\":true}")
                    }
                    Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
                }
            }
            (None, _) => String::from("{\"ok\":false,\"error\":\"no port in the request\"}"),
            (_, None) => String::from("{\"ok\":false,\"error\":\"no name in the request\"}"),
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    if path == "/api/board/identify/stop" {
        let Some(request) = post_only(request) else { return };
        ctx.hub.lock().unwrap().identify_stop();
        return answer(request, String::from("{\"ok\":true}"));
    }

    // Which board is which, without touching any of them: the operator unplugs the one
    // they mean and plugs it back in, and discovery sees it go and come back. POST
    // starts a watch, GET asks how it is going, and both answer the same shape.
    if path == "/api/board/identify" {
        let start = request.method().as_str() == "POST";
        let (watching, found, left) = {
            let mut hub = ctx.hub.lock().unwrap();
            if start {
                hub.identify_start();
                println!("identify: unplug the board you mean, then plug it back in");
            }
            hub.identify_state()
        };
        let body = format!(
            "{{\"watching\":{},\"found\":{},\"seconds_left\":{}}}",
            watching,
            match &found {
                Some(p) => format!("\"{}\"", esc(p)),
                None => String::from("null"),
            },
            left
        );
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // Five seconds of listening to an unknown board. Nothing is sent to it, not even
    // #GET: this is the call for a board nobody has vouched for yet. It blocks for
    // those five seconds, which is fine because every request has its own thread.
    if path == "/api/board/test" {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let text = match crate::json::str_field(&body, "port") {
            None => String::from("{\"ok\":false,\"error\":\"no port in the request\"}"),
            Some(p) if flight_controller(&ctx, &p).is_some() => failed(&flight_controller(&ctx, &p).unwrap()),
            Some(p) => {
                // The hub lock is let go before the read, or poll() stops for five seconds.
                let held = ctx.hub.lock().unwrap().begin_probe(&p);
                match held {
                    Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
                    Ok(()) => {
                        let heard = crate::serial::probe(&p);
                        ctx.hub.lock().unwrap().end_probe(&p);
                        match heard {
                            Ok((lines, bytes)) => {
                                // Judged on everything it said, shown a readable slice of it.
                                let what = crate::serial::looks_like(&lines, bytes);
                                println!("{} sounds like {} ({} lines)", p, what, lines.len());
                                let items: Vec<String> = lines
                                    .iter()
                                    .take(crate::serial::PROBE_LINES)
                                    .map(|l| format!("\"{}\"", esc(&crate::serial::clip(l, crate::serial::PROBE_CHARS))))
                                    .collect();
                                format!(
                                    "{{\"ok\":true,\"lines\":[{}],\"looks_like\":\"{}\"}}",
                                    items.join(","),
                                    what
                                )
                            }
                            Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
                        }
                    }
                }
            }
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // Start a new log without restarting the app, for a second drop on the same day.
    // Map tiles the browser saved while it had a connection.
    if path.starts_with("/tiles/") {
        match crate::tiles::parse(&path) {
            Some((z, x, y)) => match crate::tiles::read(&ctx.dir, z, x, y) {
                Some(bytes) => {
                    let _ = request.respond(
                        Response::from_data(bytes)
                            .with_header(header("Content-Type", "image/png"))
                            .with_header(header("Cache-Control", "max-age=604800")),
                    );
                }
                // Missing is ordinary: the page draws an empty square and carries on.
                None => {
                    let _ = request.respond(Response::from_string("no tile").with_status_code(404));
                }
            },
            None => {
                let _ = request.respond(Response::from_string("not a tile").with_status_code(400));
            }
        }
        return;
    }

    if path.starts_with("/api/tile/") {
        let as_tile = path.replacen("/api/tile/", "/tiles/", 1);
        let (z, x, y) = match crate::tiles::parse(&as_tile) {
            Some(t) => t,
            None => {
                let _ = request.respond(Response::from_string("not a tile").with_status_code(400));
                return;
            }
        };
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut request.as_reader(), &mut bytes);
        let body = match crate::tiles::write(&ctx.dir, z, x, y, &bytes) {
            Ok(()) => String::from("{\"ok\":true}"),
            Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
        };
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    if path == "/api/tiles" {
        let (count, bytes) = crate::tiles::stored(&ctx.dir);
        let _ = request.respond(
            Response::from_string(format!(
                "{{\"tiles\":{},\"bytes\":{},\"dir\":\"{}\"}}",
                count,
                bytes,
                esc(&crate::tiles::dir(&ctx.dir).to_string_lossy())
            ))
            .with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    if path == "/api/logs" {
        let _ = request.respond(
            Response::from_string(logs_json(&ctx.data, &ctx.chips.lock().unwrap()))
                .with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // A log off the data folder, so the page can replay what was written there without
    // anyone standing up a second web server to hand it over. Read-only, that folder and
    // the pull folders in it only, no walking out of it.
    if let Some(asked) = path.strip_prefix("/api/log/").filter(|n| *n != "rotate") {
        let Some(name) = log_name(asked) else {
            let _ = request.respond(Response::from_string("no").with_status_code(400));
            return;
        };
        match std::fs::read(ctx.data.join(&name)) {
            Ok(body) => {
                let _ = request.respond(
                    Response::from_data(body)
                        .with_header(header("Content-Type", content_type(&name)))
                        .with_header(header(
                            "Content-Disposition",
                            &format!("inline; filename=\"{}\"", save_name(&name)),
                        )),
                );
            }
            Err(e) => {
                let _ = request.respond(
                    Response::from_string(format!("{}: {}", name, e)).with_status_code(404),
                );
            }
        }
        return;
    }

    if path == "/api/log/rotate" {
        let text = match logfile::rotate(&ctx.log, &ctx.data) {
            Ok(p) => {
                println!("logging every line to {}", p.display());
                format!("{{\"ok\":true,\"log\":\"{}\"}}", esc(&p.to_string_lossy()))
            }
            Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e)),
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // Pulling a flight log off a chip. The app runs what it was told to run and shows what
    // that says; it does not know an ST-Link from a cross compiler and is not going to.
    // Pull a flight log off a ChipSat. Everything happens in this program: the debug
    // probe is driven directly and the dump firmware is carried inside the binary, so
    // there is nothing to install and nothing to configure.
    if path == "/api/pull" {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let board = crate::json::str_field(&body, "port").unwrap_or_default();
        let wipe = crate::json::int_field(&body, "wipe").unwrap_or(0) != 0;
        // unticked by default, nothing is saved unless asked
        let keep_fsw = crate::json::int_field(&body, "keepFsw").unwrap_or(0) != 0;
        let probe = crate::json::str_field(&body, "probe").filter(|p| !p.is_empty());
        // anyConsole is Pull anyway
        let check_console = crate::json::int_field(&body, "anyConsole").unwrap_or(0) == 0;
        // board the page thinks is on the probe. The pull checks it and names saves after it
        let asked_uid = crate::json::str_field(&body, "uid").filter(|u| !u.is_empty());
        let uid = asked_uid.as_deref().and_then(crate::stm32::a_uid);
        let name = uid
            .as_ref()
            .and_then(|u| ctx.chips.lock().unwrap().get(u).map(|c| c.name.clone()))
            .filter(|n| !n.is_empty());

        // tree's dump is fetched inside the job, it may need building
        let on = ctx.local.tree().is_some();
        let dump = if on { Some(&[][..]) } else { crate::flash::CHIPSAT_DUMP };
        let text = match dump {
            None => String::from("{\"ok\":false,\"error\":\"this build carries no dump firmware\"}"),
            Some(_) if board.is_empty() => String::from("{\"ok\":false,\"error\":\"no board in the request\"}"),
            Some(_) if flight_controller(&ctx, &board).is_some() => failed(&flight_controller(&ctx, &board).unwrap()),
            Some(_) if asked_uid.is_some() && uid.is_none() => {
                failed(&format!("{:?} is not a board id, which is 24 hex characters", asked_uid.unwrap_or_default()))
            }
            Some(image) => match ctx.gate.take(&format!("{} {}", if wipe { "clearing the flash log on" } else { "pulling the log from" }, board)) {
                Err(e) => failed(&e),
                Ok(held) => {
                    // The board is taken off discovery the same way a flash takes it, so
                    // nothing else opens the port while the probe is resetting it.
                    let released = ctx.hub.lock().unwrap().release_for_flash(&board);
                    match released.and_then(|()| {
                        crate::pull::start(
                            crate::pull::Job {
                                shared: ctx.pull.clone(),
                                broadcast: ctx.broadcast.clone(),
                                dir: ctx.data.clone(),
                                port_name: board.clone(),
                                probe,
                                wipe,
                                keep_fsw,
                                dump_image: image.to_vec(),
                                dump_from: if on { String::new() } else { ctx.local.use_json(&crate::localfw::Use::Carried(image, None), "flash_dump") },
                                prepare: on.then(|| {
                                    let local = ctx.local.clone();
                                    Box::new(move |say: &dyn Fn(&str)| {
                                        let u = local.pick("flash_dump", true, say)?;
                                        let note = match &u {
                                            crate::localfw::Use::Carried(_, why) => why.clone(),
                                            crate::localfw::Use::Tree(_, id) => Some(format!("using the {}", id.words((crate::logfile::now_ms() / 1000) as u64))),
                                        };
                                        Ok((u.bytes().to_vec(), local.use_json(&u, "flash_dump"), note))
                                    }) as crate::pull::Prepare
                                }),
                                known: Some(crate::chipflash::Known::with(&ctx.local)),
                                uid,
                                name,
                                check_console,
                            },
                            held,
                        )
                    }) {
                        Ok(()) => {
                            let hub = ctx.hub.clone();
                            let pull = ctx.pull.clone();
                            let board2 = board.clone();
                            std::thread::spawn(move || {
                                loop {
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    if !pull.lock().map(|p| p.running).unwrap_or(false) {
                                        break;
                                    }
                                }
                                hub.lock().unwrap().take_back(&board2);
                            });
                            println!("pulling from {}{}", board, if keep_fsw { ", keeping the flight software" } else { ", NOT keeping the flight software" });
                            String::from("{\"ok\":true}")
                        }
                        Err(e) => {
                            ctx.hub.lock().unwrap().take_back(&board);
                            format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e))
                        }
                    }
                }
            },
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // R toggles the sensor bench radio. Only ever sends that one byte
    if path == "/api/bench/radio" {
        let Some(request) = post_only(request) else { return };
        let port = query(&url).and_then(|q| asked(&q, "port").map(String::from)).unwrap_or_default();
        let text = {
            let mut hub = ctx.hub.lock().unwrap();
            let key = hub.boards().into_iter().find(|b| b.port == port).map(|b| b.key);
            match key {
                None => failed(&format!("{:?} is not plugged in", port)),
                Some(key) => match hub.send(&key, "R") {
                    Ok(()) => String::from("{\"ok\":true}"),
                    Err(e) => failed(&format!("{}: is the port being read? {}", port, e)),
                },
            }
        };
        return answer(request, text);
    }

    // ends like a failed pull. A pull told not to keep the flight software stays on the dump firmware
    if path == "/api/pull/stop" {
        let Some(request) = post_only(request) else { return };
        let text = match crate::pull::stop(&ctx.pull) {
            Ok(()) => {
                println!("stop asked for on the pull from {}", ctx.pull.lock().unwrap().board);
                String::from("{\"ok\":true}")
            }
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    // Asked for, not pushed: a reloaded page or a reopened panel picks the job back up.
    if path == "/api/pull/state" {
        let body = {
            let p = ctx.pull.lock().unwrap();
            crate::pull::state_json(&p, crate::logfile::now_ms())
        };
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    if path == "/api/flash/candidates" {
        let body = {
            let hub = ctx.hub.lock().unwrap();
            flash::candidates_json(&hub)
        };
        let _ = request.respond(
            Response::from_string(body).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    if path == "/api/flash" {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let text = match crate::json::str_field(&body, "port").and_then(|p| flight_controller(&ctx, &p)) {
            Some(e) => failed(&e),
            None => flash::handle_request(&body, &ctx),
        };
        let _ = request.respond(
            Response::from_string(text).with_header(header("Content-Type", "application/json")),
        );
        return;
    }

    // Debug probes on USB. Only what the bus says about them: nothing is opened, so
    // asking does not disturb a board that is on one.
    if path == "/api/chip/probes" {
        return answer(request, probes_json(&crate::stm32::probes()));
    }

    if path == "/api/chips" {
        let body = ctx.chips.lock().unwrap().json();
        return answer(request, body);
    }

    // What the operator calls a ChipSat, and which module is soldered to it. Neither can
    // be read off the board, so both are kept against its id.
    if path == "/api/chip/name" || path == "/api/chip/module" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let what = if path.ends_with("name") { "name" } else { "module" };
        let value = if what == "name" { crate::json::text_field(&body, what) } else { crate::json::str_field(&body, what) };
        let done = match (crate::json::str_field(&body, "uid"), value) {
            (None, _) => Err(String::from("no uid in the request")),
            (_, None) => Err(format!("no {} in the request", what)),
            (Some(uid), Some(value)) => {
                let mut chips = ctx.chips.lock().unwrap();
                if what == "name" { chips.set_name(&uid, &value) } else { chips.set_module(&uid, &value) }
            }
        };
        let text = match done {
            Ok(()) => String::from("{\"ok\":true}"),
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    // Drops a ChipSat from the list. The board is not touched and neither are the images
    // and logs saved off it.
    if path == "/api/chip/forget" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let done = match crate::json::str_field(&body, "uid") {
            None => Err(String::from("no uid in the request")),
            Some(uid) => ctx.chips.lock().unwrap().forget(&uid),
        };
        let text = match done {
            Ok(()) => String::from("{\"ok\":true}"),
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    // Which board is on the probe. It answers when the board has been read, a few
    // seconds, which is fine because every request has its own thread.
    if path == "/api/chip/read" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let probe = crate::json::str_field(&body, "probe").filter(|p| !p.is_empty());
        let text = match ctx.gate.take("reading a board") {
            Err(e) => failed(&e),
            Ok(_held) => crate::chipflash::read(
                &ctx.chip,
                &ctx.chips,
                &crate::chipflash::StLink { probe: probe.as_deref() },
                &crate::chipflash::Known::with(&ctx.local),
            ),
        };
        return answer(request, text);
    }

    // Which board is under the clip, asked without the reset a read can fall back to, so
    // the page can ask every couple of seconds. uid is null when nothing answered, and
    // when a job has the probe.
    if path == "/api/chip/peek" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let probe = crate::json::str_field(&body, "probe").filter(|p| !p.is_empty());
        let uid = match ctx.gate.take("looking at which board is on the probe") {
            Err(_) => None,
            Ok(_held) => crate::stm32::peek(probe.as_deref()),
        };
        let uid = uid.map(|u| format!("\"{}\"", esc(&u))).unwrap_or_else(|| String::from("null"));
        return answer(request, format!("{{\"ok\":true,\"uid\":{}}}", uid));
    }

    if path == "/api/chip/flash" {
        let Some(mut request) = post_only(request) else { return };
        let mut body = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(request.as_reader(), MAX_UPLOAD + 1), &mut body);
        let text = match chip_flash(&url, body, &ctx) {
            Ok(()) => String::from("{\"ok\":true}"),
            Err(e) => failed(&e),
        };
        return answer(request, text);
    }

    // Asked for, not pushed, like the pull's: a reloaded page picks the job back up.
    if path == "/api/chip/job" {
        let body = {
            let s = ctx.chip.lock().unwrap();
            crate::chipflash::state_json(&s, crate::logfile::now_ms())
        };
        return answer(request, body);
    }

    // What the flight software will accept for a radio, as its own build wrote it down.
    // tree's table when there is a tree
    if path == "/api/radio" {
        match ctx.local.radio_text(true) {
            Ok((table, _)) => answer(request, table),
            Err(e) => {
                let _ = request.respond(Response::from_string(e).with_status_code(404));
            }
        }
        return;
    }

    if path == "/api/fsw" {
        return answer(request, ctx.local.state_json());
    }

    match assets::get(&path) {
        Some((body, ct)) => {
            // Revalidated on every load, so a restarted app never serves a page that a tab
            // cached from the build before it. Local and small: the cost is nothing
            let _ = request.respond(
                Response::from_data(body)
                    .with_header(header("Content-Type", ct))
                    .with_header(header("Cache-Control", "no-cache")),
            );
        }
        None => {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets;

    #[test]
    fn the_dashboard_is_embedded_with_the_right_content_types() {
        let (body, ct) = assets::get("/").unwrap();
        assert!(String::from_utf8_lossy(body).contains("DeSCENT"));
        assert_eq!(ct, "text/html; charset=utf-8");
        assert_eq!(assets::get("/js/packet.js").unwrap().1, "text/javascript");
        assert_eq!(assets::get("/css/style.css").unwrap().1, "text/css");
        assert_eq!(
            assets::get("/vendor/fonts/ibm-plex-mono-latin-400-normal.woff2").unwrap().1,
            "font/woff2"
        );
        assert!(assets::get("/../Cargo.toml").is_none());
        assert!(assets::get("/nope.js").is_none());
    }

    #[test]
    fn ports_json_is_shaped_the_way_the_page_expects() {
        let ports = vec![PortInfo {
            key: "rx1".into(),
            port: "/dev/ttyUSB0".into(),
            usb: "10c4:ea60".into(),
            status: "open".into(),
            error: None,
            rx_id: None,
            rx_name: None,
            rx_fw: None,
            name: None,
        }];
        let boards = vec![Board {
            port: "/dev/ttyACM0".into(),
            usb: "1a86:55d4".into(),
            label: "CH9102".into(),
            serial: "58A1".into(),
            key: "1a86:55d4:58A1".into(),
            state: "waiting".into(),
            name: "Left T-Beam".into(),
        }];
        let json = ports_json(&ports, &boards, Some("/tmp/a.log"));
        assert_eq!(json, "{\"ports\":[{\"key\":\"rx1\",\"port\":\"/dev/ttyUSB0\",\"usb\":\"10c4:ea60\",\"status\":\"open\",\"error\":null,\"rxId\":null,\"rxName\":null,\"rxFw\":null,\"name\":null}],\"boards\":[{\"port\":\"/dev/ttyACM0\",\"usb\":\"1a86:55d4\",\"label\":\"CH9102\",\"serial\":\"58A1\",\"state\":\"waiting\",\"name\":\"Left T-Beam\",\"rxId\":null,\"rxName\":null,\"rxFw\":null,\"rxLocalName\":null}],\"log\":\"/tmp/a.log\"}");
    }

    #[test]
    fn a_port_being_read_says_which_receiver_is_on_it_and_what_it_is_called() {
        let ports = vec![PortInfo {
            key: "rx2".into(),
            port: "/dev/ttyUSB0".into(),
            usb: "10c4:ea60".into(),
            status: "open".into(),
            error: None,
            rx_id: Some("A4C1F3".into()),
            rx_name: Some("on the \"board\"".into()),
            rx_fw: Some("1.2.1".into()),
            name: Some("01".into()),
        }];
        let board = |port: &str, name: &str| Board {
            port: port.into(),
            usb: "10c4:ea60".into(),
            label: "CP2102".into(),
            serial: "0001".into(),
            key: "10c4:ea60:0001".into(),
            state: "receiver".into(),
            name: name.into(),
        };
        let json = ports_json(&ports, &[board("/dev/ttyUSB0", "Left T-Beam"), board("/dev/ttyUSB1", "")], None);
        assert!(
            json.contains("{\"key\":\"rx2\",\"port\":\"/dev/ttyUSB0\",\"usb\":\"10c4:ea60\",\"status\":\"open\",\"error\":null,\"rxId\":\"A4C1F3\",\"rxName\":\"on the \\\"board\\\"\",\"rxFw\":\"1.2.1\",\"name\":\"01\"}"),
            "{}",
            json
        );
        // The board on that port is the same receiver. What it was already called by its
        // USB serial stays under "name", so the name kept against the id has its own key.
        assert!(
            json.contains("\"state\":\"receiver\",\"name\":\"Left T-Beam\",\"rxId\":\"A4C1F3\",\"rxName\":\"on the \\\"board\\\"\",\"rxFw\":\"1.2.1\",\"rxLocalName\":\"01\"}"),
            "{}",
            json
        );
        // A board nobody is reading has said nothing about itself.
        assert!(json.contains("\"port\":\"/dev/ttyUSB1\",\"usb\":\"10c4:ea60\",\"label\":\"CP2102\",\"serial\":\"0001\",\"state\":\"receiver\",\"name\":\"\",\"rxId\":null,\"rxName\":null,\"rxFw\":null,\"rxLocalName\":null}"), "{}", json);
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()
    }

    #[test]
    fn the_settings_for_a_carried_image_come_all_or_not_at_all() {
        assert_eq!(settings_asked(&pairs(&[("uid", "x"), ("embedded", "flight")])).unwrap(), None);
        let mut all: Vec<(&str, &str)> = vec![("csid", "9"), ("sf", "12"), ("dbm", "15")];
        let per = [("fixed0", "60000"), ("follow0", "0"), ("soc0", "75,50,35,20"), ("socms0", "10000,30000,60000,120000,180000"),
            ("fixed1", "10000"), ("follow1", "0"), ("soc1", "75,50,35,20"), ("socms1", "10000,30000,60000,120000,180000"),
            ("fixed2", "0"), ("follow2", "1"), ("soc2", "90, 60,30,5"), ("socms2", "1,2,3,4,5"),
            ("fixed3", "60000"), ("follow3", "0"), ("soc3", "75,50,35,20"), ("socms3", "10000,30000,60000,120000,180000"),
            ("log0", "1000"), ("log1", "50"), ("log2", "50"), ("log3", "1000"), ("logblind", "200")];
        all.extend_from_slice(&per);
        let got = settings_asked(&pairs(&all)).unwrap().unwrap();
        assert_eq!((got.csid, got.sf, got.dbm, got.rates.len()), (9, 12, 15, 4));
        assert_eq!((got.logs.clone(), got.log_blind_ms), (vec![1000, 50, 50, 1000], 200));
        let e = settings_asked(&pairs(&all[..all.len() - 1])).unwrap_err();
        assert!(e.contains("no log gap with no altitude source"), "{}", e);
        let e = settings_asked(&pairs(&all[..all.len() - 3])).unwrap_err();
        assert!(e.contains("no falling log gap"), "{}", e);
        assert_eq!(got.rates[2], crate::radio::RateRequest { fixed_ms: 0, follow: true, percent: vec![90, 60, 30, 5], interval_ms: vec![1, 2, 3, 4, 5] });
        assert_eq!(got.rates[0].fixed_ms, 60000);
        let e = settings_asked(&pairs(&all[..3])).unwrap_err();
        assert!(e.contains("older than the app") && e.contains("reload"), "{}", e);
        // Half a form with the log gaps present is a missing field, named.
        let mut half: Vec<(&str, &str)> = all[..7].to_vec();
        half.extend_from_slice(&[("log0", "1000"), ("log1", "50"), ("log2", "50"), ("log3", "1000"), ("logblind", "200")]);
        let e = settings_asked(&pairs(&half)).unwrap_err();
        assert!(e.contains("no rising gap"), "{}", e);
        let mut bad = all.clone();
        bad[0] = ("csid", "");
        let e = settings_asked(&pairs(&bad)).unwrap_err();
        assert!(e.contains("board id") && e.contains("whole number"), "{}", e);
        let mut bad = all.clone();
        bad[12] = ("follow2", "yes");
        assert!(settings_asked(&pairs(&bad)).unwrap_err().contains("falling battery flag is 0 or 1"));
        let mut bad = all.clone();
        bad[13] = ("soc2", "90,6x,30,5");
        assert!(settings_asked(&pairs(&bad)).unwrap_err().contains("whole numbers"));
    }

    #[test]
    fn a_carried_image_is_given_a_boards_settings_and_is_still_that_image() {
        // Whatever fsw/firmware held at build time. With no images there is nothing to try.
        let flight = crate::flight::carried();
        let Ok(radio) = crate::radio::carried() else { return };
        let known = crate::chipflash::Known::carried();
        let rr = |fixed_ms: i64| crate::radio::RateRequest { fixed_ms, follow: false, percent: vec![75, 50, 35, 20], interval_ms: vec![10_000, 30_000, 60_000, 120_000, 180_000] };
        for image in flight.usable() {
            let power = if image.module == "hp" { 22 } else { 15 };
            let wanted = crate::radio::Request { csid: 9, sf: 12, dbm: power, rates: vec![rr(60_000), rr(500), rr(500), rr(60_000)], logs: vec![1000, 50, 50, 1000], log_blind_ms: 200 };
            let (bytes, words) = set_for(image.bytes, image.module, &radio, &wanted).unwrap();
            assert_eq!(words, format!("set for board 9, SF12 at {} dBm; ground every 60 s, air every 0.5 s; log ground every 1 s, air every 0.05 s, blind 0.2 s", power));
            assert_eq!(bytes.len(), image.bytes.len());
            match crate::settings::find_at(&bytes).unwrap() {
                (crate::settings::Found::Block(s), Some(_)) => {
                    assert_eq!((s.csid, s.module, s.sf, s.dbm as i64, s.set_by, s.tx_interval_ms), (9, image.module, 12, power, "tool", 500));
                }
                other => panic!("{}: {:?}", image.file, other),
            }
            // With a phase on the battery, the words say what that phase will follow.
            let mut on = wanted.clone();
            on.rates[3].follow = true;
            assert_eq!(
                set_for(image.bytes, image.module, &radio, &on).unwrap().1,
                format!("set for board 9, SF12 at {} dBm; waiting every 60 s, rising every 0.5 s, falling every 0.5 s, landed by battery: 10 s above 75 %, 30 s to 50 %, 60 s to 35 %, 120 s to 20 %, then 180 s (60 s when the gauge cannot be read); log every 1 s, 0.05 s, 0.05 s, 1 s, blind 0.2 s", power)
            );
            // A read of the board still says which image it is and which module it is for.
            let seen = crate::chipflash::recognise(&bytes, &known);
            assert_eq!((seen.kind, seen.module, seen.id.as_deref()), ("flight", Some(image.module), Some(image.id())));

            // What the firmware would turn down is refused here, in words, and nothing comes back.
            let other_modules = if image.module == "hp" { 10 } else { 20 };
            let e = set_for(image.bytes, image.module, &radio, &crate::radio::Request { dbm: other_modules, ..wanted.clone() }).unwrap_err();
            assert!(e.contains(&image.module.to_uppercase()) && e.contains("dBm"), "{}", e);
            assert!(set_for(image.bytes, image.module, &radio, &crate::radio::Request { csid: 0, ..wanted.clone() }).is_err());
            let mut btb = wanted.clone();
            btb.rates[2].fixed_ms = 0;
            assert_eq!(set_for(image.bytes, image.module, &radio, &btb).unwrap().1,
                format!("set for board 9, SF12 at {} dBm; waiting every 60 s, rising every 0.5 s, falling back to back, landed every 60 s; log every 1 s, 0.05 s, 0.05 s, 1 s, blind 0.2 s", power));
        }
    }

    #[test]
    fn the_settings_are_said_as_two_phases_only_when_the_slots_are_paired() {
        let rate = |fixed_ms: u32| crate::settings::Rate { fixed_ms, follow: false, percent: [75, 50, 35, 20], interval_ms: [10_000, 30_000, 60_000, 120_000, 180_000] };
        let asked = crate::settings::Asked { csid: 9, sf: 7, dbm: 14, rates: [rate(60_000), rate(1000), rate(1000), rate(60_000)], logs: [10_000, 50, 50, 10_000], log_blind_ms: 200 };
        assert_eq!(settings_words(&asked), "set for board 9, SF7 at 14 dBm; ground every 60 s, air every 1 s; log ground every 10 s, air every 0.05 s, blind 0.2 s");
        // landed log gap differs from waiting, so slot by slot
        let apart = crate::settings::Asked { logs: [10_000, 50, 50, 1000], ..asked.clone() };
        assert_eq!(settings_words(&apart), "set for board 9, SF7 at 14 dBm; waiting every 60 s, rising every 1 s, falling every 1 s, landed every 60 s; log every 10 s, 0.05 s, 0.05 s, 1 s, blind 0.2 s");
    }

    #[test]
    fn health_is_shaped_the_way_a_launcher_reads_it() {
        let id = crate::flash::CHIPSAT_DUMP_ID;
        let flight = |fsw: bool| {
            format!(
                "\"flightImages\":{},\"flightBuild\":{},\"fswTree\":{}",
                crate::flight::carried().images_json(),
                crate::flight::carried().build_json(),
                fsw
            )
        };
        assert_eq!(
            health_json("0.4.0", 8790, Some("/tmp/a.log"), 2, 1, 1_791_216_000, false, true),
            format!("{{\"ok\":true,\"version\":\"0.4.0\",\"built\":1791216000,\"newerOnDisk\":false,\"port\":8790,\"dumpFirmware\":\"{}\",{},\"log\":\"/tmp/a.log\",\"boards\":2,\"receivers\":1}}", id, flight(true))
        );
        assert_eq!(
            health_json("0.4.0", 8765, None, 0, 0, 1_791_216_000, true, false),
            format!("{{\"ok\":true,\"version\":\"0.4.0\",\"built\":1791216000,\"newerOnDisk\":true,\"port\":8765,\"dumpFirmware\":\"{}\",{},\"log\":null,\"boards\":0,\"receivers\":0}}", id, flight(false))
        );
        // The build time is the one compiled into this binary.
        assert!(crate::opts::built() > 1_790_000_000, "{}", crate::opts::built());
        // A build that carries a dump image says which one, so a stale copy is visible.
        assert!(!id.is_empty(), "the dump firmware should be compiled in");
        // The version is the crate's, not a string typed twice.
        assert!(health_json(env!("CARGO_PKG_VERSION"), 1, None, 0, 0, 0, false, false)
            .contains(&format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn a_program_file_written_after_this_copy_started_is_a_newer_build_on_disk() {
        let started = std::time::UNIX_EPOCH + Duration::from_secs(1_791_216_000);
        let at = |secs: u64| Some(std::time::UNIX_EPOCH + Duration::from_secs(secs));
        // Built, then started: the file is older than the process, as it always is at first.
        assert!(!newer_on_disk(started, at(1_791_215_990)));
        assert!(!newer_on_disk(started, at(1_791_216_000)));
        // Rebuilt while it runs.
        assert!(newer_on_disk(started, at(1_791_216_001)));
        assert!(newer_on_disk(started, at(1_791_300_000)));
        // The file has gone, or was never found: nothing to say it is newer.
        assert!(!newer_on_disk(started, None));
    }

    #[test]
    fn a_quote_in_a_port_error_cannot_break_the_json() {
        let ports = vec![PortInfo {
            key: "rx1".into(),
            port: "/dev/ttyUSB0".into(),
            usb: "".into(),
            status: "error".into(),
            error: Some("bad \"thing\"\n".into()),
            rx_id: None,
            rx_name: None,
            rx_fw: None,
            name: None,
        }];
        let json = ports_json(&ports, &[], None);
        assert!(json.contains("bad \\\"thing\\\"\\n"), "{}", json);
        assert!(json.ends_with("\"log\":null}"));
    }

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn written(path: &Path, body: &str, secs: u64) {
        std::fs::write(path, body).unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn the_log_list_has_session_logs_and_pulled_ones_newest_first() {
        let dir = scratch("logs");
        written(&dir.join("descent-ground-2026-09-22T19-30-00.log"), "abc", 1_790_100_000);
        written(&dir.join("descent-ground-2026-09-27T09-00-00.log"), "a", 1_790_500_000);
        std::fs::write(dir.join("notes.txt"), "not a log").unwrap();
        let pull = dir.join("descent-pull-2026-09-26T16-45-50-ttyACM1");
        std::fs::create_dir_all(&pull).unwrap();
        written(&pull.join("ground.log"), "abcde", 1_790_441_150);
        std::fs::write(pull.join("dump.txt"), "x").unwrap();
        // A pull that read nothing leaves a folder with no log in it, and a folder of any
        // other name is not one of ours whatever it holds.
        std::fs::create_dir_all(dir.join("descent-pull-2026-09-26T17-00-00-ttyACM1")).unwrap();
        std::fs::create_dir_all(dir.join("tiles")).unwrap();
        std::fs::write(dir.join("tiles").join("ground.log"), "x").unwrap();

        assert_eq!(
            logs_json(&dir, &crate::chips::Registry::empty()),
            "{\"logs\":[\
{\"name\":\"descent-ground-2026-09-27T09-00-00.log\",\"bytes\":1,\"when\":1790500000,\"kind\":\"session\"},\
{\"name\":\"descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log\",\"bytes\":5,\"when\":1790441150,\"kind\":\"pull\"},\
{\"name\":\"descent-ground-2026-09-22T19-30-00.log\",\"bytes\":3,\"when\":1790100000,\"kind\":\"session\"}]}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_served_from_the_data_folder_or_one_pull_folder_down() {
        let same = |n: &str| assert_eq!(log_name(n).as_deref(), Some(n), "{}", n);
        same("descent-ground-2026-09-22T19-30-00.log");
        same("descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log");
        same("descent-pull-2026-09-26T16-45-50-ttyACM1/dump.txt");
        same("descent-pull-2026-09-26T16-45-50-ttyACM1/report.json");
        same("descent-pull-2026-09-26T16-45-50-ttyACM1/descent-ground-fsw-ttyACM1-2026-09-26T16-45-50.bin");
        // The page sends the name through encodeURIComponent, so the slash arrives as %2F.
        assert_eq!(
            log_name("descent-pull-2026-09-26T16-45-50-ttyACM1%2Fground.log").as_deref(),
            Some("descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log")
        );
        assert_eq!(log_name("/a%20b.log").as_deref(), Some("a b.log"));

        for no in [
            "",
            "..",
            "../Cargo.toml",
            "%2e%2e%2FCargo.toml",
            "%2Fetc%2Fpasswd",
            "etc/passwd",
            "tiles/14",
            "descent-pull-x/../descent-ground.conf",
            "descent-pull-x%2F..%2Fdescent-ground.conf",
            "descent-pull-x/a/b",
            "descent-pull-x/",
            "descent-pull-x\\dump.txt",
            "descent-pull-x%5Cdump.txt",
            "a%",
            "a%zz",
            "%ff.log",
            "a%0D%0AX-Evil:%201.log",
            "a%00.log",
        ] {
            assert_eq!(log_name(no), None, "{:?} should have been refused", no);
        }
    }

    #[test]
    fn a_query_string_is_read_as_pairs_with_its_escapes_undone() {
        let q = query("/api/chip/flash?uid=0038002A5553500920393256&backup=0&name=my%20build%2B2%26more.bin").unwrap();
        assert_eq!(asked(&q, "uid"), Some("0038002A5553500920393256"));
        assert_eq!(asked(&q, "backup"), Some("0"));
        assert_eq!(asked(&q, "name"), Some("my build+2&more.bin"));
        assert_eq!(asked(&q, "force"), None);

        // A form writes a space as +, and a real + always arrives as %2B.
        let q = query("/x?name=flight+spare.bin&saved=descent-pull-x%2Fa.bin&force=1").unwrap();
        assert_eq!(asked(&q, "name"), Some("flight spare.bin"));
        assert_eq!(asked(&q, "saved"), Some("descent-pull-x/a.bin"));
        assert_eq!(asked(&q, "force"), Some("1"));

        // A key on its own, a value with = in it, and the first of two wins.
        let q = query("/x?force&name=a=b&name=second&=nothing&&").unwrap();
        assert_eq!(asked(&q, "force"), Some(""));
        assert_eq!(asked(&q, "name"), Some("a=b"));
        assert_eq!(q.len(), 4, "{:?}", q);

        assert_eq!(query("/api/chip/flash").unwrap(), Vec::new());
        assert_eq!(query("/api/chip/flash?").unwrap(), Vec::new());
        assert_eq!(query("/x?name=caf%C3%A9.bin").unwrap(), vec![("name".to_string(), "caf\u{e9}.bin".to_string())]);
        // An escape that does not decode, or decodes to something that is not text.
        for no in ["/x?name=a%", "/x?name=a%zz", "/x?name=%ff", "/x?na%me=1"] {
            assert_eq!(query(no), None, "{}", no);
        }
    }

    #[test]
    fn the_log_list_has_saved_images_and_says_which_board_when_the_name_does() {
        let dir = scratch("images");
        written(&dir.join("descent-ground-2026-09-27T09-00-00.log"), "a", 1_790_500_000);
        written(&dir.join("descent-image-20393256-Flight-spare-2-2026-09-28T10-00-00.bin"), "abcdef", 1_790_600_000);
        written(&dir.join("descent-ground-fsw-ttyACM1-2026-09-20T10-00-00.bin"), "abc", 1_789_900_000);
        // A receiver's backup is somebody else's kind of image, and so is anything
        // the operator left here.
        written(&dir.join("descent-ground-backup-ttyUSB0-2026-09-20T10-00-00.bin"), "x", 1_790_700_000);
        written(&dir.join("chipsat.bin"), "x", 1_790_700_000);

        let pull = dir.join("descent-pull-2026-09-26T16-45-50-left");
        std::fs::create_dir_all(&pull).unwrap();
        written(&pull.join("ground.log"), "abcde", 1_790_441_150);
        written(&pull.join("descent-fsw-2039abcd-left-2026-09-26T16-45-50.bin"), "ab", 1_790_441_100);
        // A pull that read nothing still saved what was on the board, under the older name.
        let empty = dir.join("descent-pull-2026-09-25T16-45-50-ttyACM1");
        std::fs::create_dir_all(&empty).unwrap();
        written(&empty.join("descent-ground-fsw-ttyACM1-2026-09-25T16-45-50.bin"), "abcd", 1_790_354_750);
        std::fs::write(empty.join("dump.txt"), "x").unwrap();

        // A name carries the end of the id. A board this laptop has read is given by its
        // whole id, which is what the page knows it by, and any other by what the name has.
        let mut chips = crate::chips::Registry::empty();
        chips.saw("0038002A5553500920393256", "unknown", 1).unwrap();
        assert_eq!(
            logs_json(&dir, &chips),
            "{\"logs\":[\
{\"name\":\"descent-image-20393256-Flight-spare-2-2026-09-28T10-00-00.bin\",\"bytes\":6,\"when\":1790600000,\"kind\":\"image\",\"uid\":\"0038002A5553500920393256\"},\
{\"name\":\"descent-ground-2026-09-27T09-00-00.log\",\"bytes\":1,\"when\":1790500000,\"kind\":\"session\"},\
{\"name\":\"descent-pull-2026-09-26T16-45-50-left/ground.log\",\"bytes\":5,\"when\":1790441150,\"kind\":\"pull\",\"uid\":\"2039ABCD\"},\
{\"name\":\"descent-pull-2026-09-26T16-45-50-left/descent-fsw-2039abcd-left-2026-09-26T16-45-50.bin\",\"bytes\":2,\"when\":1790441100,\"kind\":\"image\",\"uid\":\"2039ABCD\"},\
{\"name\":\"descent-pull-2026-09-25T16-45-50-ttyACM1/descent-ground-fsw-ttyACM1-2026-09-25T16-45-50.bin\",\"bytes\":4,\"when\":1790354750,\"kind\":\"image\"},\
{\"name\":\"descent-ground-fsw-ttyACM1-2026-09-20T10-00-00.bin\",\"bytes\":3,\"when\":1789900000,\"kind\":\"image\"}]}"
        );

        // Two boards whose ids end the same cannot be told apart by a name, so it is
        // left as the name has it.
        chips.saw("FFFFFFFFFFFFFFFF20393256", "unknown", 1).unwrap();
        assert!(logs_json(&dir, &chips).contains("\"kind\":\"image\",\"uid\":\"20393256\"}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_list_reads_the_day_folders_beside_the_older_flat_names() {
        let dir = scratch("days");
        written(&dir.join("descent-ground-2026-10-17T18-02-58.log"), "old", 1_790_000_000);
        let day = dir.join("2026-10-18");
        for kind in ["logs", "pulls/1432-board9", "images", "clears/1125-board9", "receiver-images"] {
            std::fs::create_dir_all(day.join(kind)).unwrap();
        }
        written(&day.join("logs/1402-ground.log"), "ab", 1_790_100_000);
        written(&day.join("pulls/1432-board9/ground.log"), "abcde", 1_790_200_000);
        written(&day.join("pulls/1432-board9/1432-board9-0027003A.bin"), "abc", 1_790_199_000);
        std::fs::write(day.join("pulls/1432-board9/dump.txt"), "x").unwrap();
        written(&day.join("images/1101-2-board9-0027003A.bin"), "abcd", 1_790_150_000);
        // not listed: a clear's dump.txt, a receiver image, a folder that only looks like a day
        written(&day.join("clears/1125-board9/dump.txt"), "x", 1_790_160_000);
        written(&day.join("receiver-images/1647-ttyACM0.bin"), "x", 1_790_300_000);
        std::fs::create_dir_all(dir.join("2026-10-1x/logs")).unwrap();
        written(&dir.join("2026-10-1x/logs/1402-ground.log"), "x", 1_790_400_000);

        assert_eq!(
            logs_json(&dir, &crate::chips::Registry::empty()),
            "{\"logs\":[\
{\"name\":\"2026-10-18/pulls/1432-board9/ground.log\",\"bytes\":5,\"when\":1790200000,\"kind\":\"pull\",\"uid\":\"0027003A\"},\
{\"name\":\"2026-10-18/pulls/1432-board9/1432-board9-0027003A.bin\",\"bytes\":3,\"when\":1790199000,\"kind\":\"image\",\"uid\":\"0027003A\"},\
{\"name\":\"2026-10-18/images/1101-2-board9-0027003A.bin\",\"bytes\":4,\"when\":1790150000,\"kind\":\"image\",\"uid\":\"0027003A\"},\
{\"name\":\"2026-10-18/logs/1402-ground.log\",\"bytes\":2,\"when\":1790100000,\"kind\":\"session\"},\
{\"name\":\"descent-ground-2026-10-17T18-02-58.log\",\"bytes\":3,\"when\":1790000000,\"kind\":\"session\"}]}"
        );

        for yes in [
            "2026-10-18/logs/1402-ground.log",
            "2026-10-18/pulls/1432-board9/ground.log",
            "2026-10-18/pulls/1432-board9/dump.txt",
            "2026-10-18/clears/1125-board9/dump.txt",
            "2026-10-18/images/1101-2-board9-0027003A.bin",
        ] {
            assert_eq!(log_name(yes).as_deref(), Some(yes), "{}", yes);
        }
        assert_eq!(log_name("2026-10-18%2Fpulls%2F1432-board9%2Fground.log").as_deref(), Some("2026-10-18/pulls/1432-board9/ground.log"));
        for no in [
            "2026-10-18/pulls/1432-board9/x/y",
            "2026-10-18/logs/a/b",
            "2026-10-18/receiver-images/1647-ttyACM0.bin",
            "2026-10-18/tiles/a",
            "2026-10-1x/logs/1402-ground.log",
            "2026-10-18/pulls/../../descent-ground.conf",
            "2026-10-18//logs/a.log",
            "D:descent-ground.conf",
            "2026-10-18/logs/1402-ground.log:stream",
        ] {
            assert_eq!(log_name(no), None, "{:?} should have been refused", no);
        }
        assert!(is_image("2026-10-18/images/1101-2-board9-0027003A.bin"));
        assert!(is_image("2026-10-18/pulls/1432-board9/1432-board9-0027003A.bin"));
        assert!(!is_image("2026-10-18/receiver-images/1647-ttyACM0.bin"));
        assert!(is_image("2026-10-18/clears/1125-board9/1125-board9-0027003A.bin"), "a clear's image can be a board's newest");
        assert!(!is_image("2026-10-18/logs/1402-ground.log"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pull_with_no_image_left_beside_it_still_says_which_board() {
        let dir = scratch("pullboard");
        let day = dir.join("2026-10-18");
        for d in ["pulls/1432-board9", "pulls/1437-x", "pulls/1440-ttyUSB0", "clears/1125-board12"] {
            std::fs::create_dir_all(day.join(d)).unwrap();
        }
        // board from board.txt, from a list name, and none
        written(&day.join("pulls/1437-x/ground.log"), "a", 1_790_300_000);
        std::fs::write(day.join("pulls/1437-x/board.txt"), "2032374555435010002C002E\n").unwrap();
        written(&day.join("pulls/1432-board9/ground.log"), "a", 1_790_200_000);
        written(&day.join("pulls/1440-ttyUSB0/ground.log"), "a", 1_790_100_000);
        written(&day.join("clears/1125-board12/1125-board12-002C002E.bin"), "ab", 1_790_000_000);
        let mut chips = crate::chips::Registry::empty();
        chips.saw("20343542323250010027003A", "dump", 1).unwrap();
        chips.set_name("20343542323250010027003A", "9").unwrap();
        chips.saw("2032374555435010002C002E", "flight", 1).unwrap();
        assert_eq!(
            logs_json(&dir, &chips),
            "{\"logs\":[\
{\"name\":\"2026-10-18/pulls/1437-x/ground.log\",\"bytes\":1,\"when\":1790300000,\"kind\":\"pull\",\"uid\":\"2032374555435010002C002E\"},\
{\"name\":\"2026-10-18/pulls/1432-board9/ground.log\",\"bytes\":1,\"when\":1790200000,\"kind\":\"pull\",\"uid\":\"20343542323250010027003A\"},\
{\"name\":\"2026-10-18/pulls/1440-ttyUSB0/ground.log\",\"bytes\":1,\"when\":1790100000,\"kind\":\"pull\"},\
{\"name\":\"2026-10-18/clears/1125-board12/1125-board12-002C002E.bin\",\"bytes\":2,\"when\":1790000000,\"kind\":\"image\",\"uid\":\"2032374555435010002C002E\"}]}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_image_the_list_offers_can_be_fetched_and_put_back_and_nothing_else_can() {
        for yes in [
            "descent-image-20393256-Flight-spare-2-2026-09-28T10-00-00.bin",
            "descent-ground-fsw-ttyACM1-2026-09-20T10-00-00.bin",
            "descent-pull-2026-09-26T16-45-50-left/descent-fsw-2039abcd-left-2026-09-26T16-45-50.bin",
            "descent-pull-2026-09-25T16-45-50-ttyACM1/descent-ground-fsw-ttyACM1-2026-09-25T16-45-50.bin",
        ] {
            assert!(is_image(yes), "{}", yes);
            assert_eq!(log_name(yes).as_deref(), Some(yes), "{}", yes);
            assert!(data_file(yes), "{}", yes);
            assert_eq!(content_type(yes), "application/octet-stream");
        }
        for no in [
            "descent-ground-backup-ttyUSB0-2026-09-20T10-00-00.bin",
            "chipsat.bin",
            "descent-ground-2026-09-27T09-00-00.log",
            "descent-pull-2026-09-26T16-45-50-left/ground.log",
            "descent-pull-2026-09-26T16-45-50-left/dump.txt",
            "tiles/a.bin",
            "descent-fsw-20393256-x.bin.log",
            "",
        ] {
            assert!(!is_image(no), "{}", no);
        }
        // Walking out of the data folder is refused before it is asked whether it is an image.
        assert!(!data_file("descent-pull-x/../../descent-fsw-20393256-x.bin"));
        assert!(!data_file("../descent-fsw-20393256-x.bin"));
    }

    #[test]
    fn the_probe_list_says_which_ones_this_user_can_open() {
        use crate::stm32::Probe;
        assert_eq!(probes_json(&[]), "{\"probes\":[]}");
        let list = vec![
            Probe { serial: "002A00".into(), kind: "STLink V3".into(), access: true },
            Probe { serial: "".into(), kind: "STLink \"V2\"".into(), access: false },
        ];
        assert_eq!(
            probes_json(&list),
            "{\"probes\":[{\"serial\":\"002A00\",\"kind\":\"STLink V3\",\"access\":true},{\"serial\":\"\",\"kind\":\"STLink \\\"V2\\\"\",\"access\":false}]}"
        );
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn the_firmware_ids_health_reports_are_the_start_of_each_images_sha256() {
        // The flight repo publishes a sha256 for every image it builds, so an id that is
        // the start of one can be held against it.
        let dump = crate::flash::CHIPSAT_DUMP.expect("the dump firmware should be compiled in");
        let id = &sha256(dump)[..16];
        assert_eq!(crate::flash::CHIPSAT_DUMP_ID, id);
        let health = health_json("0.4.0", 8765, None, 0, 0, 0, false, false);
        assert!(health.contains(&format!("\"dumpFirmware\":\"{}\",", id)), "{}", health);

        for image in &crate::flight::carried().images {
            assert_eq!(image.sha256, sha256(image.bytes), "{}", image.file);
            assert!(
                health.contains(&format!("{{\"id\":\"{}\",\"module\":\"{}\",\"bytes\":{}}}", &sha256(image.bytes)[..16], image.module, image.bytes.len())),
                "{}",
                health
            );
        }
    }

    #[test]
    fn the_name_a_file_is_saved_under_is_fit_for_a_header() {
        assert_eq!(
            save_name("descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log"),
            "descent-pull-2026-09-26T16-45-50-ttyACM1-ground.log"
        );
        assert_eq!(save_name("a \"b\" \u{e9}.log"), "a _b_ _.log");
    }

    #[test]
    fn a_saved_image_is_not_sent_as_text() {
        assert_eq!(content_type("descent-ground-2026-09-22T19-30-00.log"), "text/plain; charset=utf-8");
        assert_eq!(content_type("descent-pull-x/dump.txt"), "text/plain; charset=utf-8");
        assert_eq!(content_type("descent-pull-x/report.json"), "application/json");
        assert_eq!(content_type("descent-pull-x/descent-ground-fsw-ttyACM1.bin"), "application/octet-stream");
    }
}
