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

/// Runs the build for one radio module and returns the image. About 13 s, because changing a -D
/// forces a fresh configure rather than reusing the last build's objects. The board's settings are
/// not compiled in: the built image carries a settings block with the repo's defaults, and the
/// caller patches that block, the same way it patches the image the app carries. One way of
/// writing settings, so a damaged block always falls back to the same thing, the repo's defaults
pub fn build(at: &Path, module: &str) -> Result<Vec<u8>, String> {
    let module = match module {
        "le" => 1,
        "hp" => 2,
        other => return Err(format!("{:?} is not a radio module; it is le or hp", other)),
    };
    let out = std::process::Command::new("./build.sh")
        .args(["build", &format!("-DCHIPSAT_RADIO_MODULE={}", module)])
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
        assert!(build(Path::new("/nonexistent"), "xp").unwrap_err().contains("not a radio module"));
    }

    #[test]
    fn a_missing_tree_says_which_path_it_tried() {
        let e = build(Path::new("/nonexistent"), "le").unwrap_err();
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
