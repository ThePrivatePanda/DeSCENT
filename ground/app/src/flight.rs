// The flight images this build carries, and what the flight repo says they are.
//
// There is one image per module, because the PA settings are compiled in: an LE image
// on an HP board transmits almost nothing and still reports success, which has already
// cost a day. So an image is only ever offered for the module flight.json says it was
// built for, and a copy that is not the file flight.json describes is not offered at all.
use crate::json::{bool_field, esc, int_field, str_field};
use std::sync::OnceLock;

// A file build.rs looked for: its name, its bytes if it was there, and their sha256.
pub type Carried = (&'static str, Option<&'static [u8]>, &'static str);

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub file: String,
    pub module: &'static str,
    pub bytes: &'static [u8],
    pub sha256: String,   // of the bytes in this build
    pub listed: String,   // what flight.json says it should be
}

// What every image in flight.json was built with.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    pub sf: i64,
    pub dbm: i64,
    pub csid: i64,
    pub settings_block: bool,
}

pub struct Flight {
    pub build: Option<Build>,
    pub images: Vec<Image>,
    pub trouble: Vec<String>,   // said once at startup
}

struct Listed {
    file: String,
    module: &'static str,
    sha256: String,
}

// flight.json with the crate's own field readers. They take the first field of a name
// they find, so the list of images is cut out and read an object at a time, and what is
// left is the build.
fn parse(text: &str) -> Result<(Build, Vec<Listed>), String> {
    let at = text.find("\"images\"").ok_or("it has no list of images")?;
    let open = at + text[at..].find('[').ok_or("its list of images does not open")?;
    let close = open + text[open..].find(']').ok_or("its list of images does not close")?;
    let top = format!("{}{}", &text[..at], &text[close + 1..]);
    let number = |name: &str| int_field(&top, name).ok_or_else(|| format!("it does not say {}", name));
    let build = Build {
        sf: number("sf")?,
        dbm: number("dbm")?,
        csid: number("csid")?,
        settings_block: bool_field(&top, "settingsBlock").ok_or("it does not say settingsBlock")?,
    };

    let mut listed = Vec::new();
    for object in text[open + 1..close].split('}').filter(|o| o.contains('{')) {
        let field = |name: &str| str_field(object, name).ok_or_else(|| format!("an image in it has no {}", name));
        let file = field("file")?;
        let module = field("module")?;
        let module = crate::chips::module_word(&module)
            .ok_or_else(|| format!("it says {} is built for {:?}, which is neither le nor hp", file, module))?;
        let sha256 = field("sha256")?;
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("its sha256 for {} is not one", file));
        }
        listed.push(Listed { file, module, sha256 });
    }
    if listed.is_empty() {
        return Err(String::from("it lists no images"));
    }
    Ok((build, listed))
}

impl Image {
    fn stale(&self) -> bool {
        !self.sha256.eq_ignore_ascii_case(&self.listed)
    }

    pub fn id(&self) -> &str {
        &self.sha256[..self.sha256.len().min(16)]
    }
}

impl Flight {
    // Without a flight.json that reads, nothing is carried: an image nobody can say the
    // module of is the one that must not be written.
    pub fn from(json: Option<&str>, files: &[Carried]) -> Flight {
        let none = |trouble: Vec<String>| Flight { build: None, images: Vec::new(), trouble };
        let Some(json) = json else {
            return match files.iter().find(|(_, bytes, _)| bytes.is_some()) {
                Some((file, _, _)) => none(vec![format!(
                    "{} is in this build with no flight.json to say what it is built for, so no flight image is offered",
                    file
                )]),
                None => none(Vec::new()),
            };
        };
        let (build, listed) = match parse(json) {
            Ok(read) => read,
            Err(e) => return none(vec![format!("flight.json in this build does not read ({}), so no flight image is offered", e)]),
        };

        let mut images = Vec::new();
        let mut trouble = Vec::new();
        for l in listed {
            match files.iter().find(|(file, _, _)| *file == l.file) {
                Some((_, Some(bytes), sha256)) => {
                    let image = Image { file: l.file, module: l.module, bytes, sha256: sha256.to_string(), listed: l.sha256 };
                    if image.stale() {
                        trouble.push(format!(
                            "{} in this build is not the file flight.json describes, so it is not offered. Copy both from the flight repo again and rebuild",
                            image.file
                        ));
                    }
                    images.push(image);
                }
                _ => trouble.push(format!("flight.json lists {}, which is not in this build", l.file)),
            }
        }
        Flight { build: Some(build), images, trouble }
    }

