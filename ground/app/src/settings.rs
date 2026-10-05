// The settings block a flight image carries: which ChipSat it is, which module it drives
// and how it transmits, as 32 bytes inside the image so they can be found without
// symbols. The firmware finds the block by its magic and believes it only if its CRC
// passes; with a block it does not believe it runs the values it was compiled with.
// The layout is the flight repo's and the block is written by its build. When a flight
// software tree is at hand a board gets other settings by being built and flashed again.
// When there is none, as with a downloaded release, the block in the image this app
// carries is rewritten here before that image is flashed (patch, below).
//
//   0..16  magic
//   16     layout version
//   17     ChipSat id, 1..254, 255 = unconfigured
//   18     module, 1 = LE, 2 = HP
//   19     spreading factor, 5..12
//   20     target dBm, signed
//   21     set by, 1 = the build, 2 = a tool
//   22..26 transmit interval in ms, 0 = back to back
//   26..30 reserved, zero
//   30..32 CRC16 over 0..30, low byte first, the same way dump.rs reads a slot's
use crate::dump::crc16;

pub const MAGIC: [u8; 16] = [
    0x5A, 0xA5, 0x43, 0x48, 0x49, 0x50, 0x53, 0x41, 0x54, 0x43, 0x46, 0x47, 0x5A, 0xA5, 0x7B, 0x3C,
];
pub const SIZE: usize = 32;
const CRC_AT: usize = 30;
const LAYOUT: u8 = 1;

// Said to the operator as it is. The second half is the part that matters at a bench:
// the board still transmits, with what its build gave it and not with what was asked.
pub const DAMAGED: &str = "the settings block is damaged; the board is running the values it was built with";

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub csid: u8,
    pub module: &'static str,
    pub sf: u8,
    pub dbm: i8,
    pub set_by: &'static str,
    pub tx_interval_ms: u32,
    // False means something this layout calls reserved is in use, and a writer that
    // patched the block would be overwriting it without knowing what it is.
    pub reserved_clear: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Nothing,
    Block(Settings),
    // A block with a good check and a layout this build does not know. Not decoded:
    // the fields may have moved.
    Layout(u8),
    // The magic is there and the check after it fails. Not the same as no block: the
    // firmware was built to read one and is not reading this one.
    Damaged,
}

// What a tool may set. The module is not here: the image's compiled fallback is for one
// module, and a block naming the other would put the board on the wrong amplifier the
// day the block was damaged. The other module means the other image.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub csid: u8,
    pub sf: u8,
    pub dbm: i8,
    pub tx_interval_ms: u32,
}

// Where the block is, by its magic alone, which is how the firmware finds it. Its build
// refuses to ship an image with any other number of them than one, so two here is an
// image that did not come from that build and nothing can say which the board reads.
pub fn locate(image: &[u8]) -> Result<Option<usize>, String> {
    let at: Vec<usize> = image.windows(MAGIC.len()).enumerate().filter(|(_, w)| *w == MAGIC).map(|(i, _)| i).collect();
    match at.len() {
        0 => Ok(None),
        1 => Ok(Some(at[0])),
        n => Err(format!(
            "{} settings blocks were found in the board's image, and there is no saying which one it reads",
            n
        )),
    }
}

// A value the layout has no meaning for, under a good CRC, is a block written by
// something that disagrees with this file about the layout. Guessing at it would put a
// wrong module on the screen, so it is an error.
fn decode(b: &[u8]) -> Result<Found, String> {
    if b[16] != LAYOUT {
        return Ok(Found::Layout(b[16]));
    }
    let odd = |what: &str, v: u8| format!("the settings block on the board has {} {}, which layout {} does not allow", what, v, LAYOUT);
    let csid = b[17];
    if csid == 0 {
        return Err(odd("ChipSat id", csid));
    }
    let module = match b[18] {
        1 => "le",
        2 => "hp",
        v => return Err(odd("module", v)),
    };
    let sf = b[19];
    if !(5..=12).contains(&sf) {
        return Err(odd("spreading factor", sf));
    }
    let set_by = match b[21] {
        1 => "build",
        2 => "tool",
        v => return Err(odd("set by", v)),
    };
    Ok(Found::Block(Settings {
        csid,
        module,
        sf,
        dbm: b[20] as i8,
        set_by,
        tx_interval_ms: u32::from_le_bytes([b[22], b[23], b[24], b[25]]),
        reserved_clear: b[26..CRC_AT].iter().all(|v| *v == 0),
    }))
}

// The block and where it is.
pub fn find_at(image: &[u8]) -> Result<(Found, Option<usize>), String> {
    let Some(at) = locate(image)? else { return Ok((Found::Nothing, None)) };
    // An image that ends inside the block has a block that cannot pass its check.
    let Some(b) = image.get(at..at + SIZE) else { return Ok((Found::Damaged, Some(at))) };
    if u16::from_le_bytes([b[CRC_AT], b[CRC_AT + 1]]) != crc16(&b[..CRC_AT]) {
        return Ok((Found::Damaged, Some(at)));
    }
    Ok((decode(b)?, Some(at)))
}

