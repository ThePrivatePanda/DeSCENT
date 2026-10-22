// Flashing a receiver. These are the only receivers the team has, so the board's
// existing firmware is read off and saved before anything is erased.
use crate::json::{esc, int_field, str_field};
use crate::receiver_image::check_image;
use crate::server::Ctx;
use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
use espflash::flasher::Flasher;
use espflash::target::ProgressCallbacks;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

include!(concat!(env!("OUT_DIR"), "/firmware.rs"));

const BACKUP_BAUD: u32 = 460_800;
const BACKUP_CHUNK: u32 = 256 * 1024;   // read in pieces so there is something to report

// Every port is offered, including boards that are receiving right now: reflashing a
// working receiver is the normal case, not an exception.
pub fn candidates_from(hub: &crate::serial::Hub, found: &[String]) -> Vec<String> {
    let mut v: Vec<String> = found.iter().filter(|p| !hub.is_flashing(p)).cloned().collect();
    v.sort();
    v
}

// Only boards the app is already reading, or ones the operator has explicitly
// allowed. A port we have never been told to touch is never offered for flashing:
// writing firmware to somebody's ChipSat is not a recoverable mistake.
pub fn candidate_ports(hub: &crate::serial::Hub) -> Vec<String> {
    let mut found: Vec<String> = hub.ports().into_iter().map(|p| p.port).collect();
    let live: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect();
    for b in hub.boards() {
        if b.state == "receiver" && live.contains(&b.port) && !found.contains(&b.port) {
            found.push(b.port);
        }
    }
    candidates_from(hub, &found)
}

// The image this build carries and the version in its name, as /api/flash/candidates
// gives them. The version is there so the page can hold it against the "rxFw" a
// receiver reports without taking file names apart itself.
fn firmware_json(name: Option<&str>) -> String {
    let Some(name) = name else { return String::from("\"firmware\":null,\"firmwareVersion\":null") };
    // The same numbers build.rs chose this image by.
    let numbers: Vec<String> = crate::receiver_image::version(name).iter().map(|n| n.to_string()).collect();
    let version = if numbers.is_empty() { String::from("null") } else { format!("\"{}\"", numbers.join(".")) };
    format!("\"firmware\":\"{}\",\"firmwareVersion\":{}", esc(name), version)
}

// What the operator is about to overwrite, so nobody flashes the wrong board.
pub fn candidates_json(hub: &crate::serial::Hub) -> String {
    let open = hub.ports();
    let items: Vec<String> = candidate_ports(hub)
        .into_iter()
        .map(|port| {
            let key = open.iter().find(|p| p.port == port).map(|p| p.key.clone());
            format!(
                "{{\"port\":\"{}\",\"key\":{}}}",
                esc(&port),
                match key {
                    Some(k) => format!("\"{}\"", esc(&k)),
                    None => String::from("null"),
                }
            )
        })
        .collect();
    candidates_json_of(&items)
}

fn candidates_json_of(items: &[String]) -> String {
    let carried = if IMAGE.is_some() { Some(IMAGE_NAME) } else { None };
    format!("{{\"candidates\":[{}],{}}}", items.join(","), firmware_json(carried))
}

struct Progress<'a> {
    broadcast: &'a crate::ws::Broadcast,
    total: usize,
}

impl ProgressCallbacks for Progress<'_> {
    fn init(&mut self, _addr: u32, total: usize) {
        self.total = total;
        self.note(0, "writing");
    }
    fn update(&mut self, current: usize) {
        let pct = if self.total == 0 { 0 } else { current * 100 / self.total };
        self.note(pct as u32, "writing");
    }
    fn verifying(&mut self) {
        self.note(100, "verifying");
    }
    fn finish(&mut self, _skipped: bool) {
        self.note(100, "done");
    }
}

impl Progress<'_> {
    fn note(&self, pct: u32, what: &str) {
        self.broadcast.send(&format!(
            "{{\"type\":\"flash\",\"pct\":{},\"note\":\"{}\"}}",
            pct, what
        ));
    }
}

fn open_flasher(port: &str) -> Result<Flasher, String> {
    let serial = serialport::new(port, 115_200)
        .timeout(Duration::from_secs(3))
        .open_native()
        .map_err(|e| format!("{}: {}", port, e))?;
    let info = serialport::UsbPortInfo {
        vid: 0,
        pid: 0,
        serial_number: None,
        manufacturer: None,
        product: None,
        // serialport's usbportinfo-interface feature adds this; the manifest asks for it
        // so the shape does not depend on what another crate happens to turn on.
        interface: None,
    };
    let connection = Connection::new(
        serial,
        info,
        ResetAfterOperation::HardReset,
        ResetBeforeOperation::DefaultReset,
        115_200,
    );
    // Connect at the ROM's 115200 and then move up. The whole chip has to be read
    // before anything is erased, and at 115200 that is four megabytes through a
    // 11.5 kB/s pipe, about six minutes of looking hung.
    Flasher::connect(connection, true, true, false, None, Some(BACKUP_BAUD))
        .map_err(|e| format!("could not talk to the board on {}: {}", port, e))
}

