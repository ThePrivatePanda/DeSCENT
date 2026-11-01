//! Images and radio.json built from the fsw tree on this laptop instead of the copies baked into
//! the app, cached by source hash. No tree, the carried copies. Flight and bench images never fall
//! back to the carried copy: writing the old image while someone thinks their edit is on the board
//! is worse than refusing
use crate::fswbuild::{self, Origin};
use crate::json::esc;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// dot folder so the log/image lists skip it
pub const FOLDER: &str = ".fsw-builds";
const REGISTRY: &str = "registry.tsv";

// name is the file name in fsw/firmware
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,             // "flight", "dump", "bench" or "radio"
    pub module: Option<&'static str>,
}

pub const PARTS: [Part; 6] = [
    Part { name: "flight-le", label: "flight LE", kind: "flight", module: Some("le") },
    Part { name: "flight-hp", label: "flight HP", kind: "flight", module: Some("hp") },
    Part { name: "flash_dump", label: "dump firmware", kind: "dump", module: None },
    Part { name: "bench-le", label: "sensor bench LE", kind: "bench", module: Some("le") },
    Part { name: "bench-hp", label: "sensor bench HP", kind: "bench", module: Some("hp") },
    Part { name: "radio.json", label: "radio table", kind: "radio", module: None },
];

pub fn part(name: &str) -> Option<&'static Part> {
    PARTS.iter().find(|p| p.name == name)
}

pub fn part_for(kind: &str, module: &str) -> Option<&'static Part> {
    PARTS.iter().find(|p| p.kind == kind && p.module == Some(module))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

// settings block zeroed, it's rewritten per board
pub fn masked(bytes: &[u8], block_at: Option<usize>) -> String {
    match block_at {
        Some(at) if at + crate::settings::SIZE <= bytes.len() => {
            let mut copy = bytes.to_vec();
            copy[at..at + crate::settings::SIZE].fill(0);
            sha256_hex(&copy)
        }
        _ => sha256_hex(bytes),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub part: String,
    pub at: u64,              // unix s
    pub origin: Origin,
    pub sha256: String,
    pub masked: String,       // see masked()
    pub len: usize,
    pub block_at: Option<usize>,
}

// "14:02" today, "20 Oct 14:02" otherwise
fn when_words(at: u64, now: u64) -> String {
    let local = |s: u64| chrono::DateTime::from_timestamp(s as i64, 0).map(|t| t.with_timezone(&chrono::Local));
    match (local(at), local(now)) {
        (Some(t), Some(n)) if t.date_naive() == n.date_naive() => t.format("%H:%M").to_string(),
        (Some(t), _) => t.format("%-d %b %H:%M").to_string(),
        _ => String::from("at an unknown time"),
    }
}

// "config.h, Phase.cpp and 3 more"
fn files_words(changed: &[String]) -> String {
    let names: Vec<&str> = changed.iter().map(|f| f.rsplit('/').next().unwrap_or(f)).collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].to_string(),
        2 => format!("{} and {}", names[0], names[1]),
        3 => format!("{}, {} and {}", names[0], names[1], names[2]),
        n => format!("{}, {} and {} more", names[0], names[1], n - 2),
    }
}

// "from abc1234 + config.h changed"
pub fn origin_words(o: &Origin) -> String {
    match &o.commit {
        None => format!("from a tree that is not a git checkout (sources {})", &o.source[..o.source.len().min(8)]),
        Some(c) => {
            let short = &c[..c.len().min(7)];
            match (o.dirty, o.changed.is_empty()) {
                (_, false) => format!("from {} + {} changed", short, files_words(&o.changed)),
                (None, true) => format!("from {}, changes unknown", short),
                _ => format!("from {}", short),
            }
        }
    }
}

