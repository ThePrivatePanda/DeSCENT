// The settings block a flight image carries: which ChipSat it is, which module it drives
// and how it transmits, as bytes inside the image so they can be found without symbols.
// The firmware finds the block by its magic and believes it only if its CRC passes; with
// a block it does not believe it runs the values it was compiled with. The layout is the
// flight repo's and the block is written by its build. When a flight software tree is at
// hand a board gets other settings by being built and then patched; when there is none,
// as with a downloaded release, the block in the image this app carries is rewritten
// here before that image is flashed (patch, below).
//
// Layout 4, 160 bytes, what every image built since 2026-10-31 carries:
//   0..16   magic
//   16      layout version
//   17      ChipSat id, 1..253, 254 = the sensor bench (refused for a flight board), 255 = unconfigured
//   18      module, 1 = LE, 2 = HP
//   19      spreading factor, 5..12
//   20      target dBm, signed
//   21      set by, 1 = the build, 2 = a tool
//   22..138 one rate per slot, 29 bytes each: ground transmit, air transmit, ground log, air log
//             +0  fixed gap in ms
//             +4  1 = the gap follows the battery table below, 0 = the fixed gap always
//             +5  four state-of-charge thresholds in percent, falling, each 1..99
//             +9  five gaps in ms, one per band: at or above each threshold, then below them all
//           A slot that follows the battery uses its fixed gap while the gauge cannot be read.
//           Transmit 0 is back to back. Air log 0 is every IMU reading, ground log 0 keeps
//           nothing. 1 to 19 ms the firmware refuses in every log value, fixed or band, whether
//           or not the slot follows the battery. The page writes one flag and one set of
//           thresholds per phase to both slots; the firmware treats the slots independently.
//   138..158 reserved, zero
//   158..160 CRC16 over 0..158, low byte first, the same way dump.rs reads a slot's
//
// Older layouts are read forever and never written: firmware takes only its own exact layout,
// so a block rewritten as another one would boot on compiled defaults while the read-back
// still passed. They are normalised here so the page sees one shape.
// Layout 3 (2026-10-16) had the same 160 bytes, but its four slots were all transmit gaps
// (waiting, rising, falling, landed), then four log gaps at 138, a gap at 154 for no altitude
// source, and the CRC at 158. Its log table here is the board's fixed log gap for that phase,
// follow off, bands equal to it. Slots 2 and 3 are never log tables: they hold transmit gaps.
// Since 2026-10-22 that firmware only read slots 0 and 1, and older firmware read all four.
// Layout 2 (2026-10) was the same with 138..158 reserved and zero: no log gaps at all.
// Layout 1 was 32 bytes: the same through byte 22, one gap in ms at 22, reserved to 30,
// CRC at 30. Boards flashed before the phases still carry it, so it is shown as a block with
// no rates.
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
pub const LAYOUT: u8 = 4;
const V1_SIZE: usize = 32;
const V1_CRC_AT: usize = 30;
pub const PHASES: usize = 2;
pub const SLOTS: usize = 2 * PHASES;
pub const BANDS: usize = 5;
const RATES_AT: usize = 22;
const RATE_BYTES: usize = 29;
const RESERVED_AT: usize = RATES_AT + RATE_BYTES * SLOTS;
// Layouts 2 and 3 had four transmit slots; layout 3's log gaps sat after them.
const OLD_SLOTS: usize = 4;
const OLD_LOGS_AT: usize = 138;
const OLD_LOG_BLIND_AT: usize = 154;
const OLD_SLOT_WORDS: [&str; OLD_SLOTS] = ["ground", "air", "air (falling slot)", "ground (landed slot)"];
// The firmware refuses a log gap under this: the sensors make nothing faster.
pub const LOG_MIN_MS: u32 = 20;
// What messages call the phases, as the page does.
pub const PHASE_WORDS: [&str; PHASES] = ["ground", "air"];
// radio.json's names for the slots, in the block's order.
pub const SLOT_NAMES: [&str; SLOTS] = ["ground transmit", "air transmit", "ground log", "air log"];

// Said to the operator as it is. The second half is the part that matters at a bench:
// the board still transmits, with what its build gave it and not with what was asked.
pub const DAMAGED: &str = "the settings block is damaged; the board is running the values it was built with";

// One slot: the gap after each packet, or between log records. With `follow`, the gap is
// interval_ms[i] for the first threshold percent[i] the charge is at or above, and the last
// one below them all; the fixed gap then only applies while the gauge cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Rate {
    pub fixed_ms: u32,
    pub follow: bool,
    pub percent: [u8; BANDS - 1],
    pub interval_ms: [u32; BANDS],
}

// A slot of layout 2 or 3 that is not its phase's slot 0 or 1 and differs from it. Those were
// real phases on the firmware of the day, so a board still carrying one is told as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct OlderSlot {
    pub phase: usize,
    pub name: &'static str,
    pub rate: Rate,
    pub log_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub layout: u8,
    pub csid: u8,
    pub module: &'static str,
    pub sf: u8,
    pub dbm: i8,
    pub set_by: &'static str,
    // The flight rate: layout 1's one gap, layout 2's falling gap, the air gap from layout 3.
    pub tx_interval_ms: u32,
    // None on a layout-1 block: that firmware has one gap from switch-on to the end.
    pub transmit: Option<[Rate; PHASES]>,
    // None on a block from before layout 3: that firmware logs at 20 Hz in the air and 1 Hz
    // on the ground.
    pub log: Option<[Rate; PHASES]>,
    pub older: Vec<OlderSlot>,
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
    pub transmit: [Rate; PHASES],
    pub log: [Rate; PHASES],
}

