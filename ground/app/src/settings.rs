// The settings block a flight image carries: which ChipSat it is, which module it drives
// and how it transmits, as bytes inside the image so they can be found without symbols.
// The firmware finds the block by its magic and believes it only if its CRC passes; with
// a block it does not believe it runs the values it was compiled with. The layout is the
// flight repo's and the block is written by its build. When a flight software tree is at
// hand a board gets other settings by being built and then patched; when there is none,
// as with a downloaded release, the block in the image this app carries is rewritten
// here before that image is flashed (patch, below).
//
// Layout 3, 160 bytes, what every image built since 2026-10-16 carries:
//   0..16   magic
//   16      layout version
//   17      ChipSat id, 1..253, 254 = the sensor bench (refused for a flight board), 255 = unconfigured
//   18      module, 1 = LE, 2 = HP
//   19      spreading factor, 5..12
//   20      target dBm, signed
//   21      set by, 1 = the build, 2 = a tool
//   22..138 one rate per slot, 29 bytes each, waiting, rising, falling, landed:
//             +0  fixed gap after a packet, ms
//             +4  1 = the gap follows the battery table below, 0 = the fixed gap always
//             +5  four state-of-charge thresholds in percent, falling, each 1..99
//             +9  five gaps in ms, one per band: at or above each threshold, then below them all
//           A phase that follows the battery uses its fixed gap while the gauge cannot be read.
//   138..154 the gap between flash log records in ms, one per slot in the same order.
//            0 is a record per IMU reading in the air and nothing at all on the ground;
//            1 to 19 ms the firmware refuses
//   154..158 the gap while no altitude source is live. Unused since 2026-10-22, but the
//            firmware still turns a block down over it, so it is still checked here
//   158..160 CRC16 over 0..158, low byte first, the same way dump.rs reads a slot's
// Since 2026-10-22 the board only has ground and air, slots 0 and 1. The page writes ground
// into waiting and landed and air into rising and falling, so older firmware still works.
// Layout 2 (2026-10) was the same with 138..158 reserved and zero: read, shown as a block
// with no log gaps, and rewritten as layout 3 when patched.
// Layout 1 was 32 bytes: the same through byte 22, one gap in ms at 22, reserved to 30,
// CRC at 30. Boards flashed before the phases still carry it, so it is read, and shown
// as a block with no rates. It is never written: the images this app patches are its
// own, and they are layout 3.
//
// The layout byte is read before the CRC, because it says where the CRC is. A layout
// nobody knows is reported as that and never decoded: there is no telling where its
// check sits, let alone its fields.
use crate::dump::crc16;

pub const MAGIC: [u8; 16] = [
    0x5A, 0xA5, 0x43, 0x48, 0x49, 0x50, 0x53, 0x41, 0x54, 0x43, 0x46, 0x47, 0x5A, 0xA5, 0x7B, 0x3C,
];
pub const SIZE: usize = 160;
const CRC_AT: usize = 158;
const LAYOUT: u8 = 3;
const V1_SIZE: usize = 32;
const V1_CRC_AT: usize = 30;
pub const PHASES: usize = 4;
pub const BANDS: usize = 5;
const RATES_AT: usize = 22;
const RATE_BYTES: usize = 29;
const LOGS_AT: usize = 138;
const LOG_BLIND_AT: usize = 154;
// The firmware refuses a log gap under this: the sensors make nothing faster.
pub const LOG_MIN_MS: u32 = 20;
pub const PHASE_NAMES: [&str; PHASES] = ["waiting", "rising", "falling", "landed"];
// What messages call them. The firmware reads slot 0 on the ground and 1 in the air since
// 2026-10-22, and the page fills 3 like 0 and 2 like 1 for older firmware; PHASE_NAMES is
// only what radio.json still calls them
pub const PHASE_WORDS: [&str; PHASES] = ["ground", "air", "air (falling slot)", "ground (landed slot)"];

// Said to the operator as it is. The second half is the part that matters at a bench:
// the board still transmits, with what its build gave it and not with what was asked.
pub const DAMAGED: &str = "the settings block is damaged; the board is running the values it was built with";

// How often one phase transmits: the gap after each packet. With `follow`, the gap is
// interval_ms[i] for the first threshold percent[i] the charge is at or above, and the last
// one below them all; the fixed gap then only applies while the gauge cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Rate {
    pub fixed_ms: u32,
    pub follow: bool,
    pub percent: [u8; BANDS - 1],
    pub interval_ms: [u32; BANDS],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub layout: u8,
    pub csid: u8,
    pub module: &'static str,
    pub sf: u8,
    pub dbm: i8,
    pub set_by: &'static str,
    // The flight rate: layout 1's one gap, layout 2's falling gap, layout 3's rising gap.
    pub tx_interval_ms: u32,
    // None on a layout-1 block: that firmware has one gap from switch-on to the end.
    pub rates: Option<[Rate; PHASES]>,
    // The flash log gap per phase, and the one with no altitude source. None on a block
    // from before layout 3: that firmware logs at 20 Hz in the air and 1 Hz on the ground.
    pub logs: Option<[u32; PHASES]>,
    pub log_blind_ms: Option<u32>,
    // False means something this layout calls reserved is in use, and a writer that
    // patched the block would be overwriting it without knowing what it is.
    pub reserved_clear: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Nothing,
    Block(Settings),
    // A block whose layout this build does not know. Not decoded, and not checked: the
    // fields may have moved, and so may the CRC.
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
    pub rates: [Rate; PHASES],
    pub logs: [u32; PHASES],
    pub log_blind_ms: u32,
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

