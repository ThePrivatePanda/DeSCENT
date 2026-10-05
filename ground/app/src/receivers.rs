// What a receiver says it is, and what this computer calls it.
//
// A receiver's key, rx1 or rx2, is the order its port was found in, and that changes
// when a cable moves to another socket. The id in its header line does not: it is the
// last three bytes of the ESP32's MAC, printed at boot, every 30 s and on #GET. Names
// are kept against that id, in a file of their own for the reason the ChipSat list has
// one: the serial side writes descent-ground-boards.txt out whole from what it knows.
use crate::chips::{word, write_whole};
use std::path::{Path, PathBuf};

pub const FILE: &str = "descent-ground-receivers.txt";

// kNameMax in DescentRawReceiver.ino. A longer one is refused there, so it is cut here.
const NAME_CHARS: usize = 24;

// The last #DG,RX line a port sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub id: String,
    // Some when the line had a name= field, empty or not: firmware that prints one is
    // firmware that takes #SET,name. None is firmware from before names.
    pub name: Option<String>,
    // The firmware's version, which it prints from 1.2.1 on. Shown, and nothing more:
    // what a board can do is read from what its header carries, not from this.
    pub fw: Option<String>,
}

// What a version looks like, and short: digits and dots, starting with a digit, 16
// characters at most. Anything else after fw= is a damaged line.
fn a_version(text: &str) -> Option<String> {
    let fine = text.len() <= 16
        && text.starts_with(|c: char| c.is_ascii_digit())
        && text.chars().all(|c| c.is_ascii_digit() || c == '.');
    if fine { Some(text.to_string()) } else { None }
}

// Six hex characters, either case, kept uppercase as the receiver prints it.
pub fn an_id(text: &str) -> Option<String> {
    if text.len() == 6 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(text.to_ascii_uppercase())
    } else {
        None
    }
}

// #DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8[,name=<text>][,fw=1.2.1]
// A header whose id does not read is no header: half a line after a reset looks like one.
pub fn header(line: &str) -> Option<Header> {
    let fields = line.trim_end().strip_prefix("#DG,RX,")?;
    let mut id = None;
    let mut name = None;
    let mut fw = None;
    for field in fields.split(',') {
        match field.split_once('=') {
            Some(("id", v)) => id = an_id(v),
            Some(("name", v)) => name = Some(v.trim().to_string()),
            Some(("fw", v)) => fw = a_version(v),
            _ => {}
        }
    }
    Some(Header { id: id?, name, fw })
}