    // The image for a board, by the module the operator said it has. Nothing here can be
    // forced: there is no reading the module off the board, so a guess is all a force
    // would be.
    pub fn choose(&self, module: Option<&str>) -> Result<&Image, String> {
        if self.images.is_empty() {
            return Err(String::from("this build carries no flight image"));
        }
        let Some(module) = module else {
            return Err(String::from(
                "this board has not been marked LE or HP. The flight images are built for one or the other, and the wrong one transmits almost nothing while reporting success, so say which module the board has first",
            ));
        };
        let image = self
            .images
            .iter()
            .find(|i| i.module == module)
            .ok_or_else(|| format!("this build carries no flight image for the {} module", module.to_uppercase()))?;
        if image.stale() {
            return Err(format!(
                "the {} flight image in this build is not the one its flight.json describes (it is {}, flight.json says {}), so it is not written. Copy both from the flight repo again and rebuild",
                module.to_uppercase(),
                image.id(),
                &image.listed[..16]
            ));
        }
        Ok(image)
    }

    // The images that would be written if asked for: carried, and the files flight.json
    // describes.
    pub fn usable(&self) -> Vec<&Image> {
        self.images.iter().filter(|i| !i.stale()).collect()
    }

    // As /api/health lists them. An image that would be refused is not listed, so the
    // page does not offer it.
    pub fn images_json(&self) -> String {
        let items: Vec<String> = self
            .images
            .iter()
            .filter(|i| !i.stale())
            .map(|i| format!("{{\"id\":\"{}\",\"module\":\"{}\",\"bytes\":{}}}", esc(i.id()), i.module, i.bytes.len()))
            .collect();
        format!("[{}]", items.join(","))
    }

    // "LE f84691175ddf2641 and HP c2b044e470464918", for the line printed at startup.
    pub fn offered(&self) -> String {
        let each: Vec<String> = self
            .images
            .iter()
            .filter(|i| !i.stale())
            .map(|i| format!("{} {}", i.module.to_uppercase(), i.id()))
            .collect();
        each.join(" and ")
    }

    pub fn build_json(&self) -> String {
        match &self.build {
            Some(b) => format!(
                "{{\"sf\":{},\"dbm\":{},\"csid\":{},\"settingsBlock\":{}}}",
                b.sf, b.dbm, b.csid, b.settings_block
            ),
            None => String::from("null"),
        }
    }
}

