// The ChipSats this laptop has had on the probe, and what the operator said about each.
//
// A ChipSat has no USB serial number to go by and the boards look alike, so the one
// thing that says which is which is the id ST programs into the chip. This keeps a name
// and a module against that id. It is a file of its own, not more lines in
// descent-ground-boards.txt: the serial side writes that one out whole from what it
// knows, and would drop these.
use crate::json::esc;
use crate::stm32::a_uid;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const FILE: &str = "descent-ground-chipsats.txt";
const NAME_CHARS: usize = 40;   // the same cut the page makes for a receiver's name

#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub uid: String,
    pub name: String,                     // empty until the operator gives one
    pub module: Option<&'static str>,     // "le" or "hp". Nothing on the board says which
    pub last_seen: u64,                   // epoch s
    pub firmware: Option<&'static str>,   // "dump", "flight" or "unknown", as of last_seen
    // Which module that flight image is built for. It is the image's module and not the
    // board's, which is the point of keeping it: the two can differ.
    pub firmware_module: Option<&'static str>,
}

pub struct Registry {
    path: Option<PathBuf>,
    chips: Vec<Chip>,
    // Lines that were in the file and are not ours to understand. Written back as they
    // were: the names in here were typed by a person and exist nowhere else, and a newer
    // build may keep things in this file that this one has not heard of.
    kept: Vec<String>,
}

pub type Shared = Arc<Mutex<Registry>>;

pub fn module_word(text: &str) -> Option<&'static str> {
    match text {
        "le" => Some("le"),
        "hp" => Some("hp"),
        _ => None,
    }
}

// What a board was last seen running, as one word: the kind, and for flight software
// the module the image is built for. One word because the file gives it one column, and
// the name that follows is everything to the end of the line.
pub fn firmware_words(text: &str) -> Option<(&'static str, Option<&'static str>)> {
    match text {
        "dump" => Some(("dump", None)),
        "unknown" => Some(("unknown", None)),
        "flight" => Some(("flight", None)),
        "flight-le" => Some(("flight", Some("le"))),
        "flight-hp" => Some(("flight", Some("hp"))),
        _ => None,
    }
}

// The file is one board per line and the page reads names back out of hand-rolled
// JSON, so a name cannot carry a line end, a quote or a backslash.
pub fn clean_name(name: &str) -> String {
    let kept: String = name.chars().filter(|c| *c != '"' && *c != '\\' && !c.is_control()).collect();
    kept.trim().chars().take(NAME_CHARS).collect::<String>().trim_end().to_string()
}

const HEADER: &str = "# ChipSats this laptop has had on the probe, one per line. Written by DeSCENT Ground.\n\
# chip <id> <le|hp|-> <last seen, unix seconds> <dump|flight-le|flight-hp|unknown|-> <what you call it>\n";

// The next word of a line and what follows it.
pub fn word(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some(rest.split_once(char::is_whitespace).unwrap_or((rest, "")))
}

// A line is understood whole or not at all. One with a module this build has not heard
// of is kept as it was instead of being read as a board with no module.
fn parse_line(line: &str) -> Option<Chip> {
    let (tag, rest) = word(line)?;
    if tag != "chip" {
        return None;
    }
    let (uid, rest) = word(rest)?;
    let (module, rest) = word(rest)?;
    let (seen, rest) = word(rest)?;
    let (firmware, name) = word(rest)?;
    let module = match module {
        "-" => None,
        other => Some(module_word(other)?),
    };
    let (firmware, firmware_module) = match firmware {
        "-" => (None, None),
        other => {
            let (kind, of) = firmware_words(other)?;
            (Some(kind), of)
        }
    };
    Some(Chip { uid: a_uid(uid)?, name: clean_name(name), module, last_seen: seen.parse().ok()?, firmware, firmware_module })
}

fn parse(text: &str) -> (Vec<Chip>, Vec<String>) {
    let mut chips: Vec<Chip> = Vec::new();
    let mut kept = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        match parse_line(line) {
            // The same board twice is a file somebody edited. The first line stands.
            Some(chip) if chips.iter().any(|c| c.uid == chip.uid) => kept.push(line.to_string()),
            Some(chip) => chips.push(chip),
            None => kept.push(line.to_string()),
        }
    }
    (chips, kept)
}