#[cfg(test)]
pub fn find(image: &[u8]) -> Result<Found, String> {
    find_at(image).map(|(found, _)| found)
}

// The block a tool may rewrite and where it is, or why it may not, in words for whoever
// asked.
pub fn writable(image: &[u8]) -> Result<(usize, Settings), String> {
    match find_at(image)? {
        (Found::Block(s), Some(at)) if s.reserved_clear => Ok((at, s)),
        (Found::Block(_), _) => Err(String::from(
            "the settings block uses bytes this build calls reserved, so it is left alone: something newer than this keeps a value there",
        )),
        (Found::Damaged, _) => Err(String::from(
            "the settings block in the image is damaged, so it is not patched",
        )),
        (Found::Layout(v), _) => Err(format!(
            "the settings block is layout {}, which this build does not know how to write",
            v
        )),
        (Found::Nothing, _) => Err(String::from("this firmware has no settings block")),
    }
}

// The image with its block rewritten: what was asked for, set by a tool, reserved bytes
// zero, CRC to match. The module byte is left as it is, and so is every byte outside
// the block.
pub fn patch(image: &[u8], at: usize, asked: &Asked) -> Result<Vec<u8>, String> {
    if image.get(at..at + MAGIC.len()) != Some(&MAGIC[..]) || image.len() < at + SIZE {
        return Err(format!("there is no settings block at {:#x} to patch", at));
    }
    let mut out = image.to_vec();
    let b = &mut out[at..at + SIZE];
    b[16] = LAYOUT;
    b[17] = asked.csid;
    b[19] = asked.sf;
    b[20] = asked.dbm as u8;
    b[21] = 2;   // a tool wrote it
    b[22..26].copy_from_slice(&asked.tx_interval_ms.to_le_bytes());
    b[26..CRC_AT].fill(0);
    let crc = crc16(&b[..CRC_AT]);
    b[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
    // This goes onto a board whole. A byte changed anywhere else would be flight
    // software changed, so it is checked and not assumed.
    if out[..at] != image[..at] || out[at + SIZE..] != image[at + SIZE..] {
        return Err(String::from("patching the settings block changed bytes outside it, so nothing was written"));
    }
    Ok(out)
}

impl Found {
    // As /api/chip/read gives it under "settings". A layout this build does not know and
    // a damaged block give nothing here; the caller says what is there in words.
    pub fn json(&self) -> String {
        match self {
            Found::Nothing | Found::Layout(_) | Found::Damaged => String::from("null"),
            Found::Block(s) => format!(
                "{{\"layoutVersion\":{},\"csid\":{},\"module\":\"{}\",\"sf\":{},\"dbm\":{},\"setBy\":\"{}\",\"txIntervalMs\":{},\"reservedClear\":{}}}",
                LAYOUT, s.csid, s.module, s.sf, s.dbm, s.set_by, s.tx_interval_ms, s.reserved_clear
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, interval: u32) -> [u8; SIZE] {
        let mut b = [0u8; SIZE];
        b[..16].copy_from_slice(&MAGIC);
        b[16] = version;
        b[17] = csid;
        b[18] = module;
        b[19] = sf;
        b[20] = dbm as u8;
        b[21] = set_by;
        b[22..26].copy_from_slice(&interval.to_le_bytes());
        sealed(b)
    }

    fn sealed(mut b: [u8; SIZE]) -> [u8; SIZE] {
        let crc = crc16(&b[..CRC_AT]);
        b[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    // Something image-like around the blocks: not zeros and not erased.
    fn image(blocks: &[(usize, [u8; SIZE])]) -> Vec<u8> {
        let mut image: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        for (at, b) in blocks {
            image[*at..*at + SIZE].copy_from_slice(b);
        }
        image
    }

    #[test]
    fn a_block_in_an_image_is_found_and_read() {
        let b = block(1, 7, 2, 9, -4, 2, 30_000);
        let (found, at) = find_at(&image(&[(1001, b)])).unwrap();
        assert_eq!(at, Some(1001));
        assert_eq!(
            found,
            Found::Block(Settings {
                csid: 7,
                module: "hp",
                sf: 9,
                dbm: -4,
                set_by: "tool",
                tx_interval_ms: 30_000,
                reserved_clear: true,
            })
        );
        assert_eq!(
            found.json(),
            "{\"layoutVersion\":1,\"csid\":7,\"module\":\"hp\",\"sf\":9,\"dbm\":-4,\"setBy\":\"tool\",\"txIntervalMs\":30000,\"reservedClear\":true}"
        );

        // What a build with fixed settings carries.
        let built = find(&image(&[(2048, block(1, 255, 1, 7, 14, 1, 1000))])).unwrap();
        assert_eq!(
            built.json(),
            "{\"layoutVersion\":1,\"csid\":255,\"module\":\"le\",\"sf\":7,\"dbm\":14,\"setBy\":\"build\",\"txIntervalMs\":1000,\"reservedClear\":true}"
        );

        // Right at the end of what was read, with nothing after it.
        let mut tail = image(&[]);
        tail.extend_from_slice(&b);
        assert!(matches!(find(&tail).unwrap(), Found::Block(_)));
    }

    #[test]
    fn the_crc_is_stored_low_byte_first() {
        let b = block(1, 7, 2, 9, -4, 2, 30_000);
        let crc = crc16(&b[..30]);
        assert_eq!([b[30], b[31]], [(crc & 0xFF) as u8, (crc >> 8) as u8]);
        // The same block with the two bytes the other way round does not pass.
        let mut swapped = b;
        swapped.swap(30, 31);
        assert_ne!(b[30], b[31], "a block whose CRC reads the same both ways proves nothing");
        assert_eq!(find(&image(&[(1001, swapped)])).unwrap(), Found::Damaged);
    }

    #[test]
    fn an_image_with_no_magic_in_it_has_no_settings_block() {
        assert_eq!(find_at(&image(&[])).unwrap(), (Found::Nothing, None));
        assert_eq!(find(&[]).unwrap(), Found::Nothing);
        assert_eq!(find(&MAGIC[..15]).unwrap(), Found::Nothing);
        assert_eq!(Found::Nothing.json(), "null");
    }

    #[test]
    fn the_magic_with_a_check_that_fails_is_a_damaged_block_and_not_no_block() {
        let b = block(1, 7, 2, 9, -4, 2, 30_000);
        for bit in [16 * 8, 17 * 8 + 3, 22 * 8, 29 * 8 + 7, 30 * 8, 31 * 8 + 7] {
            let mut bad = b;
            bad[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(find_at(&image(&[(1001, bad)])).unwrap(), (Found::Damaged, Some(1001)), "bit {}", bit);
        }
        assert_eq!(Found::Damaged.json(), "null");
        // A layout byte nobody knows is only believed under a check that passes.
        let mut odd = block(2, 7, 2, 9, -4, 2, 30_000);
        odd[31] ^= 0xFF;
        assert_eq!(find(&image(&[(1001, odd)])).unwrap(), Found::Damaged);
        // The magic with the image ending before the block does.
        assert_eq!(find(&MAGIC).unwrap(), Found::Damaged);
        let mut cut = image(&[]);
        cut.extend_from_slice(&b[..31]);
        assert_eq!(find(&cut).unwrap(), Found::Damaged);
    }

    #[test]
    fn two_magics_are_an_error_that_says_how_many() {
        let b = block(1, 7, 2, 9, -4, 2, 30_000);
        let e = find(&image(&[(1001, b), (3000, block(1, 8, 1, 7, 14, 1, 0))])).unwrap_err();
        assert!(e.contains("2 settings blocks"), "{}", e);
        // The same block twice is still two.
        assert!(find(&image(&[(1001, b), (3000, b)])).unwrap_err().contains("2 settings blocks"));
        // So is a good one beside a damaged one, or beside the magic on its own: the
        // firmware goes by the magic, and so does this.
        let mut bad = b;
        bad[17] ^= 1;
        assert!(find(&image(&[(1001, b), (3000, bad)])).unwrap_err().contains("2 settings blocks"));
        let mut bare = image(&[(1001, b)]);
        bare[200..216].copy_from_slice(&MAGIC);
        assert!(locate(&bare).unwrap_err().contains("2 settings blocks"));
        let mut three = bare.clone();
        three[300..316].copy_from_slice(&MAGIC);
        assert!(find(&three).unwrap_err().contains("3 settings blocks"));
    }

    #[test]
    fn a_layout_this_build_does_not_know_is_named_and_not_decoded() {
        let found = find(&image(&[(1001, block(2, 7, 2, 9, -4, 2, 30_000))])).unwrap();
        assert_eq!(found, Found::Layout(2));
        // Nothing of it goes to the page as settings: fields read by the wrong layout
        // would look like settings and be wrong.
        assert_eq!(found.json(), "null");
    }

    #[test]
    fn reserved_bytes_in_use_are_read_past_and_flagged() {
        let mut b = block(1, 7, 2, 9, -4, 2, 30_000);
        b[27] = 1;
        match find(&image(&[(1001, sealed(b))])).unwrap() {
            Found::Block(s) => {
                assert!(!s.reserved_clear);
                assert_eq!(s.csid, 7);
            }
            other => panic!("{:?}", other),
        }
        assert!(find(&image(&[(1001, sealed(b))])).unwrap().json().ends_with("\"reservedClear\":false}"));
    }

    #[test]
    fn a_field_the_layout_does_not_allow_is_an_error_and_not_a_guess() {
        for (what, b) in [
            ("module 3", block(1, 7, 3, 9, -4, 2, 0)),
            ("spreading factor 13", block(1, 7, 2, 13, -4, 2, 0)),
            ("spreading factor 4", block(1, 7, 2, 4, -4, 2, 0)),
            ("ChipSat id 0", block(1, 0, 2, 9, -4, 2, 0)),
            ("set by 0", block(1, 7, 2, 9, -4, 0, 0)),
        ] {
            let e = find(&image(&[(1001, b)])).unwrap_err();
            assert!(e.contains(what), "{}: {}", what, e);
        }
    }

    const ASKED: Asked = Asked { csid: 12, sf: 9, dbm: 20, tx_interval_ms: 1000 };

    #[test]
    fn a_patch_changes_the_block_and_nothing_else() {
        let before = image(&[(1001, block(1, 255, 2, 7, 14, 1, 1000))]);
        let (at, was) = writable(&before).unwrap();
        assert_eq!((at, was.module, was.set_by), (1001, "hp", "build"));

        let after = patch(&before, at, &ASKED).unwrap();
        assert_eq!(after.len(), before.len());
        let changed: Vec<usize> = (0..before.len()).filter(|i| before[*i] != after[*i]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|i| (1001..1001 + SIZE).contains(i)), "{:?}", changed);

        // And this file's own reader takes what it wrote.
        assert_eq!(
            find_at(&after).unwrap(),
            (
                Found::Block(Settings {
                    csid: 12,
                    module: "hp",
                    sf: 9,
                    dbm: 20,
                    set_by: "tool",
                    tx_interval_ms: 1000,
                    reserved_clear: true,
                }),
                Some(1001)
            )
        );
        // Byte for byte, against a block built here by hand.
        assert_eq!(after[1001..1001 + SIZE], block(1, 12, 2, 9, 20, 2, 1000));

        // A negative power and the widest interval fit their fields.
        let odd = patch(&before, at, &Asked { csid: 1, sf: 12, dbm: -9, tx_interval_ms: u32::MAX }).unwrap();
        assert_eq!(odd[1001..1001 + SIZE], block(1, 1, 2, 12, -9, 2, u32::MAX));
        // Patching what a tool already set works the same way.
        let again = patch(&after, at, &Asked { csid: 13, sf: 7, dbm: 14, tx_interval_ms: 0 }).unwrap();
        assert_eq!(again[1001..1001 + SIZE], block(1, 13, 2, 7, 14, 2, 0));
    }

    #[test]
    fn a_block_that_cannot_be_trusted_is_not_one_to_patch() {
        let e = writable(&image(&[])).unwrap_err();
        assert_eq!(e, "this firmware has no settings block");

        let mut bad = block(1, 255, 1, 7, 14, 1, 1000);
        bad[19] ^= 0x10;
        let e = writable(&image(&[(1001, bad)])).unwrap_err();
        assert!(e.contains("damaged"), "{}", e);

        let e = writable(&image(&[(1001, block(2, 255, 1, 7, 14, 1, 1000))])).unwrap_err();
        assert!(e.contains("layout 2"), "{}", e);

        let mut reserved = block(1, 255, 1, 7, 14, 1, 1000);
        reserved[28] = 0x40;
        let e = writable(&image(&[(1001, sealed(reserved))])).unwrap_err();
        assert!(e.contains("reserved"), "{}", e);

        // Told to patch somewhere that is not a block, it does not.
        let whole = image(&[(1001, block(1, 255, 1, 7, 14, 1, 1000))]);
        assert!(patch(&whole, 1000, &ASKED).is_err());
        assert!(patch(&whole, 4090, &ASKED).is_err());
    }

    #[test]
    fn each_flight_image_in_this_build_has_one_block_that_says_what_it_was_built_with() {
        // Whatever chipsat/firmware held at build time: no images, or images whose
        // flight.json says they carry a block.
        let flight = crate::flight::carried();
        let Some(build) = &flight.build else { return };
        if !build.settings_block {
            return;
        }
        for image in flight.usable() {
            let s = match find_at(image.bytes) {
                Ok((Found::Block(s), Some(_))) => s,
                other => panic!("{}: {:?}", image.file, other),
            };
            assert_eq!(s.module, image.module, "{}", image.file);
            assert_eq!((s.set_by, s.csid as i64, s.sf as i64, s.dbm as i64), ("build", build.csid, build.sf, build.dbm), "{}", image.file);
            assert!(s.reserved_clear, "{}", image.file);
        }
    }
}
