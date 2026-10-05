// The receiver firmware is embedded when a merged image has been built. Until one
// exists the app still compiles and everything but flashing works.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// The check the app makes before it writes a receiver image, from the app's own source.
#[path = "src/receiver_image.rs"]
mod receiver_image;

fn main() {
    // include_dir! reads web/ when the macro expands and does not tell cargo about it,
    // so without this a build after only changing the dashboard keeps the old copy
    // inside the binary. It bit once already: the app served a UI that no longer
    // existed in the tree.
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web");
    println!("cargo:rerun-if-changed={}", web.display());

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../receiver/firmware");
    println!("cargo:rerun-if-changed={}", dir.display());

    // The newest image the app would agree to write. The newest file is not always one:
    // a release can leave its bare app image in this folder too, and embedding that
    // gives an app whose own flash button refuses its own firmware. Whatever is passed
    // over is said, so a release that only has the bare image does not go unnoticed.
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "bin").unwrap_or(false) {
                let name = e.file_name().to_string_lossy().to_string();
                match std::fs::read(&p) {
                    Ok(bytes) => images.push((name, bytes)),
                    Err(e) => println!("cargo:warning=receiver/firmware/{} is not embedded: it cannot be read ({})", name, e),
                }
            }
        }
    }
    let (carried, skipped) = receiver_image::newest_whole(&images);
    for (name, why) in &skipped {
        println!("cargo:warning=receiver/firmware/{} is not embedded: {}", name, why);
    }
    let newest: Option<PathBuf> = carried.map(|name| dir.join(name));

    // Stamped so the program can notice it is older than the tree it was built from, and
    // say which build it is. Every build has the same version number, so this is the only
    // thing that tells two of them apart. cargo only runs this script again for the paths
    // named, so the program's own source is named too: without it a change to the source
    // alone would be built under the stamp of the build before.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    let built = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=DG_BUILT={}", built);

    // What the app carries for a ChipSat. Each file is optional: one that is missing
    // compiles to None and the part of the app that wanted it says so, and dropping the
    // file in later is all it takes.
    //
    // The folder is watched so that adding or removing a file is noticed, and each file
    // that exists is watched as well. The folder alone is not enough: replacing an image
    // in place leaves the folder's own timestamp untouched, so a rebuilt flight image
    // would not reach the binary and the app would carry the old one while reporting the
    // new hash from flight.json. A file that does NOT exist is deliberately not watched,
    // because cargo reruns this script every time for a path it cannot find.
    let chipsat = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../chipsat/firmware");
    println!("cargo:rerun-if-changed={}", chipsat.display());
    for file in ["flash_dump.bin", "flight-le.bin", "flight-hp.bin", "flight.json", "radio.json"] {
        let at = chipsat.join(file);
        if at.is_file() {
            println!("cargo:rerun-if-changed={}", at.display());
        }
    }

    // The dump sketch, so a pull needs nothing installed. It has no version of its own,
    // so the start of its sha256 is compiled in and shown: the flight repo publishes the
    // same hash, and a copy that has fallen behind the sketch can be told by comparing.
    let (dump, dump_sha) = image(&chipsat, "flash_dump.bin", "CHIPSAT_DUMP");
    let dump_id = format!("pub static CHIPSAT_DUMP_ID: &str = {:?};\n", dump_sha.get(..16).unwrap_or(""));

    // One flight image per module, and the flight repo's own account of them. Nothing in
    // flight.json is read here; the app reads it when it starts.
    let mut flight = String::new();
    for (file, name) in [("flight-le.bin", "CHIPSAT_FLIGHT_LE"), ("flight-hp.bin", "CHIPSAT_FLIGHT_HP")] {
        let (decl, sha) = image(&chipsat, file, name);
        flight.push_str(&decl);
        flight.push_str(&format!("pub static {}_SHA256: &str = {:?};\n", name, sha));
    }
    let flight_json = text(&chipsat, "flight.json", "CHIPSAT_FLIGHT_JSON");
    let radio = text(&chipsat, "radio.json", "CHIPSAT_RADIO");
    let chip_decl = [dump, dump_id, flight, flight_json, radio].concat();

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("firmware.rs");
    let body = match &newest {
        Some(p) => format!(
            "pub static IMAGE: Option<&[u8]> = Some(include_bytes!({:?}));\npub static IMAGE_NAME: &str = {:?};\n",
            p,
            p.file_name().unwrap().to_string_lossy()
        ),
        None => String::from("pub static IMAGE: Option<&[u8]> = None;\npub static IMAGE_NAME: &str = \"\";\n"),
    };
    std::fs::write(out, format!("{}{}", body, chip_decl)).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

// A binary the app may carry: the declaration that holds it, and its sha256. None and
// an empty hash when the file is not there.
fn image(dir: &Path, file: &str, name: &str) -> (String, String) {
    match std::fs::read(dir.join(file)) {
        Ok(bytes) => (
            format!("pub static {}: Option<&[u8]> = Some(include_bytes!({:?}));\n", name, dir.join(file)),
            sha256_hex(&bytes),
        ),
        Err(_) => (format!("pub static {}: Option<&[u8]> = None;\n", name), String::new()),
    }
}

fn text(dir: &Path, file: &str, name: &str) -> String {
    if dir.join(file).is_file() {
        format!("pub static {}: Option<&str> = Some(include_str!({:?}));\n", name, dir.join(file))
    } else {
        format!("pub static {}: Option<&str> = None;\n", name)
    }
}