// Written whole, to one side and then moved over the old one, so a crash part way
// through leaves the old list and not half of the new.
pub fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    let aside = path.with_extension("txt.part");
    std::fs::write(&aside, text)
        .and_then(|()| std::fs::rename(&aside, path))
        .map_err(|e| format!("could not write {}: {}", path.display(), e))
}

fn not_a_uid(text: &str) -> String {
    format!("a board id is 24 hex characters, and {:?} is not", text)
}

impl Registry {
    #[cfg(test)]
    pub fn empty() -> Registry {
        Registry { path: None, chips: Vec::new(), kept: Vec::new() }
    }

    // No file is an ordinary first run.
    pub fn load(dir: &Path) -> Registry {
        let path = dir.join(FILE);
        let (chips, kept) = parse(&std::fs::read_to_string(&path).unwrap_or_default());
        Registry { path: Some(path), chips, kept }
    }

    fn text(&self) -> String {
        let mut rows: Vec<String> = self
            .chips
            .iter()
            .map(|c| {
                let firmware = match (c.firmware, c.firmware_module) {
                    (Some("flight"), Some("le")) => "flight-le",
                    (Some("flight"), Some("hp")) => "flight-hp",
                    (Some(word), _) => word,
                    (None, _) => "-",
                };
                let row = format!("chip {} {} {} {} {}", c.uid, c.module.unwrap_or("-"), c.last_seen, firmware, c.name);
                row.trim_end().to_string()
            })
            .collect();
        rows.sort();
        let mut text = String::from(HEADER);
        for row in rows.iter().chain(self.kept.iter()) {
            text.push_str(row);
            text.push('\n');
        }
        text
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        write_whole(path, &self.text())
    }

    fn known(&mut self, uid: &str) -> Result<&mut Chip, String> {
        let uid = a_uid(uid).ok_or_else(|| not_a_uid(uid))?;
        self.chips
            .iter_mut()
            .find(|c| c.uid == uid)
            .ok_or_else(|| format!("board {} has not been read over the probe yet, so there is nothing to change", uid))
    }

    pub fn get(&self, uid: &str) -> Option<&Chip> {
        self.chips.iter().find(|c| c.uid == uid)
    }

    // The whole id of the one board whose id ends this way. A saved image's name only
    // has room for the end.
    pub fn whole_uid(&self, tail: &str) -> Option<String> {
        if tail.is_empty() {
            return None;
        }
        let mut ending = self.chips.iter().filter(|c| c.uid.ends_with(tail));
        match (ending.next(), ending.next()) {
            (Some(only), None) => Some(only.uid.clone()),
            _ => None,
        }
    }

    // A board that was just read over the probe, and what was on it, as firmware_words
    // takes it.
    pub fn saw(&mut self, uid: &str, firmware: &'static str, now_s: u64) -> Result<(), String> {
        let uid = a_uid(uid).ok_or_else(|| not_a_uid(uid))?;
        let (firmware, firmware_module) = match firmware_words(firmware) {
            Some((kind, of)) => (Some(kind), of),
            None => (None, None),
        };
        match self.chips.iter_mut().find(|c| c.uid == uid) {
            Some(chip) => {
                chip.last_seen = now_s;
                chip.firmware = firmware;
                chip.firmware_module = firmware_module;
            }
            None => self.chips.push(Chip { uid, name: String::new(), module: None, last_seen: now_s, firmware, firmware_module }),
        }
        self.save()
    }

    // An empty name clears it.
    pub fn set_name(&mut self, uid: &str, name: &str) -> Result<(), String> {
        self.known(uid)?.name = clean_name(name);
        self.save()
    }

    pub fn set_module(&mut self, uid: &str, module: &str) -> Result<(), String> {
        let module = module_word(module).ok_or_else(|| format!("a module is le or hp, and {:?} is neither", module))?;
        self.known(uid)?.module = Some(module);
        self.save()
    }

    // Drops what is kept about a board: its name, its module and when it was seen.
    // Nothing is done to the board, and one that is read again is a new board with
    // neither, which is better than a board that stays hidden.
    pub fn forget(&mut self, uid: &str) -> Result<(), String> {
        let uid = a_uid(uid).ok_or_else(|| not_a_uid(uid))?;
        let before = self.chips.len();
        self.chips.retain(|c| c.uid != uid);
        if self.chips.len() == before {
            return Err(format!("board {} is not in the list", uid));
        }
        self.save()
    }

