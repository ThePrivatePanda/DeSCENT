// What the flight software will take for a radio, and whether the settings asked for a
// board are inside it. Only the carried image needs this: a build refuses a bad setting
// itself, and a patched block the firmware turns down flies on what the image was built
// with and says so only on its console.
//
// The table is the flight repo's radio.json, generated there from the firmware's own
// constants and carried here as it came. Nothing about the radio is written down in
// this file: a second copy of that table is how a board ends up set for a power its
// amplifier does not have, and the firmware turns such a block down at boot and flies
// on what it was built with.
use crate::json::{bool_field, int_field};
use crate::settings::{Asked, Rate, BANDS, PHASES, PHASE_NAMES};

#[derive(Debug, Clone, PartialEq)]
pub struct Radio {
    pub sf_min: i64,
    pub sf_max: i64,
    pub le: Vec<i64>,   // the transmit powers each module has a row for, in dBm
    pub hp: Vec<i64>,
    // The rate per phase a fresh image ships, so the page can offer them before any board
    // has been read, and the flash log gaps it ships the same way.
    pub rates: [Rate; PHASES],
    pub logs: [u32; PHASES],
    pub log_blind_ms: u32,
}

// What the page asked for, as it came. Numbers of any size, because saying "csid 256 is
// not a board id" needs the 256. Tables come as lists so that "five thresholds" can be
// refused in words rather than silently cut to four.
#[derive(Debug, Clone, PartialEq)]
pub struct RateRequest {
    pub fixed_ms: i64,
    pub follow: bool,
    pub percent: Vec<i64>,
    pub interval_ms: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub csid: i64,
    pub sf: i64,
    pub dbm: i64,
    pub rates: Vec<RateRequest>,   // waiting, rising, falling, landed
    pub logs: Vec<i64>,            // the flash log gap in ms, same order
    pub log_blind_ms: i64,         // and with no altitude source
}

// The numbers inside the first [ ] after a field of that name.
fn int_list(body: &str, name: &str) -> Option<Vec<i64>> {
    let at = body.find(&format!("\"{}\"", name))?;
    let rest = &body[at..];
    let open = rest.find('[')?;
    let close = rest[open..].find(']')? + open;
    rest[open + 1..close].split(',').map(|v| v.trim().parse::<i64>().ok()).collect()
}

// radio.json with the crate's own field readers. They take the first field of a name
// they find, so each part is cut out before it is read.
pub fn parse(text: &str) -> Result<Radio, String> {
    let at = text.find("\"spreadingFactor\"").ok_or("it has no spreadingFactor")?;
    let range = &text[at..];
    let range = &range[..range.find('}').ok_or("its spreadingFactor does not close")?];
    let sf_min = int_field(range, "min").ok_or("its spreadingFactor has no min")?;
    let sf_max = int_field(range, "max").ok_or("its spreadingFactor has no max")?;
    if sf_min > sf_max {
        return Err(format!("its spreadingFactor runs from {} down to {}", sf_min, sf_max));
    }
    let (rates, logs) = rates(text)?;
    let log_blind_ms = int_field(text, "logBlindMs").ok_or("it has no logBlindMs")?;
    let log_blind_ms = u32::try_from(log_blind_ms).map_err(|_| format!("its logBlindMs of {} does not fit", log_blind_ms))?;
    if let Some(t) = crate::settings::log_trouble(log_blind_ms) {
        return Err(format!("with no altitude source it has {}", t));
    }
    Ok(Radio { sf_min, sf_max, le: powers(text, "le")?, hp: powers(text, "hp")?, rates, logs, log_blind_ms })
}

