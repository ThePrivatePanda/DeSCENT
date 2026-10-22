// Where saves go under the data folder, one folder per local day
//
//   2026-10-18/pulls/1432-board9/            dump.txt, ground.log, report.json, the image
//   2026-10-18/clears/1125-board9/
//   2026-10-18/images/1101-board9-0027003A.bin
//   2026-10-18/logs/1402-ground.log
//   2026-10-18/receiver-images/1647-ttyACM0.bin
//
// Local time, it's what the operator reads at the field. Older saves sit flat as descent-...
// and are still read
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const PULLS: &str = "pulls";
pub const CLEARS: &str = "clears";
pub const IMAGES: &str = "images";
pub const LOGS: &str = "logs";
// ESP32 images off receivers, kept apart so they are never offered for a ChipSat
pub const RECEIVER_IMAGES: &str = "receiver-images";

fn local(at: SystemTime) -> chrono::DateTime<chrono::Local> {
    at.into()
}

// 2026-10-18
pub fn day(at: SystemTime) -> String {
    local(at).format("%Y-%m-%d").to_string()
}

// 1432
pub fn hhmm(at: SystemTime) -> String {
    local(at).format("%H%M").to_string()
}

// rel is the path /api/log uses
pub struct Place {
    pub dir: PathBuf,
    pub rel: String,
}

impl Place {
    pub fn rel_of(&self, leaf: &str) -> String {
        format!("{}/{}", self.rel, leaf)
    }
}

// <data>/<day>/<kind>, created if missing
pub fn folder(data: &Path, kind: &str, at: SystemTime) -> Result<Place, String> {
    let day = day(at);
    let dir = data.join(&day).join(kind);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {}", dir.display(), e))?;
    Ok(Place { dir, rel: format!("{}/{}", day, kind) })
}

// a bare 9 in a folder name says nothing, so numbers become board9
pub fn board_word(name: &str) -> String {
    let t = crate::stm32::tidy(name);
    if t.chars().all(|c| c.is_ascii_digit()) {
        format!("board{}", t)
    } else {
        t
    }
}

// <hhmm>-<rest><ext>, then <hhmm>-2-<rest><ext> and so on, two saves in one minute do happen.
// The count goes after the time since at the end it would look like part of "Flight spare 2".
// Empty ext for a folder
pub fn unused(dir: &Path, hhmm: &str, rest: &str, ext: &str) -> Result<PathBuf, String> {
    for n in 1..1000 {
        let path = if n < 2 {
            dir.join(format!("{}-{}{}", hhmm, rest, ext))
        } else {
            dir.join(format!("{}-{}-{}{}", hhmm, n, rest, ext))
        };
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(format!("{} already holds 999 things called {}-{}{}", dir.display(), hhmm, rest, ext))
}

// exactly 2026-10-18 style
pub fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

// 0027003A from 1101-board9-0027003A.bin or 1101-2-board9-0027003A.bin, None otherwise
pub fn uid_in_image(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".bin")?;
    let parts: Vec<&str> = stem.split('-').collect();
    let first = parts.first()?;
    if first.len() != 4 || !first.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let tail = parts.last()?;
    if parts.len() >= 3 && tail.len() == 8 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(tail.to_ascii_uppercase())
    } else {
        None
    }
}

// board id, one line, next to a pull's or clear's files. The image used to be the only record
// of it and only the newest image per board is kept
pub const BOARD_FILE: &str = "board.txt";

// board9 from 1432-board9 or 1432-2-board9
pub fn board_of_folder(leaf: &str) -> Option<&str> {
    let (hm, rest) = leaf.split_once('-')?;
    if hm.len() != 4 || !hm.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = match rest.split_once('-') {
        Some((n, after)) if n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) && !after.is_empty() => after,
        _ => rest,
    };
    Some(rest).filter(|r| !r.is_empty())
}

pub fn board_in(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(BOARD_FILE)).ok()?;
    crate::stm32::a_uid(text.trim())
}

// known dump builds by hash, or anything with the sketch's name in it
pub fn looks_like_dump(image: &[u8]) -> bool {
    crate::chipflash::recognise(image, &crate::chipflash::Known::carried()).kind == "dump"
        || image.windows(9).any(|w| w == b"flashdump")
}

// ChipSat images in both layouts, never receiver images
pub fn saved_images(data: &Path) -> Vec<PathBuf> {
    let list = |d: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).collect()
    };
    let bins = |d: &Path| -> Vec<PathBuf> {
        list(d).into_iter().filter(|p| p.extension().is_some_and(|x| x == "bin") && p.is_file()).collect()
    };
    let mut out = Vec::new();
    for entry in list(data) {
        let name = entry.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if is_day(&name) {
            out.extend(bins(&entry.join(IMAGES)));
            for kind in [PULLS, CLEARS] {
                for folder in list(&entry.join(kind)) {
                    out.extend(bins(&folder));
                }
            }
        } else if name.starts_with("descent-pull-") || name.starts_with("descent-clear-") {
            out.extend(bins(&entry));
        } else if name.starts_with(crate::stm32::IMAGE_PREFIX) && name.ends_with(".bin") {
            out.push(entry);
        }
    }
    out
}