pub fn backup(port: &str, dir: &Path, broadcast: &crate::ws::Broadcast) -> Result<PathBuf, String> {
    let mut flasher = open_flasher(port)?;
    let size = flasher
        .flash_detect()
        .map_err(|e| format!("could not read the flash size: {}", e))?
        .unwrap_or(espflash::flasher::FlashSize::_4Mb);
    let bytes = size.size();
    // own folder so it's never offered for a ChipSat
    let now = std::time::SystemTime::now();
    let place = crate::layout::folder(dir, crate::layout::RECEIVER_IMAGES, now)?;
    let path = crate::layout::unused(&place.dir, &crate::layout::hhmm(now), &crate::layout::board_word(port), ".bin")?;
    let dir = place.dir.as_path();

    // espflash reads to a file of its own and truncates it, so the chip is taken a
    // piece at a time and the pieces appended. Without this the operator watches one
    // unmoving line for several minutes and cannot tell it from a board that never
    // answered.
    let mut out = std::fs::File::create(&path).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    let part_path = dir.join("descent-ground-backup-part.tmp");
    let mut done: u32 = 0;
    while done < bytes {
        let n = BACKUP_CHUNK.min(bytes - done);
        flasher
            .read_flash(done, n, 0x1000, 64, part_path.clone())
            .map_err(|e| format!("could not read the board's firmware at {:#x}: {}", done, e))?;
        let part = std::fs::read(&part_path).map_err(|e| format!("cannot read back the piece: {}", e))?;
        use std::io::Write;
        out.write_all(&part).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        done += n;
        broadcast.send(&format!(
            "{{\"type\":\"flash\",\"pct\":{},\"note\":\"saving the board's current firmware, {} of {} kB\"}}",
            done as u64 * 100 / bytes as u64,
            done / 1024,
            bytes / 1024
        ));
    }
    drop(out);
    let _ = std::fs::remove_file(&part_path);

    let written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if written < bytes as u64 {
        return Err(format!(
            "the backup is short: {} of {} bytes. Nothing was erased.",
            written, bytes
        ));
    }
    Ok(path)
}

pub fn write_image(port: &str, image: &[u8], broadcast: &crate::ws::Broadcast) -> Result<(), String> {
    let mut flasher = open_flasher(port)?;
    let mut progress = Progress { broadcast, total: image.len() };
    flasher
        .write_bin_to_flash(0, image, &mut progress)
        .map_err(|e| format!("writing failed: {}", e))
}

// POST /api/flash  {"port":"/dev/ttyUSB0","sf":12}         the embedded image
//                  {"port":"...","sf":12,"image":"/path"}  one the operator built
pub fn handle_request(body: &str, ctx: &Arc<Ctx>) -> String {
    let port = match str_field(body, "port") {
        Some(p) => p,
        None => return err("no port in the request"),
    };
    let sf = int_field(body, "sf").unwrap_or(9);
    if !(6..=12).contains(&sf) {
        return err("spreading factor must be between 6 and 12");
    }

    let owned: Vec<u8>;
    let image: &[u8] = match str_field(body, "image") {
        Some(path) => {
            owned = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) => return err(&format!("cannot read {}: {}", path, e)),
            };
            &owned
        }
        None => match IMAGE {
            Some(i) => i,
            None => return err("this build has no receiver firmware in it, so point it at a .bin"),
        },
    };
    if let Err(e) = check_image(image) {
        return err(&e);
    }

    // reading the chip takes minutes and the app has the firmware anyway (decided 2026-10-21)
    let keep_backup = crate::json::int_field(body, "backup").unwrap_or(0) != 0;

    if let Err(e) = ctx.hub.lock().unwrap().release_for_flash(&port) {
        return err(&e);
    }

    let result = if keep_backup {
        backup(&port, &ctx.data, &ctx.broadcast).and_then(|saved| {
            println!("saved the board's existing firmware to {}", saved.display());
            ctx.broadcast.send(&format!(
                "{{\"type\":\"flash\",\"pct\":0,\"note\":\"saved {}\"}}",
                esc(&saved.to_string_lossy())
            ));
            write_image(&port, image, &ctx.broadcast).map(|()| saved)
        })
    } else {
        println!("{}: not saving the existing firmware, you asked to skip it", port);
        ctx.broadcast.send("{\"type\":\"flash\",\"pct\":0,\"note\":\"not saving the old firmware, you asked to skip it\"}");
        write_image(&port, image, &ctx.broadcast).map(|()| std::path::PathBuf::new())
    };

    ctx.hub.lock().unwrap().take_back(&port);

    match result {
        Ok(saved) => {
            let saved_text = if saved.as_os_str().is_empty() {
                String::from("not saved, you skipped it")
            } else {
                saved.to_string_lossy().to_string()
            };
            // The board is rebooting. poll() reopens the port; the spreading factor
            // goes out once its boot header has arrived, because a command sent to a
            // booting board is lost.
            let hub = ctx.hub.clone();
            let port2 = port.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(6));
                let key = hub.lock().unwrap().ports().into_iter()
                    .find(|p| p.port == port2)
                    .map(|p| p.key);
                if let Some(k) = key {
                    let _ = hub.lock().unwrap().send(&k, &format!("#SET,sf,{}\n", sf));
                }
            });
            format!("{{\"ok\":true,\"backup\":\"{}\",\"sf\":{}}}", esc(&saved_text), sf)
        }
        Err(e) => err(&e),
    }
}