// A name as it can be kept here and sent to a board: one line, no commas, because a
// #SET line is fields with commas between them and the receiver reads it a line at a time.
// Plain ASCII only, which is what nameIsUsable in DescentRawReceiver.ino takes: a name
// kept here that the board turned down would be two names for one receiver.
pub fn a_name(text: &str) -> Result<String, String> {
    let one_line: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    let name = one_line.trim();
    if name.contains(',') {
        return Err(String::from(
            "a receiver's name cannot have a comma in it: it goes to the board in a line where commas separate the fields",
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(String::from(
            "a receiver's name cannot have a tab or any other control character in it: it goes to the board as one line of text",
        ));
    }
    if !name.is_ascii() {
        return Err(String::from(
            "a receiver's name has to be plain letters, digits and punctuation: the board takes nothing else",
        ));
    }
    Ok(name.chars().take(NAME_CHARS).collect::<String>().trim_end().to_string())
}

const HEADER: &str = "# What this computer calls each receiver, by the id it prints. Written by DeSCENT Ground.\n\
# name <id> <what you call it>\n";

// name <id> <the name, spaces and all>. A line is understood whole or not at all.
fn parse_line(line: &str) -> Option<(String, String)> {
    let (tag, rest) = word(line)?;
    if tag != "name" {
        return None;
    }
    let (id, name) = word(rest)?;
    let name = a_name(name).ok().filter(|n| !n.is_empty())?;
    Some((an_id(id)?, name))
}

pub struct Names {
    path: Option<PathBuf>,
    names: Vec<(String, String)>,   // id, name
    // Lines that are not ours to understand, written back as they were. See chips.rs.
    kept: Vec<String>,
}

impl Names {
    pub fn empty() -> Names {
        Names { path: None, names: Vec::new(), kept: Vec::new() }
    }

    // No file is an ordinary first run.
    pub fn load(dir: &Path) -> Names {
        let path = dir.join(FILE);
        let mut names: Vec<(String, String)> = Vec::new();
        let mut kept = Vec::new();
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            match parse_line(line) {
                // The same receiver twice is a file somebody edited. The first line stands.
                Some((id, _)) if names.iter().any(|(i, _)| *i == id) => kept.push(line.to_string()),
                Some(named) => names.push(named),
                None => kept.push(line.to_string()),
            }
        }
        Names { path: Some(path), names, kept }
    }

    fn text(&self) -> String {
        let mut rows: Vec<String> = self.names.iter().map(|(id, name)| format!("name {} {}", id, name)).collect();
        rows.sort();
        let mut text = String::from(HEADER);
        for row in rows.iter().chain(self.kept.iter()) {
            text.push_str(row);
            text.push('\n');
        }
        text
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.names.iter().find(|(i, _)| i == id).map(|(_, n)| n.as_str())
    }

    // An empty name clears it. Takes an id and a name that have already been checked.
    pub fn set(&mut self, id: &str, name: &str) -> Result<(), String> {
        self.names.retain(|(i, _)| i != id);
        if !name.is_empty() {
            self.names.push((id.to_string(), name.to_string()));
        }
        match &self.path {
            Some(path) => write_whole(path, &self.text()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-receivers-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_header_line_says_which_receiver_it_is() {
        let h = header("#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8").unwrap();
        assert_eq!(h, Header { id: "A4C1F3".into(), name: None, fw: None });
        // Newer firmware says what it is called, or that it is called nothing yet.
        let h = header("#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=Left one").unwrap();
        assert_eq!(h.name.as_deref(), Some("Left one"));
        let h = header("#DG,RX,v1,id=a4c1f3,name=,f=915.0").unwrap();
        assert_eq!(h, Header { id: "A4C1F3".into(), name: Some(String::new()), fw: None });

        for no in [
            "PKT,55,00FF,-50.0,13.75,-12",
            "HB,1,2,3",
            "#DG,FATAL,3",
            "#DG,RX,v1,f=915.0",
            "#DG,RX,v1,id=A4C1F,f=915.0",
            "#DG,RX,v1,id=A4C1F3A,f=915.0",
            "#DG,RX,v1,id=A4C1FZ,f=915.0",
            "#DG,RX",
            "x#DG,RX,v1,id=A4C1F3",
            "",
        ] {
            assert_eq!(header(no), None, "{:?}", no);
        }
    }

    #[test]
    fn a_header_says_which_firmware_it_runs_from_1_2_1_on() {
        let h = header("#DG,RX,v1,id=A4C1F3,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=Left one,fw=1.2.1").unwrap();
        assert_eq!(h, Header { id: "A4C1F3".into(), name: Some("Left one".into()), fw: Some("1.2.1".into()) });
        assert_eq!(header("#DG,RX,v1,id=A4C1F3,name=,fw=1.10.0\r").unwrap().fw.as_deref(), Some("1.10.0"));
        assert_eq!(header("#DG,RX,v1,id=A4C1F3,fw=2").unwrap().fw.as_deref(), Some("2"));
        assert_eq!(header("#DG,RX,v1,id=A4C1F3,fw=1234567.123456.1").unwrap().fw.as_deref(), Some("1234567.123456.1"));

        // Older firmware does not say, and that is not the same as saying nothing useful.
        assert_eq!(header("#DG,RX,v1,id=A4C1F3,f=915.0,name=Left one").unwrap().fw, None);

        // A line cut short or run into the next one. The id still stands.
        for garbage in ["fw=", "fw=1.2.1-beta", "fw=v1.2.1", "fw=1.2.1PKT", "fw=1 2", "fw=\u{fffd}", "fw=1234567.1234567.1", "fw=...", "fw=.1"] {
            let h = header(&format!("#DG,RX,v1,id=A4C1F3,name=x,{}", garbage)).unwrap();
            assert_eq!(h.fw, None, "{:?}", garbage);
            assert_eq!(h.id, "A4C1F3");
            assert_eq!(h.name.as_deref(), Some("x"));
        }
    }

    #[test]
    fn an_id_is_six_hex_characters_and_nothing_else() {
        assert_eq!(an_id("A4C1F3").as_deref(), Some("A4C1F3"));
        assert_eq!(an_id("a4c1f3").as_deref(), Some("A4C1F3"));
        for no in ["", "A4C1F", "A4C1F3A", "A4C1FG", " A4C1F3", "A4C1F3 ", "A4-1F3", "rx1"] {
            assert_eq!(an_id(no), None, "{:?}", no);
        }
    }

    #[test]
    fn a_name_is_one_line_with_no_commas_and_fits_a_label() {
        assert_eq!(a_name("  01 ").unwrap(), "01");
        assert_eq!(a_name("the \"left\" one \\ roof").unwrap(), "the \"left\" one \\ roof");
        assert_eq!(a_name("two\r\nlines").unwrap(), "twolines");
        assert_eq!(a_name("").unwrap(), "");
        assert_eq!(a_name(&"x".repeat(40)).unwrap().chars().count(), 24);
        assert_eq!(a_name("#01 = roof (north) ~").unwrap(), "#01 = roof (north) ~");
        // The receiver's nameIsUsable takes 0x20 to 0x7E and nothing else, so a name with
        // an accent would be kept here and turned down there.
        let e = a_name("toit n\u{ba} 2").unwrap_err();
        assert!(e.contains("plain"), "{}", e);
        assert!(a_name("caf\u{e9}").is_err());

        let e = a_name("left, roof").unwrap_err();
        assert!(e.contains("comma"), "{}", e);
        let e = a_name("left\troof").unwrap_err();
        assert!(e.contains("tab"), "{}", e);
        assert!(a_name("bell\u{7}").is_err());
    }

    #[test]
    fn names_survive_being_saved_and_read_back_spaces_quotes_and_all() {
        let dir = scratch("names");
        let mut n = Names::load(&dir);
        assert_eq!(n.get("A4C1F3"), None);
        assert!(!dir.join(FILE).exists(), "looking at the list must not write it");

        n.set("A4C1F3", "the \"left\" one \\ roof").unwrap();
        n.set("00B2C4", "01").unwrap();
        let again = Names::load(&dir);
        assert_eq!(again.get("A4C1F3"), Some("the \"left\" one \\ roof"));
        assert_eq!(again.get("00B2C4"), Some("01"));
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("name A4C1F3 the \"left\" one \\ roof\n"), "{}", text);

        // A new name replaces the old, an empty one clears it, and nothing is left beside the file.
        n.set("A4C1F3", "right").unwrap();
        n.set("00B2C4", "").unwrap();
        let again = Names::load(&dir);
        assert_eq!(again.get("A4C1F3"), Some("right"));
        assert_eq!(again.get("00B2C4"), None);
        let files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(files, vec![FILE.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_it_does_not_understand_is_written_back_as_it_was() {
        let dir = scratch("kept");
        std::fs::write(
            dir.join(FILE),
            "# an older header\n\
name A4C1F3 Left one\n\
colour A4C1F3 red\n\
name NOTHEX x\n\
name a4c1f3 the same board again\n\
\n\
name 00b2c4 02\n",
        )
        .unwrap();
        let mut n = Names::load(&dir);
        assert_eq!(n.get("A4C1F3"), Some("Left one"));
        assert_eq!(n.get("00B2C4"), Some("02"));

        n.set("00B2C4", "03").unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("colour A4C1F3 red\n"), "{}", text);
        assert!(text.contains("name NOTHEX x\n"), "{}", text);
        assert!(text.contains("name a4c1f3 the same board again\n"), "{}", text);
        assert!(text.contains("name 00B2C4 03\n"), "{}", text);
        assert!(!text.contains("an older header"), "{}", text);
        assert_eq!(Names::load(&dir).kept.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
