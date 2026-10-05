// Reading a ChipSat's flight software off the board and writing it back, over an ST-Link.
// This used to be a Python tool driving OpenOCD from another checkout; nothing on the far
// side of it is more than SWD, so the app does it itself and assumes nothing but a probe.
use crate::json::esc;
use crate::ws::Broadcast;
use probe_rs::config::MemoryRegion;
use probe_rs::flashing::{DownloadOptions, FlashProgress, ProgressEvent, ProgressOperation};
use probe_rs::probe::list::{Accessibility, Lister, ProbeListItem};
use probe_rs::{MemoryInterface, Permissions, Session};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

// The ChipSat's MCU is a Wio-E5 module, so an STM32WLE5JC (hardware.md section 2).
// probe-rs has no autodetection for ST parts, so the name is fixed rather than guessed.
// Only the flash size differs across the 13 WLE5 variants, and that is read from the
// target's memory map below rather than from this name.
const CHIP: &str = "STM32WLE5JCIx";

// 256 kB over SWD is not quick. Small enough that the bar moves, large enough that the
// per-read overhead is not what we are waiting for.
const READ_CHUNK: u64 = 32 * 1024;

const HALT_TIMEOUT: Duration = Duration::from_millis(500);

// The 96-bit id ST programs into every part. UID_BASE in ST's stm32wle5xx.h, which is
// ENGI_BYTES_BASE 0x1FFF7400 + 0x190 and the address HAL_GetUIDw0 reads. It is outside
// the memory map probe-rs carries for this chip, which only lists the flash and the RAM.
const UID_AT: u64 = 0x1FFF_7590;

// An attached debug probe: the serial number to ask for it by, something to show, and
// whether this user is allowed to open it.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub serial: String,
    pub kind: String,
    pub access: bool,
}

// One job on the probe at a time, whichever screen started it. Every step of a job opens
// the probe afresh, so without this a second job would get in between two steps of the
// first and leave a board with one job's image and the other's reset.
#[derive(Clone)]
pub struct Gate(Arc<Mutex<Option<String>>>);

// Lets go when it is dropped, however the job ended.
pub struct Held(Gate);

impl Gate {
    pub fn new() -> Gate {
        Gate(Arc::new(Mutex::new(None)))
    }

    // `doing` finishes the sentence "the probe is busy ...".
    pub fn take(&self, doing: &str) -> Result<Held, String> {
        let mut running = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(other) = running.as_ref() {
            return Err(format!("the probe is busy {}, so wait for that to finish", other));
        }
        *running = Some(doing.to_string());
        Ok(Held(self.clone()))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // A job that panicked still has to let go, so a poisoned lock is opened anyway.
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

// What a long step is doing. Whoever is listening picks the words and where they go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Doing {
    Saving,
    Erasing,
    Writing,
    Checking,
}

// What, bytes done, bytes in all. Erasing has nothing to count and says 0 of 0.
pub type Said<'a> = &'a dyn Fn(Doing, u64, u64);

// The frames a pull has always sent while it works on the board.
pub fn to_page(broadcast: &Broadcast) -> impl Fn(Doing, u64, u64) + '_ {
    move |doing, done, total| match doing {
        Doing::Saving => say(broadcast, pct_of(done, total), &kb_note("saving the flight software", done, total)),
        Doing::Erasing => say(broadcast, 0, "erasing the flash"),
        Doing::Writing => say(broadcast, pct_of(done, total), &kb_note("writing the flight software", done, total)),
        Doing::Checking => say(broadcast, pct_of(done, total), &kb_note("checking what was written", done, total)),
    }
}

// Probes this user cannot open are listed too. Otherwise "nothing is plugged in" and
// "the udev rule is missing" look the same from the dashboard.
pub fn probes() -> Vec<Probe> {
    attached().iter().map(|item| describe(item)).collect()
}

fn attached() -> Vec<ProbeListItem> {
    Lister::new().list_all_with_access()
}

fn describe(item: &ProbeListItem) -> Probe {
    Probe {
        serial: item.info.serial_number.clone().unwrap_or_default(),
        kind: item.info.identifier.clone(),
        access: item.accessibility != Accessibility::PermissionDenied,
    }
}