    // Most recently seen first: the board on the probe now is the one being asked about.
    pub fn json(&self) -> String {
        let mut chips: Vec<&Chip> = self.chips.iter().collect();
        chips.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.uid.cmp(&b.uid)));
        let word = |w: Option<&str>| match w {
            Some(w) => format!("\"{}\"", w),
            None => String::from("null"),
        };
        let items: Vec<String> = chips
            .iter()
            .map(|c| {
                format!(
                    "{{\"uid\":\"{}\",\"name\":\"{}\",\"module\":{},\"lastSeen\":{},\"firmware\":{},\"firmwareModule\":{}}}",
                    c.uid,
                    esc(&c.name),
                    word(c.module),
                    c.last_seen,
                    word(c.firmware),
                    word(c.firmware_module)
                )
            })
            .collect();
        format!("{{\"chips\":[{}]}}", items.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0038002A5553500920393256";
    const B: &str = "0038002A5553500920393301";

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-chips-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_first_run_has_no_file_and_makes_one_when_a_board_is_seen() {
        let dir = scratch("first");
        let mut r = Registry::load(&dir);
        assert_eq!(r.json(), "{\"chips\":[]}");
        assert!(!dir.join(FILE).exists(), "looking at the list must not write it");

        r.saw(A, "unknown", 1_790_000_000).unwrap();
        assert_eq!(
            r.json(),
            format!("{{\"chips\":[{{\"uid\":\"{}\",\"name\":\"\",\"module\":null,\"lastSeen\":1790000000,\"firmware\":\"unknown\",\"firmwareModule\":null}}]}}", A)
        );
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains(&format!("chip {} - 1790000000 unknown\n", A)), "{}", text);
        // Nothing is left beside it.
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec![FILE.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_with_spaces_and_a_module_survive_being_saved_and_read_back() {
        let dir = scratch("names");
        let mut r = Registry::load(&dir);
        r.saw(A, "dump", 1_790_000_000).unwrap();
        r.saw(B, "unknown", 1_790_000_500).unwrap();
        r.set_name(A, "  Flight spare 2 ").unwrap();
        r.set_module(A, "hp").unwrap();

        let again = Registry::load(&dir);
        assert_eq!(
            again.get(A),
            Some(&Chip {
                uid: A.into(),
                name: "Flight spare 2".into(),
                module: Some("hp"),
                last_seen: 1_790_000_000,
                firmware: Some("dump"),
                firmware_module: None,
            })
        );
        assert_eq!(again.get(B).unwrap().name, "");
        assert_eq!(again.get(B).unwrap().module, None);
        // And the file does not change by being read and written again.
        let before = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let mut again = again;
        again.set_name(B, "").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), before);

        // An empty name clears it, and nothing brings it back.
        r.set_name(A, "").unwrap();
        assert_eq!(Registry::load(&dir).get(A).unwrap().name, "");
        assert_eq!(Registry::load(&dir).get(A).unwrap().module, Some("hp"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_forgotten_board_goes_from_the_file_and_comes_back_as_a_new_one() {
        let dir = scratch("forget");
        std::fs::write(dir.join(FILE), "colour red\n").unwrap();
        let mut r = Registry::load(&dir);
        r.saw(A, "dump", 1_790_000_000).unwrap();
        r.saw(B, "unknown", 1_790_000_500).unwrap();
        r.set_name(A, "Flight spare 2").unwrap();
        r.set_module(A, "hp").unwrap();

        r.forget(&A.to_lowercase()).unwrap();
        assert_eq!(r.get(A), None);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!text.contains(A), "{}", text);
        // The other board and a line that is not ours are still there.
        assert!(text.contains(&format!("chip {} - 1790000500 unknown\n", B)), "{}", text);
        assert!(text.contains("colour red\n"), "{}", text);
        assert_eq!(Registry::load(&dir).get(A), None);

        // Read again, it is a board nobody has named or given a module.
        r.saw(A, "flight-le", 1_790_001_000).unwrap();
        let back = Registry::load(&dir);
        assert_eq!((back.get(A).unwrap().name.as_str(), back.get(A).unwrap().module), ("", None));

        assert!(r.forget("0038002A5553500920393999").unwrap_err().contains("not in the list"));
        assert!(r.forget("nonsense").unwrap_err().contains("24 hex"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_is_most_recently_seen_first() {
        let mut r = Registry::empty();
        r.saw(A, "dump", 100).unwrap();
        r.saw(B, "unknown", 200).unwrap();
        let j = r.json();
        assert!(j.find(B).unwrap() < j.find(A).unwrap(), "{}", j);
        // Seeing a board again moves it up and says what is on it now.
        r.saw(A, "unknown", 300).unwrap();
        let j = r.json();
        assert!(j.find(A).unwrap() < j.find(B).unwrap(), "{}", j);
        assert!(j.contains(&format!("\"uid\":\"{}\",\"name\":\"\",\"module\":null,\"lastSeen\":300,\"firmware\":\"unknown\"", A)), "{}", j);
        assert_eq!(r.chips.len(), 2);
    }

    #[test]
    fn a_line_it_does_not_understand_is_written_back_as_it_was() {
        let dir = scratch("kept");
        std::fs::write(
            dir.join(FILE),
            format!(
                "# an older header\n\
chip {a} le 1790000000 dump Left one\n\
settings {a} csid=7 sf=9\n\
chip not-a-uid le 1 dump x\n\
chip {b} xp 1 dump a module nobody has heard of\n\
\n\
chip {b} - 1790000500 -\n",
                a = A,
                b = B
            ),
        )
        .unwrap();
        let mut r = Registry::load(&dir);
        assert_eq!(r.get(A).unwrap().name, "Left one");
        assert_eq!(r.get(A).unwrap().module, Some("le"));
        assert_eq!(r.get(B).unwrap().last_seen, 1_790_000_500);
        assert_eq!(r.get(B).unwrap().firmware, None);
        assert_eq!(r.chips.len(), 2);

        r.set_name(B, "Right one").unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains(&format!("settings {} csid=7 sf=9\n", A)), "{}", text);
        assert!(text.contains("chip not-a-uid le 1 dump x\n"), "{}", text);
        assert!(text.contains(&format!("chip {} xp 1 dump a module nobody has heard of\n", B)), "{}", text);
        assert!(text.contains(&format!("chip {} - 1790000500 - Right one\n", B)), "{}", text);
        // The header is ours and is written fresh, so an old one does not pile up.
        assert!(!text.contains("an older header"), "{}", text);
        assert_eq!(text.lines().filter(|l| l.starts_with('#')).count(), 2, "{}", text);

        let again = Registry::load(&dir);
        assert_eq!(again.kept.len(), 3);
        assert_eq!(again.get(B).unwrap().name, "Right one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_cannot_carry_what_would_break_the_file_or_the_json() {
        assert_eq!(clean_name("  a \"quoted\" \\name\r\n second line\t"), "a quoted name second line");
        assert_eq!(clean_name(&"x".repeat(60)).chars().count(), 40);
        assert_eq!(clean_name("Vol d'essai n\u{ba} 2 \u{e9}t\u{e9}"), "Vol d'essai n\u{ba} 2 \u{e9}t\u{e9}");

        let mut r = Registry::empty();
        r.saw(A, "dump", 1).unwrap();
        r.set_name(A, "two\nlines \"here\"").unwrap();
        assert_eq!(r.get(A).unwrap().name, "twolines here");
        assert!(r.json().contains("\"name\":\"twolines here\""));
    }

    #[test]
    fn a_board_has_to_have_been_read_before_it_can_be_named() {
        let mut r = Registry::empty();
        r.saw(A, "dump", 1).unwrap();
        assert!(r.set_name(B, "x").unwrap_err().contains("has not been read"));
        assert!(r.set_module(B, "le").unwrap_err().contains("has not been read"));
        assert!(r.set_name("nope", "x").unwrap_err().contains("24"));
        // Whatever case the id arrives in, it is the same board.
        r.set_name(&A.to_lowercase(), "lower").unwrap();
        assert_eq!(r.get(A).unwrap().name, "lower");
        assert!(r.saw("12", "dump", 1).unwrap_err().contains("24"));
    }

    #[test]
    fn the_end_of_an_id_finds_the_board_when_only_one_ends_that_way() {
        let mut r = Registry::empty();
        r.saw(A, "dump", 1).unwrap();
        r.saw(B, "dump", 1).unwrap();
        assert_eq!(r.whole_uid("20393256").as_deref(), Some(A));
        assert_eq!(r.whole_uid("20393301").as_deref(), Some(B));
        assert_eq!(r.whole_uid("12345678"), None);
        assert_eq!(r.whole_uid(""), None);
        r.saw("FFFFFFFFFFFFFFFF20393256", "dump", 1).unwrap();
        assert_eq!(r.whole_uid("20393256"), None);
    }

    #[test]
    fn a_board_last_seen_on_flight_software_says_which_module_the_image_was_for() {
        let dir = scratch("flight");
        let mut r = Registry::load(&dir);
        r.saw(A, "flight-le", 1_790_000_000).unwrap();
        r.saw(B, "flight-hp", 1_790_000_500).unwrap();
        // The board is marked HP and the image on it is the LE one: both are kept, so
        // the page can say they disagree.
        r.set_module(A, "hp").unwrap();
        r.set_name(A, "Flight spare 2").unwrap();

        let again = Registry::load(&dir);
        let a = again.get(A).unwrap();
        assert_eq!((a.firmware, a.firmware_module, a.module), (Some("flight"), Some("le"), Some("hp")));
        assert_eq!(a.name, "Flight spare 2");
        let b = again.get(B).unwrap();
        assert_eq!((b.firmware, b.firmware_module, b.module), (Some("flight"), Some("hp"), None));
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains(&format!("chip {} hp 1790000000 flight-le Flight spare 2\n", A)), "{}", text);
        assert!(
            again.json().contains(&format!("{{\"uid\":\"{}\",\"name\":\"Flight spare 2\",\"module\":\"hp\",\"lastSeen\":1790000000,\"firmware\":\"flight\",\"firmwareModule\":\"le\"}}", A)),
            "{}",
            again.json()
        );

        // Seen again on something else, the module of the image goes with the image.
        r.saw(A, "dump", 1_790_001_000).unwrap();
        let a = Registry::load(&dir);
        let a = a.get(A).unwrap();
        assert_eq!((a.firmware, a.firmware_module, a.module), (Some("dump"), None, Some("hp")));
        r.saw(A, "flight-hp", 1_790_002_000).unwrap();
        r.saw(A, "unknown", 1_790_003_000).unwrap();
        assert!(r.json().contains("\"firmware\":\"unknown\",\"firmwareModule\":null}"), "{}", r.json());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_list_written_before_flight_software_could_be_told_still_loads() {
        let dir = scratch("before-flight");
        std::fs::write(
            dir.join(FILE),
            format!("chip {} le 1790000000 dump Left one\nchip {} - 1790000500 unknown\n", A, B),
        )
        .unwrap();
        let r = Registry::load(&dir);
        let a = r.get(A).unwrap();
        assert_eq!((a.firmware, a.firmware_module, a.module, a.name.as_str()), (Some("dump"), None, Some("le"), "Left one"));
        assert_eq!(r.get(B).unwrap().firmware, Some("unknown"));
        assert_eq!(r.kept.len(), 0);
        // A flight image whose module was not written down is still flight software.
        std::fs::write(dir.join(FILE), format!("chip {} le 1790000000 flight Left one\n", A)).unwrap();
        let r = Registry::load(&dir);
        assert_eq!((r.get(A).unwrap().firmware, r.get(A).unwrap().firmware_module), (Some("flight"), None));
        // A firmware word nobody has heard of keeps its line, as before.
        std::fs::write(dir.join(FILE), format!("chip {} le 1790000000 flight-xp Left one\n", A)).unwrap();
        let r = Registry::load(&dir);
        assert!(r.get(A).is_none());
        assert_eq!(r.kept.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_module_is_le_or_hp_and_nothing_else() {
        let mut r = Registry::empty();
        r.saw(A, "dump", 1).unwrap();
        for no in ["", "LE", "lp", "1", "hp "] {
            let e = r.set_module(A, no).unwrap_err();
            assert!(e.contains("le or hp"), "{:?}: {}", no, e);
        }
        assert_eq!(r.get(A).unwrap().module, None);
        r.set_module(A, "le").unwrap();
        assert_eq!(r.get(A).unwrap().module, Some("le"));
        assert!(r.json().contains("\"module\":\"le\""));
    }
}