pub fn carried() -> &'static Flight {
    static CARRIED: OnceLock<Flight> = OnceLock::new();
    CARRIED.get_or_init(|| {
        Flight::from(
            crate::flash::CHIPSAT_FLIGHT_JSON,
            &[
                ("flight-le.bin", crate::flash::CHIPSAT_FLIGHT_LE, crate::flash::CHIPSAT_FLIGHT_LE_SHA256),
                ("flight-hp.bin", crate::flash::CHIPSAT_FLIGHT_HP, crate::flash::CHIPSAT_FLIGHT_HP_SHA256),
            ],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LE: &[u8] = b"an image built for the LE module";
    const HP: &[u8] = b"an image built for the HP module, a little longer";
    const LE_SHA: &str = "f84691175ddf2641a8520e55932f690602f51f65872cb149320297218b0264fb";
    const HP_SHA: &str = "c2b044e4704649181ff3c3a9e3d4bddf3138d6719814eb28761fe0df886fe1b9";

    // The shape the flight repo's tool writes, line breaks and all.
    fn json(le: &str, hp: &str) -> String {
        format!(
            "{{\n  \"settingsBlock\": false,\n  \"sf\": 7,\n  \"dbm\": 14,\n  \"csid\": 255,\n  \"images\": [\n    \
{{\"file\": \"flight-le.bin\", \"module\": \"le\", \"bytes\": 63396, \"sha256\": \"{}\"}},\n    \
{{\"file\": \"flight-hp.bin\", \"module\": \"hp\", \"bytes\": 65064, \"sha256\": \"{}\"}}\n  ]\n}}\n",
            le, hp
        )
    }

    fn both() -> Flight {
        Flight::from(
            Some(&json(LE_SHA, HP_SHA)),
            &[("flight-le.bin", Some(LE), LE_SHA), ("flight-hp.bin", Some(HP), HP_SHA)],
        )
    }

    #[test]
    fn the_image_written_is_the_one_built_for_the_boards_module() {
        let f = both();
        assert!(f.trouble.is_empty(), "{:?}", f.trouble);
        assert_eq!(f.choose(Some("le")).unwrap().bytes, LE);
        assert_eq!(f.choose(Some("hp")).unwrap().bytes, HP);
        assert_eq!(f.choose(Some("hp")).unwrap().id(), "c2b044e470464918");
    }

    #[test]
    fn a_board_whose_module_was_never_set_gets_no_flight_image() {
        let e = both().choose(None).unwrap_err();
        assert!(e.contains("LE or HP"), "{}", e);
        assert!(e.contains("transmits almost nothing"), "the reason has to be in it: {}", e);
        assert!(!e.contains("force"), "{}", e);
    }

    #[test]
    fn a_module_this_build_has_no_image_for_is_refused() {
        let only_le = Flight::from(
            Some(&json(LE_SHA, HP_SHA)),
            &[("flight-le.bin", Some(LE), LE_SHA), ("flight-hp.bin", None, "")],
        );
        assert_eq!(only_le.choose(Some("le")).unwrap().bytes, LE);
        let e = only_le.choose(Some("hp")).unwrap_err();
        assert!(e.contains("no flight image for the HP module"), "{}", e);
        assert!(only_le.trouble.iter().any(|t| t.contains("flight-hp.bin")), "{:?}", only_le.trouble);

        let none = Flight::from(None, &[("flight-le.bin", None, ""), ("flight-hp.bin", None, "")]);
        assert_eq!(none.choose(Some("le")).unwrap_err(), "this build carries no flight image");
        assert_eq!(none.choose(None).unwrap_err(), "this build carries no flight image");
        assert!(none.trouble.is_empty(), "a build without flight images is an ordinary one");
    }

    #[test]
    fn an_image_that_is_not_the_one_flight_json_describes_is_refused() {
        // A newer flight.json beside the image from the build before.
        let newer = "0000000000000000000000000000000000000000000000000000000000000001";
        let f = Flight::from(
            Some(&json(newer, HP_SHA)),
            &[("flight-le.bin", Some(LE), LE_SHA), ("flight-hp.bin", Some(HP), HP_SHA)],
        );
        let e = f.choose(Some("le")).unwrap_err();
        assert!(e.contains("f84691175ddf2641") && e.contains("0000000000000000"), "{}", e);
        assert!(e.contains("not written"), "{}", e);
        assert_eq!(f.choose(Some("hp")).unwrap().bytes, HP);
        // And the page is not offered what would be refused.
        assert_eq!(f.images_json(), "[{\"id\":\"c2b044e470464918\",\"module\":\"hp\",\"bytes\":49}]");
        assert!(f.trouble.iter().any(|t| t.contains("flight-le.bin")), "{:?}", f.trouble);
        // Hex is hex whichever case the tool wrote it in.
        let upper = Flight::from(
            Some(&json(&LE_SHA.to_uppercase(), HP_SHA)),
            &[("flight-le.bin", Some(LE), LE_SHA), ("flight-hp.bin", Some(HP), HP_SHA)],
        );
        assert!(upper.choose(Some("le")).is_ok());
    }

    #[test]
    fn health_lists_each_image_with_its_module_and_what_they_were_built_with() {
        let f = both();
        assert_eq!(
            f.images_json(),
            "[{\"id\":\"f84691175ddf2641\",\"module\":\"le\",\"bytes\":32},{\"id\":\"c2b044e470464918\",\"module\":\"hp\",\"bytes\":49}]"
        );
        assert_eq!(f.build_json(), "{\"sf\":7,\"dbm\":14,\"csid\":255,\"settingsBlock\":false}");

        let none = Flight::from(None, &[("flight-le.bin", None, ""), ("flight-hp.bin", None, "")]);
        assert_eq!(none.images_json(), "[]");
        assert_eq!(none.build_json(), "null");
    }

    #[test]
    fn a_flight_json_that_does_not_read_means_no_image_is_carried() {
        let files: [Carried; 2] = [("flight-le.bin", Some(LE), LE_SHA), ("flight-hp.bin", Some(HP), HP_SHA)];
        for bad in [
            "",
            "not json",
            "{\"settingsBlock\": false, \"sf\": 7, \"dbm\": 14, \"csid\": 255}",
            "{\"sf\": 7, \"dbm\": 14, \"csid\": 255, \"images\": []}",
            &json(LE_SHA, HP_SHA).replace("\"module\": \"hp\"", "\"module\": \"xp\""),
            &json(LE_SHA, HP_SHA).replace("\"sf\": 7,", ""),
            &json("", HP_SHA),
        ] {
            let f = Flight::from(Some(bad), &files);
            assert_eq!(f.images_json(), "[]", "{}", bad);
            assert_eq!(f.build_json(), "null", "{}", bad);
            assert_eq!(f.choose(Some("le")).unwrap_err(), "this build carries no flight image", "{}", bad);
            assert!(f.trouble.iter().any(|t| t.contains("flight.json")), "{:?}", f.trouble);
        }
        // Images with no flight.json beside them are as good as absent, and said to be.
        let f = Flight::from(None, &files);
        assert_eq!(f.images_json(), "[]");
        assert!(f.trouble.iter().any(|t| t.contains("flight.json")), "{:?}", f.trouble);
    }

    #[test]
    fn the_images_in_this_build_are_the_ones_its_flight_json_describes() {
        // Whatever chipsat/firmware held when this was built: nothing, or a set that agrees.
        let f = carried();
        assert!(f.trouble.is_empty(), "{:?}", f.trouble);
        for image in &f.images {
            assert!(!image.stale(), "{}", image.file);
            assert!(f.choose(Some(image.module)).is_ok());
            // And what flight.json says about a settings block is what the image holds:
            // none, or one that names the module the image is listed under.
            let block = crate::settings::find(image.bytes).unwrap();
            match (f.build.as_ref().unwrap().settings_block, block) {
                (false, crate::settings::Found::Nothing) => {}
                (true, crate::settings::Found::Block(s)) => assert_eq!(s.module, image.module, "{}", image.file),
                (says, found) => panic!("{}: flight.json says settingsBlock {} and the image has {:?}", image.file, says, found),
            }
        }
    }
}