fn odd(layout: u8, what: &str, v: impl std::fmt::Display) -> String {
    format!("the settings block on the board has {} {}, which layout {} does not allow", what, v, layout)
}

// The fields both layouts share. A value the layout has no meaning for, under a good
// CRC, is a block written by something that disagrees with this file about the layout.
// Guessing at it would put a wrong module on the screen, so it is an error.
fn decode_head(b: &[u8], layout: u8) -> Result<(u8, &'static str, u8, i8, &'static str), String> {
    let csid = b[17];
    if csid == 0 {
        return Err(odd(layout, "ChipSat id", csid));
    }
    let module = match b[18] {
        1 => "le",
        2 => "hp",
        v => return Err(odd(layout, "module", v)),
    };
    let sf = b[19];
    if !(5..=12).contains(&sf) {
        return Err(odd(layout, "spreading factor", sf));
    }
    let set_by = match b[21] {
        1 => "build",
        2 => "tool",
        v => return Err(odd(layout, "set by", v)),
    };
    Ok((csid, module, sf, b[20] as i8, set_by))
}

fn decode_v1(b: &[u8]) -> Result<Found, String> {
    let (csid, module, sf, dbm, set_by) = decode_head(b, 1)?;
    Ok(Found::Block(Settings {
        layout: 1,
        csid,
        module,
        sf,
        dbm,
        set_by,
        tx_interval_ms: u32::from_le_bytes([b[22], b[23], b[24], b[25]]),
        rates: None,
        logs: None,
        log_blind_ms: None,
        reserved_clear: b[26..V1_CRC_AT].iter().all(|v| *v == 0),
    }))
}

