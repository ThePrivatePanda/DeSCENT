// A log as a table, as web/js/exportcsv.js writes it: one row per packet that passes its
// CRC, a packet two receivers heard being two rows. Same columns and the same digits, so
// a sheet made from either lines up.
use crate::decode::js_num;
use crate::lines::{js_number, parse_line, Info, Kind};

// The fields after csid, counter and validity, which lead the row: key, column name, and
// the decimals packet.js csvDecimals gives it (the field's own resolution)
const FIELDS: [(&str, &str, usize, usize); 21] = [
    ("lat", "lat", 0, 7),
    ("lon", "lon", 1, 7),
    ("gpsAlt", "gps_alt_m", 2, 3),
    ("envAlt", "baro_alt_m", 3, 2),
    ("ax", "ax", 4, 2),
    ("ay", "ay", 5, 2),
    ("az", "az", 6, 2),
    ("gx", "gx", 7, 1),
    ("gy", "gy", 8, 1),
    ("gz", "gz", 9, 1),
    ("mx", "mx", 10, 1),
    ("my", "my", 11, 1),
    ("mz", "mz", 12, 1),
    ("qi", "qi", 13, 5),
    ("qj", "qj", 14, 5),
    ("qk", "qk", 15, 5),
    ("qr", "qr", 16, 5),
    ("temp", "temp_c", 17, 2),
    ("pressure", "pressure_hpa", 18, 1),
    ("humidity", "humidity", 19, 2),
    ("battery", "cell_v", 22, 2),
];
// Which validity bit each of FIELDS needs, in the same order (packet.js groups)
const GROUP_BIT: [usize; 21] = [4, 4, 4, 6, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 6, 6, 6, 5];

pub fn columns() -> String {
    let mut c = vec!["time_ms", "time_utc", "source", "boot", "uptime_ms", "csid", "counter", "validity"];
    c.extend(FIELDS.iter().map(|f| f.1));
    c.extend(["rssi", "snr", "freq_err"]);
    c.join(",")
}

// Number.prototype.toFixed, then trailing zeros off. toFixed rounds the exact binary
// value, half away from zero; Rust's {:.N} rounds half to even, so a tie is settled here.
fn fixed(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return js_num(v).replace("null", "NaN");
    }
    let mut s = format!("{:.*}", decimals, v);
    let wide = format!("{:.*}", decimals + 30, v);
    // an exact tie shows as 5 then only zeros past the cut
    let tail = &wide[wide.len() - 30..];
    if tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0') {
        let step = 10f64.powi(-(decimals as i32));
        let away = if v < 0.0 { -step / 2.0 } else { step / 2.0 };
        s = format!("{:.*}", decimals, v + away);
    }
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

fn plain(v: f64) -> String {
    if v.is_finite() {
        js_num(v)
    } else {
        String::new()
    }
}

fn quoted(s: &str) -> String {
    if s.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// Date.prototype.toISOString; empty where new Date(t) is an invalid date
fn utc(t: f64) -> String {
    if !(t.abs() <= 8.64e15) {
        return String::new();
    }
    match chrono::DateTime::from_timestamp_millis(t.trunc() as i64) {
        Some(d) => {
            let y = chrono::Datelike::year(&d);
            let rest = d.format("-%m-%dT%H:%M:%S%.3fZ");
            if (0..=9999).contains(&y) {
                format!("{:04}{}", y, rest)
            } else {
                format!("{}{:06}{}", if y < 0 { '-' } else { '+' }, y.abs(), rest)
            }
        }
        None => String::new(),
    }
}

fn get<'a>(info: &'a Info, key: &str) -> Option<&'a str> {
    info.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[derive(Default)]
struct From {
    src: Option<Info>,
    boot: Option<String>,   // undefined in exportcsv.js when the boot line had no n
}

