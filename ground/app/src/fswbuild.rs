//! Builds the flight software for one board, with that board's settings compiled into it.
//!
//! Which board it is, which radio module is on it, the spreading factor, the transmit power and the
//! transmit interval are compile-time constants in the flight software. Rather than carry one
//! prebuilt image and edit it, this runs the flight software's own build with those values as -D
//! flags and flashes what comes out.
//!
//! The build is also what validates the settings. A power the module has no row for does not produce
//! a quietly wrong image: it fails to compile, with a sentence saying which powers that module has.
//! So this does no checking of its own beyond the ranges needed to build a command line, and hands
//! the compiler's own words back when it refuses.

use std::path::{Path, PathBuf};

pub struct Settings {
    pub csid: u16,
    pub module: String,
    pub sf: u16,
    pub dbm: i16,
    pub interval: Option<u32>,
}

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

/// Runs the build and returns the image and a label naming what was built. Takes about 13 s, because
/// changing a -D forces a fresh configure rather than reusing the last build's objects.
pub fn check(s: &Settings) -> Result<u8, String> {
    let module = match s.module.as_str() {
        "le" => 1,
        "hp" => 2,
        other => return Err(format!("{:?} is not a radio module; it is le or hp", other)),
    };
    if s.csid == 0 || s.csid > 255 {
        return Err(String::from("the board id is 1 to 255, and 255 means nobody set it"));
    }
    if s.sf < 5 || s.sf > 12 {
        return Err(String::from("the spreading factor is 5 to 12"));
    }
    Ok(module)
}

/// What the build will be called, before it has run, so a job can name what it is doing.
pub fn words(s: &Settings) -> String {
    format!(
        "a flight image for board {} on the {} module, SF{} at {} dBm",
        s.csid,
        s.module.to_uppercase(),
        s.sf,
        s.dbm
    )
}

pub fn build(at: &Path, s: &Settings) -> Result<(Vec<u8>, String), String> {
    let module = check(s)?;

    let mut args: Vec<String> = vec![
        String::from("build"),
        format!("-DCHIPSAT_ID={}", s.csid),
        format!("-DCHIPSAT_RADIO_MODULE={}", module),
        format!("-DCHIPSAT_SPREADING_FACTOR={}", s.sf),
        format!("-DCHIPSAT_TARGET_DBM={}", s.dbm),
    ];
    if let Some(ms) = s.interval {
        args.push(format!("-DCHIPSAT_TX_INTERVAL_MS={}", ms));
    }

    let out = std::process::Command::new("./build.sh")
        .args(&args)
        .current_dir(at)
        .output()
        .map_err(|e| format!("cannot run {}/build.sh: {}", at.display(), e))?;

    if !out.status.success() {
        return Err(format!(
            "the flight software did not build with those settings.\n{}",
            refusal(&out.stderr, &out.stdout)
        ));
    }

    let bin = at.join("build").join("chipsat.bin");
    let bytes = std::fs::read(&bin)
        .map_err(|e| format!("the build reported success but {} cannot be read: {}", bin.display(), e))?;
    if bytes.is_empty() {
        return Err(format!("{} is empty", bin.display()));
    }
    Ok((bytes, words(s)))
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
    fn a_module_that_is_not_le_or_hp_is_refused_before_anything_runs() {
        let s = Settings { csid: 9, module: String::from("xp"), sf: 7, dbm: 14, interval: None };
        assert!(check(&s).unwrap_err().contains("not a radio module"));
        assert!(build(Path::new("/nonexistent"), &s).unwrap_err().contains("not a radio module"));
    }

    #[test]
    fn a_spreading_factor_the_radio_does_not_have_is_refused_without_a_build() {
        for sf in [4u16, 13, 24] {
            let s = Settings { csid: 9, module: String::from("le"), sf, dbm: 14, interval: None };
            assert!(check(&s).unwrap_err().contains("5 to 12"), "SF{}", sf);
        }
    }

    #[test]
    fn the_words_name_the_board_and_the_module_before_the_build_runs() {
        let s = Settings { csid: 12, module: String::from("hp"), sf: 9, dbm: 20, interval: Some(0) };
        let w = words(&s);
        assert!(w.contains("board 12") && w.contains("HP") && w.contains("SF9") && w.contains("20 dBm"), "{}", w);
    }

    #[test]
    fn board_id_0_is_refused_because_it_reads_as_a_board_that_exists() {
        let s = Settings { csid: 0, module: String::from("le"), sf: 7, dbm: 14, interval: None };
        assert!(check(&s).unwrap_err().contains("board id"));
    }

    #[test]
    fn a_missing_tree_says_which_path_it_tried() {
        let s = Settings { csid: 9, module: String::from("le"), sf: 7, dbm: 14, interval: None };
        let e = build(Path::new("/nonexistent"), &s).unwrap_err();
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