impl Identity {
    pub fn label(&self) -> &'static str {
        part(&self.part).map(|p| p.label).unwrap_or("image")
    }

    pub fn id(&self) -> &str {
        &self.sha256[..self.sha256.len().min(16)]
    }

    /// "flight LE built here 14:02 from abc1234 + config.h changed"
    pub fn words(&self, now_s: u64) -> String {
        format!("{} built here {} {}", self.label(), when_words(self.at, now_s), origin_words(&self.origin))
    }

    pub fn json(&self, now_s: u64) -> String {
        let changed: Vec<String> = self.origin.changed.iter().map(|c| format!("\"{}\"", esc(c))).collect();
        format!(
            "{{\"part\":\"{}\",\"label\":\"{}\",\"at\":{},\"words\":\"{}\",\"commit\":{},\"dirty\":{},\"changed\":[{}],\"source\":\"{}\",\"id\":\"{}\",\"bytes\":{}}}",
            esc(&self.part),
            self.label(),
            self.at,
            esc(&self.words(now_s)),
            self.origin.commit.as_ref().map(|c| format!("\"{}\"", esc(c))).unwrap_or_else(|| String::from("null")),
            self.origin.dirty.map(|d| d.to_string()).unwrap_or_else(|| String::from("null")),
            changed.join(","),
            esc(&self.origin.source),
            esc(self.id()),
            self.len,
        )
    }

    // tab separated, changed files joined with |
    fn line(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r', '|'], "?");
        let changed: Vec<String> = self.origin.changed.iter().map(|c| clean(c)).collect();
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            self.masked,
            self.len,
            self.block_at.map(|b| b.to_string()).unwrap_or_else(|| String::from("-")),
            clean(&self.part),
            self.at,
            self.origin.source,
            self.origin.commit.as_deref().unwrap_or("-"),
            match self.origin.dirty {
                Some(true) => "1",
                Some(false) => "0",
                None => "-",
            },
            self.sha256,
            changed.join("|"),
        )
    }

    fn from_line(line: &str) -> Option<Identity> {
        let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
        if f.len() != 10 {
            return None;
        }
        let hex = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
        if !hex(f[0]) || !hex(f[5]) || !hex(f[8]) {
            return None;
        }
        Some(Identity {
            masked: f[0].to_string(),
            len: f[1].parse().ok()?,
            block_at: if f[2] == "-" { None } else { Some(f[2].parse().ok()?) },
            part: part(f[3])?.name.to_string(),
            at: f[4].parse().ok()?,
            origin: Origin {
                source: f[5].to_string(),
                commit: (f[6] != "-").then(|| f[6].to_string()),
                dirty: match f[7] {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                },
                changed: f[9].split('|').filter(|s| !s.is_empty()).map(String::from).collect(),
            },
            sha256: f[8].to_string(),
        })
    }

    pub fn matches(&self, flash: &[u8]) -> bool {
        self.len > 0 && flash.len() >= self.len && masked(&flash[..self.len], self.block_at) == self.masked
    }
}

// tests swap in one that compiles nothing
pub trait Builder: Send + Sync {
    fn origin(&self, tree: &Path) -> Origin;
    fn build(&self, tree: &Path, part: &str) -> Result<Vec<u8>, String>;
}

pub struct RealBuilder;

impl Builder for RealBuilder {
    fn origin(&self, tree: &Path) -> Origin {
        fswbuild::origin(tree)
    }
    fn build(&self, tree: &Path, part: &str) -> Result<Vec<u8>, String> {
        fswbuild::build_part(tree, part)
    }
}

pub struct Carried {
    pub parts: Vec<(&'static str, Option<&'static [u8]>)>,
}

impl Carried {
    pub fn this_build() -> Carried {
        Carried {
            parts: vec![
                ("flight-le", crate::flash::CHIPSAT_FLIGHT_LE),
                ("flight-hp", crate::flash::CHIPSAT_FLIGHT_HP),
                ("flash_dump", crate::flash::CHIPSAT_DUMP),
                ("bench-le", crate::flash::CHIPSAT_BENCH_LE),
                ("bench-hp", crate::flash::CHIPSAT_BENCH_HP),
                ("radio.json", crate::flash::CHIPSAT_RADIO.map(str::as_bytes)),
            ],
        }
    }

    pub fn get(&self, name: &str) -> Option<&'static [u8]> {
        self.parts.iter().find(|(n, _)| *n == name).and_then(|(_, b)| *b).filter(|b| !b.is_empty())
    }
}

pub enum Use {
    Tree(Vec<u8>, Identity),
    // with why the tree's wasn't used, if it was meant to be
    Carried(&'static [u8], Option<String>),
}

impl Use {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Use::Tree(b, _) => b,
            Use::Carried(b, _) => b,
        }
    }
}

pub struct Local {
    tree: Option<PathBuf>,
    dir: PathBuf,
    builder: Box<dyn Builder>,
    carried: Carried,
    // (source, part) -> compiler output, so a broken tree isn't rebuilt on every pull. A write retries
    failed: Mutex<HashMap<(String, String), String>>,
    registry: Mutex<()>,
}

pub type Shared = Arc<Local>;

fn refused(label: &str, e: &str) -> String {
    format!(
        "the {} from this computer's flight software did not build, so nothing was written. {}",
        label, e
    )
}

fn now_s() -> u64 {
    (crate::logfile::now_ms() / 1000) as u64
}

impl Local {
    // a mode.txt left by 1.2.0 and earlier is ignored
    pub fn new(tree: Option<PathBuf>, data: &Path, builder: Box<dyn Builder>, carried: Carried) -> Local {
        Local {
            tree,
            dir: data.join(FOLDER),
            builder,
            carried,
            failed: Mutex::new(HashMap::new()),
            registry: Mutex::new(()),
        }
    }