// None means "the one that is plugged in". Picking for the operator when there are two
// would write firmware to somebody else's board, so it is an error instead.
fn pick(list: &[Probe], want: Option<&str>) -> Result<usize, String> {
    if list.is_empty() {
        return Err(String::from("no debug probe is attached: plug the ST-Link in"));
    }
    match want {
        Some(serial) => list.iter().position(|p| p.serial == serial).ok_or_else(|| {
            format!("no debug probe with serial {} is attached. Attached: {}", serial, names(list))
        }),
        None if list.len() == 1 => Ok(0),
        None => Err(format!(
            "{} debug probes are attached, so say which one: {}",
            list.len(),
            names(list)
        )),
    }
}

fn names(list: &[Probe]) -> String {
    list.iter()
        .map(|p| {
            if p.serial.is_empty() {
                format!("{} (no serial number)", p.kind)
            } else {
                format!("{} ({})", p.kind, p.serial)
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

fn open(probe: Option<&str>) -> Result<Session, String> {
    open_how(probe).map(|o| o.session)
}

struct Opened {
    session: Session,
    probe: Probe,
    // The board only answered with NRST held, so it has been reset and is sitting
    // halted at its first instruction.
    under_reset: bool,
}

fn open_how(probe: Option<&str>) -> Result<Opened, String> {
    let attached = attached();
    let list: Vec<Probe> = attached.iter().map(describe).collect();
    let which = pick(&list, probe)?;
    if attached[which].accessibility == Accessibility::PermissionDenied {
        return Err(format!(
            "{} is plugged in but this user cannot open it, usually the missing udev rule for ST-Link probes",
            list[which].kind
        ));
    }
    let selected = || {
        attached[which]
            .info
            .open()
            .map_err(|e| format!("could not open {}: {}", list[which].kind, e))
    };
    // A ChipSat asleep in Stop2 has its debug block powered down and does not answer a
    // plain attach; holding NRST while connecting is the usual way through that. NRST is
    // on J1.8 of the pogo header, so it depends on the clip carrying that pin — not tried
    // yet, hence the first error is the one reported if both fail.
    let first = match selected()?.attach(CHIP, Permissions::default()) {
        Ok(session) => return Ok(Opened { session, probe: list[which].clone(), under_reset: false }),
        Err(e) => e,
    };
    match selected()?.attach_under_reset(CHIP, Permissions::default()) {
        Ok(session) => Ok(Opened { session, probe: list[which].clone(), under_reset: true }),
        Err(_) => Err(format!(
            "could not attach to the {} through {}: {}",
            CHIP, list[which].kind, first
        )),
    }
}

// Where the flight software lives, from the chip database rather than from 0x08000000 and
// a size written down here: a smaller WLE5 would otherwise be read short and written past.
// probe-rs marks the WLE5's BANK_1 write: false (its targets/STM32WL_Series.yaml), which is
// about plain memory writes and not about the flash algorithm, so access is not a filter.
fn main_flash(map: &[MemoryRegion]) -> Result<Range<u64>, String> {
    let mut best: Option<Range<u64>> = None;
    for region in map {
        let MemoryRegion::Nvm(nvm) = region else { continue };
        if nvm.is_alias || nvm.range.end <= nvm.range.start {
            continue;
        }
        let size = nvm.range.end - nvm.range.start;
        let better = match &best {
            None => true,
            Some(b) => {
                let had = b.end - b.start;
                size > had || (size == had && nvm.range.start < b.start)
            }
        };
        if better {
            best = Some(nvm.range.clone());
        }
    }
    best.ok_or_else(|| format!("{} has no flash region in probe-rs's chip database", CHIP))
}

// descent-ground-fsw-rx1-2026-09-25T19-30-00.bin, the same shape as the log file names so
// the two sort together in a field directory.
pub fn backup_name(name_hint: &str, at: SystemTime) -> String {
    format!("{}{}-{}.bin", OLDER_IMAGE_PREFIX, tidy(name_hint), stamp(at))
}

// 2026-09-25T19-30-00: no colons, because Windows will not have them in a file name.
pub fn stamp(at: SystemTime) -> String {
    humantime::format_rfc3339_seconds(at)
        .to_string()
        .replace(':', "-")
        .replace('Z', "")
}

// The hint is usually a port name or a board key, and both have to survive becoming part
// of a file name.
pub fn tidy(hint: &str) -> String {
    let last = hint.rsplit('/').find(|p| !p.is_empty()).unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        String::from("board")
    } else {
        cleaned
    }
}

// The id as one number, top byte first, which is the bytes at UID_AT backwards. That
// way round the lot number leads and the die's place on its wafer comes last
// (LL_GetUID_Word0..2 in ST's stm32wlxx_ll_utils.h), so the end of the id is the part
// that differs between two boards from one reel, and the end is what goes in a file name.
pub fn uid_hex(bytes: &[u8; 12]) -> String {
    bytes.iter().rev().map(|b| format!("{:02X}", b)).collect()
}

// An id as the page sends it back: 24 hex characters, either case.
pub fn a_uid(text: &str) -> Option<String> {
    if text.len() == 24 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(text.to_ascii_uppercase())
    } else {
        None
    }
}

const UID_TAIL: usize = 8;
pub const IMAGE_PREFIX: &str = "descent-image-";
pub const OLDER_IMAGE_PREFIX: &str = "descent-ground-fsw-";
const LEGACY_IMAGE_PREFIX: &str = "descent-fsw-";

pub fn uid_tail(uid: &str) -> &str {
    &uid[uid.len().saturating_sub(UID_TAIL)..]
}

// descent-image-20393256-Flight-spare-2-2026-09-25T19-30-00.bin: the board it came off is
// in the name, because the name is all that is left to say so once the file is copied
// somewhere else.
pub fn image_name(uid: &str, board: &str, at: SystemTime) -> String {
    format!("{}{}-{}-{}.bin", IMAGE_PREFIX, uid_tail(uid), tidy(board), stamp(at))
}

// The end of the id of the board a saved image came off, when its name says. Images
// saved before names carried one say nothing. Accepts both current and legacy prefixes.
pub fn uid_in_name(name: &str) -> Option<String> {
    let file = name.rsplit('/').next().unwrap_or("");
    let rest = file
        .strip_prefix(IMAGE_PREFIX)
        .or_else(|| file.strip_prefix(LEGACY_IMAGE_PREFIX))?;
    let (tail, after) = (rest.get(..UID_TAIL)?, rest.get(UID_TAIL..)?);
    if tail.chars().all(|c| c.is_ascii_hexdigit()) && after.starts_with('-') {
        Some(tail.to_ascii_uppercase())
    } else {
        None
    }
}

pub fn pct_of(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (done.min(total) * 100 / total) as u32
}

fn kb_note(what: &str, done: u64, total: u64) -> String {
    format!("{}, {} of {} kB", what, done / 1024, total / 1024)
}

fn frame(pct: u32, note: &str) -> String {
    format!("{{\"type\":\"flash\",\"pct\":{},\"note\":\"{}\"}}", pct, esc(note))
}

fn say(broadcast: &Broadcast, pct: u32, note: &str) {
    broadcast.send(&frame(pct, note));
}

// The bytes that are on the board, which is not the same thing as a fresh build of the
// flight repo. This is the image write_firmware puts back, so a short read is a failure
// rather than a smaller file.
pub fn save_firmware(probe: Option<&str>, dir: &Path, file: &str, said: Said) -> Result<PathBuf, String> {
    let mut session = open(probe)?;
    let flash = main_flash(&session.target().memory_map)?;
    let path = dir.join(file);
    match read_flash_into(&mut session, &flash, &path, said) {
        Ok(()) => Ok(path),
        Err(e) => {
            // Part of an image under a name that says it is one is worse than no file:
            // somebody would write it back.
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

fn read_flash_into(
    session: &mut Session,
    flash: &Range<u64>,
    path: &Path,
    said: Said,
) -> Result<(), String> {
    let total = flash.end - flash.start;
    let mut out = std::fs::File::create(path)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;

    let mut core = session.core(0).map_err(|e| format!("could not reach the core: {}", e))?;
    // Halted while it is read: the flight software writes its own flash, and half of one
    // write with half of the next is not an image anyone can put back.
    core.halt(HALT_TIMEOUT).map_err(|e| format!("could not halt the board: {}", e))?;

    let mut buf = vec![0u8; READ_CHUNK as usize];
    let mut done = 0u64;
    while done < total {
        let n = READ_CHUNK.min(total - done) as usize;
        let at = flash.start + done;
        core.read(at, &mut buf[..n])
            .map_err(|e| format!("could not read the board's firmware at {:#x}: {}", at, e))?;
        out.write_all(&buf[..n])
            .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        done += n as u64;
        said(Doing::Saving, done, total);
    }
    let _ = core.run();   // left as it was found; the caller resets when it means to
    drop(out);

    let written = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if written < total {
        return Err(format!(
            "the saved firmware is short: {} of {} bytes. Nothing was erased.",
            written, total
        ));
    }
    Ok(())
}

// Writing past the end of flash is something the loader would refuse on its own, but the
// operator hears about it before anything is erased rather than after.
pub fn fits(len: usize, region: u64) -> Result<(), String> {
    if len == 0 {
        return Err(String::from("that image is empty, so there is nothing to write"));
    }
    if len as u64 > region {
        return Err(format!(
            "that image is {} bytes and the flash on the board is {}",
            len, region
        ));
    }
    Ok(())
}

pub fn write_firmware(probe: Option<&str>, image: &[u8], said: Said) -> Result<(), String> {
    let mut session = open(probe)?;
    let flash = main_flash(&session.target().memory_map)?;
    fits(image.len(), flash.end - flash.start)?;

    let mut loader = session.target().flash_loader();
    loader
        .add_data(flash.start, image)
        .map_err(|e| format!("cannot stage the image for {:#x}: {}", flash.start, e))?;

    let mut options = DownloadOptions::default();
    options.keep_unwritten_bytes = false;
    options.verify = false;   // done below instead, because that one can name an address
    options.progress = writing_progress(said);
    loader
        .commit(&mut session, options)
        .map_err(|e| format!("writing the flight software failed: {}", e))?;

    verify(&mut session, flash.start, image, said)
}

fn writing_progress(said: Said<'_>) -> FlashProgress<'_> {
    let mut total = 0u64;
    let mut written = 0u64;
    FlashProgress::new(move |event| match event {
        ProgressEvent::AddProgressBar { operation: ProgressOperation::Program, total: size } => {
            total = size.unwrap_or(0);
        }
        ProgressEvent::Started(ProgressOperation::Erase) => {
            said(Doing::Erasing, 0, 0);
        }
        ProgressEvent::Progress { operation: ProgressOperation::Program, size, .. } => {
            written += size;
            said(Doing::Writing, written, total);
        }
        _ => {}
    })
}

// probe-rs can verify as part of the write, but it only answers yes or no. Reading it
// back here costs the same and names the first byte that differs, which is the difference
// between trying again and knowing the image never landed.
fn verify(session: &mut Session, start: u64, image: &[u8], said: Said) -> Result<(), String> {
    let mut core = session.core(0).map_err(|e| format!("could not reach the core: {}", e))?;
    let total = image.len() as u64;
    let mut buf = vec![0u8; READ_CHUNK as usize];
    let mut done = 0u64;
    while done < total {
        let n = READ_CHUNK.min(total - done) as usize;
        let at = start + done;
        core.read(at, &mut buf[..n])
            .map_err(|e| format!("could not read the flash back at {:#x}: {}", at, e))?;
        let want = &image[done as usize..done as usize + n];
        if let Some(off) = first_difference(want, &buf[..n]) {
            return Err(format!(
                "the flash does not match the image at {:#x}: wrote {:#04x}, read back {:#04x}",
                at + off as u64,
                want[off],
                buf[off]
            ));
        }
        done += n as u64;
        said(Doing::Checking, done, total);
    }
    Ok(())   // still halted: reset_and_run is what starts it
}

fn first_difference(want: &[u8], got: &[u8]) -> Option<usize> {
    want.iter().zip(got).position(|(a, b)| a != b)
}

pub fn reset_and_run(probe: Option<&str>) -> Result<(), String> {
    let mut session = open(probe)?;
    let mut core = session.core(0).map_err(|e| format!("could not reach the core: {}", e))?;
    core.reset().map_err(|e| format!("could not reset the board: {}", e))
}

// A board as it was found on the probe.
pub struct Seen {
    pub uid: String,
    pub flash_bytes: u64,
    pub image: Vec<u8>,     // the flash from its start to where it runs out, see programmed()
    pub probe: Probe,
    pub was_reset: bool,    // it would not answer until it was reset, so it has been
}

// Flash from its start up to the first stretch that was never written. An image is one
// run from the bottom, and reading 256 kB to find the 64 kB that are in use is most of
// the wait. The price is that anything sitting beyond a READ_CHUNK of erased flash is
// not seen. Not timed on a board.
fn programmed(total: u64, mut read: impl FnMut(u64, &mut [u8]) -> Result<(), String>) -> Result<Vec<u8>, String> {
    let mut image = Vec::new();
    let mut buf = vec![0u8; READ_CHUNK as usize];
    let mut done = 0u64;
    while done < total {
        let n = READ_CHUNK.min(total - done) as usize;
        read(done, &mut buf[..n])?;
        if buf[..n].iter().all(|b| *b == 0xFF) {
            break;
        }
        image.extend_from_slice(&buf[..n]);
        done += n as u64;
    }
    Ok(image)
}

// Which board is on the probe, without changing what it is doing: nothing is halted,
// because the id and the image do not move under a running core. Not tried on a board.
// The exception is a board that only answers with NRST held. That one has been reset by
// the time it can be read, and is started again here so it is not left stopped.
pub fn look(probe: Option<&str>) -> Result<Seen, String> {
    looked(probe, true)
}

// Only which board it is, for a job that is about to read the flash itself anyway.
pub fn uid(probe: Option<&str>) -> Result<String, String> {
    looked(probe, false).map(|seen| seen.uid)
}

fn looked(probe: Option<&str>, with_image: bool) -> Result<Seen, String> {
    let Opened { mut session, probe, under_reset } = open_how(probe)?;
    let flash = main_flash(&session.target().memory_map)?;
    let mut core = session.core(0).map_err(|e| format!("could not reach the core: {}", e))?;

    let read = (|| -> Result<([u8; 12], Vec<u8>), String> {
        let mut uid = [0u8; 12];
        core.read(UID_AT, &mut uid)
            .map_err(|e| format!("could not read the board's id at {:#x}: {}", UID_AT, e))?;
        if !with_image {
            return Ok((uid, Vec::new()));
        }
        let image = programmed(flash.end - flash.start, |at, buf| {
            core.read(flash.start + at, buf)
                .map_err(|e| format!("could not read the board's firmware at {:#x}: {}", flash.start + at, e))
        })?;
        Ok((uid, image))
    })();
    if under_reset {
        let _ = core.run();
    }
    let (uid, image) = read?;
    Ok(Seen {
        uid: uid_hex(&uid),
        flash_bytes: flash.end - flash.start,
        image,
        probe,
        was_reset: under_reset,
    })
}

// Twelve bytes that are all one value are a bus that read nothing, not an id.
fn an_id(bytes: &[u8; 12]) -> Option<String> {
    if bytes.iter().all(|b| *b == bytes[0]) {
        None
    } else {
        Some(uid_hex(bytes))
    }
}

// Which board is under the clip, asked in a way that cannot reset it: the plain attach
// and none of open_how's NRST fallback. None is a board that did not answer, which says
// nothing about what is there: it may be half clipped, held in reset or gone. The page
// asks every couple of seconds, so a board swapped under a probe that stays plugged in is
// noticed. The plain attach is what every read has started with; asking this often has
// not been tried on a board.
// Even a plain attach sets the core's debug enable, which can keep the debug domain
// powered until the board is power cycled. A BOARD THAT HAS SAT UNDER THE CLIP WITH THE
// CHIPSATS SCREEN UP IS NOT ONE TO MEASURE CURRENT ON. Power it off and on first.
pub fn peek(probe: Option<&str>) -> Option<String> {
    let attached = attached();
    let list: Vec<Probe> = attached.iter().map(describe).collect();
    let which = pick(&list, probe).ok()?;
    if attached[which].accessibility == Accessibility::PermissionDenied {
        return None;
    }
    let mut session = attached[which].info.open().ok()?.attach(CHIP, Permissions::default()).ok()?;
    let mut core = session.core(0).ok()?;
    let mut uid = [0u8; 12];
    core.read(UID_AT, &mut uid).ok()?;
    an_id(&uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use probe_rs::config::{NvmRegion, RamRegion};

    #[test]
    fn twelve_bytes_that_are_all_the_same_are_not_an_id() {
        assert_eq!(an_id(&[0xFF; 12]), None);
        assert_eq!(an_id(&[0x00; 12]), None);
        let mut real = [0u8; 12];
        real[0] = 0x56;
        real[11] = 0x38;
        assert_eq!(an_id(&real).as_deref(), Some("380000000000000000000056"));
    }


    fn probe(kind: &str, serial: &str) -> Probe {
        Probe { serial: String::from(serial), kind: String::from(kind), access: true }
    }

    fn nvm(start: u64, end: u64, alias: bool) -> MemoryRegion {
        MemoryRegion::Nvm(NvmRegion {
            name: Some(String::from("BANK_1")),
            range: start..end,
            cores: vec![String::from("main")],
            is_alias: alias,
            // The real STM32WLE5JC entry says write: false here, so the map we test
            // against says it too.
            access: Some(probe_rs::config::MemoryAccess {
                read: true,
                write: false,
                execute: true,
                boot: true,
            }),
        })
    }

    fn ram(start: u64, end: u64) -> MemoryRegion {
        MemoryRegion::Ram(RamRegion {
            name: Some(String::from("SRAM1")),
            range: start..end,
            cores: vec![String::from("main")],
            is_alias: false,
            access: None,
        })
    }

    #[test]
    fn one_attached_probe_needs_no_choosing_and_two_do() {
        let none: Vec<Probe> = Vec::new();
        assert!(pick(&none, None).unwrap_err().contains("no debug probe is attached"));
        assert!(pick(&none, Some("002A00")).unwrap_err().contains("no debug probe is attached"));

        let one = vec![probe("STLink V3", "002A00")];
        assert_eq!(pick(&one, None).unwrap(), 0);
        assert_eq!(pick(&one, Some("002A00")).unwrap(), 0);
        let wrong = pick(&one, Some("002B00")).unwrap_err();
        assert!(wrong.contains("002B00") && wrong.contains("002A00"), "{}", wrong);

        // A probe that reports no serial number is still the only one attached.
        let bare = vec![probe("STLink V2", "")];
        assert_eq!(pick(&bare, None).unwrap(), 0);
        assert!(pick(&bare, Some("002A00")).unwrap_err().contains("no serial number"));

        let two = vec![probe("STLink V3", "002A00"), probe("STLink V2", "066EFF")];
        let both = pick(&two, None).unwrap_err();
        assert!(both.contains("2 debug probes"), "{}", both);
        assert!(both.contains("002A00") && both.contains("066EFF"), "{}", both);
        assert_eq!(pick(&two, Some("066EFF")).unwrap(), 1);

        // Two probes with no serials: nothing to pick by, and the count has to say so.
        let blind = vec![probe("STLink V2", ""), probe("STLink V2", "")];
        assert!(pick(&blind, None).unwrap_err().contains("2 debug probes"));
    }

    #[test]
    fn the_flash_bounds_come_from_the_memory_map() {
        // What the STM32WLE5JC entry actually holds: one 256 kB bank and two RAMs.
        let wle5 = vec![
            nvm(0x0800_0000, 0x0804_0000, false),
            ram(0x2000_0000, 0x2000_8000),
            ram(0x2000_8000, 0x2001_0000),
        ];
        assert_eq!(main_flash(&wle5).unwrap(), 0x0800_0000..0x0804_0000);

        // The alias at 0 is the same bytes seen twice, and an OTP-sized region is not the
        // main flash, so the bank wins on size.
        let aliased = vec![
            nvm(0x0000_0000, 0x0804_0000, true),
            nvm(0x1FFF_7000, 0x1FFF_7400, false),
            nvm(0x0800_0000, 0x0804_0000, false),
        ];
        assert_eq!(main_flash(&aliased).unwrap(), 0x0800_0000..0x0804_0000);

        let no_flash = vec![ram(0x2000_0000, 0x2000_8000)];
        assert!(main_flash(&no_flash).unwrap_err().contains("no flash region"));
    }

    #[test]
    fn a_saved_image_is_named_after_the_board_and_the_minute_it_was_read() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert_eq!(
            backup_name("rx1", at),
            "descent-ground-fsw-rx1-2026-09-21T14-13-20.bin"
        );
        assert_eq!(
            backup_name("/dev/ttyACM0", at),
            "descent-ground-fsw-ttyACM0-2026-09-21T14-13-20.bin"
        );
        // A hint with nothing usable in it still has to produce one file name.
        assert!(backup_name("/", at).starts_with("descent-ground-fsw-board-"));
        assert!(backup_name("", at).starts_with("descent-ground-fsw-board-"));
        assert_eq!(tidy("chip sat #2"), "chip-sat--2");
    }

    #[test]
    fn an_image_that_does_not_fit_is_refused_before_anything_is_erased() {
        let flash = 256 * 1024;
        assert!(fits(flash as usize, flash).is_ok());
        assert!(fits(1, flash).is_ok());
        let over = fits(flash as usize + 1, flash).unwrap_err();
        assert!(over.contains("262145") && over.contains("262144"), "{}", over);
        assert!(fits(0, flash).unwrap_err().contains("empty"));
    }

    #[test]
    fn progress_frames_carry_a_percentage_and_a_line_to_read() {
        assert_eq!(pct_of(0, 262_144), 0);
        assert_eq!(pct_of(65_536, 262_144), 25);
        assert_eq!(pct_of(262_144, 262_144), 100);
        assert_eq!(pct_of(300_000, 262_144), 100);
        assert_eq!(pct_of(1, 0), 0);   // a total probe-rs did not tell us

        assert_eq!(
            kb_note("saving the flight software", 65_536, 262_144),
            "saving the flight software, 64 of 256 kB"
        );
        assert_eq!(
            frame(25, "saving the flight software, 64 of 256 kB"),
            "{\"type\":\"flash\",\"pct\":25,\"note\":\"saving the flight software, 64 of 256 kB\"}"
        );
        // Probe names and OS errors end up in these notes, so quotes cannot break the frame.
        assert!(frame(0, "no \"probe\"\tfound").contains("no \\\"probe\\\"\\tfound"));
    }

    #[test]
    fn an_id_is_written_top_byte_first_so_its_end_is_the_part_that_differs() {
        // As the bytes sit at 0x1FFF7590: word 0 is where the die was on its wafer, word 1
        // the wafer and the low end of the lot, word 2 the rest of the lot in ASCII.
        let at_uid_base: [u8; 12] = [0x2A, 0x00, 0x38, 0x00, 0x09, 0x50, 0x53, 0x55, 0x56, 0x32, 0x39, 0x20];
        let uid = uid_hex(&at_uid_base);
        assert_eq!(uid, "20393256555350090038002A");
        assert_eq!(uid_tail(&uid), "0038002A", "the die's own coordinates, not the lot the whole reel shares");

        assert_eq!(a_uid("0038002A5553500920393256").as_deref(), Some("0038002A5553500920393256"));
        assert_eq!(a_uid("0038002a5553500920393256").as_deref(), Some("0038002A5553500920393256"));
        for no in ["", "0038002A555350092039325", "0038002A55535009203932566", "0038002A555350092039325G", "0038002A 553500920393256"] {
            assert_eq!(a_uid(no), None, "{:?}", no);
        }
    }

    #[test]
    fn a_saved_image_says_in_its_name_which_board_it_came_off() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let uid = "0038002A5553500920393256";
        assert_eq!(
            image_name(uid, "Flight spare 2", at),
            "descent-image-20393256-Flight-spare-2-2026-09-21T14-13-20.bin"
        );
        assert_eq!(image_name(uid, "", at), "descent-image-20393256-board-2026-09-21T14-13-20.bin");

        assert_eq!(uid_in_name(&image_name(uid, "Flight spare 2", at)).as_deref(), Some("20393256"));
        assert_eq!(uid_in_name(&image_name(uid, "", at)).as_deref(), Some("20393256"));
        // As /api/logs lists one that a pull saved, and whatever case a file system kept.
        assert_eq!(
            uid_in_name("descent-pull-2026-09-21T14-13-20-left/descent-image-20393256-left-2026-09-21T14-13-20.bin").as_deref(),
            Some("20393256")
        );
        assert_eq!(uid_in_name("descent-image-2039abcd-board-2026-09-21T14-13-20.bin").as_deref(), Some("2039ABCD"));

        // Legacy descent-fsw- prefix still works for backwards compatibility
        assert_eq!(
            uid_in_name("descent-pull-2026-09-21T14-13-20-left/descent-fsw-20393256-left-2026-09-21T14-13-20.bin").as_deref(),
            Some("20393256")
        );
        assert_eq!(uid_in_name("descent-fsw-2039abcd-board-2026-09-21T14-13-20.bin").as_deref(), Some("2039ABCD"));

        // Images from before the id went into the name, and things that only look alike.
        for none in [
            "descent-ground-fsw-ttyACM1-2026-09-21T14-13-20.bin",
            "descent-pull-2026-09-21T14-13-20-ttyACM1/descent-ground-fsw-ttyACM1-2026-09-21T14-13-20.bin",
            "descent-image-board-2026-09-21T14-13-20.bin",
            "descent-image-20393256.bin",
            "descent-image-2039325-x.bin",
            "chipsat.bin",
            "",
        ] {
            assert_eq!(uid_in_name(none), None, "{:?}", none);
        }
    }

    #[test]
    fn one_job_has_the_probe_at_a_time_and_the_second_is_told_which() {
        let gate = Gate::new();
        let held = gate.take("pulling the log from /dev/ttyACM1").unwrap();
        let e = gate.take("reading a board").err().unwrap();
        assert_eq!(e, "the probe is busy pulling the log from /dev/ttyACM1, so wait for that to finish");
        // A clone is the same gate: the server hands one to each request.
        assert!(gate.clone().take("flashing a board").is_err());
        drop(held);
        let held = gate.take("reading a board").unwrap();
        assert!(gate.take("flashing a board").err().unwrap().contains("reading a board"));
        drop(held);

        // A job that dies holding it still lets go.
        let inside = gate.clone();
        let _ = std::thread::spawn(move || {
            let _held = inside.take("flashing a board").unwrap();
            panic!("the job fell over");
        })
        .join();
        assert!(gate.take("reading a board").is_ok());
    }

    #[test]
    fn a_board_is_read_from_the_bottom_of_flash_to_where_the_image_runs_out() {
        // 70 kB of image in 256 kB of flash: three chunks hold some of it, and the fourth
        // is the first that is all erased.
        let flash = |at: u64, buf: &mut [u8]| {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = if at + (i as u64) < 70_000 { 0x42 } else { 0xFF };
            }
            Ok(())
        };
        let mut asked: Vec<(u64, usize)> = Vec::new();
        let image = programmed(256 * 1024, |at, buf| {
            asked.push((at, buf.len()));
            flash(at, buf)
        })
        .unwrap();
        assert_eq!(asked, vec![(0, 32768), (32768, 32768), (65536, 32768), (98304, 32768)]);
        assert_eq!(image.len(), 98304);
        assert!(image[..70_000].iter().all(|b| *b == 0x42) && image[70_000..].iter().all(|b| *b == 0xFF));

        // A full chip is read to its end and no further, whatever its size.
        let full = programmed(100_000, |_, buf| {
            buf.fill(0x42);
            Ok(())
        })
        .unwrap();
        assert_eq!(full.len(), 100_000);
        // An erased chip is nothing at all, and a read that fails is a failure.
        let erased = programmed(256 * 1024, |_, buf| {
            buf.fill(0xFF);
            Ok(())
        })
        .unwrap();
        assert!(erased.is_empty());
        let e = programmed(256 * 1024, |at, _| if at == 0 { Ok(()) } else { Err(String::from("the clip slipped")) });
        assert_eq!(e.unwrap_err(), "the clip slipped");
    }

    #[test]
    fn a_readback_points_at_the_first_byte_that_differs() {
        assert_eq!(first_difference(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(first_difference(&[1, 2, 3], &[1, 0xFF, 3]), Some(1));
        assert_eq!(first_difference(&[1, 2, 3], &[0xFF, 0xFF, 0xFF]), Some(0));
    }
}
