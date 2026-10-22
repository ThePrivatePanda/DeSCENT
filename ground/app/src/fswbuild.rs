//! Builds the flight software for one board, with that board's settings compiled into it.
//!
//! Which radio module is on the board is a compile-time choice in the flight software: the power
//! amplifier rows differ. Everything else a board is given lives in the settings block, which the
//! caller patches into the built image exactly as it patches the image the app carries. So this
//! runs the flight software's own build for the module, and hands the compiler's own words back
//! when it refuses.

use std::path::{Path, PathBuf};

/// The flight software's folder, the one holding build.sh. Looked for rather than configured,
/// because the usual case is the dashboard and the flight software sitting in one repository:
/// `fsw/` beside `ground/`. CHIPSAT_FSW overrides it for a tree somewhere else.
pub fn tree(exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(set) = std::env::var_os("CHIPSAT_FSW") {
        let at = PathBuf::from(set);
        return if is_tree(&at) { Some(at) } else { None };
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Some(exe) = exe {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    for start in starts {
        let mut at: &Path = &start;
        loop {
            if is_tree(at) {
                return Some(at.to_path_buf());
            }
            let beside = at.join("fsw");
            if is_tree(&beside) {
                return Some(beside);
            }
            match at.parent() {
                Some(up) => at = up,
                None => break,
            }
        }
    }
    None
}

fn is_tree(at: &Path) -> bool {
    at.join("build.sh").is_file() && at.join("V2_6_X_Code").is_dir()
}

// everything shares the tree's build/, so one at a time
static BUILDING: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Same flags as tools/emit_firmware.sh, part names as in fsw/firmware. ~13 s each since a new -D
// forces a reconfigure. Flight images only get the module so config.h edits stick; board
// settings get patched into the block afterwards like the carried image
pub fn build_part(at: &Path, part: &str) -> Result<Vec<u8>, String> {
    let args: &[&str] = match part {
        "flight-le" => &["build", "-DCHIPSAT_RADIO_MODULE=1"],
        "flight-hp" => &["build", "-DCHIPSAT_RADIO_MODULE=2"],
        "flash_dump" => &["build", "--sketch", "flash_dump"],
        "bench-le" => &["build", "--sketch", "sensor_bench", "-DCHIPSAT_RADIO_MODULE=1"],
        "bench-hp" => &["build", "--sketch", "sensor_bench", "-DCHIPSAT_RADIO_MODULE=2"],
        "radio.json" => return radio_json(at),
        other => return Err(format!("{:?} is not something the flight software builds", other)),
    };
    let _one = BUILDING.lock().unwrap_or_else(|e| e.into_inner());
    let out = std::process::Command::new("./build.sh")
        .args(args)
        .current_dir(at)
        .output()
        .map_err(|e| format!("cannot run {}/build.sh: {}", at.display(), e))?;

    if !out.status.success() {
        return Err(format!(
            "the flight software did not build.\n{}",
            refusal(&out.stderr, &out.stdout)
        ));
    }

    let bin = at.join("build").join("chipsat.bin");
    let bytes = std::fs::read(&bin)
        .map_err(|e| format!("the build reported success but {} cannot be read: {}", bin.display(), e))?;
    if bytes.is_empty() {
        return Err(format!("{} is empty", bin.display()));
    }
    Ok(bytes)
}

// needs g++ on the laptop
fn radio_json(at: &Path) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("sh")
        .arg("tools/emit_radio_json.sh")
        .current_dir(at)
        .output()
        .map_err(|e| format!("cannot run {}/tools/emit_radio_json.sh: {}", at.display(), e))?;
    if !out.status.success() {
        return Err(format!("the radio table did not build.\n{}", refusal(&out.stderr, &out.stdout)));
    }
    if out.stdout.is_empty() {
        return Err(String::from("tools/emit_radio_json.sh printed nothing"));
    }
    Ok(out.stdout)
}

// source hash is the cache key, the git bits are what gets shown
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub source: String,
    pub commit: Option<String>,   // None outside a git checkout
    pub dirty: Option<bool>,
    pub changed: Vec<String>,     // relative to the tree, sources only
}

// tests and test data left out so editing a test doesn't invalidate every image
const SOURCE_DIRS: &[&str] = &["V2_6_X_Code", "V2_6_X_Tests/bench", "V2_6_X_Tests/test/mock", "cmake"];
const SOURCE_FILES: &[&str] = &["CMakeLists.txt", "build.sh", "tools/emit_radio_json.sh"];

