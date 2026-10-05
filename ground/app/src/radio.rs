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
use crate::json::int_field;
use crate::settings::Asked;

#[derive(Debug, Clone, PartialEq)]
pub struct Radio {
    pub sf_min: i64,
    pub sf_max: i64,
    pub le: Vec<i64>,   // the transmit powers each module has a row for, in dBm
    pub hp: Vec<i64>,
}

// What the page asked for, as it came. Numbers of any size, because saying "csid 256 is
// not a board id" needs the 256.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub csid: i64,
    pub sf: i64,
    pub dbm: i64,
    pub tx_interval_ms: i64,
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
    Ok(Radio { sf_min, sf_max, le: powers(text, "le")?, hp: powers(text, "hp")? })
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
        if !(0..=u32::MAX as i64).contains(&r.tx_interval_ms) {
            return Err(format!("an interval of {} ms does not fit: it has to be 0 to {}", r.tx_interval_ms, u32::MAX));
        }
        Ok(Asked { csid: r.csid as u8, sf: r.sf as u8, dbm: r.dbm as i8, tx_interval_ms: r.tx_interval_ms as u32 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // As the flight repo's tool writes it.
    const TABLE: &str = r#"{
  "layoutVersion": 1,
  "spreadingFactor": {"min": 5, "max": 12},
  "le": {
    "sleepsDuringTx": false,
    "dbm": [
      {"dbm": 10, "txparams": 13, "dutycycle": 1, "hpmax": 0, "devicesel": 1, "lut": 1},
      {"dbm": 14, "txparams": 14, "dutycycle": 4, "hpmax": 0, "devicesel": 1, "lut": 1},
      {"dbm": 15, "txparams": 14, "dutycycle": 6, "hpmax": 0, "devicesel": 1, "lut": 1}
    ]
  },
  "hp": {
    "sleepsDuringTx": true,
    "dbm": [
      {"dbm": 14, "txparams": 22, "dutycycle": 2, "hpmax": 2, "devicesel": 0, "lut": 1},
      {"dbm": 17, "txparams": 22, "dutycycle": 2, "hpmax": 3, "devicesel": 0, "lut": 1},
      {"dbm": 20, "txparams": 22, "dutycycle": 3, "hpmax": 5, "devicesel": 0, "lut": 1},
      {"dbm": 22, "txparams": 22, "dutycycle": 4, "hpmax": 7, "devicesel": 0, "lut": 1}
    ]
  }
}
"#;

    fn table() -> Radio {
        parse(TABLE).unwrap()
    }

    fn asking(csid: i64, sf: i64, dbm: i64, tx_interval_ms: i64) -> Request {
        Request { csid, sf, dbm, tx_interval_ms }
    }

    #[test]
    fn the_radio_table_is_read_as_the_flight_repo_writes_it() {
        assert_eq!(table(), Radio { sf_min: 5, sf_max: 12, le: vec![10, 14, 15], hp: vec![14, 17, 20, 22] });
        // Whichever module comes first, and however it is spaced.
        let turned = r#"{"hp":{"dbm":[{"dbm":22,"lut":1}]},"spreadingFactor":{"max":11,"min":6},"le":{"dbm":[{"dbm":-2},{"dbm":10}]}}"#;
        assert_eq!(parse(turned).unwrap(), Radio { sf_min: 6, sf_max: 11, le: vec![-2, 10], hp: vec![22] });

        for bad in [
            "",
            "not json",
            r#"{"le":{"dbm":[{"dbm":10}]},"hp":{"dbm":[{"dbm":14}]}}"#,
            r#"{"spreadingFactor":{"min":5,"max":12},"le":{"dbm":[{"dbm":10}]}}"#,
            r#"{"spreadingFactor":{"min":5,"max":12},"le":{"dbm":[]},"hp":{"dbm":[{"dbm":14}]}}"#,
            r#"{"spreadingFactor":{"min":12,"max":5},"le":{"dbm":[{"dbm":10}]},"hp":{"dbm":[{"dbm":14}]}}"#,
        ] {
            assert!(parse(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn the_table_this_build_carries_reads() {
        // Whatever chipsat/firmware held at build time: no table, or one that reads.
        if crate::flash::CHIPSAT_RADIO.is_some() {
            let r = carried().unwrap();
            assert!(r.sf_min <= r.sf_max && !r.le.is_empty() && !r.hp.is_empty(), "{:?}", r);
        }
    }

    #[test]
    fn a_request_inside_the_table_becomes_what_goes_in_the_block() {
        let t = table();
        assert_eq!(t.check(&asking(12, 9, 20, 1000), "hp").unwrap(), Asked { csid: 12, sf: 9, dbm: 20, tx_interval_ms: 1000 });
        assert_eq!(t.check(&asking(1, 5, 10, 0), "le").unwrap(), Asked { csid: 1, sf: 5, dbm: 10, tx_interval_ms: 0 });
        assert_eq!(
            t.check(&asking(255, 12, 14, 4_294_967_295), "le").unwrap(),
            Asked { csid: 255, sf: 12, dbm: 14, tx_interval_ms: u32::MAX }
        );
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
            assert!(e.contains("interval") && e.contains("4294967295"), "{}", e);
        }
    }
}
