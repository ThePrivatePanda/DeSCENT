// A map that works with no internet. The browser fetches tiles while there is a
// connection and posts them here; this keeps them on disk next to the log files and
// serves them back, so the same page draws a map in a field with no signal.
//
// The app does no fetching of its own on purpose: no HTTP client, no TLS stack, and
// nothing in the binary that talks to the outside world.
//
// Each source is its own layer, /tiles/{layer}/z/x/y. Tiles from before layers sit at
// /tiles/z/x/y and are still served, never written.

use std::path::{Path, PathBuf};

pub const MAX_ZOOM: u32 = 19;
const MAX_TILE_BYTES: usize = 512 * 1024;
pub const LAYERS: [&str; 3] = ["imagery-usgs", "imagery-esri", "terrain"];
const KINDS: [(&str, &str); 3] = [("png", "image/png"), ("jpg", "image/jpeg"), ("webp", "image/webp")];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub layer: Option<&'static str>,   // None for the unlayered tiles from before
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

pub fn dir(base: &Path) -> PathBuf {
    base.join("tiles")
}

// A tile from a path like /tiles/imagery-usgs/14/4823/6160 or the older /tiles/14/4823/6160.png,
// refusing anything that could escape the tile directory, a layer not on the list, or a
// tile that cannot exist at that zoom.
pub fn parse(path: &str) -> Option<Tile> {
    let rest = path.strip_prefix("/tiles/")?;
    let rest = rest.strip_suffix(".png").unwrap_or(rest);
    let mut parts: Vec<&str> = rest.split('/').collect();
    let layer = match parts.first() {
        Some(p) if p.parse::<u32>().is_err() => Some(*LAYERS.iter().find(|l| *l == p)?),
        _ => None,
    };
    if layer.is_some() {
        parts.remove(0);
    }
    let [z, x, y] = parts[..] else { return None };
    let (z, x, y): (u32, u32, u32) = (z.parse().ok()?, x.parse().ok()?, y.parse().ok()?);
    if z > MAX_ZOOM {
        return None;
    }
    let side = 1u32 << z;
    if x >= side || y >= side {
        return None;
    }
    Some(Tile { layer, z, x, y })
}

// The file without its extension: the extension says what kind of image it is
fn stem(base: &Path, t: &Tile) -> PathBuf {
    let mut p = dir(base);
    if let Some(l) = t.layer {
        p.push(l);
    }
    p.join(t.z.to_string()).join(t.x.to_string()).join(t.y.to_string())
}

// The bytes and their content type. The older tiles were all written as .png whatever they
// held, so the type is read off the bytes.
pub fn read(base: &Path, t: &Tile) -> Option<(Vec<u8>, &'static str)> {
    let stem = stem(base, t);
    let bytes = KINDS.iter().find_map(|(ext, _)| std::fs::read(stem.with_extension(ext)).ok())?;
    let kind = image_kind(&bytes).map_or("application/octet-stream", |k| k.1);
    Some((bytes, kind))
}

// PNG, JPEG or WebP. A tile server that answers an error page in HTML must not be
// written to disk, or the map shows that page forever after.
fn image_kind(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(KINDS[0])
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(KINDS[1])
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(KINDS[2])
    } else {
        None
    }
}

pub fn write(base: &Path, t: &Tile, bytes: &[u8]) -> Result<(), String> {
    if t.layer.is_none() {
        return Err(String::from("a tile is posted under its layer"));
    }
    if bytes.is_empty() {
        return Err(String::from("empty tile"));
    }
    if bytes.len() > MAX_TILE_BYTES {
        return Err(format!("tile is {} bytes, larger than a map tile should be", bytes.len()));
    }
    let (ext, _) = image_kind(bytes).ok_or_else(|| String::from("not an image, so probably an error page"))?;
    let p = stem(base, t).with_extension(ext);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, bytes).map_err(|e| e.to_string())
}

fn walk(p: &Path, count: &mut u64, bytes: &mut u64) {
    if let Ok(entries) = std::fs::read_dir(p) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, count, bytes);
            } else if path.extension().is_some_and(|x| KINDS.iter().any(|(k, _)| x == *k)) {
                *count += 1;
                *bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
}

