// The 55-byte telemetry packet, as web/js/packet.js reads it. Same field names, same
// scaling and the same f64 arithmetic, so a packet decoded here and one decoded in the
// page give the same numbers. Nothing here judges whether a value makes sense.
// Bytes 0-52 are data, 53-54 the CRC-16/CCITT-FALSE of bytes 0-52, little-endian.
#![allow(dead_code)]   // the server starts using it in the next part of the 1.4 split

pub const PACKET_LENGTH: usize = 55;
pub const DATA_LENGTH: usize = 53;

#[derive(Clone, Copy)]
enum Ty {
    I32,
    I16,
    U16,
    U8,
}

struct Field {
    key: &'static str,
    ty: Ty,
    scale: f64,
    base: f64,   // packet.js adds (f.base || 0), so no base is a 0 here
}

const fn f(key: &'static str, ty: Ty, scale: f64) -> Field {
    Field { key, ty, scale, base: 0.0 }
}

// Packet order, as FIELDS in packet.js
const FIELDS: [Field; 24] = [
    f("lat", Ty::I32, 1e7),
    f("lon", Ty::I32, 1e7),
    f("gpsAlt", Ty::I32, 1000.0),
    f("envAlt", Ty::I32, 100.0),
    f("ax", Ty::I16, 100.0),
    f("ay", Ty::I16, 100.0),
    f("az", Ty::I16, 100.0),
    f("gx", Ty::I16, 10.0),
    f("gy", Ty::I16, 10.0),
    f("gz", Ty::I16, 10.0),
    f("mx", Ty::I16, 10.0),
    f("my", Ty::I16, 10.0),
    f("mz", Ty::I16, 10.0),
    f("qi", Ty::I16, 32767.0),
    f("qj", Ty::I16, 32767.0),
    f("qk", Ty::I16, 32767.0),
    f("qr", Ty::I16, 32767.0),
    f("temp", Ty::I16, 100.0),
    f("pressure", Ty::U16, 10.0),
    f("humidity", Ty::U16, 100.0),
    f("counter", Ty::U16, 1.0),
    f("csid", Ty::U8, 1.0),
    // Cell voltage above 2.5 V in 10 mV steps since 2026-10-17 (percent x2 before)
    Field { key: "battery", ty: Ty::U8, scale: 100.0, base: 2.5 },
    f("validity", Ty::U8, 1.0),
];

const CSID: usize = 21;
const VALIDITY: usize = 23;

// Validity bits, bit 0 first
pub const VALIDITY_KEYS: [&str; 8] = ["accel", "gyro", "mag", "quat", "gps", "soc", "env", "fresh"];

pub const ACCEL_SATURATION_MPS2: f64 = 75.0;

fn size(ty: Ty) -> usize {
    match ty {
        Ty::I32 => 4,
        Ty::I16 | Ty::U16 => 2,
        Ty::U8 => 1,
    }
}

fn offsets() -> [usize; 24] {
    let mut out = [0; 24];
    let mut at = 0;
    for (i, fl) in FIELDS.iter().enumerate() {
        out[i] = at;
        at += size(fl.ty);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub crc_ok: bool,
    pub crc_received: u16,
    pub crc_calculated: u16,
    pub values: [f64; 24],   // in FIELDS order
    pub valid: [bool; 8],    // in VALIDITY_KEYS order
    pub saturated: bool,
}

impl Decoded {
    pub fn value(&self, key: &str) -> Option<f64> {
        FIELDS.iter().position(|fl| fl.key == key).map(|i| self.values[i])
    }

    pub fn csid(&self) -> u8 {
        self.values[CSID] as u8
    }
}

// Ok, or the error packet.js puts in { ok: false, error }
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() != PACKET_LENGTH {
        return Err(format!("length {}, expected 55", bytes.len()));
    }
    let off = offsets();
    let mut values = [0.0; 24];
    for (i, fl) in FIELDS.iter().enumerate() {
        let o = off[i];
        let raw = match fl.ty {
            Ty::I32 => i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64,
            Ty::I16 => i16::from_le_bytes([bytes[o], bytes[o + 1]]) as f64,
            Ty::U16 => u16::from_le_bytes([bytes[o], bytes[o + 1]]) as f64,
            Ty::U8 => bytes[o] as f64,
        };
        values[i] = raw / fl.scale + fl.base;
    }
    let crc_received = u16::from_le_bytes([bytes[DATA_LENGTH], bytes[DATA_LENGTH + 1]]);
    let crc_calculated = crate::dump::crc16(&bytes[..DATA_LENGTH]);
    let mask = values[VALIDITY] as u32;
    let mut valid = [false; 8];
    for (bit, v) in valid.iter_mut().enumerate() {
        *v = (mask >> bit) & 1 == 1;
    }
    let saturated = valid[0] && values[4..7].iter().any(|v| v.abs() >= ACCEL_SATURATION_MPS2);
    Ok(Decoded { crc_ok: crc_received == crc_calculated, crc_received, crc_calculated, values, valid, saturated })
}