// The dbm of every row a module has. The list of rows and the field in each row are
// both called dbm.
fn powers(text: &str, module: &str) -> Result<Vec<i64>, String> {
    let none = || format!("it has no powers for the {} module", module.to_uppercase());
    let at = text.find(&format!("\"{}\"", module)).ok_or_else(none)?;
    let rest = &text[at..];
    let rows = &rest[rest.find("\"dbm\"").ok_or_else(none)?..];
    // A brace closing before the list would mean the list found belongs to whatever
    // comes after this module.
    if rest[..rest.len() - rows.len()].contains('}') {
        return Err(none());
    }
    let (open, close) = (rows.find('[').ok_or_else(none)?, rows.find(']').ok_or_else(none)?);
    if close < open {
        return Err(none());
    }
    let mut powers = Vec::new();
    for row in rows[open + 1..close].split('}').filter(|r| r.contains('{')) {
        powers.push(int_field(row, "dbm").ok_or_else(|| format!("a row for the {} module has no dbm", module.to_uppercase()))?);
    }
    if powers.is_empty() {
        return Err(none());
    }
    Ok(powers)
}

// The phases list, as emit_battery_json.cpp writes it from config.h: one object per
// phase, named, in the block's order, each with its rate and its flash log gap.
fn rates(text: &str) -> Result<([Rate; PHASES], [u32; PHASES]), String> {
    let at = text.find("\"phases\"").ok_or("it has no phases")?;
    let part = &text[at..];
    // The list closes at the ] that matches its [, not the first one: each phase holds
    // lists of its own.
    let open = part.find('[').ok_or("its phases do not open")?;
    let mut depth = 0i32;
    let mut close = None;
    for (i, c) in part[open..].char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.ok_or("its phases do not close")?;
    let objects: Vec<&str> = part[open..close].split('}').filter(|r| r.contains('{')).collect();
    if objects.len() != PHASES {
        return Err(format!("it has {} phases, and the block holds {}", objects.len(), PHASES));
    }
    let mut out: Vec<Rate> = Vec::new();
    let mut logs = [0u32; PHASES];
    for (p, obj) in objects.iter().enumerate() {
        let name = crate::json::str_field(obj, "name").unwrap_or_default();
        if name != PHASE_NAMES[p] {
            return Err(format!("its phase {} is called {:?}, and the block's is {}", p, name, PHASE_NAMES[p]));
        }
        let fixed_ms = int_field(obj, "fixedMs").ok_or_else(|| format!("its {} phase has no fixedMs", name))?;
        let follow = bool_field(obj, "follow").ok_or_else(|| format!("its {} phase has no follow", name))?;
        let percent = int_list(obj, "percent").ok_or_else(|| format!("its {} phase has no percent list", name))?;
        let interval = int_list(obj, "intervalMs").ok_or_else(|| format!("its {} phase has no intervalMs list", name))?;
        let r = rate(&name, &RateRequest { fixed_ms, follow, percent, interval_ms: interval })?;
        if let Some(t) = crate::settings::table_trouble(&r.percent) {
            return Err(format!("its {} phase has {}", name, t));
        }
        let log = int_field(obj, "logMs").ok_or_else(|| format!("its {} phase has no logMs", name))?;
        logs[p] = u32::try_from(log).map_err(|_| format!("its {} phase has a logMs of {}, which does not fit", name, log))?;
        if let Some(t) = crate::settings::log_trouble(logs[p]) {
            return Err(format!("its {} phase has {}", name, t));
        }
        out.push(r);
    }
    Ok(([out[0].clone(), out[1].clone(), out[2].clone(), out[3].clone()], logs))
}

// One phase's rate from what was asked, with every value in its byte or its four, or why
// not, naming the phase.
fn rate(phase: &str, r: &RateRequest) -> Result<Rate, String> {
    let fits = |what: &str, ms: i64| -> Result<u32, String> {
        u32::try_from(ms).map_err(|_| format!("the {} {} of {} ms does not fit: it has to be 0 to {}", phase, what, ms, u32::MAX))
    };
    if r.percent.len() != BANDS - 1 {
        return Err(format!("the {} battery table takes {} thresholds, and {} were given", phase, BANDS - 1, r.percent.len()));
    }
    if r.interval_ms.len() != BANDS {
        return Err(format!("the {} battery table takes {} intervals, and {} were given", phase, BANDS, r.interval_ms.len()));
    }
    let mut out = Rate { fixed_ms: fits("gap", r.fixed_ms)?, follow: r.follow, percent: [0; BANDS - 1], interval_ms: [0; BANDS] };
    for (i, p) in r.percent.iter().enumerate() {
        if !(1..=99).contains(p) {
            return Err(format!("a {} battery threshold of {} %, which has to be 1 to 99", phase, p));
        }
        out.percent[i] = *p as u8;
    }
    for (i, ms) in r.interval_ms.iter().enumerate() {
        out.interval_ms[i] = fits("battery interval", *ms)?;
    }
    Ok(out)
}