pub fn is_source(rel: &str) -> bool {
    let rel = rel.trim_start_matches("./");
    if rel.split('/').any(|part| part.starts_with('.') || part == "__pycache__") {
        return false;
    }
    SOURCE_FILES.contains(&rel)
        || SOURCE_DIRS.iter().any(|d| rel.starts_with(&format!("{}/", d)))
        || rel
            .strip_prefix("V2_6_X_Tests/test/")
            .map(|r| r.starts_with("emit_") && r.ends_with(".cpp") && !r.contains('/'))
            .unwrap_or(false)
}

pub fn sources(at: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            let rel = path.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if !e.file_name().to_string_lossy().starts_with('.') {
                        walk(root, &path, out)
                    }
                }
                Ok(_) if is_source(&rel) => out.push(rel),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for d in SOURCE_DIRS.iter().chain(["V2_6_X_Tests/test"].iter()) {
        walk(at, &at.join(d), &mut out);
    }
    for f in SOURCE_FILES {
        if at.join(f).is_file() {
            out.push(f.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

// an unreadable file still changes the hash
pub fn source_hash(at: &Path, files: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for f in files {
        h.update(f.as_bytes());
        h.update([0]);
        match std::fs::read(at.join(f)) {
            Ok(bytes) => {
                h.update((bytes.len() as u64).to_le_bytes());
                h.update(&bytes);
            }
            Err(_) => h.update(b"unreadable"),
        }
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

// git is optional, a copied-out tree just has no commit
pub fn origin(at: &Path) -> Origin {
    let source = source_hash(at, &sources(at));
    let git = |args: &[&str]| -> Option<Vec<u8>> {
        let out = std::process::Command::new("git").arg("-C").arg(at).args(args).output().ok()?;
        out.status.success().then_some(out.stdout)
    };
    let Some(commit) = git(&["rev-parse", "HEAD"]).map(|b| String::from_utf8_lossy(&b).trim().to_string()).filter(|c| !c.is_empty()) else {
        return Origin { source, commit: None, dirty: None, changed: Vec::new() };
    };
    let prefix = git(&["rev-parse", "--show-prefix"]).map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let status = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."]);
    let changed = match &status {
        Some(bytes) => changed_sources(bytes, &prefix),
        None => Vec::new(),
    };
    Origin { source, commit: Some(commit), dirty: status.map(|_| !changed.is_empty()), changed }
}

// porcelain paths are repo-relative. With -z a rename is followed by its old name, skip that
fn changed_sources(status: &[u8], prefix: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(status);
    let mut out = Vec::new();
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = (&entry[..2], &entry[3..]);
        if code.contains('R') || code.contains('C') {
            entries.next();
        }
        let rel = path.strip_prefix(prefix).unwrap_or(path);
        if is_source(rel) {
            out.push(rel.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The compiler's own reason, which for a setting it will not accept is a static_assert naming the
/// values that module does take. Everything else a build prints is noise to whoever pressed the
/// button, so only the lines that say what was wrong are kept.
fn refusal(stderr: &[u8], stdout: &[u8]) -> String {
    let both = format!(
        "{}\n{}",
        String::from_utf8_lossy(stderr),
        String::from_utf8_lossy(stdout)
    );
    let mut kept: Vec<&str> = both
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("static assertion failed") || l.contains("error:") || l.contains("fatal error")
        })
        .map(|l| l.trim())
        .collect();
    kept.dedup();
    if kept.is_empty() {
        let tail: Vec<&str> = both.lines().filter(|l| !l.trim().is_empty()).rev().take(4).collect();
        return tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    }
    kept.truncate(4);
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tree_says_which_path_it_tried() {
        let e = build_part(Path::new("/nonexistent"), "flight-le").unwrap_err();
        assert!(e.contains("/nonexistent"), "{}", e);
        let e = build_part(Path::new("/nonexistent"), "radio.json").unwrap_err();
        assert!(e.contains("/nonexistent"), "{}", e);
    }

    #[test]
    fn the_static_assert_is_what_survives_the_noise() {
        let noise = b"ninja: Entering directory\n[1/5] Building CXX object\nconfig.h:60:26: error: static assertion failed: an LE module has rows for 10, 14 and 15 dBm only\nninja: build stopped.\n";
        let text = refusal(noise, b"");
        assert!(text.contains("10, 14 and 15 dBm only"), "{}", text);
        assert!(!text.contains("Entering directory"), "{}", text);
    }

    #[test]
    fn with_nothing_to_go_on_the_last_lines_are_kept() {
        let text = refusal(b"", b"something went wrong\nand then stopped\n");
        assert!(text.contains("and then stopped"), "{}", text);
    }

    #[test]
    fn a_part_the_flight_software_does_not_build_is_refused_before_anything_runs() {
        let e = build_part(Path::new("/nonexistent"), "flight-xp").unwrap_err();
        assert!(e.contains("not something the flight software builds"), "{}", e);
    }

    #[test]
    fn the_sources_are_what_a_build_reads_and_not_the_tests_or_the_output() {
        for yes in [
            "V2_6_X_Code/src/config.h",
            "V2_6_X_Code/V2_6_X_Code.ino",
            "V2_6_X_Tests/bench/flash_dump/flash_dump.ino",
            "V2_6_X_Tests/test/mock/Arduino.h",
            "V2_6_X_Tests/test/emit_radio_table.cpp",
            "cmake/chipsat.cmake",
            "CMakeLists.txt",
            "build.sh",
            "tools/emit_radio_json.sh",
        ] {
            assert!(is_source(yes), "{}", yes);
        }
        for no in [
            "V2_6_X_Tests/test/test_phase.cpp",
            "V2_6_X_Tests/test/data/drop.csv",
            "V2_6_X_Tests/test/data/emit_x.cpp",
            "build/chipsat.bin",
            "deps/ninja/ninja",
            "firmware/flight-le.bin",
            "tools/descent_decode.py",
            "V2_6_X_Code/src/.config.h.swp",
            "README.md",
        ] {
            assert!(!is_source(no), "{}", no);
        }
    }

    fn fake_tree(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fswbuild-{}-{}", what, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["V2_6_X_Code/src", "V2_6_X_Tests/test/data", "build"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("build.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("V2_6_X_Code/src/config.h"), "#define CHIPSAT_ID 255\n").unwrap();
        std::fs::write(dir.join("V2_6_X_Tests/test/data/x.csv"), "1,2\n").unwrap();
        std::fs::write(dir.join("build/chipsat.bin"), "out").unwrap();
        dir
    }

    #[test]
    fn the_source_hash_moves_with_a_source_and_not_with_anything_else() {
        let dir = fake_tree("hash");
        let files = sources(&dir);
        assert_eq!(files, vec!["V2_6_X_Code/src/config.h".to_string(), "build.sh".to_string()]);
        let before = source_hash(&dir, &files);
        assert_eq!(before.len(), 64);
        assert_eq!(source_hash(&dir, &sources(&dir)), before, "the same files hash the same");
        std::fs::write(dir.join("build/chipsat.bin"), "other output").unwrap();
        std::fs::write(dir.join("V2_6_X_Tests/test/data/x.csv"), "3,4\n").unwrap();
        assert_eq!(source_hash(&dir, &sources(&dir)), before, "output and test data are not sources");
        std::fs::write(dir.join("V2_6_X_Code/src/config.h"), "#define CHIPSAT_ID 254\n").unwrap();
        assert_ne!(source_hash(&dir, &sources(&dir)), before, "a changed constant is a new source");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_status_is_read_as_the_changed_sources_relative_to_the_tree() {
        let status = b" M fsw/V2_6_X_Code/src/config.h\0?? fsw/V2_6_X_Tests/bench/sensor_bench/sensor_bench.ino\0 M fsw/V2_6_X_Tests/test/test_phase.cpp\0R  fsw/V2_6_X_Code/src/New.h\0fsw/V2_6_X_Code/src/Old.h\0 M fsw/README.md\0";
        assert_eq!(
            changed_sources(status, "fsw/"),
            vec![
                "V2_6_X_Code/src/New.h".to_string(),
                "V2_6_X_Code/src/config.h".to_string(),
                "V2_6_X_Tests/bench/sensor_bench/sensor_bench.ino".to_string(),
            ]
        );
        assert!(changed_sources(b"", "fsw/").is_empty());
    }

    #[test]
    fn a_tree_outside_any_checkout_has_a_source_hash_and_no_commit() {
        let dir = fake_tree("nogit");
        // temp dir could be inside a checkout
        let inside = std::process::Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "HEAD"]).output().map(|o| o.status.success()).unwrap_or(false);
        let o = origin(&dir);
        assert_eq!(o.source, source_hash(&dir, &sources(&dir)));
        if !inside {
            assert_eq!((o.commit, o.dirty), (None, None));
            assert!(o.changed.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_is_the_flight_software_only_with_both_marks() {
        let dir = std::env::temp_dir().join("fswbuild-test-tree");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_tree(&dir));
        std::fs::write(dir.join("build.sh"), "#!/bin/sh\n").unwrap();
        assert!(!is_tree(&dir), "build.sh alone is not the flight software");
        std::fs::create_dir_all(dir.join("V2_6_X_Code")).unwrap();
        assert!(is_tree(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