// Math.round: halves go up, toward +infinity, not away from zero
fn js_round(x: f64) -> f64 {
    let r = x.floor();
    if x - r >= 0.5 {
        r + 1.0
    } else {
        r
    }
}

// What a DataView set does with a number, ToInt32/ToUint32 and keep the low bits
fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(4294967296.0) as u32
}

// Rebuild the 55 bytes from values, as packet.js encode() does for the old CSV receiver,
// which printed values but not bytes. Values are pinned to what the field can hold.
pub fn encode(values: &[f64; 24], crc: f64) -> [u8; PACKET_LENGTH] {
    let off = offsets();
    let mut out = [0u8; PACKET_LENGTH];
    for (i, fl) in FIELDS.iter().enumerate() {
        let (lo, hi) = match fl.ty {
            Ty::I32 => (-2147483648.0, 2147483647.0),
            Ty::I16 => (-32768.0, 32767.0),
            Ty::U16 => (0.0, 65535.0),
            Ty::U8 => (0.0, 255.0),
        };
        let raw = to_uint32(js_round((values[i] - fl.base) * fl.scale).min(hi).max(lo));
        let o = off[i];
        out[o..o + size(fl.ty)].copy_from_slice(&raw.to_le_bytes()[..size(fl.ty)]);
    }
    out[DATA_LENGTH..].copy_from_slice(&((to_uint32(crc) & 0xffff) as u16).to_le_bytes());
    out
}

// Drops anything that is not a hex digit first, then pairs; an odd count is None
pub fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = hex.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    if clean.len() % 2 != 0 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).unwrap() as u8;
    Some(clean.chunks(2).map(|p| digit(p[0]) << 4 | digit(p[1])).collect())
}

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

// A number written as JSON.stringify writes it: NaN and infinities are null, -0 is 0,
// and the digits are the shortest that read back, with JS's switch to e-notation
// below 1e-6 and from 1e21.
pub fn js_num(x: f64) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    // {:e} gives the shortest digits that round-trip, as JS's Number::toString wants
    let sci = format!("{:e}", x.abs());
    let (mant, exp) = sci.split_once('e').unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap() + 1;
    let mut s = String::from(if x < 0.0 { "-" } else { "" });
    if k <= n && n <= 21 {
        s.push_str(&digits);
        s.extend(std::iter::repeat('0').take((n - k) as usize));
    } else if 0 < n && n <= 21 {
        s.push_str(&digits[..n as usize]);
        s.push('.');
        s.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        s.push_str("0.");
        s.extend(std::iter::repeat('0').take((-n) as usize));
        s.push_str(&digits);
    } else {
        s.push_str(&digits[..1]);
        if k > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push_str(&format!("e{}{}", if n - 1 < 0 { '-' } else { '+' }, (n - 1).abs()));
    }
    s
}