// A band as the page's row says it: above 75 %, to 50 %, to 35 %, to 20 %, below 20 %.
fn band_words(percent: &[u8; BANDS - 1], band: usize) -> String {
    match band {
        0 => format!("above {} %", percent[0]),
        b if b == BANDS - 1 => format!("below {} %", percent[BANDS - 2]),
        b => format!("to {} %", percent[b]),
    }
}

// The row of a rate's fixed gap: with the battery it is the one for an unread gauge.
fn fixed_words(r: &Rate) -> &'static str {
    if r.follow { "gauge unread" } else { "always" }
}

// What differs in one rate, each value named the way a refusal names it.
fn rate_differences(phase: &str, column: &str, wanted: &Rate, on_board: &Rate, out: &mut Vec<String>) {
    if wanted.follow != on_board.follow {
        out.push(format!("{}, {} battery switch", phase, column));
    }
    if wanted.fixed_ms != on_board.fixed_ms {
        out.push(format!("{}, {}, {} gap", phase, fixed_words(wanted), column));
    }
    for i in 0..BANDS - 1 {
        if wanted.percent[i] != on_board.percent[i] {
            out.push(format!("{}, {} battery threshold {}", phase, column, i + 1));
        }
    }
    for i in 0..BANDS {
        if wanted.interval_ms[i] != on_board.interval_ms[i] {
            out.push(format!("{}, {}, {} gap", phase, band_words(&wanted.percent, i), column));
        }
    }
}

fn table_differences(column: &str, wanted: &Option<[Rate; PHASES]>, on_board: &Option<[Rate; PHASES]>, out: &mut Vec<String>) {
    match (wanted, on_board) {
        (Some(w), Some(b)) => {
            for p in 0..PHASES {
                rate_differences(PHASE_WORDS[p], column, &w[p], &b[p], out);
            }
        }
        (None, None) => {}
        _ => out.push(format!("{} gaps", column)),
    }
}