// What is already stored, in all and per layer, so the page can say whether the site is
// covered and roughly what saving more will take.
pub fn stored(base: &Path) -> ((u64, u64), Vec<(&'static str, u64, u64)>) {
    let mut all = (0, 0);
    walk(&dir(base), &mut all.0, &mut all.1);
    let layers = LAYERS
        .iter()
        .map(|l| {
            let (mut count, mut bytes) = (0, 0);
            walk(&dir(base).join(l), &mut count, &mut bytes);
            (*l, count, bytes)
        })
        .collect();
    (all, layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(layer: Option<&'static str>, z: u32, x: u32, y: u32) -> Option<Tile> {
        Some(Tile { layer, z, x, y })
    }

    #[test]
    fn parses_a_tile_path() {
        assert_eq!(parse("/tiles/14/4823/6160.png"), tile(None, 14, 4823, 6160));
        assert_eq!(parse("/tiles/0/0/0.png"), tile(None, 0, 0, 0));
        assert_eq!(parse("/tiles/14/4823/6160"), tile(None, 14, 4823, 6160));
        assert_eq!(parse("/tiles/imagery-usgs/19/153400/193300"), tile(Some("imagery-usgs"), 19, 153400, 193300));
        assert_eq!(parse("/tiles/terrain/15/9590/12083"), tile(Some("terrain"), 15, 9590, 12083));
        assert_eq!(parse("/tiles/imagery-esri/0/0/0"), tile(Some("imagery-esri"), 0, 0, 0));
    }

    #[test]
    fn refuses_anything_that_is_not_a_tile() {
        assert_eq!(parse("/tiles/14/4823.png"), None, "too few parts");
        assert_eq!(parse("/tiles/14/4823/6160/1.png"), None, "too many parts");
        assert_eq!(parse("/tiles/20/1/1.png"), None, "past the deepest zoom");
        assert_eq!(parse("/tiles/1/2/0.png"), None, "x does not exist at zoom 1");
        assert_eq!(parse("/tiles/../../etc/passwd"), None);
        assert_eq!(parse("/tiles/14/-1/0.png"), None);
        assert_eq!(parse("/api/tiles"), None);
        assert_eq!(parse("/tiles/osm/14/1/1"), None, "a layer not on the list");
        assert_eq!(parse("/tiles/../14/1/1"), None);
        assert_eq!(parse("/tiles/terrain/14/1"), None, "a layer and too few parts");
        assert_eq!(parse("/tiles/terrain/terrain/14/1/1"), None);
        assert_eq!(parse("/tiles/terrain/20/1/1"), None);
        assert_eq!(parse("/tiles/imagery-usgs/1/0/2"), None, "y does not exist at zoom 1");
        assert_eq!(parse("/tiles/IMAGERY-USGS/1/0/0"), None);
    }

    #[test]
    fn only_images_are_stored_each_under_its_layer() {
        let dir = std::env::temp_dir().join(format!("dg-tiles-{}", std::process::id()));
        let png = [0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0];
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        let usgs = Tile { layer: Some("imagery-usgs"), z: 14, x: 1, y: 2 };
        let terrain = Tile { layer: Some("terrain"), z: 14, x: 1, y: 2 };
        assert!(write(&dir, &usgs, &jpg).is_ok());
        assert!(write(&dir, &terrain, &png).is_ok());
        assert_eq!(read(&dir, &usgs), Some((jpg.to_vec(), "image/jpeg")));
        assert_eq!(read(&dir, &terrain), Some((png.to_vec(), "image/png")), "same z/x/y, another layer");
        let esri = Tile { layer: Some("imagery-esri"), z: 14, x: 1, y: 3 };
        assert!(write(&dir, &esri, b"<html>rate limited</html>").is_err(), "an error page is not a tile");
        assert!(write(&dir, &esri, b"").is_err());
        assert_eq!(read(&dir, &esri), None);
        assert!(write(&dir, &Tile { layer: None, z: 14, x: 1, y: 2 }, &png).is_err(), "nothing new goes in the old place");

        // a tile from before layers, JPEG under a .png name as the old write left them
        std::fs::create_dir_all(dir.join("tiles/14/1")).unwrap();
        std::fs::write(dir.join("tiles/14/1/5.png"), jpg).unwrap();
        assert_eq!(read(&dir, &parse("/tiles/14/1/5.png").unwrap()), Some((jpg.to_vec(), "image/jpeg")));

        let ((count, bytes), layers) = stored(&dir);
        assert_eq!((count, bytes), (3, (2 * jpg.len() + png.len()) as u64), "the three real tiles, the old one included");
        assert_eq!(layers, vec![("imagery-usgs", 1, jpg.len() as u64), ("imagery-esri", 0, 0), ("terrain", 1, png.len() as u64)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