pub fn carried() -> Result<Radio, String> {
    match crate::flash::CHIPSAT_RADIO {
        Some(text) => parse(text).map_err(|e| format!("the radio table in this build does not read ({}), so settings cannot be checked against it", e)),
        None => Err(String::from("this build carries no radio table, so there is nothing to check settings against")),
    }
}

fn listed(values: &[i64]) -> String {
    values.iter().map(|v| v.to_string()).collect::<Vec<String>>().join(", ")
}

impl Radio {
    // The request as it can go into a block, or what is wrong with it, by the rules the
    // firmware's own ChipSatSettings::usable applies at boot. `module` is the one the
    // image is built for. Id 255 is let through because the firmware takes it: it is the
    // value that says nobody set this board up.
    pub fn check(&self, r: &Request, module: &str) -> Result<Asked, String> {
        if !(1..=255).contains(&r.csid) {
            return Err(format!("{} is not a board id: it has to be 1 to 255, and 255 means nobody set it", r.csid));
        }
        if !(self.sf_min..=self.sf_max).contains(&r.sf) {
            return Err(format!(
                "SF{} is not a spreading factor the firmware takes: it has to be {} to {}",
                r.sf, self.sf_min, self.sf_max
            ));
        }
        let (name, rows) = match module {
            "le" => ("LE", &self.le),
            "hp" => ("HP", &self.hp),
            other => return Err(format!("{:?} is not a radio module; it is le or hp", other)),
        };
        if !rows.contains(&r.dbm) {
            return Err(format!("{} dBm is not a power the {} module has: it has to be one of {}", r.dbm, name, listed(rows)));
        }
        if r.rates.len() != PHASES {
            return Err(format!("a rate for each of the {} phases is needed, and {} were given", PHASES, r.rates.len()));
        }
        if r.logs.len() != PHASES {
            return Err(format!("a flash log gap for each of the {} phases is needed, and {} were given", PHASES, r.logs.len()));
        }
        let log_fits = |what: &str, ms: i64| -> Result<u32, String> {
            let ms = u32::try_from(ms).map_err(|_| format!("the {} log gap of {} ms does not fit: it has to be {} to {}", what, ms, crate::settings::LOG_MIN_MS, u32::MAX))?;
            match crate::settings::log_trouble(ms) {
                Some(t) => Err(format!("{}: {}", what, t)),
                None => Ok(ms),
            }
        };
        let mut logs = [0u32; PHASES];
        for (p, ms) in r.logs.iter().enumerate() {
            logs[p] = log_fits(PHASE_NAMES[p], *ms)?;
        }
        let log_blind_ms = log_fits("no altitude source", r.log_blind_ms)?;
        // The same rules the firmware applies at boot: a table it turns down costs the
        // board every setting in the block, not just the table.
        let mut rates: Vec<Rate> = Vec::new();
        for (p, rr) in r.rates.iter().enumerate() {
            let out = rate(PHASE_NAMES[p], rr)?;
            if let Some(t) = crate::settings::table_trouble(&out.percent) {
                return Err(format!("for {}, {}", PHASE_NAMES[p], t));
            }
            rates.push(out);
        }
        Ok(Asked {
            csid: r.csid as u8,
            sf: r.sf as u8,
            dbm: r.dbm as i8,
            rates: [rates[0].clone(), rates[1].clone(), rates[2].clone(), rates[3].clone()],
            logs,
            log_blind_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHASES_JSON: &str = r#""phases": [
    {"name": "waiting", "fixedMs": 60000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000], "logMs": 1000},
    {"name": "rising", "fixedMs": 1000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000], "logMs": 50},
    {"name": "falling", "fixedMs": 1000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000], "logMs": 50},
    {"name": "landed", "fixedMs": 60000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000], "logMs": 1000}
  ],
  "logBlindMs": 200"#;
    const LOGS: [u32; PHASES] = [1000, 50, 50, 1000];

    // As the flight repo's tool writes it.
    fn table_text() -> String {
        format!(r#"{{
  "layoutVersion": 3,
  "spreadingFactor": {{"min": 5, "max": 12}},
  {},
  "le": {{
    "sleepsDuringTx": false,
    "dbm": [
      {{"dbm": 10, "txparams": 13, "dutycycle": 1, "hpmax": 0, "devicesel": 1, "lut": 1}},
      {{"dbm": 14, "txparams": 14, "dutycycle": 4, "hpmax": 0, "devicesel": 1, "lut": 1}},
      {{"dbm": 15, "txparams": 14, "dutycycle": 6, "hpmax": 0, "devicesel": 1, "lut": 1}}
    ]
  }},
  "hp": {{
    "sleepsDuringTx": true,
    "dbm": [
      {{"dbm": 14, "txparams": 22, "dutycycle": 2, "hpmax": 2, "devicesel": 0, "lut": 1}},
      {{"dbm": 17, "txparams": 22, "dutycycle": 2, "hpmax": 3, "devicesel": 0, "lut": 1}},
      {{"dbm": 20, "txparams": 22, "dutycycle": 3, "hpmax": 5, "devicesel": 0, "lut": 1}},
      {{"dbm": 22, "txparams": 22, "dutycycle": 4, "hpmax": 7, "devicesel": 0, "lut": 1}}
    ]
  }}
}}
"#, PHASES_JSON)
    }

    const PCT: [u8; 4] = [75, 50, 35, 20];
    const MS: [u32; 5] = [10_000, 30_000, 60_000, 120_000, 180_000];
    fn rates_default() -> [Rate; PHASES] {
        [60_000, 1_000, 1_000, 60_000].map(|f| Rate { fixed_ms: f, follow: false, percent: PCT, interval_ms: MS })
    }

    fn table() -> Radio {
        parse(&table_text()).unwrap()
    }

    fn rr(fixed_ms: i64) -> RateRequest {
        RateRequest { fixed_ms, follow: false, percent: vec![75, 50, 35, 20], interval_ms: vec![10_000, 30_000, 60_000, 120_000, 180_000] }
    }

    fn asking(csid: i64, sf: i64, dbm: i64, tx_interval_ms: i64) -> Request {
        Request { csid, sf, dbm, rates: vec![rr(60_000), rr(1_000), rr(tx_interval_ms), rr(60_000)], logs: vec![1000, 50, 50, 1000], log_blind_ms: 200 }
    }
    fn asked(csid: u8, sf: u8, dbm: i8, rates: [Rate; PHASES]) -> Asked {
        Asked { csid, sf, dbm, rates, logs: LOGS, log_blind_ms: 200 }
    }

    #[test]
    fn the_radio_table_is_read_as_the_flight_repo_writes_it() {
        assert_eq!(table(), Radio { sf_min: 5, sf_max: 12, le: vec![10, 14, 15], hp: vec![14, 17, 20, 22], rates: rates_default(), logs: LOGS, log_blind_ms: 200 });
        // Whichever module comes first, and however it is spaced.
        let turned = format!(r#"{{"hp":{{"dbm":[{{"dbm":22,"lut":1}}]}},"spreadingFactor":{{"max":11,"min":6}},"le":{{"dbm":[{{"dbm":-2}},{{"dbm":10}}]}},{}}}"#, PHASES_JSON);
        assert_eq!(parse(&turned).unwrap(), Radio { sf_min: 6, sf_max: 11, le: vec![-2, 10], hp: vec![22], rates: rates_default(), logs: LOGS, log_blind_ms: 200 });

        let base = r#""spreadingFactor":{"min":5,"max":12},"le":{"dbm":[{"dbm":10}]},"hp":{"dbm":[{"dbm":14}]}"#;
        for bad in [
            String::new(),
            String::from("not json"),
            format!(r#"{{"le":{{"dbm":[{{"dbm":10}}]}},"hp":{{"dbm":[{{"dbm":14}}]}},{}}}"#, PHASES_JSON),
            format!(r#"{{"spreadingFactor":{{"min":5,"max":12}},"le":{{"dbm":[{{"dbm":10}}]}},{}}}"#, PHASES_JSON),
            format!(r#"{{"spreadingFactor":{{"min":5,"max":12}},"le":{{"dbm":[]}},"hp":{{"dbm":[{{"dbm":14}}]}},{}}}"#, PHASES_JSON),
            format!(r#"{{"spreadingFactor":{{"min":12,"max":5}},"le":{{"dbm":[{{"dbm":10}}]}},"hp":{{"dbm":[{{"dbm":14}}]}},{}}}"#, PHASES_JSON),
            // No phases, three phases, one misnamed, one with a table the firmware would refuse.
            format!("{{{}}}", base),
            format!(r#"{{{},"phases":[{{"name":"waiting","fixedMs":1,"follow":false,"percent":[75,50,35,20],"intervalMs":[1,2,3,4,5]}},{{"name":"rising","fixedMs":1,"follow":false,"percent":[75,50,35,20],"intervalMs":[1,2,3,4,5]}},{{"name":"falling","fixedMs":1,"follow":false,"percent":[75,50,35,20],"intervalMs":[1,2,3,4,5]}}]}}"#, base),
            format!("{{{},{}}}", base, PHASES_JSON.replace("\"landed\"", "\"down\"")),
            format!("{{{},{}}}", base, PHASES_JSON.replacen("[75, 50, 35, 20]", "[75, 80, 35, 20]", 1)),
            // A log gap missing, one too small, the blind one missing.
            format!("{{{},{}}}", base, PHASES_JSON.replacen(", \"logMs\": 50", "", 1)),
            format!("{{{},{}}}", base, PHASES_JSON.replacen("\"logMs\": 50", "\"logMs\": 10", 1)),
            format!("{{{},{}}}", base, PHASES_JSON.replacen(",\n  \"logBlindMs\": 200", "", 1)),
        ] {
            assert!(parse(&bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn the_table_this_build_carries_reads() {
        // Whatever fsw/firmware held at build time: no table, or one that reads.
        if crate::flash::CHIPSAT_RADIO.is_some() {
            let r = carried().unwrap();
            assert!(r.sf_min <= r.sf_max && !r.le.is_empty() && !r.hp.is_empty(), "{:?}", r);
            for rate in &r.rates {
                assert!(crate::settings::table_trouble(&rate.percent).is_none(), "{:?}", rate);
            }
        }
    }

    #[test]
    fn a_request_inside_the_table_becomes_what_goes_in_the_block() {
        let t = table();
        assert_eq!(t.check(&asking(12, 9, 20, 1000), "hp").unwrap(), asked(12, 9, 20, rates_default()));
        let mut zero = rates_default();
        zero[2].fixed_ms = 0;
        assert_eq!(t.check(&asking(1, 5, 10, 0), "le").unwrap(), asked(1, 5, 10, zero));
        let mut widest = rates_default();
        widest[2].fixed_ms = u32::MAX;
        assert_eq!(t.check(&asking(255, 12, 14, 4_294_967_295), "le").unwrap(), asked(255, 12, 14, widest));
        // The log gaps go through with the same floor and ceiling the firmware applies.
        let mut r = asking(12, 9, 14, 1000);
        r.logs = vec![20, 4_294_967_295, 21, 3_600_000];
        r.log_blind_ms = 20;
        let a = t.check(&r, "le").unwrap();
        assert_eq!((a.logs, a.log_blind_ms), ([20, u32::MAX, 21, 3_600_000], 20));
        for (what, logs, blind) in [
            ("rising", vec![1000, 19, 50, 1000], 200),
            ("landed", vec![1000, 50, 50, -1], 200),
            ("no altitude source", vec![1000, 50, 50, 1000], 9),
            ("no altitude source", vec![1000, 50, 50, 1000], 4_294_967_296),
            ("4 phases", vec![1000, 50, 50], 200),
        ] {
            let mut r = asking(12, 9, 14, 1000);
            r.logs = logs;
            r.log_blind_ms = blind;
            let e = t.check(&r, "le").unwrap_err();
            assert!(e.contains(what), "{}: {}", what, e);
        }
    }

    #[test]
    fn each_phase_has_its_own_table_checked_as_the_firmware_checks_it() {
        let t = table();
        let mut r = asking(12, 9, 14, 1000);
        r.rates[3] = RateRequest { fixed_ms: 300_000, follow: true, percent: vec![90, 60, 30, 5], interval_ms: vec![0, 1, 4_294_967_295, 7, 9] };
        let a = t.check(&r, "le").unwrap();
        assert_eq!(a.rates[3], Rate { fixed_ms: 300_000, follow: true, percent: [90, 60, 30, 5], interval_ms: [0, 1, u32::MAX, 7, 9] });
        assert_eq!(a.rates[2], rates_default()[2], "the other phases keep theirs");
        for (what, phase, bad) in [
            ("4 thresholds", 0, RateRequest { percent: vec![75, 50, 35], ..rr(1) }),
            ("5 intervals", 1, RateRequest { interval_ms: vec![1, 2, 3, 4, 5, 6], ..rr(1) }),
            ("1 to 99", 2, RateRequest { percent: vec![100, 50, 35, 20], ..rr(1) }),
            ("1 to 99", 3, RateRequest { percent: vec![75, 50, 35, 0], ..rr(1) }),
            ("have to fall", 0, RateRequest { percent: vec![75, 75, 35, 20], ..rr(1) }),
            ("have to fall", 1, RateRequest { percent: vec![20, 35, 50, 75], ..rr(1) }),
            ("4294967295", 2, RateRequest { interval_ms: vec![1, 2, 3, 4, 4_294_967_296], ..rr(1) }),
            ("4294967295", 3, rr(-1)),
        ] {
            let mut r = asking(12, 9, 14, 1000);
            r.rates[phase] = bad;
            let e = t.check(&r, "le").unwrap_err();
            assert!(e.contains(what) && e.contains(PHASE_NAMES[phase]), "{} in {}: {}", what, PHASE_NAMES[phase], e);
        }
        let mut three = asking(12, 9, 14, 1000);
        three.rates.pop();
        assert!(t.check(&three, "le").unwrap_err().contains("4 phases"));
    }

    #[test]
    fn a_board_id_is_1_to_255() {
        let t = table();
        for no in [0, 256, -1] {
            let e = t.check(&asking(no, 9, 14, 1000), "le").unwrap_err();
            assert!(e.contains(&no.to_string()) && e.contains("1 to 255"), "{}", e);
        }
    }

    #[test]
    fn a_spreading_factor_outside_the_table_is_refused_with_the_range() {
        let t = table();
        for no in [4, 13, 0, -7] {
            let e = t.check(&asking(12, no, 14, 1000), "le").unwrap_err();
            assert!(e.contains(&format!("SF{}", no)) && e.contains("5 to 12"), "{}", e);
        }
    }

    #[test]
    fn a_power_has_to_be_a_row_of_the_module_the_image_is_for() {
        let t = table();
        // 20 dBm is an HP row and not an LE one, and 10 the other way round.
        let e = t.check(&asking(12, 9, 20, 1000), "le").unwrap_err();
        assert!(e.contains("20 dBm") && e.contains("LE") && e.contains("10, 14, 15"), "{}", e);
        let e = t.check(&asking(12, 9, 10, 1000), "hp").unwrap_err();
        assert!(e.contains("HP") && e.contains("14, 17, 20, 22"), "{}", e);
        assert!(t.check(&asking(12, 9, 14, 1000), "le").is_ok());
        assert!(t.check(&asking(12, 9, 14, 1000), "hp").is_ok());
        assert!(t.check(&asking(12, 9, 14, 1000), "xl").unwrap_err().contains("le or hp"));
    }

    #[test]
    fn an_interval_has_to_fit_the_four_bytes_it_is_kept_in() {
        let t = table();
        for no in [-1, 4_294_967_296] {
            let e = t.check(&asking(12, 9, 14, no), "le").unwrap_err();
            assert!(e.contains("falling gap") && e.contains("4294967295"), "{}", e);
        }
    }
}
