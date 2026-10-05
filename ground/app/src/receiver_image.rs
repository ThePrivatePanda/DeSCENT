// Whether a file is a receiver image that can be written at 0x0, and which of several to
// carry. Compiled twice: into the app, which checks an image before it is flashed, and
// into build.rs, which uses the same check to choose the image the app carries. One
// file, so what is embedded and what is accepted cannot come apart. It has to stand on
// its own for that: nothing from the rest of the crate can be used here.
const MIN_IMAGE: usize = 200_000;
const MAX_IMAGE: usize = 4 * 1024 * 1024;

// We write at 0x0, so the file has to be a merged image: the first 4 kB are padding
// and the bootloader's 0xE9 magic sits at 0x1000. A bare app image also starts with
// 0xE9, and writing one at 0x0 leaves a board that needs a cable and a rescue, so it
// is named and refused rather than accepted.
pub const BOOTLOADER_AT: usize = 0x1000;

pub fn check_image(data: &[u8]) -> Result<(), String> {
    if data.len() < MIN_IMAGE {
        return Err(format!("that file is too small for a receiver image ({} bytes)", data.len()));
    }
    if data.len() > MAX_IMAGE {
        return Err(format!("that file is larger than the flash ({} bytes)", data.len()));
    }
    if data.get(BOOTLOADER_AT) == Some(&0xE9) {
        return Ok(());
    }
    if data.first() == Some(&0xE9) {
        return Err(String::from(
            "that is an app image on its own. It needs merging with the bootloader and partitions first (esptool merge_bin).",
        ));
    }
    Err(String::from("that file has no esp32 bootloader at 0x1000, so it is not a merged image"))
}

// The numbers in a file name, in order: descent-raw-receiver-1.10.0.bin is [1, 10, 0].
// Compared as numbers, because as text 1.10.0 sorts below 1.2.0.
pub fn version(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect()
}

// The newest, by the version in its name, of the images that pass, and each one that does
// not with why.
// A release folder can hold an app image beside the merged ones, and the newest file in
// it is then one the app would embed and refuse to write. Only build.rs calls this.
#[cfg_attr(not(test), allow(dead_code))]
pub fn newest_whole(images: &[(String, Vec<u8>)]) -> (Option<String>, Vec<(String, String)>) {
    let mut newest: Option<&String> = None;
    let mut skipped = Vec::new();
    for (name, bytes) in images {
        match check_image(bytes) {
            Ok(()) => {
                if newest.map(|n| (version(name), name) > (version(n), n)).unwrap_or(true) {
                    newest = Some(name);
                }
            }
            Err(why) => skipped.push((name.clone(), why)),
        }
    }
    skipped.sort();
    (newest.cloned(), skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(name: &str) -> (String, Vec<u8>) {
        let mut bytes = vec![0xFF; 300_000];
        bytes[BOOTLOADER_AT] = 0xE9;
        (name.to_string(), bytes)
    }

    fn bare(name: &str) -> (String, Vec<u8>) {
        let mut bytes = vec![0x00; 300_000];
        bytes[0] = 0xE9;
        (name.to_string(), bytes)
    }

    #[test]
    fn the_image_carried_is_the_newest_one_that_can_be_written_at_zero() {
        // An app image on its own, newer than the merged ones beside it.
        let folder = vec![whole("rx-1.0.0.bin"), bare("rx-1.2.0.bin"), whole("rx-1.1.0.bin")];
        let (carried, skipped) = newest_whole(&folder);
        assert_eq!(carried.as_deref(), Some("rx-1.1.0.bin"));
        assert_eq!(skipped.len(), 1, "{:?}", skipped);
        assert_eq!(skipped[0].0, "rx-1.2.0.bin");
        assert!(skipped[0].1.contains("app image on its own"), "{}", skipped[0].1);

        // Whatever order the folder was read in.
        let folder = vec![whole("rx-1.1.0.bin"), whole("rx-1.0.0.bin")];
        assert_eq!(newest_whole(&folder), (Some("rx-1.1.0.bin".to_string()), Vec::new()));

        // One that is passed over is named even when it would not have been chosen.
        let folder = vec![bare("rx-0.9.0.bin"), whole("rx-1.0.0.bin")];
        let (carried, skipped) = newest_whole(&folder);
        assert_eq!(carried.as_deref(), Some("rx-1.0.0.bin"));
        assert_eq!(skipped[0].0, "rx-0.9.0.bin");
    }

    #[test]
    fn a_folder_with_nothing_that_can_be_written_carries_nothing() {
        assert_eq!(newest_whole(&[]), (None, Vec::new()));

        let folder = vec![bare("rx-1.2.0.bin"), ("notes.bin".to_string(), vec![0u8; 10])];
        let (carried, skipped) = newest_whole(&folder);
        assert_eq!(carried, None);
        assert_eq!(skipped.len(), 2);
        assert!(skipped.iter().any(|(name, why)| name == "notes.bin" && why.contains("too small")), "{:?}", skipped);
    }

    #[test]
    fn ten_is_newer_than_two() {
        // By name alone 1.10.0 sorts below 1.2.0, and the day the tenth minor version
        // ships the app would go on carrying the second.
        let images = vec![
            whole("descent-raw-receiver-1.2.0.bin"),
            whole("descent-raw-receiver-1.10.0.bin"),
            whole("descent-raw-receiver-1.9.3.bin"),
        ];
        assert_eq!(newest_whole(&images).0.as_deref(), Some("descent-raw-receiver-1.10.0.bin"));
        // A name with no numbers in it still has a place: below every numbered one.
        let odd = vec![whole("spare.bin"), whole("descent-raw-receiver-0.1.0.bin")];
        assert_eq!(newest_whole(&odd).0.as_deref(), Some("descent-raw-receiver-0.1.0.bin"));
    }
}