// The decoded object with packet.js's keys, in its order
pub fn to_json(r: &Result<Decoded, String>) -> String {
    let d = match r {
        Ok(d) => d,
        Err(e) => return format!("{{\"ok\":false,\"error\":\"{}\"}}", crate::json::esc(e)),
    };
    let values: Vec<String> = FIELDS.iter().zip(d.values.iter()).map(|(fl, v)| format!("\"{}\":{}", fl.key, js_num(*v))).collect();
    let valid: Vec<String> = VALIDITY_KEYS.iter().zip(d.valid.iter()).map(|(k, v)| format!("\"{}\":{}", k, v)).collect();
    format!(
        "{{\"ok\":true,\"crcOk\":{},\"crcReceived\":{},\"crcCalculated\":{},\"values\":{{{}}},\"valid\":{{{}}},\"saturated\":{}}}",
        d.crc_ok,
        d.crc_received,
        d.crc_calculated,
        values.join(","),
        valid.join(","),
        d.saturated
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(hex: &str) -> Decoded {
        decode(&hex_to_bytes(hex).unwrap()).unwrap()
    }

    // The packet packet.test.js uses, off a ChipSat console hex dump
    // (SSDS/logs/20260909_195429_baseline_prior_fw.log, line 34)
    const CONSOLE: &str = "00 00 00 00 00 00 00 00 98 BD FF FF 23 5B 00 00 FD FF 00 00 FD FF 01 00 FF FF FF FF AC FF 18 FF D4 FD 4A 06 8E 02 ED 82 56 1A 7A 0A 7F 26 0F 11 04 00 09 6D 6F 09 23";

    // Expected values below are what packet.js decode() printed for the same hex
    // (node, JSON.stringify), not worked out by hand.
    #[test]
    fn decodes_a_console_packet_as_packet_js_does() {
        let d = packet(CONSOLE);
        assert_eq!(
            to_json(&Ok(d.clone())),
            r#"{"ok":true,"crcOk":true,"crcReceived":8969,"crcCalculated":8969,"values":{"lat":0,"lon":0,"gpsAlt":-17,"envAlt":233.31,"ax":-0.03,"ay":0,"az":-0.03,"gx":0.1,"gy":-0.1,"gz":-0.1,"mx":-8.4,"my":-23.2,"mz":-55.6,"qi":0.04913480025635548,"qj":0.019959105197302165,"qk":-0.9771721549119541,"qr":0.20575579088717308,"temp":26.82,"pressure":985.5,"humidity":43.67,"counter":4,"csid":9,"battery":3.59,"validity":111},"valid":{"accel":true,"gyro":true,"mag":true,"quat":true,"gps":false,"soc":true,"env":true,"fresh":false},"saturated":false}"#
        );
        assert_eq!(d.csid(), 9);
    }

    #[test]
    fn crc_is_ccitt_false_and_a_flipped_bit_fails_it() {
        let mut b = hex_to_bytes(CONSOLE).unwrap();
        assert!(decode(&b).unwrap().crc_ok);
        b[10] ^= 1;
        assert!(!decode(&b).unwrap().crc_ok);
    }

    #[test]
    fn wrong_length_is_an_error_not_a_panic() {
        assert_eq!(decode(&[0; 54]), Err("length 54, expected 55".into()));
        assert_eq!(to_json(&decode(&[])), r#"{"ok":false,"error":"length 0, expected 55"}"#);
    }

    #[test]
    fn encode_of_decode_gives_the_bytes_back() {
        let b = hex_to_bytes(CONSOLE).unwrap();
        let d = decode(&b).unwrap();
        assert_eq!(encode(&d.values, d.crc_received as f64).to_vec(), b);
    }

    #[test]
    fn numbers_print_as_js_prints_them() {
        let cases = [
            (0.1 + 0.2, "0.30000000000000004"),
            (-0.0, "0"),
            (f64::NAN, "null"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (-1.5e-7, "-1.5e-7"),
            (2.58, "2.58"),
            (-17.0, "-17"),
        ];
        for (x, want) in cases {
            assert_eq!(js_num(x), want, "{}", x);
        }
    }
}