// events: (t ms, receiver, line text), in the order a replay reads them
pub fn to_csv<'a>(events: impl Iterator<Item = (f64, &'a str, &'a str)>) -> String {
    let mut out = columns();
    out.push('\n');
    let mut from: Vec<(String, From)> = Vec::new();
    for (t, rx, text) in events {
        let ev = parse_line(text);
        let slot = |from: &mut Vec<(String, From)>| -> usize {
            match from.iter().position(|(k, _)| k == rx) {
                Some(i) => i,
                None => {
                    from.push((rx.to_string(), From::default()));
                    from.len() - 1
                }
            }
        };
        let (d, rssi, snr, freq_err, more) = match &ev.kind {
            Kind::Device { kind: "source", info, .. } => {
                let i = slot(&mut from);
                from[i].1.src = Some(info.clone());
                continue;
            }
            Kind::Device { kind: "boot", info, .. } => {
                let i = slot(&mut from);
                from[i].1.boot = get(info, "n").map(String::from);
                continue;
            }
            Kind::Packet { decoded: Ok(d), rssi, snr, freq_err, more, .. } if d.crc_ok => (d, *rssi, *snr, *freq_err, more),
            _ => continue,
        };
        let empty = From::default();
        let f = from.iter().find(|(k, _)| k == rx).map(|(_, f)| f).unwrap_or(&empty);
        let no_info = Info::new();
        let src = f.src.as_ref().unwrap_or(&no_info);
        // anything not anchored to GPS is a derived time, and as an ISO string it would
        // look exactly like a measured one
        let derived = get(src, "time_source").is_some_and(|s| !s.is_empty() && s != "anchor");
        let uptime0 = get(src, "uptime0_ms").map(js_number).unwrap_or(f64::NAN);
        let uptime = match more.as_ref().and_then(|m| get(m, "up")) {
            Some(up) => js_number(up),
            None => t - uptime0,
        };
        let mut row = vec![
            plain(t),
            if derived { String::new() } else { utc(t) },
            quoted(rx),
            f.boot.as_deref().map(quoted).unwrap_or_default(),
            plain(uptime),
            plain(d.values[21]),
            plain(d.values[20]),
            format!("{:02X}", d.values[23] as u32),
        ];
        // a reading the board marked invalid is left out, so nobody plots it
        for (k, (_, _, at, decimals)) in FIELDS.iter().enumerate() {
            row.push(if d.valid[GROUP_BIT[k]] { fixed(d.values[*at], *decimals) } else { String::new() });
        }
        row.push(plain(rssi));
        row.push(plain(snr));
        row.push(plain(freq_err));
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_is_to_fixed_with_the_zeros_off() {
        assert_eq!(fixed(42.4659609, 7), "42.4659609");
        assert_eq!(fixed(2.5, 2), "2.5");
        assert_eq!(fixed(-0.001, 2), "0");
        assert_eq!(fixed(1.0, 2), "1");
        // ties: toFixed goes away from zero, {:.N} would go to even
        assert_eq!(fixed(0.125, 2), "0.13");
        assert_eq!(fixed(-0.125, 2), "-0.13");
        assert_eq!(fixed(2.5, 0), "3");
    }

    #[test]
    fn the_columns_are_exportcsv_js_columns() {
        assert_eq!(
            columns(),
            "time_ms,time_utc,source,boot,uptime_ms,csid,counter,validity,lat,lon,gps_alt_m,baro_alt_m,ax,ay,az,gx,gy,gz,mx,my,mz,qi,qj,qk,qr,temp_c,pressure_hpa,humidity,cell_v,rssi,snr,freq_err"
        );
    }

    #[test]
    fn utc_is_to_iso_string() {
        assert_eq!(utc(1790000000123.0), "2026-09-21T14:13:20.123Z");
        assert_eq!(utc(f64::NAN), "");
    }

    // Not a fixture: the Rust half of the check against exportcsv.js on real logs, which
    // ground/tools/golden_csv.sh runs and diffs.
    // DG_GOLDEN_LIST=<file of paths> DG_GOLDEN_CSV_OUT=<folder> cargo test csv::tests::golden
    #[test]
    fn golden() {
        let Ok(list) = std::env::var("DG_GOLDEN_LIST") else { return };
        let Ok(out) = std::env::var("DG_GOLDEN_CSV_OUT") else { return };
        const BASE_MS: f64 = 1793232000000.0;   // as golden_csv.js
        let list = std::fs::read_to_string(&list).unwrap();
        for (n, path) in list.lines().filter(|l| !l.is_empty()).enumerate() {
            let bytes = std::fs::read(path).unwrap();
            let content = String::from_utf8_lossy(&bytes);
            let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy();
            let events = crate::lines::merge_files(vec![crate::lines::log_events(&content, &name, BASE_MS)]);
            let csv = to_csv(events.iter().map(|e| (e.t, e.rx.as_str(), e.text)));
            std::fs::write(format!("{}/{}.rust.csv", out, n), csv).unwrap();
        }
    }
}