    pub fn tree(&self) -> Option<&Path> {
        self.tree.as_deref()
    }

    pub fn origin(&self) -> Option<Origin> {
        self.tree.as_deref().map(|t| self.builder.origin(t))
    }

    pub fn carried(&self, name: &str) -> Option<&'static [u8]> {
        self.carried.get(name)
    }

    fn folder(&self, source: &str) -> PathBuf {
        self.dir.join(&source[..source.len().min(16)])
    }

    fn cached(&self, origin: &Origin, name: &str) -> Option<(Vec<u8>, Identity)> {
        let at = self.folder(&origin.source);
        let bytes = std::fs::read(at.join(name)).ok().filter(|b| !b.is_empty())?;
        let id = std::fs::read_to_string(at.join(format!("{}.id", name))).ok().and_then(|l| Identity::from_line(&l))?;
        (id.sha256 == sha256_hex(&bytes) && id.origin.source == origin.source).then_some((bytes, id))
    }

    #[cfg(test)]
    pub fn built(&self, name: &str) -> Option<(Vec<u8>, Identity)> {
        self.cached(&self.origin()?, name)
    }

    // say() only hears about real builds (~13 s), not cache hits
    pub fn get(&self, name: &str, say: &dyn Fn(&str)) -> Result<(Vec<u8>, Identity), String> {
        let p = part(name).ok_or_else(|| format!("{:?} is not something the flight software builds", name))?;
        let tree = self.tree.as_deref().ok_or_else(|| String::from("there is no flight software tree on this computer"))?;
        let origin = self.builder.origin(tree);
        if let Some(hit) = self.cached(&origin, name) {
            return Ok(hit);
        }
        let key = (origin.source.clone(), name.to_string());
        if let Some(e) = self.failed.lock().unwrap().get(&key) {
            return Err(e.clone());
        }
        say(&format!("building the {} from the flight software in {}", p.label, tree.display()));
        // someone may have saved a file mid-build, don't cache under the old hash
        let built = self.builder.build(tree, name).and_then(|bytes| {
            if self.builder.origin(tree).source != origin.source {
                return Err(format!("a source in {} changed while the {} was building, so it was not kept. Build again", tree.display(), p.label));
            }
            self.keep(p, &origin, bytes)
        });
        if built.is_err() && self.builder.origin(tree).source != origin.source {
            return built;
        }
        if let Err(e) = &built {
            self.failed.lock().unwrap().insert(key, e.clone());
        }
        built
    }

    fn keep(&self, p: &Part, origin: &Origin, bytes: Vec<u8>) -> Result<(Vec<u8>, Identity), String> {
        if bytes.is_empty() {
            return Err(format!("the {} built empty", p.label));
        }
        let block_at = match p.kind {
            "radio" => {
                let text = String::from_utf8_lossy(&bytes);
                crate::radio::parse(&text).map_err(|e| format!("the radio table the tree printed does not read ({})", e))?;
                None
            }
            "flight" => crate::settings::locate(&bytes).ok().flatten(),
            _ => None,
        };
        let id = Identity {
            part: p.name.to_string(),
            at: now_s(),
            origin: origin.clone(),
            sha256: sha256_hex(&bytes),
            masked: masked(&bytes, block_at),
            len: bytes.len(),
            block_at,
        };
        let at = self.folder(&origin.source);
        let write = |file: &str, what: &[u8]| {
            std::fs::write(at.join(file), what).map_err(|e| format!("cannot keep the {} in {}: {}", p.label, at.display(), e))
        };
        std::fs::create_dir_all(&at).map_err(|e| format!("cannot make {}: {}", at.display(), e))?;
        write(p.name, &bytes)?;
        write(&format!("{}.id", p.name), id.line().as_bytes())?;
        if p.kind != "radio" {
            self.register(&id)?;
        }
        Ok((bytes, id))
    }

    fn register(&self, id: &Identity) -> Result<(), String> {
        let _one = self.registry.lock().unwrap();
        if self.registered().iter().any(|r| r.masked == id.masked && r.len == id.len && r.part == id.part) {
            return Ok(());
        }
        use std::io::Write;
        let path = self.dir.join(REGISTRY);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(id.line().as_bytes()))
            .map_err(|e| format!("cannot add to {}: {}", path.display(), e))
    }

    // oldest first, bad lines skipped
    pub fn registered(&self) -> Vec<Identity> {
        std::fs::read_to_string(self.dir.join(REGISTRY))
            .map(|t| t.lines().filter_map(Identity::from_line).collect())
            .unwrap_or_default()
    }

    // no tree: carried copy. Tree: its part, and if it won't build either the carried copy plus the
    // reason (fallback) or just the reason
    pub fn pick(&self, name: &str, fallback: bool, say: &dyn Fn(&str)) -> Result<Use, String> {
        let label = part(name).map(|p| p.label).unwrap_or(name);
        if self.tree.is_none() {
            return self
                .carried(name)
                .map(|b| Use::Carried(b, None))
                .ok_or_else(|| format!("this build of the app carries no {}", label));
        }
        match self.get(name, say) {
            Ok((bytes, id)) => Ok(Use::Tree(bytes, id)),
            Err(e) => match self.carried(name) {
                Some(b) if fallback => Ok(Use::Carried(
                    b,
                    Some(format!("the {} from this computer's flight software did not build, so the copy this app carries is used. {}", label, e)),
                )),
                _ if fallback => Err(format!("the {} from this computer's flight software did not build, and this app carries none. {}", label, e)),
                _ => Err(refused(label, &e)),
            },
        }
    }

    // for images about to be written, never falls back. Builds again after a failure
    pub fn get_or_refuse(&self, name: &str, say: &dyn Fn(&str)) -> Result<(Vec<u8>, Identity), String> {
        let label = part(name).map(|p| p.label).unwrap_or(name);
        if let Some(origin) = self.origin() {
            self.failed.lock().unwrap().remove(&(origin.source, name.to_string()));
        }
        self.get(name, say).map_err(|e| refused(label, &e))
    }

    pub fn radio(&self, tree: bool) -> Result<(crate::radio::Radio, Option<String>), String> {
        let (text, note) = self.radio_text(tree)?;
        let r = crate::radio::parse(&text).map_err(|e| {
            format!("the radio table does not read ({}), so settings cannot be checked against it", e)
        })?;
        Ok((r, note))
    }

    pub fn radio_text(&self, tree: bool) -> Result<(String, Option<String>), String> {
        let got = if tree && self.tree.is_some() {
            match self.get("radio.json", &|_| {}) {
                Ok((bytes, _)) => return Ok((String::from_utf8_lossy(&bytes).to_string(), None)),
                Err(e) => self.carried("radio.json").map(|b| {
                    Use::Carried(b, Some(format!("the radio table from this computer's flight software did not build, so the one this app carries is used. {}", e)))
                }),
            }
        } else {
            self.carried("radio.json").map(|b| Use::Carried(b, None))
        };
        match got {
            Some(Use::Carried(b, note)) => Ok((String::from_utf8_lossy(b).to_string(), note)),
            _ => Err(String::from("there is no radio table: this build of the app carries none and the tree's did not build, so settings cannot be checked")),
        }
    }

    // for report.json
    pub fn use_json(&self, u: &Use, name: &str) -> String {
        match u {
            Use::Tree(_, id) => id.json(now_s()),
            Use::Carried(b, note) => {
                let label = part(name).map(|p| p.label).unwrap_or(name);
                let id = &sha256_hex(b)[..16];
                format!(
                    "{{\"part\":\"{}\",\"carried\":true,\"id\":\"{}\",\"words\":\"{} this app carries, {}\",\"note\":{}}}",
                    esc(name),
                    id,
                    label,
                    &id[..8],
                    note.as_ref().map(|n| format!("\"{}\"", esc(n))).unwrap_or_else(|| String::from("null"))
                )
            }
        }
    }

    // /api/fsw, for the write confirm
    pub fn state_json(&self) -> String {
        let now = now_s();
        let q = |s: &Option<String>| s.as_ref().map(|t| format!("\"{}\"", esc(t))).unwrap_or_else(|| String::from("null"));
        let origin = self.origin();
        let parts: Vec<String> = PARTS
            .iter()
            .map(|p| {
                let carried = self.carried(p.name);
                let (built, same) = match origin.as_ref().and_then(|o| self.cached(o, p.name)) {
                    Some((bytes, id)) => (id.json(now), carried.map(|c| (c == &bytes[..]).to_string()).unwrap_or_else(|| String::from("false"))),
                    None => (String::from("null"), String::from("null")),
                };
                let failed = origin.as_ref().and_then(|o| self.failed.lock().unwrap().get(&(o.source.clone(), p.name.to_string())).cloned());
                format!(
                    "{{\"part\":\"{}\",\"label\":\"{}\",\"kind\":\"{}\",\"module\":{},\"built\":{},\"failed\":{},\"carried\":{},\"same\":{}}}",
                    p.name,
                    p.label,
                    p.kind,
                    p.module.map(|m| format!("\"{}\"", m)).unwrap_or_else(|| String::from("null")),
                    built,
                    q(&failed),
                    carried.map(|c| format!("\"{}\"", &sha256_hex(c)[..16])).unwrap_or_else(|| String::from("null")),
                    same,
                )
            })
            .collect();
        let source = match &origin {
            Some(o) => {
                let changed: Vec<String> = o.changed.iter().map(|c| format!("\"{}\"", esc(c))).collect();
                format!(
                    "{{\"hash\":\"{}\",\"commit\":{},\"dirty\":{},\"changed\":[{}],\"words\":\"{}\"}}",
                    esc(&o.source),
                    q(&o.commit),
                    o.dirty.map(|d| d.to_string()).unwrap_or_else(|| String::from("null")),
                    changed.join(","),
                    esc(&origin_words(o)),
                )
            }
            None => String::from("null"),
        };
        format!(
            "{{\"ok\":true,\"tree\":{},\"source\":{},\"parts\":[{}]}}",
            q(&self.tree.as_ref().map(|t| t.display().to_string())),
            source,
            parts.join(","),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // fake image = part name + source hash, so a new source means new bytes
    struct Fake {
        source: Mutex<String>,
        changed: Vec<String>,
        builds: AtomicUsize,
        broken: Mutex<Vec<&'static str>>,
        // simulates a file saved mid-build
        edit: Mutex<Option<String>>,
        old_layout: Mutex<bool>,
    }

    impl Builder for Arc<Fake> {
        fn origin(&self, _: &Path) -> Origin {
            Origin {
                source: self.source.lock().unwrap().clone(),
                commit: Some(String::from("abc1234def5678abc1234def5678abc1234def56")),
                dirty: Some(!self.changed.is_empty()),
                changed: self.changed.clone(),
            }
        }
        fn build(&self, _: &Path, part: &str) -> Result<Vec<u8>, String> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            if let Some(after) = self.edit.lock().unwrap().take() {
                *self.source.lock().unwrap() = after;
            }
            if self.broken.lock().unwrap().contains(&part) {
                return Err(String::from("the flight software did not build.\nconfig.h:60:26: error: static assertion failed: an LE module has rows for 10, 14 and 15 dBm only"));
            }
            if part == "radio.json" {
                if *self.old_layout.lock().unwrap() {
                    return Ok(RADIO.replace("\"layoutVersion\": 4", "\"layoutVersion\": 3").into_bytes());
                }
                return Ok(RADIO.replace("\"min\": 5", "\"min\": 6").into_bytes());
            }
            Ok(format!("{} built from {}", part, &self.source.lock().unwrap()[..8]).into_bytes())
        }
    }

    const RADIO: &str = r#"{
  "layoutVersion": 4,
  "spreadingFactor": {"min": 5, "max": 12},
  "slots": [
    {"name": "ground transmit", "fixedMs": 60000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000]},
    {"name": "air transmit", "fixedMs": 1000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [10000, 30000, 60000, 120000, 180000]},
    {"name": "ground log", "fixedMs": 1000, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [1000, 1000, 1000, 1000, 1000]},
    {"name": "air log", "fixedMs": 50, "follow": false, "percent": [75, 50, 35, 20], "intervalMs": [50, 50, 50, 50, 50]}
  ],
  "le": {"dbm": [{"dbm": 10}, {"dbm": 14}, {"dbm": 15}]},
  "hp": {"dbm": [{"dbm": 14}, {"dbm": 17}, {"dbm": 20}, {"dbm": 22}]}
}"#;

    const SRC_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SRC_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CARRIED_DUMP: &[u8] = b"the dump firmware the app carries";

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dg-localfw-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn carried() -> Carried {
        let radio: &'static str = Box::leak(RADIO.to_string().into_boxed_str());
        Carried {
            parts: vec![
                ("flight-le", Some(b"flight-le built from aaaaaaaa".as_slice())),
                ("flight-hp", None),
                ("flash_dump", Some(CARRIED_DUMP)),
                ("bench-le", None),
                ("bench-hp", None),
                ("radio.json", Some(radio.as_bytes())),
            ],
        }
    }

    fn local(what: &str, changed: &[&str]) -> (Arc<Local>, Arc<Fake>, PathBuf) {
        let data = scratch(what);
        let fake = Arc::new(Fake {
            source: Mutex::new(SRC_A.to_string()),
            changed: changed.iter().map(|c| c.to_string()).collect(),
            builds: AtomicUsize::new(0),
            broken: Mutex::new(Vec::new()),
            edit: Mutex::new(None),
            old_layout: Mutex::new(false),
        });
        let l = Local::new(Some(data.join("fsw")), &data, Box::new(fake.clone()), carried());
        (Arc::new(l), fake, data)
    }

    #[test]
    fn a_tree_at_another_layout_gets_the_carried_table_and_says_why() {
        let (l, fake, data) = local("old-layout", &[]);
        *fake.old_layout.lock().unwrap() = true;
        let e = l.get("radio.json", &|_| {}).unwrap_err();
        assert!(e.contains("it is for settings block layout 3, and this app writes layout 4"), "{}", e);
        let (radio, note) = l.radio(true).unwrap();
        assert_eq!(radio.sf_min, 5, "the carried table");
        assert!(note.unwrap().contains("layout 3"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_part_is_built_once_per_source_and_a_changed_source_builds_again() {
        let (l, fake, data) = local("cache", &["V2_6_X_Code/src/config.h"]);
        let said = Mutex::new(Vec::new());
        let (bytes, id) = l.get("flight-le", &|s| said.lock().unwrap().push(s.to_string())).unwrap();
        assert_eq!(bytes, b"flight-le built from aaaaaaaa");
        assert_eq!(fake.builds.load(Ordering::SeqCst), 1);
        assert!(said.lock().unwrap()[0].starts_with("building the flight LE from the flight software in"), "{:?}", said);
        assert!(data.join(FOLDER).join(&SRC_A[..16]).join("flight-le").is_file());
        // cache hit, no build, nothing said
        said.lock().unwrap().clear();
        let (again, again_id) = l.get("flight-le", &|s| said.lock().unwrap().push(s.to_string())).unwrap();
        assert_eq!((again, again_id), (bytes, id.clone()));
        assert_eq!(fake.builds.load(Ordering::SeqCst), 1);
        assert!(said.lock().unwrap().is_empty());
        // survives a restart
        let fresh = Local::new(Some(data.join("fsw")), &data, Box::new(fake.clone()), carried());
        assert_eq!(fresh.built("flight-le").unwrap().1, id);
        assert_eq!(fake.builds.load(Ordering::SeqCst), 1);
        *fake.source.lock().unwrap() = SRC_B.to_string();
        assert!(l.built("flight-le").is_none());
        let (b, _) = l.get("flight-le", &|_| {}).unwrap();
        assert_eq!(b, b"flight-le built from bbbbbbbb");
        assert_eq!(fake.builds.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn every_image_built_is_in_the_registry_with_where_it_came_from() {
        let (l, fake, data) = local("registry", &["V2_6_X_Code/src/config.h"]);
        l.get("flight-le", &|_| {}).unwrap();
        l.get("flight-le", &|_| {}).unwrap();
        l.get("radio.json", &|_| {}).unwrap();
        *fake.source.lock().unwrap() = SRC_B.to_string();
        l.get("flash_dump", &|_| {}).unwrap();
        let r = l.registered();
        assert_eq!(r.len(), 2, "one line per image, and the radio table is not an image: {:?}", r);
        assert_eq!((r[0].part.as_str(), r[0].origin.source.as_str()), ("flight-le", SRC_A));
        assert_eq!(r[0].origin.commit.as_deref(), Some("abc1234def5678abc1234def5678abc1234def56"));
        assert_eq!(r[0].origin.dirty, Some(true));
        assert_eq!(r[0].origin.changed, vec!["V2_6_X_Code/src/config.h".to_string()]);
        assert_eq!(r[1].part, "flash_dump");
        assert!(r[0].words(r[0].at).starts_with("flight LE built here "), "{}", r[0].words(r[0].at));
        assert!(r[0].words(r[0].at).ends_with(" from abc1234 + config.h changed"), "{}", r[0].words(r[0].at));
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(data.join(FOLDER).join(REGISTRY)).unwrap().write_all(b"not a line\n").unwrap();
        assert_eq!(l.registered().len(), 2);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_settings_block_is_no_part_of_which_image_a_board_runs() {
        let mut image: Vec<u8> = (0..2000u32).map(|i| (i * 13 + 1) as u8).collect();
        let at = 500;
        let id = Identity {
            part: "flight-le".into(),
            at: 0,
            origin: Origin { source: SRC_A.into(), commit: None, dirty: None, changed: Vec::new() },
            sha256: sha256_hex(&image),
            masked: masked(&image, Some(at)),
            len: image.len(),
            block_at: Some(at),
        };
        let mut flash = image.clone();
        flash.extend_from_slice(&[0xFF; 300]);
        assert!(id.matches(&flash));
        flash[at + 10] ^= 0x55;   // another board id in the block
        assert!(id.matches(&flash));
        flash[at - 1] ^= 1;       // a byte outside it
        assert!(!id.matches(&flash));
        assert!(!id.matches(&image[..1999]), "cut short is not the image");
        image[0] ^= 1;
        assert!(!id.matches(&image));
    }

    #[test]
    fn a_registry_line_reads_back_as_it_was_written() {
        let id = Identity {
            part: "bench-hp".into(),
            at: 1792600000,
            origin: Origin { source: SRC_B.into(), commit: Some("0123456789".into()), dirty: Some(true), changed: vec!["a/b.h".into(), "c|d\t.cpp".into()] },
            sha256: SRC_A.into(),
            masked: SRC_B.into(),
            len: 4096,
            block_at: Some(1000),
        };
        let back = Identity::from_line(&id.line()).unwrap();
        assert_eq!(back.origin.changed, vec!["a/b.h".to_string(), "c?d?.cpp".to_string()]);
        assert_eq!(Identity { origin: Origin { changed: id.origin.changed.clone(), ..back.origin.clone() }, ..back }, id);
        let none = Identity { block_at: None, origin: Origin { commit: None, dirty: None, changed: Vec::new(), ..id.origin.clone() }, ..id.clone() };
        assert_eq!(Identity::from_line(&none.line()).unwrap(), none);
        assert!(Identity::from_line("").is_none());
        assert!(Identity::from_line(&id.line().replace("bench-hp", "bench-xp")).is_none());
    }

    #[test]
    fn the_words_say_the_commit_and_the_changed_files_by_name() {
        let o = |commit: Option<&str>, dirty: Option<bool>, changed: &[&str]| Origin {
            source: SRC_A.into(),
            commit: commit.map(String::from),
            dirty,
            changed: changed.iter().map(|c| c.to_string()).collect(),
        };
        assert_eq!(origin_words(&o(Some("abc1234def"), Some(false), &[])), "from abc1234");
        assert_eq!(origin_words(&o(Some("abc1234def"), Some(true), &["V2_6_X_Code/src/config.h"])), "from abc1234 + config.h changed");
        assert_eq!(
            origin_words(&o(Some("abc1234def"), Some(true), &["V2_6_X_Code/src/config.h", "V2_6_X_Code/src/app/Phase.cpp"])),
            "from abc1234 + config.h and Phase.cpp changed"
        );
        assert_eq!(origin_words(&o(Some("abc1234def"), Some(true), &["a", "b", "c", "d", "e"])), "from abc1234 + a, b and 3 more changed");
        assert_eq!(origin_words(&o(None, None, &[])), "from a tree that is not a git checkout (sources aaaaaaaa)");
        let now = 1_792_600_000;
        assert!(when_words(now, now).len() == 5 && when_words(now, now).contains(':'), "{}", when_words(now, now));
        assert!(when_words(now - 3 * 86400, now).len() > 5, "{}", when_words(now - 3 * 86400, now));
    }

    #[test]
    fn with_no_tree_the_carried_copy_is_used_and_nothing_is_built() {
        let (_, fake, data) = local("none", &[]);
        let none = Local::new(None, &data, Box::new(fake.clone()), carried());
        match none.pick("flash_dump", true, &|_| {}).unwrap() {
            Use::Carried(b, None) => assert_eq!(b, CARRIED_DUMP),
            _ => panic!("the carried dump, with nothing to say"),
        }
        let e = none.pick("bench-le", false, &|_| {}).err().unwrap();
        assert!(e.contains("carries no sensor bench LE"), "{}", e);
        assert_eq!(none.radio(true).unwrap().0.sf_min, 5);
        assert_eq!(fake.builds.load(Ordering::SeqCst), 0);
        assert!(none.state_json().starts_with("{\"ok\":true,\"tree\":null,\"source\":null,\"parts\":["), "{}", none.state_json());
        assert!(none.state_json().contains("\"built\":null,\"failed\":null,"), "{}", none.state_json());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn with_a_tree_its_part_is_used_and_a_leftover_mode_file_is_ignored() {
        let (_, fake, data) = local("tree", &[]);
        // 1.2.0 wrote this when its switch was off
        std::fs::create_dir_all(data.join(FOLDER)).unwrap();
        std::fs::write(data.join(FOLDER).join("mode.txt"), "off\n").unwrap();
        let l = Local::new(Some(data.join("fsw")), &data, Box::new(fake.clone()), carried());
        match l.pick("flash_dump", true, &|_| {}).unwrap() {
            Use::Tree(b, id) => {
                assert_eq!(b, b"flash_dump built from aaaaaaaa");
                assert_eq!(id.part, "flash_dump");
                assert!(l.use_json(&Use::Tree(b.clone(), id), "flash_dump").contains("\"words\":\"dump firmware built here "));
            }
            _ => panic!("the tree's dump"),
        }
        // fake tree table starts at SF6
        assert_eq!(l.radio(true).unwrap().0.sf_min, 6);
        assert_eq!(l.radio(false).unwrap().0.sf_min, 5, "asked for the carried one, it is the carried one");
        let s = l.state_json();
        assert!(s.starts_with("{\"ok\":true,\"tree\":\""), "{}", s);
        assert!(!s.contains("\"on\":") && !s.contains("\"job\":"), "{}", s);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_part_that_does_not_build_falls_back_only_where_that_is_safe_and_says_why() {
        let (l, fake, data) = local("broken", &[]);
        fake.broken.lock().unwrap().extend(["flash_dump", "flight-le", "radio.json"]);
        match l.pick("flash_dump", true, &|_| {}).unwrap() {
            Use::Carried(b, Some(why)) => {
                assert_eq!(b, CARRIED_DUMP);
                assert!(why.contains("did not build, so the copy this app carries is used"), "{}", why);
                assert!(why.contains("static assertion failed: an LE module has rows for 10, 14 and 15 dBm only"), "the compiler's own words: {}", why);
            }
            _ => panic!("the carried dump, and the reason"),
        }
        let e = l.pick("flight-le", false, &|_| {}).err().unwrap();
        assert!(e.contains("nothing was written") && e.contains("static assertion failed"), "{}", e);
        let (radio, note) = l.radio(true).unwrap();
        assert_eq!(radio.sf_min, 5);
        assert!(note.unwrap().contains("the one this app carries is used"));
        // failure is remembered, not rebuilt
        let before = fake.builds.load(Ordering::SeqCst);
        let _ = l.pick("flash_dump", true, &|_| {});
        assert_eq!(fake.builds.load(Ordering::SeqCst), before);
        let s = l.state_json();
        assert!(s.contains("\"part\":\"flash_dump\",\"label\":\"dump firmware\",\"kind\":\"dump\",\"module\":null,\"built\":null,\"failed\":\"the flight software did not build.\\nconfig.h:60:26"), "{}", s);
        assert!(l.registered().is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_write_after_a_failed_build_builds_again_and_a_pull_does_not() {
        let (l, fake, data) = local("retry", &["V2_6_X_Code/src/config.h"]);
        fake.broken.lock().unwrap().extend(["flight-hp", "flash_dump"]);
        let e = l.get_or_refuse("flight-hp", &|_| {}).unwrap_err();
        assert!(e.starts_with("the flight HP from this computer's flight software did not build, so nothing was written. the flight software did not build.\nconfig.h:60:26: error:"), "{}", e);
        assert!(!e.contains("Switch off"), "{}", e);
        let _ = l.pick("flash_dump", true, &|_| {});
        let before = fake.builds.load(Ordering::SeqCst);
        let _ = l.pick("flash_dump", true, &|_| {});
        assert_eq!(fake.builds.load(Ordering::SeqCst), before, "a pull uses the remembered failure");
        fake.broken.lock().unwrap().clear();
        let (b, id) = l.get_or_refuse("flight-hp", &|_| {}).unwrap();
        assert_eq!(b, b"flight-hp built from aaaaaaaa");
        assert_eq!(fake.builds.load(Ordering::SeqCst), before + 1);
        assert!(id.words(id.at).ends_with(" from abc1234 + config.h changed"), "{}", id.words(id.at));
        l.get("flight-le", &|_| {}).unwrap();
        let s = l.state_json();
        assert!(s.contains("\"changed\":[\"V2_6_X_Code/src/config.h\"],\"words\":\"from abc1234 + config.h changed\"}"), "{}", s);
        // fake flight-le happens to equal the carried one
        assert!(s.contains("\"part\":\"flight-le\"") && s.contains(",\"same\":true}"), "{}", s);
        assert!(s.contains("\"part\":\"flight-hp\",\"label\":\"flight HP\",\"kind\":\"flight\",\"module\":\"hp\",\"built\":{\"part\":\"flight-hp\""), "{}", s);
        assert!(s.contains("\"carried\":null,\"same\":false}"), "nothing carried is never the same: {}", s);
        assert!(s.contains("\"part\":\"flash_dump\",\"label\":\"dump firmware\",\"kind\":\"dump\",\"module\":null,\"built\":null,\"failed\":\"the flight software"), "{}", s);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_source_saved_during_a_build_means_nothing_is_kept_under_the_old_sources() {
        let (l, fake, data) = local("edited", &[]);
        *fake.edit.lock().unwrap() = Some(SRC_B.to_string());
        let e = l.get("flight-le", &|_| {}).unwrap_err();
        assert!(e.contains("changed while the flight LE was building"), "{}", e);
        assert!(!data.join(FOLDER).join(&SRC_A[..16]).join("flight-le").exists());
        assert!(l.registered().is_empty());
        assert_eq!(l.get("flight-le", &|_| {}).unwrap().0, b"flight-le built from bbbbbbbb");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_cached_file_that_is_not_what_was_built_is_built_again() {
        let (l, fake, data) = local("tamper", &[]);
        l.get("flash_dump", &|_| {}).unwrap();
        std::fs::write(data.join(FOLDER).join(&SRC_A[..16]).join("flash_dump"), b"something else").unwrap();
        assert!(l.built("flash_dump").is_none());
        assert_eq!(l.get("flash_dump", &|_| {}).unwrap().0, b"flash_dump built from aaaaaaaa");
        assert_eq!(fake.builds.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&data);
    }
}