fn err(msg: &str) -> String {
    format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver_image::BOOTLOADER_AT;

    #[test]
    fn an_open_receiver_is_still_a_flash_candidate_because_that_is_the_normal_case() {
        // Discovery opens every board it recognises, so the one the operator wants to
        // reflash is always already a receiver. Listing only closed ports would list none.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = crate::serial::Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        let c = candidates_from(&hub, &["/dev/ttyUSB0".to_string(), "/dev/ttyUSB1".to_string()]);
        assert_eq!(c, vec!["/dev/ttyUSB0".to_string(), "/dev/ttyUSB1".to_string()]);
    }

    #[test]
    fn releasing_a_port_for_a_flash_closes_it_and_keeps_discovery_off_it() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut hub = crate::serial::Hub::new(tx);
        hub.pretend_open("rx1", "/dev/ttyUSB0");
        hub.release_for_flash("/dev/ttyUSB0").unwrap();
        assert!(hub.ports().is_empty());
        let e = hub.release_for_flash("/dev/ttyUSB0").unwrap_err();
        assert!(e.contains("already being flashed"));
        hub.take_back("/dev/ttyUSB0");
        hub.release_for_flash("/dev/ttyUSB0").unwrap();
    }

    #[test]
    fn the_carried_firmware_is_given_with_the_version_in_its_name() {
        assert_eq!(
            firmware_json(Some("descent-raw-receiver-1.2.1.bin")),
            "\"firmware\":\"descent-raw-receiver-1.2.1.bin\",\"firmwareVersion\":\"1.2.1\""
        );
        assert_eq!(
            firmware_json(Some("descent-raw-receiver-1.10.0.bin")),
            "\"firmware\":\"descent-raw-receiver-1.10.0.bin\",\"firmwareVersion\":\"1.10.0\""
        );
        // A name with no numbers in it has no version to give, and neither has no image.
        assert_eq!(firmware_json(Some("spare.bin")), "\"firmware\":\"spare.bin\",\"firmwareVersion\":null");
        assert_eq!(firmware_json(None), "\"firmware\":null,\"firmwareVersion\":null");

        // And it is what the list of candidates ends with, for the image in this build.
        let carried = if IMAGE.is_some() { Some(IMAGE_NAME) } else { None };
        assert!(candidates_tail().ends_with(&format!(",{}}}", firmware_json(carried))), "{}", candidates_tail());
    }

    // The list with no ports in it, which is all of it that does not depend on what
    // is plugged into the machine running the test.
    fn candidates_tail() -> String {
        candidates_json_of(&[])
    }

    #[test]
    fn an_operator_supplied_image_must_be_a_merged_one() {
        let mut merged = vec![0xFF; 300_000];
        merged[BOOTLOADER_AT] = 0xE9;
        assert!(check_image(&merged).is_ok());

        // An app image on its own: 0xE9 at the front, nothing at 0x1000.
        let mut app = vec![0x00; 300_000];
        app[0] = 0xE9;
        assert!(check_image(&app).unwrap_err().contains("merging"));

        assert!(check_image(&[0x00; 300_000]).unwrap_err().contains("0x1000"));
        assert!(check_image(&[0xE9; 10]).unwrap_err().contains("too small"));
    }

    #[test]
    fn the_image_we_ship_passes_our_own_check() {
        match IMAGE {
            Some(i) => check_image(i).expect("the embedded firmware must be flashable"),
            None => panic!("this build has no embedded firmware; build the merged image first"),
        }
    }
}