// What differs between two blocks, in the words the page uses for them. Every value is
// looked at, because a value left out here is one a read-back would call right.
pub fn differences(wanted: &Settings, on_board: &Settings) -> Vec<String> {
    let checks = [
        (wanted.layout != on_board.layout, "block layout"),
        (wanted.csid != on_board.csid, "board id"),
        (wanted.module != on_board.module, "module"),
        (wanted.sf != on_board.sf, "spreading factor"),
        (wanted.dbm != on_board.dbm, "transmit power"),
        (wanted.tx_interval_ms != on_board.tx_interval_ms && wanted.transmit.is_none(), "transmit gaps"),
    ];
    let mut out: Vec<String> = checks.iter().filter(|(off, _)| *off).map(|(_, word)| word.to_string()).collect();
    table_differences("transmit", &wanted.transmit, &on_board.transmit, &mut out);
    table_differences("log", &wanted.log, &on_board.log, &mut out);
    out
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
        transmit: None,
        log: None,
        older: Vec::new(),
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

fn decode_rate(b: &[u8], layout: u8, slot: usize, words: &str) -> Result<Rate, String> {
    let r = &b[RATES_AT + RATE_BYTES * slot..RATES_AT + RATE_BYTES * (slot + 1)];
    let follow = match r[4] {
        0 => false,
        1 => true,
        v => return Err(odd(layout, &format!("{} battery flag", words), v)),
    };
    let mut percent = [0u8; BANDS - 1];
    percent.copy_from_slice(&r[5..9]);
    if let Some(t) = table_trouble(&percent) {
        return Err(odd(layout, &format!("for {}", words), t));
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
        return Some(format!("{} ms, which has to be 0 or at least {}", ms, LOG_MIN_MS));
    }
    None
}

// The same rule on every value of a log rate, fixed or band, whether or not it follows the
// battery: the flag is one byte away from putting a band to use. Names the row and the column,
// "air, below 20 %, log gap: 5 ms, ...". The firmware refusing a block silently runs what it
// was built with, so anything that gets past this must pass usable().
pub fn log_rate_trouble(phase: usize, r: &Rate) -> Option<String> {
    if let Some(t) = log_trouble(r.fixed_ms) {
        return Some(format!("{}, {}, log gap: {}", PHASE_WORDS[phase], fixed_words(r), t));
    }
    for (i, ms) in r.interval_ms.iter().enumerate() {
        if let Some(t) = log_trouble(*ms) {
            return Some(format!("{}, {}, log gap: {}", PHASE_WORDS[phase], band_words(&r.percent, i), t));
        }
    }
    None
}

// Layout 4.
fn decode_v4(b: &[u8]) -> Result<Found, String> {
    let (csid, module, sf, dbm, set_by) = decode_head(b, LAYOUT)?;
    let slot = |n: usize| decode_rate(b, LAYOUT, n, SLOT_NAMES[n]);
    let transmit = [slot(0)?, slot(1)?];
    let log = [slot(2)?, slot(3)?];
    for (p, r) in log.iter().enumerate() {
        if let Some(t) = log_rate_trouble(p, r) {
            return Err(odd(LAYOUT, "for", t));
        }
    }
    Ok(Found::Block(Settings {
        layout: LAYOUT,
        csid,
        module,
        sf,
        dbm,
        set_by,
        tx_interval_ms: transmit[1].fixed_ms,
        transmit: Some(transmit),
        log: Some(log),
        older: Vec::new(),
        reserved_clear: b[RESERVED_AT..CRC_AT].iter().all(|v| *v == 0),
    }))
}

// A phase's log table on a block that only held one gap for it.
fn flat_log(ms: u32, thresholds: &Rate) -> Rate {
    Rate { fixed_ms: ms, follow: false, percent: thresholds.percent, interval_ms: [ms; BANDS] }
}

// Layouts 2 and 3: four transmit slots, and layout 3 has log gaps where 2 kept zeros. The page
// is given the same shape as layout 4, with the two spare slots kept apart when they differ.
fn decode_v2(b: &[u8], layout: u8) -> Result<Found, String> {
    let (csid, module, sf, dbm, set_by) = decode_head(b, layout)?;
    let slot = |n: usize| decode_rate(b, layout, n, OLD_SLOT_WORDS[n]);
    let rates = [slot(0)?, slot(1)?, slot(2)?, slot(3)?];
    let (logs, reserved_clear) = if layout == 3 {
        let mut logs = [0u32; OLD_SLOTS];
        for (p, v) in logs.iter_mut().enumerate() {
            *v = u32_at(b, OLD_LOGS_AT + 4 * p);
            if let Some(t) = log_trouble(*v) {
                return Err(odd(3, &format!("for {} a flash log gap of", OLD_SLOT_WORDS[p]), t));
            }
        }
        if let Some(t) = log_trouble(u32_at(b, OLD_LOG_BLIND_AT)) {
            return Err(odd(3, "with no altitude source a flash log gap of", t));
        }
        (Some(logs), true)
    } else {
        (None, b[RATES_AT + RATE_BYTES * OLD_SLOTS..CRC_AT].iter().all(|v| *v == 0))
    };
    let mut older = Vec::new();
    for (phase, spare, name, first) in [(1usize, 2usize, "falling", 1usize), (0, 3, "landed", 0)] {
        let log_differs = logs.map(|l| l[spare] != l[first]).unwrap_or(false);
        if rates[spare] != rates[first] || log_differs {
            older.push(OlderSlot { phase, name, rate: rates[spare].clone(), log_ms: logs.map(|l| l[spare]) });
        }
    }
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
        log: logs.map(|l| [flat_log(l[0], &rates[0]), flat_log(l[1], &rates[1])]),
        transmit: Some([rates[0].clone(), rates[1].clone()]),
        older,
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
        2..=LAYOUT => (SIZE, CRC_AT),
        v => return Ok((Found::Layout(v), Some(at))),
    };
    let Some(b) = image.get(at..at + size) else { return Ok((Found::Damaged, Some(at))) };
    if u16::from_le_bytes([b[crc_at], b[crc_at + 1]]) != crc16(&b[..crc_at]) {
        return Ok((Found::Damaged, Some(at)));
    }
    let found = match layout {
        1 => decode_v1(b)?,
        LAYOUT => decode_v4(b)?,
        _ => decode_v2(b, layout)?,
    };
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
        // Only an image already at this layout. Firmware needs its exact layout, so a block
        // stamped newer than the code around it boots on compiled defaults and reads back fine.
        (Found::Block(s), Some(at)) if s.layout == LAYOUT && s.reserved_clear => Ok((at, s)),
        (Found::Block(s), _) if s.layout == 1 => Err(format!(
            "the settings block is layout {}, from before the flight phases, so it is not patched: flash the flight image this app carries instead",
            s.layout
        )),
        (Found::Block(s), _) if s.layout < LAYOUT => Err(format!(
            "the settings block is layout {} and this app writes layout {}. The firmware only reads its own layout, so a patched image would run what it was built with. It is not patched: flash the flight image this app carries, or update the flight software on this computer",
            s.layout, LAYOUT
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

// The image with its block rewritten: what was asked for, set by a tool, CRC to match. The
// module byte is left as it is, and so is every byte outside the block.
pub fn patch(image: &[u8], at: usize, asked: &Asked) -> Result<Vec<u8>, String> {
    if image.get(at..at + MAGIC.len()) != Some(&MAGIC[..]) || image.len() < at + SIZE {
        return Err(format!("there is no settings block at {:#x} to patch", at));
    }
    for (p, r) in asked.transmit.iter().enumerate() {
        if let Some(t) = table_trouble(&r.percent) {
            return Err(format!("not written: for {}, {}", SLOT_NAMES[p], t));
        }
    }
    for (p, r) in asked.log.iter().enumerate() {
        if let Some(t) = table_trouble(&r.percent) {
            return Err(format!("not written: for {}, {}", SLOT_NAMES[PHASES + p], t));
        }
        if let Some(t) = log_rate_trouble(p, r) {
            return Err(format!("not written: {}", t));
        }
    }
    let mut out = image.to_vec();
    let b = &mut out[at..at + SIZE];
    b[16] = LAYOUT;
    b[17] = asked.csid;
    b[19] = asked.sf;
    b[20] = asked.dbm as u8;
    b[21] = 2;   // a tool wrote it
    for (slot, rate) in asked.transmit.iter().chain(asked.log.iter()).enumerate() {
        let r = &mut b[RATES_AT + RATE_BYTES * slot..RATES_AT + RATE_BYTES * (slot + 1)];
        r[0..4].copy_from_slice(&rate.fixed_ms.to_le_bytes());
        r[4] = rate.follow as u8;
        r[5..9].copy_from_slice(&rate.percent);
        for (i, ms) in rate.interval_ms.iter().enumerate() {
            r[9 + 4 * i..13 + 4 * i].copy_from_slice(&ms.to_le_bytes());
        }
    }
    b[RESERVED_AT..CRC_AT].fill(0);
    let crc = crc16(&b[..CRC_AT]);
    b[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
    // This goes onto a board whole. A byte changed anywhere else would be flight
    // software changed, so it is checked and not assumed.
    if out[..at] != image[..at] || out[at + SIZE..] != image[at + SIZE..] {
        return Err(String::from("patching the settings block changed bytes outside it, so nothing was written"));
    }
    Ok(out)
}

// What 0 means for a log slot. On the ground that is no time anchors either.
pub fn log_zero_words(phase: usize) -> &'static str {
    if phase == 0 { "nothing written" } else { "every reading" }
}
pub const TRANSMIT_ZERO_WORDS: &str = "back to back";

fn rates_json(rates: &Option<[Rate; PHASES]>) -> String {
    rates
        .as_ref()
        .map(|r| format!("[{}]", r.iter().map(|x| x.json()).collect::<Vec<_>>().join(",")))
        .unwrap_or_else(|| String::from("null"))
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
    // 120 s to 20 %, then 180 s (every 60 s when the gauge cannot be read)". A gap of 0 is said
    // as `zero`, which depends on the slot.
    pub fn words(&self, zero: &str) -> String {
        let secs = |ms: u32| if ms == 0 { zero.to_string() } else { format!("{} s", ms as f64 / 1000.0) };
        if !self.follow {
            return if self.fixed_ms == 0 { zero.to_string() } else { format!("every {}", secs(self.fixed_ms)) };
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
                "{{\"layoutVersion\":{},\"csid\":{},\"module\":\"{}\",\"sf\":{},\"dbm\":{},\"setBy\":\"{}\",\"txIntervalMs\":{},\"transmit\":{},\"log\":{},\"olderSlots\":[{}],\"reservedClear\":{}}}",
                s.layout,
                s.csid,
                s.module,
                s.sf,
                s.dbm,
                s.set_by,
                s.tx_interval_ms,
                rates_json(&s.transmit),
                rates_json(&s.log),
                s.older
                    .iter()
                    .map(|o| format!(
                        "{{\"phase\":{},\"name\":\"{}\",\"rate\":{},\"logMs\":{}}}",
                        o.phase,
                        o.name,
                        o.rate.json(),
                        o.log_ms.map(|v| v.to_string()).unwrap_or_else(|| String::from("null"))
                    ))
                    .collect::<Vec<_>>()
                    .join(","),
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
    const fn flat(ms: u32) -> Rate {
        Rate { fixed_ms: ms, follow: false, percent: TABLE_PCT, interval_ms: [ms; BANDS] }
    }
    // What a fresh build ships.
    fn transmit(flight_ms: u32) -> [Rate; PHASES] {
        [rate(60_000, false), rate(flight_ms, false)]
    }
    fn log() -> [Rate; PHASES] {
        [flat(1000), flat(50)]
    }
    // The four transmit slots of layouts 2 and 3: the flight gap given for rising and falling.
    fn rates(flight_ms: u32) -> [Rate; OLD_SLOTS] {
        [rate(60_000, false), rate(flight_ms, false), rate(flight_ms, false), rate(60_000, false)]
    }

    const LOGS: [u32; OLD_SLOTS] = [1000, 50, 50, 1000];
    const LOG_BLIND: u32 = 200;

    fn put_rate(b: &mut [u8], slot: usize, r: &Rate) {
        let at = RATES_AT + RATE_BYTES * slot;
        b[at..at + 4].copy_from_slice(&r.fixed_ms.to_le_bytes());
        b[at + 4] = r.follow as u8;
        b[at + 5..at + 9].copy_from_slice(&r.percent);
        for (i, ms) in r.interval_ms.iter().enumerate() {
            b[at + 9 + 4 * i..at + 13 + 4 * i].copy_from_slice(&ms.to_le_bytes());
        }
    }

    fn head(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8) -> Vec<u8> {
        let mut b = vec![0u8; SIZE];
        b[..16].copy_from_slice(&MAGIC);
        b[16] = version;
        b[17] = csid;
        b[18] = module;
        b[19] = sf;
        b[20] = dbm as u8;
        b[21] = set_by;
        b
    }

    // A block of this layout, built here by hand. The golden block below is what ties these
    // offsets to the firmware's.
    fn block4(csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, transmit: &[Rate; PHASES], log: &[Rate; PHASES]) -> Vec<u8> {
        let mut b = head(LAYOUT, csid, module, sf, dbm, set_by);
        for (slot, r) in transmit.iter().chain(log.iter()).enumerate() {
            put_rate(&mut b, slot, r);
        }
        sealed(b)
    }

    // A block of an older layout, 2 or 3: layout 3 carries the log gaps a fresh build shipped.
    fn block(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, interval: u32) -> Vec<u8> {
        block_with(version, csid, module, sf, dbm, set_by, &rates(interval))
    }

    fn block_with(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, rates: &[Rate; OLD_SLOTS]) -> Vec<u8> {
        block_logs(version, csid, module, sf, dbm, set_by, rates, &LOGS, LOG_BLIND)
    }

    #[allow(clippy::too_many_arguments)]
    fn block_logs(version: u8, csid: u8, module: u8, sf: u8, dbm: i8, set_by: u8, rates: &[Rate; OLD_SLOTS], logs: &[u32; OLD_SLOTS], blind: u32) -> Vec<u8> {
        let mut b = head(version, csid, module, sf, dbm, set_by);
        for (p, r) in rates.iter().enumerate() {
            put_rate(&mut b, p, r);
        }
        if version == 3 {
            for (p, ms) in logs.iter().enumerate() {
                b[OLD_LOGS_AT + 4 * p..OLD_LOGS_AT + 4 * (p + 1)].copy_from_slice(&ms.to_le_bytes());
            }
            b[OLD_LOG_BLIND_AT..OLD_LOG_BLIND_AT + 4].copy_from_slice(&blind.to_le_bytes());
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

    // The same bytes test_settings.cpp checks against the firmware's parse().
    fn golden() -> Vec<u8> {
        include_str!("../../../fsw/V2_6_X_Tests/test/data/settings_layout4.inc")
            .lines()
            .filter(|l| !l.starts_with("//"))
            .flat_map(|l| l.split(','))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| u8::from_str_radix(v.trim_start_matches("0x"), 16).unwrap())
            .collect()
    }

    fn golden_asked() -> Asked {
        Asked {
            csid: 7,
            sf: 9,
            dbm: 20,
            transmit: [
                Rate { fixed_ms: 60_000, follow: true, percent: [80, 60, 40, 20], interval_ms: [11_000, 21_000, 31_000, 41_000, 51_000] },
                Rate { fixed_ms: 1_000, follow: false, percent: [75, 50, 35, 20], interval_ms: [1_100, 2_100, 3_100, 4_100, 5_100] },
            ],
            log: [
                Rate { fixed_ms: 10_000, follow: true, percent: [80, 60, 40, 20], interval_ms: [12_000, 22_000, 32_000, 42_000, 0] },
                Rate { fixed_ms: 50, follow: false, percent: [70, 55, 30, 10], interval_ms: [50, 100, 0, 20, 5_000] },
            ],
        }
    }

    #[test]
    fn the_golden_block_reads_field_by_field_and_is_what_a_patch_writes() {
        let g = golden();
        assert_eq!(g.len(), SIZE);
        let wanted = golden_asked();
        match find_at(&image(&[(1001, g.clone())])).unwrap() {
            (Found::Block(s), Some(1001)) => {
                assert_eq!((s.layout, s.csid, s.module, s.sf, s.dbm, s.set_by), (4, 7, "hp", 9, 20, "tool"));
                assert_eq!(s.transmit.as_ref(), Some(&wanted.transmit));
                assert_eq!(s.log.as_ref(), Some(&wanted.log));
                assert_eq!((s.tx_interval_ms, s.reserved_clear), (1_000, true));
                assert!(s.older.is_empty());
            }
            other => panic!("{:?}", other),
        }
        // And written from those values, byte for byte, CRC included.
        let built = image(&[(1001, block4(255, 2, 7, 14, 1, &transmit(1000), &log()))]);
        let (at, _) = writable(&built).unwrap();
        assert_eq!(patch(&built, at, &wanted).unwrap()[at..at + SIZE], g[..]);
    }

    #[test]
    fn a_block_in_an_image_is_found_and_read() {
        let by_battery = Rate { fixed_ms: 300_000, follow: true, percent: [90, 60, 30, 5], interval_ms: [5_000, 20_000, 60_000, 90_000, 200_000] };
        let tx = [by_battery.clone(), rate(30_000, false)];
        let logs = [flat(10_000), Rate { fixed_ms: 50, follow: true, percent: [70, 40, 20, 10], interval_ms: [50, 100, 200, 500, 1000] }];
        let b = block4(7, 2, 9, -4, 2, &tx, &logs);
        let (found, at) = find_at(&image(&[(1001, b.clone())])).unwrap();
        assert_eq!(at, Some(1001));
        assert_eq!(
            found,
            Found::Block(Settings {
                layout: 4,
                csid: 7,
                module: "hp",
                sf: 9,
                dbm: -4,
                set_by: "tool",
                tx_interval_ms: 30_000,
                transmit: Some(tx.clone()),
                log: Some(logs.clone()),
                older: Vec::new(),
                reserved_clear: true,
            })
        );
        let j = found.json();
        assert!(j.starts_with("{\"layoutVersion\":4,\"csid\":7,\"module\":\"hp\",\"sf\":9,\"dbm\":-4,\"setBy\":\"tool\",\"txIntervalMs\":30000,\"transmit\":[{\"fixedMs\":300000,\"follow\":true,\"percent\":[90,60,30,5],\"intervalMs\":[5000,20000,60000,90000,200000]},{\"fixedMs\":30000,"), "{}", j);
        assert!(j.ends_with("\"log\":[{\"fixedMs\":10000,\"follow\":false,\"percent\":[75,50,35,20],\"intervalMs\":[10000,10000,10000,10000,10000]},{\"fixedMs\":50,\"follow\":true,\"percent\":[70,40,20,10],\"intervalMs\":[50,100,200,500,1000]}],\"olderSlots\":[],\"reservedClear\":true}"), "{}", j);
        assert_eq!(by_battery.words(TRANSMIT_ZERO_WORDS), "by battery: 5 s above 90 %, 20 s to 60 %, 60 s to 30 %, 90 s to 5 %, then 200 s (300 s when the gauge cannot be read)");
        assert_eq!(rate(1_000, false).words(TRANSMIT_ZERO_WORDS), "every 1 s");
        assert_eq!(rate(0, false).words(TRANSMIT_ZERO_WORDS), "back to back");
        assert_eq!(flat(0).words(log_zero_words(0)), "nothing written");
        assert_eq!(flat(0).words(log_zero_words(1)), "every reading");

        // What a build with fixed settings carries.
        let built = find(&image(&[(2048, block4(255, 1, 7, 14, 1, &transmit(1000), &log()))])).unwrap();
        assert!(built.json().starts_with("{\"layoutVersion\":4,\"csid\":255,\"module\":\"le\",\"sf\":7,\"dbm\":14,\"setBy\":\"build\",\"txIntervalMs\":1000,\"transmit\":["));

        // Right at the end of what was read, with nothing after it.
        let mut tail = image(&[]);
        tail.extend_from_slice(&b);
        assert!(matches!(find(&tail).unwrap(), Found::Block(_)));
    }

    #[test]
    fn a_layout_3_block_gives_the_page_the_same_shape_and_never_shows_its_spare_slots_as_log_tables() {
        // Slots 2 and 3 hold transmit gaps. Read as layout 4 they would be log tables of 1 s and 60 s.
        let mut set = rates(30_000);
        set[0].percent = [90, 60, 30, 5];
        set[3].percent = [90, 60, 30, 5];
        let found = find(&image(&[(1001, block_logs(3, 7, 2, 9, -4, 2, &set, &[10_000, 50, 50, 10_000], 200))])).unwrap();
        let Found::Block(s) = &found else { panic!("{:?}", found) };
        assert_eq!((s.layout, s.tx_interval_ms, s.reserved_clear), (3, 30_000, true));
        assert_eq!(s.transmit.as_ref().unwrap(), &[set[0].clone(), set[1].clone()]);
        let [ground, air] = s.log.clone().unwrap();
        assert_eq!(ground, Rate { fixed_ms: 10_000, follow: false, percent: [90, 60, 30, 5], interval_ms: [10_000; BANDS] });
        assert_eq!(air, flat(50));
        assert!(s.older.is_empty(), "{:?}", s.older);
        let j = found.json();
        assert!(!j.contains("\"rates\"") && !j.contains("\"logMs\":[") && !j.contains("logBlindMs"), "{}", j);

        // A spare slot that differs is kept apart, with what that firmware logged in it.
        let mut set = rates(1_000);
        set[2].fixed_ms = 500;
        set[3].follow = true;
        let Found::Block(s) = find(&image(&[(1001, block_logs(3, 7, 2, 9, 14, 2, &set, &[10_000, 50, 100, 10_000], 200))])).unwrap() else { panic!() };
        assert_eq!(s.older, vec![
            OlderSlot { phase: 1, name: "falling", rate: set[2].clone(), log_ms: Some(100) },
            OlderSlot { phase: 0, name: "landed", rate: set[3].clone(), log_ms: Some(10_000) },
        ]);

        // A layout-2 block's flight gap is its falling one; layout 3's is rising.
        let mut two = rates(5_000);
        two[1].fixed_ms = 7_000;
        let Found::Block(s2) = find(&image(&[(1001, block_with(2, 7, 2, 9, -4, 2, &two))])).unwrap() else { panic!() };
        assert_eq!(s2.tx_interval_ms, 5_000);
        let Found::Block(s3) = find(&image(&[(1001, block_with(3, 7, 2, 9, -4, 2, &two))])).unwrap() else { panic!() };
        assert_eq!(s3.tx_interval_ms, 7_000);
    }

    #[test]
    fn a_layout_1_block_off_an_older_board_is_read_and_has_no_rates() {
        let b = block_v1(12, 1, 9, 14, 2, 500);
        let (found, at) = find_at(&image(&[(1001, b.clone())])).unwrap();
        assert_eq!(at, Some(1001));
        let Found::Block(s) = &found else { panic!("{:?}", found) };
        assert_eq!((s.layout, s.csid, s.module, s.sf, s.dbm, s.set_by, s.tx_interval_ms, s.reserved_clear), (1, 12, "le", 9, 14, "tool", 500, true));
        assert_eq!((s.transmit.clone(), s.log.clone()), (None, None));
        assert!(found.json().starts_with("{\"layoutVersion\":1,"));
        assert!(found.json().contains("\"txIntervalMs\":500,\"transmit\":null,\"log\":null,\"olderSlots\":[],"));
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
            assert!(e.contains(what) && e.contains(OLD_SLOT_WORDS[phase]), "{}: {}", what, e);
            // The same in layout 4, where every slot is read and a log slot is named so.
            let mut b = block4(7, 2, 9, -4, 2, &transmit(1000), &log());
            b[RATES_AT + RATE_BYTES * phase + 5..RATES_AT + RATE_BYTES * phase + 9].copy_from_slice(&pct);
            let e = find(&image(&[(1001, sealed(b))])).unwrap_err();
            assert!(e.contains(what) && e.contains(SLOT_NAMES[phase]), "{}: {}", what, e);
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
        assert_eq!((s.layout, s.csid, s.tx_interval_ms, s.log.clone(), s.reserved_clear), (2, 7, 30_000, None, true));
        assert_eq!(s.transmit, Some(transmit(30_000)));
        assert!(found.json().contains("\"log\":null,"));
        // A log gap the firmware would refuse is an error here too, naming the phase.
        let e = find(&image(&[(1001, block_logs(3, 7, 2, 9, -4, 2, &rates(1), &[1000, 10, 50, 1000], 200))])).unwrap_err();
        assert!(e.contains("for air a flash log gap of") && e.contains("at least 20"), "{}", e);
        let e = find(&image(&[(1001, block_logs(3, 7, 2, 9, -4, 2, &rates(1), &LOGS, 5))])).unwrap_err();
        assert!(e.contains("no altitude source"), "{}", e);
    }

    #[test]
    fn a_layout_this_build_does_not_know_is_named_and_not_decoded() {
        let found = find(&image(&[(1001, block(5, 7, 2, 9, -4, 2, 30_000))])).unwrap();
        assert_eq!(found, Found::Layout(5));
        // Nothing of it goes to the page as settings: fields read by the wrong layout
        // would look like settings and be wrong.
        assert_eq!(found.json(), "null");
        // And nothing of it is checked: where its CRC sits is part of what is not known,
        // so a damaged block of an unknown layout is still "layout 5" and not "damaged".
        let mut torn = block(5, 7, 2, 9, -4, 2, 30_000);
        torn[40] ^= 0xFF;
        assert_eq!(find(&image(&[(1001, torn)])).unwrap(), Found::Layout(5));
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
        // The bytes layout 3 kept log gaps in are reserved again in layout 4.
        for at in [RESERVED_AT, CRC_AT - 1] {
            let mut b = block4(7, 2, 9, -4, 2, &transmit(1000), &log());
            b[at] = 1;
            let image = image(&[(1001, sealed(b))]);
            assert!(matches!(find(&image).unwrap(), Found::Block(s) if !s.reserved_clear), "{}", at);
            assert!(writable(&image).unwrap_err().contains("reserved"), "{}", at);
        }
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
        Asked { csid: 12, sf: 9, dbm: 20, transmit: transmit(1000), log: log() }
    }

    #[test]
    fn a_patch_changes_the_block_and_nothing_else() {
        let before = image(&[(1001, block4(255, 2, 7, 14, 1, &transmit(1000), &log()))]);
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
                    layout: 4,
                    csid: 12,
                    module: "hp",
                    sf: 9,
                    dbm: 20,
                    set_by: "tool",
                    tx_interval_ms: 1000,
                    transmit: Some(transmit(1000)),
                    log: Some(log()),
                    older: Vec::new(),
                    reserved_clear: true,
                }),
                Some(1001)
            )
        );
        // Byte for byte, against a block built here by hand.
        assert_eq!(after[1001..1001 + SIZE], block4(12, 2, 9, 20, 2, &transmit(1000), &log())[..]);

        // A negative power, the widest gaps and a table per slot fit their fields.
        let odd_tx = [
            Rate { fixed_ms: 0, follow: true, percent: [99, 66, 33, 1], interval_ms: [0, 1, u32::MAX, 7, 180_000] },
            rate(u32::MAX, false),
        ];
        let odd_log = [
            Rate { fixed_ms: 20, follow: true, percent: [80, 40, 20, 10], interval_ms: [0, 20, 21, u32::MAX, 3_600_000] },
            flat(u32::MAX),
        ];
        let odd = patch(&before, at, &Asked { csid: 1, sf: 12, dbm: -9, transmit: odd_tx.clone(), log: odd_log.clone() }).unwrap();
        assert_eq!(odd[1001..1001 + SIZE], block4(1, 2, 12, -9, 2, &odd_tx, &odd_log)[..]);
        // Patching what a tool already set works the same way.
        let again = patch(&after, at, &Asked { csid: 13, sf: 7, dbm: 14, transmit: transmit(0), log: log() }).unwrap();
        assert_eq!(again[1001..1001 + SIZE], block4(13, 2, 7, 14, 2, &transmit(0), &log())[..]);
        // A table the firmware would refuse is not written, whichever slot it is in.
        let mut bad = asked();
        bad.transmit[1].percent = [50, 75, 35, 20];
        let e = patch(&before, at, &bad).unwrap_err();
        assert!(e.contains("have to fall") && e.contains("air transmit"), "{}", e);
        let mut bad = asked();
        bad.log[0].percent = [50, 75, 35, 20];
        let e = patch(&before, at, &bad).unwrap_err();
        assert!(e.contains("have to fall") && e.contains("ground log"), "{}", e);
    }

    #[test]
    fn one_to_19_ms_is_refused_in_every_log_value_naming_the_cell_with_or_without_the_battery() {
        let before = image(&[(1001, block4(255, 2, 7, 14, 1, &transmit(1000), &log()))]);
        // follow, cell (None is the fixed gap), what the refusal has to say
        let cells: [(bool, Option<usize>, &str); 8] = [
            (false, None, "always, log gap"),
            (true, None, "gauge unread, log gap"),
            (true, Some(0), "above 75 %, log gap"),
            (true, Some(1), "to 50 %, log gap"),
            (true, Some(2), "to 35 %, log gap"),
            (true, Some(3), "to 20 %, log gap"),
            (true, Some(4), "below 20 %, log gap"),
            (false, Some(4), "below 20 %, log gap"),   // off, and still refused
        ];
        for phase in 0..PHASES {
            for (follow, cell, words) in cells {
                for ms in [1, 19] {
                    let mut bad = asked();
                    bad.log[phase].follow = follow;
                    match cell {
                        None => bad.log[phase].fixed_ms = ms,
                        Some(i) => bad.log[phase].interval_ms[i] = ms,
                    }
                    let e = patch(&before, 1001, &bad).unwrap_err();
                    let want = format!("not written: {}, {}: {} ms, which has to be 0 or at least 20", PHASE_WORDS[phase], words, ms);
                    assert_eq!(e, want);
                }
                // 0 and 20 are fine in the same cell.
                for ms in [0, 20] {
                    let mut good = asked();
                    good.log[phase].follow = follow;
                    match cell {
                        None => good.log[phase].fixed_ms = ms,
                        Some(i) => good.log[phase].interval_ms[i] = ms,
                    }
                    assert!(patch(&before, 1001, &good).is_ok(), "{} ms in {}", ms, words);
                }
            }
        }
        // The same value off a board is an error, not a table to show: that board is on its build values.
        let mut hurt = log();
        hurt[1].interval_ms[4] = 7;
        let e = find(&image(&[(1001, block4(9, 2, 7, 14, 2, &transmit(1000), &hurt))])).unwrap_err();
        assert!(e.contains("air, below 20 %, log gap") && e.contains("layout 4"), "{}", e);
        // Transmit has no floor, it is not the log.
        let mut tx = asked();
        tx.transmit[1].interval_ms = [1, 2, 3, 4, 5];
        tx.transmit[0].fixed_ms = 1;
        assert!(patch(&before, 1001, &tx).is_ok());
    }

    #[test]
    fn a_read_back_names_every_value_that_differs() {
        let block_of = |a: &Asked| {
            let b = patch(&image(&[(1001, block4(255, 2, 7, 14, 1, &transmit(1000), &log()))]), 1001, a).unwrap();
            let Found::Block(s) = find(&b).unwrap() else { panic!() };
            s
        };
        let wanted = block_of(&golden_asked());
        assert_eq!(differences(&wanted, &wanted), Vec::<String>::new());
        let changed = |f: &dyn Fn(&mut Asked)| {
            let mut a = golden_asked();
            f(&mut a);
            differences(&wanted, &block_of(&a))
        };
        // Every slot, and in each one every field: flag, fixed gap, thresholds, bands.
        for (slot, phase, column) in [(0, "ground", "transmit"), (1, "air", "transmit"), (2, "ground", "log"), (3, "air", "log")] {
            let rate_of = move |a: &mut Asked, f: &dyn Fn(&mut Rate)| {
                f(if slot < PHASES { &mut a.transmit[slot] } else { &mut a.log[slot - PHASES] })
            };
            let g = golden_asked();
            let r = if slot < PHASES { &g.transmit[slot] } else { &g.log[slot - PHASES] };
            let fixed = if r.follow { "gauge unread" } else { "always" };
            assert_eq!(changed(&|a| rate_of(a, &|r| r.follow = !r.follow)), vec![format!("{}, {} battery switch", phase, column)]);
            assert_eq!(changed(&|a| rate_of(a, &|r| r.fixed_ms += 1000)), vec![format!("{}, {}, {} gap", phase, fixed, column)]);
            for k in 0..BANDS - 1 {
                let n = changed(&|a| rate_of(a, &|r| r.percent[k] = if k == 3 { r.percent[3] - 1 } else { r.percent[k] + 1 }));
                assert_eq!(n, vec![format!("{}, {} battery threshold {}", phase, column, k + 1)]);
            }
            for k in 0..BANDS {
                let n = changed(&|a| rate_of(a, &|r| r.interval_ms[k] += 1000));
                assert_eq!(n, vec![format!("{}, {}, {} gap", phase, band_words(&r.percent, k), column)]);
            }
        }
        // And the head.
        assert_eq!(changed(&|a| a.csid = 8), vec!["board id"]);
        assert_eq!(changed(&|a| a.sf = 8), vec!["spreading factor"]);
        assert_eq!(changed(&|a| a.dbm = 22), vec!["transmit power"]);
        let mut other = wanted.clone();
        other.layout = 3;
        assert_eq!(differences(&wanted, &other), vec!["block layout"]);
        other = wanted.clone();
        other.log = None;
        assert_eq!(differences(&wanted, &other), vec!["log gaps"]);
    }

    #[test]
    fn a_block_that_cannot_be_trusted_is_not_one_to_patch() {
        let e = writable(&image(&[])).unwrap_err();
        assert_eq!(e, "this firmware has no settings block");

        let mut bad = block4(255, 1, 7, 14, 1, &transmit(1000), &log());
        bad[19] ^= 0x10;
        let e = writable(&image(&[(1001, bad)])).unwrap_err();
        assert!(e.contains("damaged"), "{}", e);

        let e = writable(&image(&[(1001, block(5, 255, 1, 7, 14, 1, 1000))])).unwrap_err();
        assert!(e.contains("layout 5"), "{}", e);

        // An older image is read but not written, and the words say what to do instead.
        let e = writable(&image(&[(1001, block_v1(255, 1, 7, 14, 1, 1000))])).unwrap_err();
        assert!(e.contains("layout 1") && e.contains("flash the flight image"), "{}", e);

        // Changed on purpose: layouts 2 and 3 used to be stamped with the newest layout. The
        // firmware needs its exact layout, so that image booted on compiled defaults while the
        // read-back passed. Patching it now is refused, and so is a tree still at layout 3.
        for old in [2, 3] {
            let e = writable(&image(&[(1001, block(old, 255, 1, 7, 14, 1, 1000))])).unwrap_err();
            let says = format!("layout {} and this app writes layout 4", old);
            assert!(e.contains(&says) && e.contains("flash the flight image") && e.contains("not patched"), "{}", e);
        }

        // Told to patch somewhere that is not a block, it does not.
        let whole = image(&[(1001, block4(255, 1, 7, 14, 1, &transmit(1000), &log()))]);
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
            assert_eq!(s.transmit.as_ref(), Some(&radio.transmit), "{}", image.file);
            assert_eq!(s.log.as_ref(), Some(&radio.log), "{}", image.file);
            // Flat: a default image does what it did before the log had bands.
            for r in radio.log {
                assert_eq!((r.follow, r.interval_ms), (false, [r.fixed_ms; BANDS]), "{}", image.file);
            }
        }
    }
}