// The rules the firmware's ChipSatSettings::usable applies to a table, so a block this
// reads as settings is one the board is actually running.
pub fn table_trouble(percent: &[u8; BANDS - 1]) -> Option<String> {
    for (i, p) in percent.iter().enumerate() {
        if !(1..=99).contains(p) {
            return Some(format!("a battery threshold of {} %, which has to be 1 to 99", p));
        }
        if i > 0 && *p >= percent[i - 1] {
            return Some(format!("battery thresholds {} % then {} %, which have to fall", percent[i - 1], p));
        }
    }
    None
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn decode_rate(b: &[u8], layout: u8, phase: usize) -> Result<Rate, String> {
    let r = &b[RATES_AT + RATE_BYTES * phase..RATES_AT + RATE_BYTES * (phase + 1)];
    let follow = match r[4] {
        0 => false,
        1 => true,
        v => return Err(odd(layout, &format!("{} battery flag", PHASE_WORDS[phase]), v)),
    };
    let mut percent = [0u8; BANDS - 1];
    percent.copy_from_slice(&r[5..9]);
    if let Some(t) = table_trouble(&percent) {
        return Err(odd(layout, &format!("for {}", PHASE_WORDS[phase]), t));
    }
    let mut interval_ms = [0u32; BANDS];
    for (i, v) in interval_ms.iter_mut().enumerate() {
        *v = u32_at(r, 9 + 4 * i);
    }
    Ok(Rate { fixed_ms: u32_at(r, 0), follow, percent, interval_ms })
}

// The rule the firmware's usable() applies to a log gap.
pub fn log_trouble(ms: u32) -> Option<String> {
    // 0 is fine in any phase (every IMU reading in the air, nothing on the ground)
    if ms != 0 && ms < LOG_MIN_MS {
        return Some(format!("a flash log gap of {} ms, which has to be 0 or at least {}", ms, LOG_MIN_MS));
    }
    None
}

// Layouts 2 and 3: the same rates, and layout 3 has the log gaps where 2 kept zeros.
fn decode_v2(b: &[u8], layout: u8) -> Result<Found, String> {
    let (csid, module, sf, dbm, set_by) = decode_head(b, layout)?;
    let rates = [decode_rate(b, layout, 0)?, decode_rate(b, layout, 1)?, decode_rate(b, layout, 2)?, decode_rate(b, layout, 3)?];
    let (logs, log_blind_ms, reserved_clear) = if layout == 3 {
        let mut logs = [0u32; PHASES];
        for (p, v) in logs.iter_mut().enumerate() {
            *v = u32_at(b, LOGS_AT + 4 * p);
            if let Some(t) = log_trouble(*v) {
                return Err(odd(3, &format!("for {}", PHASE_WORDS[p]), t));
            }
        }
        let blind = u32_at(b, LOG_BLIND_AT);
        if let Some(t) = log_trouble(blind) {
            return Err(odd(3, "with no altitude source", t));
        }
        (Some(logs), Some(blind), true)
    } else {
        (None, None, b[RATES_AT + RATE_BYTES * PHASES..CRC_AT].iter().all(|v| *v == 0))
    };
    Ok(Found::Block(Settings {
        layout,
        csid,
        module,
        sf,
        dbm,
        set_by,
        // The flight gap: layout 2 flew on the falling phase; from layout 3 rising is the
        // whole flight, climb through fall, and falling is never entered
        tx_interval_ms: if layout == 3 { rates[1].fixed_ms } else { rates[2].fixed_ms },
        rates: Some(rates),
        logs,
        log_blind_ms,
        reserved_clear,
    }))
}

// The block and where it is.
pub fn find_at(image: &[u8]) -> Result<(Found, Option<usize>), String> {
    let Some(at) = locate(image)? else { return Ok((Found::Nothing, None)) };
    // An image that ends inside the block has a block that cannot pass its check.
    let Some(&layout) = image.get(at + 16) else { return Ok((Found::Damaged, Some(at))) };
    let (size, crc_at) = match layout {
        1 => (V1_SIZE, V1_CRC_AT),
        2 | 3 => (SIZE, CRC_AT),
        v => return Ok((Found::Layout(v), Some(at))),
    };
    let Some(b) = image.get(at..at + size) else { return Ok((Found::Damaged, Some(at))) };
    if u16::from_le_bytes([b[crc_at], b[crc_at + 1]]) != crc16(&b[..crc_at]) {
        return Ok((Found::Damaged, Some(at)));
    }
    let found = if layout == 1 { decode_v1(b)? } else { decode_v2(b, layout)? };
    Ok((found, Some(at)))
}

#[cfg(test)]
pub fn find(image: &[u8]) -> Result<Found, String> {
    find_at(image).map(|(found, _)| found)
}

// The block a tool may rewrite and where it is, or why it may not, in words for whoever
// asked.
pub fn writable(image: &[u8]) -> Result<(usize, Settings), String> {
    match find_at(image)? {
        // Layout 2 is patched too: it becomes layout 3, the log gaps going where its zeros were.
        (Found::Block(s), Some(at)) if (s.layout == LAYOUT || s.layout == 2) && s.reserved_clear => Ok((at, s)),
        (Found::Block(s), _) if s.layout == 1 => Err(format!(
            "the settings block is layout {}, from before the flight phases, so it is not patched: flash the flight image this app carries instead",
            s.layout
        )),
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

// The image with its block rewritten as layout 3: what was asked for, set by a tool, CRC
// to match. The module byte is left as it is, and so is every byte outside the block.
pub fn patch(image: &[u8], at: usize, asked: &Asked) -> Result<Vec<u8>, String> {
    if image.get(at..at + MAGIC.len()) != Some(&MAGIC[..]) || image.len() < at + SIZE {
        return Err(format!("there is no settings block at {:#x} to patch", at));
    }
    for (p, rate) in asked.rates.iter().enumerate() {
        if let Some(t) = table_trouble(&rate.percent) {
            return Err(format!("not written: for {}, {}", PHASE_WORDS[p], t));
        }
        if let Some(t) = log_trouble(asked.logs[p]) {
            return Err(format!("not written: for {}, {}", PHASE_WORDS[p], t));
        }
    }
    if let Some(t) = log_trouble(asked.log_blind_ms) {
        return Err(format!("not written: with no altitude source, {}", t));
    }
    let mut out = image.to_vec();
    let b = &mut out[at..at + SIZE];
    b[16] = LAYOUT;
    b[17] = asked.csid;
    b[19] = asked.sf;
    b[20] = asked.dbm as u8;
    b[21] = 2;   // a tool wrote it
    for (p, rate) in asked.rates.iter().enumerate() {
        let r = &mut b[RATES_AT + RATE_BYTES * p..RATES_AT + RATE_BYTES * (p + 1)];
        r[0..4].copy_from_slice(&rate.fixed_ms.to_le_bytes());
        r[4] = rate.follow as u8;
        r[5..9].copy_from_slice(&rate.percent);
        for (i, ms) in rate.interval_ms.iter().enumerate() {
            r[9 + 4 * i..13 + 4 * i].copy_from_slice(&ms.to_le_bytes());
        }
    }
    for (p, ms) in asked.logs.iter().enumerate() {
        b[LOGS_AT + 4 * p..LOGS_AT + 4 * (p + 1)].copy_from_slice(&ms.to_le_bytes());
    }
    b[LOG_BLIND_AT..LOG_BLIND_AT + 4].copy_from_slice(&asked.log_blind_ms.to_le_bytes());
    let crc = crc16(&b[..CRC_AT]);
    b[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
    // This goes onto a board whole. A byte changed anywhere else would be flight
    // software changed, so it is checked and not assumed.
    if out[..at] != image[..at] || out[at + SIZE..] != image[at + SIZE..] {
        return Err(String::from("patching the settings block changed bytes outside it, so nothing was written"));
    }
    Ok(out)
}

fn secs(ms: u32) -> String {
    if ms == 0 { String::from("back to back") } else { format!("{} s", ms as f64 / 1000.0) }
}

impl Rate {
    pub fn json(&self) -> String {
        format!(
            "{{\"fixedMs\":{},\"follow\":{},\"percent\":[{}],\"intervalMs\":[{}]}}",
            self.fixed_ms,
            self.follow,
            self.percent.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
            self.interval_ms.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
        )
    }

    // "every 60 s", or with the battery "10 s above 75 %, 30 s to 50 %, 60 s to 35 %,
    // 120 s to 20 %, then 180 s (every 60 s when the gauge cannot be read)".
    pub fn words(&self) -> String {
        if !self.follow {
            return format!("every {}", secs(self.fixed_ms)).replace("every back to back", "back to back");
        }
        let mut parts = vec![format!("{} above {} %", secs(self.interval_ms[0]), self.percent[0])];
        for i in 1..BANDS - 1 {
            parts.push(format!("{} to {} %", secs(self.interval_ms[i]), self.percent[i]));
        }
        parts.push(format!("then {}", secs(self.interval_ms[BANDS - 1])));
        format!("by battery: {} ({} when the gauge cannot be read)", parts.join(", "), secs(self.fixed_ms))
    }
}

impl Found {
    // As /api/chip/read gives it under "settings". A layout this build does not know and
    // a damaged block give nothing here; the caller says what is there in words.
    pub fn json(&self) -> String {
        match self {
            Found::Nothing | Found::Layout(_) | Found::Damaged => String::from("null"),
            Found::Block(s) => format!(
                "{{\"layoutVersion\":{},\"csid\":{},\"module\":\"{}\",\"sf\":{},\"dbm\":{},\"setBy\":\"{}\",\"txIntervalMs\":{},\"rates\":{},\"logMs\":{},\"logBlindMs\":{},\"reservedClear\":{}}}",
                s.layout,
                s.csid,
                s.module,
                s.sf,
                s.dbm,
                s.set_by,
                s.tx_interval_ms,
                s.rates
                    .as_ref()
                    .map(|r| format!("[{}]", r.iter().map(|x| x.json()).collect::<Vec<_>>().join(",")))
                    .unwrap_or_else(|| String::from("null")),
                s.logs
                    .as_ref()
                    .map(|l| format!("[{}]", l.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")))
                    .unwrap_or_else(|| String::from("null")),
                s.log_blind_ms.map(|v| v.to_string()).unwrap_or_else(|| String::from("null")),
                s.reserved_clear
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE_PCT: [u8; 4] = [75, 50, 35, 20];
    const TABLE_MS: [u32; 5] = [10_000, 30_000, 60_000, 120_000, 180_000];
    const fn rate(fixed_ms: u32, follow: bool) -> Rate {
        Rate { fixed_ms, follow, percent: TABLE_PCT, interval_ms: TABLE_MS }
    }
    // What a fresh build ships: the flight gap given for rising and falling both, the rest the
    // repo's defaults, nothing following.
    fn rates(flight_ms: u32) -> [Rate; PHASES] {
        [rate(60_000, false), rate(flight_ms, false), rate(flight_ms, false), rate(60_000, false)]
    }

    const LOGS: [u32; PHASES] = [1000, 50, 50, 1000];
    const LOG_BLIND: u32 = 200;

    // A block of the given layout, 2 or 3: layout 3 carries the log gaps a fresh build ships.
    fn block(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, interval: u32) -> Vec<u8> {
        block_with(version, csid, module, sf, dbm, set_by, &rates(interval))
    }

    fn block_with(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, rates: &[Rate; PHASES]) -> Vec<u8> {
        block_logs(version, csid, module, sf, dbm, set_by, rates, &LOGS, LOG_BLIND)
    }

    #[allow(clippy::too_many_arguments)]
    fn block_logs(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, rates: &[Rate; PHASES], logs: &[u32; PHASES], blind: u32) -> Vec<u8> {
        let mut b = vec![0u8; SIZE];
        b[..16].copy_from_slice(&MAGIC);
        b[16] = version;
        b[17] = csid;
        b[18] = module;
        b[19] = sf;
        b[20] = dbm as u8;
        b[21] = set_by;
        for (p, r) in rates.iter().enumerate() {
            let at = RATES_AT + RATE_BYTES * p;
            b[at..at + 4].copy_from_slice(&r.fixed_ms.to_le_bytes());
            b[at + 4] = r.follow as u8;
            b[at + 5..at + 9].copy_from_slice(&r.percent);
            for (i, ms) in r.interval_ms.iter().enumerate() {
                b[at + 9 + 4 * i..at + 13 + 4 * i].copy_from_slice(&ms.to_le_bytes());
            }
        }
        if version == 3 {
            for (p, ms) in logs.iter().enumerate() {
                b[LOGS_AT + 4 * p..LOGS_AT + 4 * (p + 1)].copy_from_slice(&ms.to_le_bytes());
            }
            b[LOG_BLIND_AT..LOG_BLIND_AT + 4].copy_from_slice(&blind.to_le_bytes());
        }
        sealed(b)
    }

    // A layout-1 block as a board flashed before the phases carries it: 32 bytes, CRC at 30.
    fn block_v1(csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, interval: u32) -> Vec<u8> {
        let mut b = vec![0u8; V1_SIZE];
        b[..16].copy_from_slice(&MAGIC);
        b[16] = 1;
        b[17] = csid;
        b[18] = module;
        b[19] = sf;
        b[20] = dbm as u8;
        b[21] = set_by;
        b[22..26].copy_from_slice(&interval.to_le_bytes());
        let crc = crc16(&b[..V1_CRC_AT]);
        b[V1_CRC_AT..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    fn sealed(mut b: Vec<u8>) -> Vec<u8> {
        let crc = crc16(&b[..CRC_AT]);
        b[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    // Something image-like around the blocks: not zeros and not erased.
    fn image(blocks: &[(usize, Vec<u8>)]) -> Vec<u8> {
        let mut image: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        for (at, b) in blocks {
            image[*at..*at + b.len()].copy_from_slice(b);
        }
        image
    }

    #[test]
    fn a_block_in_an_image_is_found_and_read() {
        let landed_by_battery = Rate { fixed_ms: 300_000, follow: true, percent: [90, 60, 30, 5], interval_ms: [5_000, 20_000, 60_000, 90_000, 200_000] };
        let mut set = rates(30_000);
        set[3] = landed_by_battery.clone();
        let b = block_with(3, 7, 2, 9, -4, 2, &set);
        let (found, at) = find_at(&image(&[(1001, b.clone())])).unwrap();
        assert_eq!(at, Some(1001));
        assert_eq!(
            found,
            Found::Block(Settings {
                layout: 3,
                csid: 7,
                module: "hp",
                sf: 9,
                dbm: -4,
                set_by: "tool",
                tx_interval_ms: 30_000,
                rates: Some(set.clone()),
                logs: Some(LOGS),
                log_blind_ms: Some(LOG_BLIND),
                reserved_clear: true,
            })
        );
        let j = found.json();
        assert!(j.starts_with("{\"layoutVersion\":3,\"csid\":7,\"module\":\"hp\",\"sf\":9,\"dbm\":-4,\"setBy\":\"tool\",\"txIntervalMs\":30000,\"rates\":[{\"fixedMs\":60000,\"follow\":false,\"percent\":[75,50,35,20],\"intervalMs\":[10000,30000,60000,120000,180000]},{\"fixedMs\":30000,"), "{}", j);
        // A layout-2 block's flight gap is its falling one; layout 3's is rising.
        let mut two = rates(5_000);
        two[1].fixed_ms = 7_000;
        let Found::Block(s2) = find(&image(&[(1001, block_with(2, 7, 2, 9, -4, 2, &two))])).unwrap() else { panic!() };
        assert_eq!(s2.tx_interval_ms, 5_000);
        let Found::Block(s3) = find(&image(&[(1001, block_with(3, 7, 2, 9, -4, 2, &two))])).unwrap() else { panic!() };
        assert_eq!(s3.tx_interval_ms, 7_000);
        assert!(j.ends_with("{\"fixedMs\":300000,\"follow\":true,\"percent\":[90,60,30,5],\"intervalMs\":[5000,20000,60000,90000,200000]}],\"logMs\":[1000,50,50,1000],\"logBlindMs\":200,\"reservedClear\":true}"), "{}", j);
        assert_eq!(landed_by_battery.words(), "by battery: 5 s above 90 %, 20 s to 60 %, 60 s to 30 %, 90 s to 5 %, then 200 s (300 s when the gauge cannot be read)");
        assert_eq!(rate(1_000, false).words(), "every 1 s");
        assert_eq!(rate(0, false).words(), "back to back");

        // What a build with fixed settings carries.
        let built = find(&image(&[(2048, block(3, 255, 1, 7, 14, 1, 1000))])).unwrap();
        assert!(built.json().starts_with("{\"layoutVersion\":3,\"csid\":255,\"module\":\"le\",\"sf\":7,\"dbm\":14,\"setBy\":\"build\",\"txIntervalMs\":1000,\"rates\":["));

        // Right at the end of what was read, with nothing after it.
        let mut tail = image(&[]);
        tail.extend_from_slice(&b);
        assert!(matches!(find(&tail).unwrap(), Found::Block(_)));
    }

    #[test]
    fn a_layout_1_block_off_an_older_board_is_read_and_has_no_rates() {
        let b = block_v1(12, 1, 9, 14, 2, 500);
        let (found, at) = find_at(&image(&[(1001, b.clone())])).unwrap();
        assert_eq!(at, Some(1001));
        let Found::Block(s) = &found else { panic!("{:?}", found) };
        assert_eq!((s.layout, s.csid, s.module, s.sf, s.dbm, s.set_by, s.tx_interval_ms, s.reserved_clear), (1, 12, "le", 9, 14, "tool", 500, true));
        assert_eq!(s.rates, None);
        assert!(found.json().starts_with("{\"layoutVersion\":1,"));
        assert!(found.json().contains("\"txIntervalMs\":500,\"rates\":null,\"logMs\":null,\"logBlindMs\":null,"));
        // Its CRC is at 30, and what follows it is whatever the image held: not read.
        let mut flipped_tail = image(&[(1001, b.clone())]);
        flipped_tail[1001 + 40] ^= 0xFF;
        assert_eq!(find(&flipped_tail).unwrap(), found);
        // Its own check still applies.
        let mut bad = b.clone();
        bad[25] ^= 1;
        assert_eq!(find(&image(&[(1001, bad)])).unwrap(), Found::Damaged);
        // A layout-1 block with a reserved byte in use is flagged the same way.
        let mut used = b;
        used[27] = 1;
        let crc = crc16(&used[..V1_CRC_AT]);
        used[V1_CRC_AT..].copy_from_slice(&crc.to_le_bytes());
        match find(&image(&[(1001, used)])).unwrap() {
            Found::Block(s) => assert!(!s.reserved_clear),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn a_table_the_firmware_would_turn_down_is_an_error_here_too_in_any_phase() {
        for (what, phase, pct) in [
            ("1 to 99", 0, [100, 50, 35, 20]),
            ("1 to 99", 3, [75, 50, 35, 0]),
            ("have to fall", 1, [75, 75, 35, 20]),
            ("have to fall", 2, [75, 50, 55, 20]),
        ] {
            let mut set = rates(1000);
            set[phase].percent = pct;
            let e = find(&image(&[(1001, block_with(2, 7, 2, 9, -4, 2, &set))])).unwrap_err();
            assert!(e.contains(what) && e.contains(PHASE_WORDS[phase]), "{}: {}", what, e);
        }
        // A flag byte that is neither 0 nor 1, in the rising phase.
        let mut b = block(2, 7, 2, 9, -4, 2, 0);
        b[RATES_AT + RATE_BYTES + 4] = 2;
        let e = find(&image(&[(1001, sealed(b))])).unwrap_err();
        assert!(e.contains("air battery flag"), "{}", e);
        // and the error names the layout the block is, not layout 2 every time
        let mut b = block(3, 7, 2, 9, -4, 2, 0);
        b[RATES_AT + RATE_BYTES + 4] = 2;
        let e = find(&image(&[(1001, sealed(b))])).unwrap_err();
        assert!(e.contains("air battery flag") && e.contains("layout 3"), "{}", e);
        let mut set = rates(1000);
        set[1].percent = [75, 75, 35, 20];
        let e = find(&image(&[(1001, block_with(3, 7, 2, 9, -4, 2, &set))])).unwrap_err();
        assert!(e.contains("layout 3"), "{}", e);
        // named as the page shows it, air, not by the slot's old name
        let mut b = block(3, 7, 2, 9, -4, 2, 0);
        b[RATES_AT + RATE_BYTES + 4] = 2;
        let e = find(&image(&[(1001, sealed(b))])).unwrap_err();
        assert!(e.contains("air battery flag") && !e.contains("rising"), "{}", e);
        // Intervals are not judged: zero is back to back, as it is for the fixed gap.
        let mut set = rates(0);
        set[2].interval_ms = [0; BANDS];
        assert!(matches!(find(&image(&[(1001, block_with(2, 7, 2, 9, -4, 2, &set))])).unwrap(), Found::Block(_)));
    }

    #[test]
    fn the_crc_is_stored_low_byte_first() {
        let b = block(2, 7, 2, 9, -4, 2, 30_000);
        let crc = crc16(&b[..CRC_AT]);
        assert_eq!([b[CRC_AT], b[CRC_AT + 1]], [(crc & 0xFF) as u8, (crc >> 8) as u8]);
        // The same block with the two bytes the other way round does not pass.
        let mut swapped = b.clone();
        swapped.swap(CRC_AT, CRC_AT + 1);
        assert_ne!(b[CRC_AT], b[CRC_AT + 1], "a block whose CRC reads the same both ways proves nothing");
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
        let b = block(2, 7, 2, 9, -4, 2, 30_000);
        for bit in [17 * 8 + 3, 22 * 8, 26 * 8, 40 * 8 + 1, 100 * 8, 157 * 8 + 7, 158 * 8, 159 * 8 + 7] {
            let mut bad = b.clone();
            bad[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(find_at(&image(&[(1001, bad)])).unwrap(), (Found::Damaged, Some(1001)), "bit {}", bit);
        }
        assert_eq!(Found::Damaged.json(), "null");
        // The layout byte says where the check is, so a flipped one that lands on a layout
        // this build knows is checked at that layout's place, and fails there.
        let mut as_v1 = b.clone();
        as_v1[16] = 1;
        assert_eq!(find(&image(&[(1001, as_v1)])).unwrap(), Found::Damaged);
        // The magic with the image ending before the block does.
        assert_eq!(find(&MAGIC).unwrap(), Found::Damaged);
        let mut cut = image(&[]);
        cut.extend_from_slice(&b[..SIZE - 1]);
        assert_eq!(find(&cut).unwrap(), Found::Damaged);
        let mut cut_v1 = image(&[]);
        cut_v1.extend_from_slice(&block_v1(7, 2, 9, -4, 2, 0)[..31]);
        assert_eq!(find(&cut_v1).unwrap(), Found::Damaged);
    }

    #[test]
    fn two_magics_are_an_error_that_says_how_many() {
        let b = block(2, 7, 2, 9, -4, 2, 30_000);
        let e = find(&image(&[(1001, b.clone()), (3000, block(2, 8, 1, 7, 14, 1, 0))])).unwrap_err();
        assert!(e.contains("2 settings blocks"), "{}", e);
        // The same block twice is still two.
        assert!(find(&image(&[(1001, b.clone()), (3000, b.clone())])).unwrap_err().contains("2 settings blocks"));
        // So is a good one beside a damaged one, or beside the magic on its own: the
        // firmware goes by the magic, and so does this.
        let mut bad = b.clone();
        bad[17] ^= 1;
        assert!(find(&image(&[(1001, b.clone()), (3000, bad)])).unwrap_err().contains("2 settings blocks"));
        let mut bare = image(&[(1001, b)]);
        bare[200..216].copy_from_slice(&MAGIC);
        assert!(locate(&bare).unwrap_err().contains("2 settings blocks"));
        let mut three = bare.clone();
        three[300..316].copy_from_slice(&MAGIC);
        assert!(find(&three).unwrap_err().contains("3 settings blocks"));
    }

    #[test]
    fn a_layout_2_block_from_before_the_log_gaps_is_read_and_has_none() {
        let found = find(&image(&[(1001, block(2, 7, 2, 9, -4, 2, 30_000))])).unwrap();
        let Found::Block(s) = &found else { panic!("{:?}", found) };
        assert_eq!((s.layout, s.csid, s.tx_interval_ms, s.logs, s.log_blind_ms, s.reserved_clear), (2, 7, 30_000, None, None, true));
        assert_eq!(s.rates, Some(rates(30_000)));
        assert!(found.json().contains("\"logMs\":null,\"logBlindMs\":null,"));
        // A log gap the firmware would refuse is an error here too, naming the phase.
        let e = find(&image(&[(1001, block_logs(3, 7, 2, 9, -4, 2, &rates(1), &[1000, 10, 50, 1000], 200))])).unwrap_err();
        assert!(e.contains("for air a flash log gap") && e.contains("at least 20"), "{}", e);
        let e = find(&image(&[(1001, block_logs(3, 7, 2, 9, -4, 2, &rates(1), &LOGS, 5))])).unwrap_err();
        assert!(e.contains("no altitude source"), "{}", e);
    }

    #[test]
    fn a_layout_this_build_does_not_know_is_named_and_not_decoded() {
        let found = find(&image(&[(1001, block(4, 7, 2, 9, -4, 2, 30_000))])).unwrap();
        assert_eq!(found, Found::Layout(4));
        // Nothing of it goes to the page as settings: fields read by the wrong layout
        // would look like settings and be wrong.
        assert_eq!(found.json(), "null");
        // And nothing of it is checked: where its CRC sits is part of what is not known,
        // so a damaged block of an unknown layout is still "layout 3" and not "damaged".
        let mut torn = block(4, 7, 2, 9, -4, 2, 30_000);
        torn[40] ^= 0xFF;
        assert_eq!(find(&image(&[(1001, torn)])).unwrap(), Found::Layout(4));
    }

    #[test]
    fn reserved_bytes_in_use_are_read_past_and_flagged() {
        let mut b = block(2, 7, 2, 9, -4, 2, 30_000);
        b[150] = 1;
        match find(&image(&[(1001, sealed(b.clone()))])).unwrap() {
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
            ("module 3", block(2, 7, 3, 9, -4, 2, 0)),
            ("spreading factor 13", block(2, 7, 2, 13, -4, 2, 0)),
            ("spreading factor 4", block(2, 7, 2, 4, -4, 2, 0)),
            ("ChipSat id 0", block(2, 0, 2, 9, -4, 2, 0)),
            ("set by 0", block(2, 7, 2, 9, -4, 0, 0)),
            ("module 3", block_v1(7, 3, 9, -4, 2, 0)),
        ] {
            let e = find(&image(&[(1001, b)])).unwrap_err();
            assert!(e.contains(what), "{}: {}", what, e);
        }
    }

    fn asked() -> Asked {
        Asked { csid: 12, sf: 9, dbm: 20, rates: rates(1000), logs: LOGS, log_blind_ms: LOG_BLIND }
    }

    #[test]
    fn a_patch_changes_the_block_and_nothing_else() {
        let before = image(&[(1001, block(3, 255, 2, 7, 14, 1, 1000))]);
        let (at, was) = writable(&before).unwrap();
        assert_eq!((at, was.module, was.set_by), (1001, "hp", "build"));

        let after = patch(&before, at, &asked()).unwrap();
        assert_eq!(after.len(), before.len());
        let changed: Vec<usize> = (0..before.len()).filter(|i| before[*i] != after[*i]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|i| (1001..1001 + SIZE).contains(i)), "{:?}", changed);

        // And this file's own reader takes what it wrote.
        assert_eq!(
            find_at(&after).unwrap(),
            (
                Found::Block(Settings {
                    layout: 3,
                    csid: 12,
                    module: "hp",
                    sf: 9,
                    dbm: 20,
                    set_by: "tool",
                    tx_interval_ms: 1000,
                    rates: Some(rates(1000)),
                    logs: Some(LOGS),
                    log_blind_ms: Some(LOG_BLIND),
                    reserved_clear: true,
                }),
                Some(1001)
            )
        );
        // Byte for byte, against a block built here by hand.
        assert_eq!(after[1001..1001 + SIZE], block(3, 12, 2, 9, 20, 2, 1000)[..]);

        // A negative power, the widest gaps and a table per phase fit their fields.
        let odd_rates = [
            Rate { fixed_ms: 0, follow: true, percent: [99, 66, 33, 1], interval_ms: [0, 1, u32::MAX, 7, 180_000] },
            rate(u32::MAX, false),
            Rate { fixed_ms: 1, follow: true, percent: [80, 40, 20, 10], interval_ms: [1, 2, 3, 4, 5] },
            rate(7, true),
        ];
        let odd_logs = [20, u32::MAX, 21, 3_600_000];
        let odd = patch(&before, at, &Asked { csid: 1, sf: 12, dbm: -9, rates: odd_rates.clone(), logs: odd_logs, log_blind_ms: u32::MAX }).unwrap();
        assert_eq!(odd[1001..1001 + SIZE], block_logs(3, 1, 2, 12, -9, 2, &odd_rates, &odd_logs, u32::MAX)[..]);
        // Patching what a tool already set works the same way.
        let again = patch(&after, at, &Asked { csid: 13, sf: 7, dbm: 14, rates: rates(0), logs: LOGS, log_blind_ms: LOG_BLIND }).unwrap();
        assert_eq!(again[1001..1001 + SIZE], block(3, 13, 2, 7, 14, 2, 0)[..]);
        // A layout-2 image, from before the log gaps, is patched into layout 3.
        let older = image(&[(1001, block(2, 255, 2, 7, 14, 1, 1000))]);
        let (at2, was2) = writable(&older).unwrap();
        assert_eq!((at2, was2.layout, was2.logs), (1001, 2, None));
        assert_eq!(patch(&older, at2, &asked()).unwrap()[1001..1001 + SIZE], block(3, 12, 2, 9, 20, 2, 1000)[..]);
        // A table the firmware would refuse is not written, and the phase is named.
        let mut bad = asked();
        bad.rates[3].percent = [50, 75, 35, 20];
        let e = patch(&before, at, &bad).unwrap_err();
        assert!(e.contains("have to fall") && e.contains("landed"), "{}", e);
        // So is a log gap it would refuse.
        let mut bad = asked();
        bad.logs[1] = 19;
        let e = patch(&before, at, &bad).unwrap_err();
        assert!(e.contains("for air,") && e.contains("at least 20"), "{}", e);
        let mut bad = asked();
        bad.log_blind_ms = 7;
        assert!(patch(&before, at, &bad).unwrap_err().contains("no altitude source"));
        // 0 stays 0
        let mut zero = asked();
        zero.logs[1] = 0;
        zero.log_blind_ms = 0;
        assert!(patch(&before, at, &zero).is_ok());
    }

    #[test]
    fn a_block_that_cannot_be_trusted_is_not_one_to_patch() {
        let e = writable(&image(&[])).unwrap_err();
        assert_eq!(e, "this firmware has no settings block");

        let mut bad = block(3, 255, 1, 7, 14, 1, 1000);
        bad[19] ^= 0x10;
        let e = writable(&image(&[(1001, bad)])).unwrap_err();
        assert!(e.contains("damaged"), "{}", e);

        let e = writable(&image(&[(1001, block(4, 255, 1, 7, 14, 1, 1000))])).unwrap_err();
        assert!(e.contains("layout 4"), "{}", e);

        // An older image is read but not written: its block has no room for the rates,
        // and the words say what to do instead.
        let e = writable(&image(&[(1001, block_v1(255, 1, 7, 14, 1, 1000))])).unwrap_err();
        assert!(e.contains("layout 1") && e.contains("flash the flight image"), "{}", e);

        let mut reserved = block(2, 255, 1, 7, 14, 1, 1000);
        reserved[145] = 0x40;
        let e = writable(&image(&[(1001, sealed(reserved))])).unwrap_err();
        assert!(e.contains("reserved"), "{}", e);

        // Told to patch somewhere that is not a block, it does not.
        let whole = image(&[(1001, block(3, 255, 1, 7, 14, 1, 1000))]);
        assert!(patch(&whole, 1000, &asked()).is_err());
        assert!(patch(&whole, 4090, &asked()).is_err());
    }

    #[test]
    fn each_flight_image_in_this_build_has_one_block_that_says_what_it_was_built_with() {
        // Whatever fsw/firmware held at build time: no images, or images whose
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
            // The rates the image carries are the ones radio.json says a fresh build ships,
            // both written by the flight repo from the same config.h.
            assert_eq!(s.layout, LAYOUT, "{}", image.file);
            let radio = crate::radio::carried().unwrap();
            assert_eq!(s.rates.as_ref(), Some(&radio.rates), "{}", image.file);
            assert_eq!((s.logs, s.log_blind_ms), (Some(radio.logs), Some(radio.log_blind_ms)), "{}", image.file);
        }
    }
}