// Deletes every other image of the same board as `keep`, returns what went
pub fn keep_only_latest(data: &Path, keep: &Path) -> Vec<PathBuf> {
    let Some(file) = keep.file_name().map(|n| n.to_string_lossy().to_string()) else { return Vec::new() };
    let Some(tail) = crate::stm32::uid_in_name(&file) else { return Vec::new() };
    let keep = std::fs::canonicalize(keep).unwrap_or_else(|_| keep.to_path_buf());
    let mut gone = Vec::new();
    for path in saved_images(data) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if crate::stm32::uid_in_name(&name).as_deref() != Some(tail.as_str()) {
            continue;
        }
        if std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()) == keep {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            gone.push(path);
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_reads_as_a_board_and_a_name_as_itself() {
        assert_eq!(board_word("9"), "board9");
        assert_eq!(board_word("Flight spare 2"), "Flight-spare-2");
        assert_eq!(board_word("/dev/ttyACM1"), "ttyACM1");
        assert_eq!(board_word(""), "board");
    }

    #[test]
    fn a_day_is_a_date_and_nothing_else() {
        assert!(is_day("2026-10-18"));
        for no in ["2026-10-1", "2026_10_18", "descent-pull", "2026-10-18x", "abcd-ef-gh"] {
            assert!(!is_day(no), "{}", no);
        }
        assert!(is_day(&day(SystemTime::now())));
        assert_eq!(hhmm(SystemTime::now()).len(), 4);
    }

    #[test]
    fn the_board_is_read_back_off_an_image_name() {
        assert_eq!(uid_in_image("1101-board9-0027003a.bin").as_deref(), Some("0027003A"));
        assert_eq!(uid_in_image("1101-2-board9-0027003A.bin").as_deref(), Some("0027003A"));
        assert_eq!(uid_in_image("1101-Flight-spare-2-20393256.bin").as_deref(), Some("20393256"));
        // no id, so named by port
        assert_eq!(uid_in_image("1101-ttyACM1.bin"), None);
        assert_eq!(uid_in_image("1101-2-ttyACM1.bin"), None);
        assert_eq!(uid_in_image("descent-image-0027003A-9-2026-10-17T15-01-23.bin"), None);
        assert_eq!(uid_in_image("1101-board9-0027003A.txt"), None);
    }

    #[test]
    fn a_second_save_in_the_same_minute_gets_a_name_of_its_own() {
        let dir = std::env::temp_dir().join(format!("dg-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let place = folder(&dir, PULLS, SystemTime::now()).unwrap();
        assert!(place.dir.is_dir());
        assert!(place.rel.ends_with("/pulls") && is_day(&place.rel[..10]), "{}", place.rel);
        let first = unused(&place.dir, "1432", "board9", "").unwrap();
        assert!(first.ends_with("1432-board9"));
        std::fs::create_dir(&first).unwrap();
        assert!(unused(&place.dir, "1432", "board9", "").unwrap().ends_with("1432-2-board9"));
        assert!(unused(&place.dir, "1432", "Flight-spare-2", "").unwrap().ends_with("1432-Flight-spare-2"));
        std::fs::write(place.dir.join("1101-board9-0027003A.bin"), b"x").unwrap();
        assert!(unused(&place.dir, "1101", "board9-0027003A", ".bin").unwrap().ends_with("1101-2-board9-0027003A.bin"));
        for n in 2..1000 {
            std::fs::create_dir(place.dir.join(format!("1433-{}-x", n))).unwrap();
        }
        std::fs::create_dir(place.dir.join("1433-x")).unwrap();
        assert!(unused(&place.dir, "1433", "x", "").is_err(), "never a name that is taken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_image_of_a_board_is_kept_and_other_boards_are_left_alone() {
        let dir = std::env::temp_dir().join(format!("dg-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["2026-10-17/images", "2026-10-17/pulls/1509-board12", "2026-10-17/clears/1512-board12", "2026-10-18/images", "2026-10-18/receiver-images"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let older = [
            "2026-10-17/images/1508-board12-002C002E.bin",
            "2026-10-17/pulls/1509-board12/1509-board12-002C002E.bin",
            "2026-10-17/clears/1512-board12/1512-board12-002C002E.bin",
            "descent-image-002C002E-12-2026-10-13T17-22-50.bin",
        ];
        let others = ["2026-10-17/images/1454-board9-0027003A.bin", "2026-10-18/receiver-images/1647-ttyACM0.bin", "2026-10-17/pulls/1509-board12/dump.txt"];
        for f in older.iter().chain(others.iter()) {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let keep = dir.join("2026-10-18/images/1001-board12-002C002E.bin");
        std::fs::write(&keep, b"new").unwrap();
        let gone = keep_only_latest(&dir, &keep);
        assert_eq!(gone.len(), older.len(), "{:?}", gone);
        for f in older {
            assert!(!dir.join(f).exists(), "{}", f);
        }
        for f in others {
            assert!(dir.join(f).exists(), "{}", f);
        }
        assert!(keep.exists());
        // no board in the name, nothing removed
        assert!(keep_only_latest(&dir, &dir.join("2026-10-18/images/1001-ttyACM1.bin")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_dump_firmware_is_told_from_flight_software() {
        assert!(looks_like_dump(b"....I BOOT start test=flashdump...."));
        assert!(!looks_like_dump(b"flight software, no such word in it"));
        if let Some(dump) = crate::flash::CHIPSAT_DUMP {
            assert!(looks_like_dump(dump));
        }
        if let Ok(flight) = crate::flight::carried().choose(Some("le")) {
            assert!(!looks_like_dump(flight.bytes));
        }
    }
}
