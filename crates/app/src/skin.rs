//! Toolbar skins: IDM-style icon packs delivered as a `.zip`.
//!
//! A pack holds a `.tbi` text file (`name=`, `large=`, `largeHot=`, `small=`, `smallHot=`) and the BMP
//! strips it points at. A strip is [`SLOTS`] icons side by side (Add URL, Resume, Stop, Stop All,
//! Delete, Delete Completed, Options, Scheduler, Start Queue, Stop Queue, Grabber, Tell a Friend);
//! the "Hot" strip is what is shown while the mouse is over a button.
//!
//! Nothing from the zip is extracted by path: the entries are read into memory, decoded, and only
//! then stored under fixed names in `<data dir>/skin/`, so a hostile archive cannot write elsewhere.
use eframe::egui::ColorImage;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Icons in one strip.
pub const SLOTS: usize = 12;
/// Smallest icon edge accepted (px); anything below is certainly not a toolbar strip.
const MIN_SLOT: usize = 8;
/// Largest strip dimension accepted (px), to keep a bad file from eating memory.
const MAX_DIM: usize = 16_384;
/// Largest single file read out of the zip.
const MAX_FILE: u64 = 32 * 1024 * 1024;

pub struct Skin {
    pub name: String,
    /// One image per slot, [`SLOTS`] entries.
    pub normal: Vec<ColorImage>,
    /// Same layout, shown on hover. `None` when the pack has no (usable) hot strip.
    pub hot: Option<Vec<ColorImage>>,
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("skin")
}

/// Skin saved by an earlier run, if any.
pub fn load_saved(data_dir: &Path) -> Option<Skin> {
    let d = dir(data_dir);
    let normal = fs::read(d.join("normal.bmp")).ok()?;
    let hot = fs::read(d.join("hot.bmp")).ok();
    let name = fs::read_to_string(d.join("name.txt")).unwrap_or_default();
    build(name.trim(), &normal, hot.as_deref()).ok()
}

/// Back to the built-in icons.
pub fn clear(data_dir: &Path) {
    let _ = fs::remove_dir_all(dir(data_dir));
}

/// Reads a skin pack, validates it, and remembers it for the next start.
pub fn import_zip(zip_path: &Path, data_dir: &Path) -> Result<Skin, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("cannot open the file ({e})"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "this is not a valid .zip file".to_string())?;

    // base name (lower-case) -> entry index; folders inside the zip are ignored
    let mut by_name: Vec<(String, usize)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index(i) else { continue };
        if f.is_dir() {
            continue;
        }
        let base = f.name().rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
        if !base.is_empty() {
            by_name.push((base, i));
        }
    }
    let find = |name: &str| by_name.iter().find(|(n, _)| n == &name.to_lowercase()).map(|(_, i)| *i);
    let mut read = |idx: usize| -> Result<Vec<u8>, String> {
        let f = zip.by_index(idx).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        f.take(MAX_FILE).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    };

    let mut name = zip_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut normal_file, mut hot_file) = (None::<String>, None::<String>);

    if let Some(tbi) = by_name.iter().find(|(n, _)| n.ends_with(".tbi")).map(|(_, i)| *i) {
        let text = String::from_utf8_lossy(&read(tbi)?).into_owned();
        let (mut large, mut large_hot, mut small, mut small_hot) = (None, None, None, None);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().trim_matches('"').to_string();
            if v.is_empty() {
                continue;
            }
            match k.trim().to_lowercase().as_str() {
                "name" => name = v,
                "large" => large = Some(v),
                "largehot" => large_hot = Some(v),
                "small" => small = Some(v),
                "smallhot" => small_hot = Some(v),
                _ => {}
            }
        }
        normal_file = large.or(small);
        hot_file = large_hot.or(small_hot);
    }
    if normal_file.is_none() {
        // no (usable) .tbi: take the BMPs by name, "...hot..." being the hover strip
        let bmps: Vec<&String> = by_name.iter().map(|(n, _)| n).filter(|n| n.ends_with(".bmp")).collect();
        hot_file = bmps.iter().find(|n| n.contains("hot")).map(|n| n.to_string());
        normal_file = bmps.iter().find(|n| !n.contains("hot")).map(|n| n.to_string());
    }
    let normal_file = normal_file.ok_or("no .tbi file or .bmp strip found in the zip")?;
    let normal_idx = find(&normal_file).ok_or_else(|| format!("{normal_file} is listed in the skin but missing from the zip"))?;
    let normal = read(normal_idx)?;
    let hot = match hot_file.and_then(|h| find(&h)) {
        Some(i) => Some(read(i)?),
        None => None,
    };

    let skin = build(&name, &normal, hot.as_deref())?;

    // validated: remember it
    let d = dir(data_dir);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).map_err(|e| format!("cannot save the skin ({e})"))?;
    let save = |n: &str, b: &[u8]| fs::write(d.join(n), b).map_err(|e| format!("cannot save the skin ({e})"));
    save("normal.bmp", &normal)?;
    if let (Some(h), Some(_)) = (&hot, &skin.hot) {
        save("hot.bmp", h)?;
    }
    save("name.txt", skin.name.as_bytes())?;
    Ok(skin)
}

fn build(name: &str, normal: &[u8], hot: Option<&[u8]>) -> Result<Skin, String> {
    let n = decode_bmp(normal).map_err(|e| format!("toolbar image: {e}"))?;
    let normal = slice(&n)?;
    // a hot strip that is broken or of another size is simply not used
    let hot = hot
        .and_then(|b| decode_bmp(b).ok())
        .filter(|h| h.w == n.w && h.h == n.h)
        .and_then(|h| slice(&h).ok());
    let name = if name.is_empty() { "Custom".to_string() } else { name.to_string() };
    Ok(Skin { name, normal, hot })
}

struct Rgba {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

/// Cuts the strip into [`SLOTS`] equally wide icons.
fn slice(img: &Rgba) -> Result<Vec<ColorImage>, String> {
    let sw = img.w / SLOTS;
    if sw < MIN_SLOT || img.h < MIN_SLOT {
        return Err(format!(
            "the strip is {}x{} px; a toolbar strip needs {SLOTS} icons side by side (e.g. 672x42)",
            img.w, img.h
        ));
    }
    Ok((0..SLOTS)
        .map(|i| {
            let mut out = Vec::with_capacity(sw * img.h * 4);
            for y in 0..img.h {
                let start = (y * img.w + i * sw) * 4;
                out.extend_from_slice(&img.px[start..start + sw * 4]);
            }
            ColorImage::from_rgba_unmultiplied([sw, img.h], &out)
        })
        .collect())
}

fn le16(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated BMP".to_string())
}
fn le32(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated BMP".to_string())
}

/// Minimal BMP reader: uncompressed 8-bit (palette), 24-bit and 32-bit, which covers toolbar skins.
/// 32-bit files use their alpha channel; the others use magenta (255,0,255) – or, when the picture
/// has none, the colour of its top-left pixel – as the transparent colour.
fn decode_bmp(b: &[u8]) -> Result<Rgba, String> {
    if b.get(0..2) != Some(b"BM") {
        return Err("not a BMP file".into());
    }
    let off = le32(b, 10)? as usize;
    let hdr = le32(b, 14)? as usize;
    if hdr < 40 {
        return Err("old-style BMP header is not supported".into());
    }
    let w = le32(b, 18)? as i32;
    let h = le32(b, 22)? as i32;
    let bpp = le16(b, 28)? as usize;
    let comp = le32(b, 30)?;
    if comp != 0 && comp != 3 {
        return Err("compressed BMP is not supported (save it uncompressed)".into());
    }
    if !matches!(bpp, 8 | 24 | 32) {
        return Err(format!("{bpp}-bit BMP is not supported (use 24-bit or 32-bit)"));
    }
    let top_down = h < 0;
    let (w, h) = (w.unsigned_abs() as usize, h.unsigned_abs() as usize);
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        return Err("bad BMP size".into());
    }
    let row = (w * bpp).div_ceil(32) * 4;
    let need = off.checked_add(row.checked_mul(h).ok_or("bad BMP size")?).ok_or("bad BMP size")?;
    if need > b.len() {
        return Err("truncated BMP".into());
    }

    let mut palette: Vec<[u8; 3]> = Vec::new();
    if bpp == 8 {
        let used = le32(b, 46)? as usize;
        let count = if used == 0 || used > 256 { 256 } else { used };
        for i in 0..count {
            let o = 14 + hdr + i * 4;
            let e = b.get(o..o + 3).ok_or("truncated BMP palette")?;
            palette.push([e[2], e[1], e[0]]);
        }
    }

    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        let src = if top_down { y } else { h - 1 - y };
        let line = &b[off + src * row..off + (src + 1) * row];
        for x in 0..w {
            let o = (y * w + x) * 4;
            match bpp {
                32 => px[o..o + 4].copy_from_slice(&[line[x * 4 + 2], line[x * 4 + 1], line[x * 4], line[x * 4 + 3]]),
                24 => px[o..o + 4].copy_from_slice(&[line[x * 3 + 2], line[x * 3 + 1], line[x * 3], 255]),
                _ => {
                    let c = palette.get(line[x] as usize).copied().unwrap_or([0, 0, 0]);
                    px[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
    }

    let has_alpha = bpp == 32 && px.chunks_exact(4).any(|p| p[3] != 0);
    if bpp == 32 && !has_alpha {
        // the fourth byte is unused padding in this file
        px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    if has_alpha {
        // Windows often stores alpha pre-multiplied (no colour channel above its alpha): undo that
        let premultiplied = px.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
            && px.chunks_exact(4).any(|p| p[3] != 0 && p[3] != 255);
        if premultiplied {
            for p in px.chunks_exact_mut(4) {
                if p[3] != 0 && p[3] != 255 {
                    let a = p[3] as u32;
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
        }
    } else {
        let magenta = |p: &[u8]| p[0] == 255 && p[1] == 0 && p[2] == 255;
        let key = if px.chunks_exact(4).any(magenta) { [255, 0, 255] } else { [px[0], px[1], px[2]] };
        for p in px.chunks_exact_mut(4) {
            if p[..3] == key {
                p.copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Ok(Rgba { w, h, px })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A `SLOTS*sw` x `h` bottom-up BMP, every pixel `colour` (B,G,R,A order given as RGBA here).
    fn bmp(sw: usize, h: usize, bpp: u16, rgba: [u8; 4]) -> Vec<u8> {
        let w = sw * SLOTS;
        let bytes = (bpp / 8) as usize;
        let row = (w * bpp as usize).div_ceil(32) * 4;
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&((54 + row * h) as u32).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&54u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&(w as i32).to_le_bytes());
        v.extend_from_slice(&(h as i32).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            let mut line = Vec::new();
            for _ in 0..w {
                line.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]][..bytes]);
            }
            line.resize(row, 0);
            v.extend_from_slice(&line);
        }
        v
    }

    fn zip_of(files: &[(&str, Vec<u8>)]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("skin-test-{}-{}.zip", std::process::id(), files.len()));
        let mut z = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (n, b) in files {
            z.start_file(*n, zip::write::FileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
        path
    }

    #[test]
    fn imports_a_pack_and_reloads_it() {
        let tbi = b"v=3\r\nname=Test\r\nlarge=a.bmp\r\nlargeHot=b.bmp\r\nsmall=a.bmp\r\nsmallHot=b.bmp".to_vec();
        let zip = zip_of(&[
            ("pack/skin.tbi", tbi),
            ("pack/A.BMP", bmp(10, 9, 32, [10, 20, 30, 200])),
            ("pack/b.bmp", bmp(10, 9, 32, [60, 70, 80, 255])),
        ]);
        let data = std::env::temp_dir().join(format!("skin-data-{}", std::process::id()));
        let s = import_zip(&zip, &data).unwrap();
        assert_eq!(s.name, "Test");
        assert_eq!(s.normal.len(), SLOTS);
        assert_eq!(s.normal[0].size, [10, 9]);
        assert!(s.hot.is_some());

        let again = load_saved(&data).expect("saved skin loads");
        assert_eq!(again.name, "Test");
        assert_eq!(again.normal[11].size, [10, 9]);
        clear(&data);
        assert!(load_saved(&data).is_none());
    }

    #[test]
    fn rejects_things_that_are_not_a_strip() {
        let data = std::env::temp_dir().join("skin-data-reject");
        let zip = zip_of(&[("readme.txt", b"hi".to_vec())]);
        assert!(import_zip(&zip, &data).is_err());
        let zip = zip_of(&[("x.bmp", bmp(2, 9, 24, [1, 2, 3, 255])), ("y.txt", vec![])]);
        let e = import_zip(&zip, &data).err().unwrap();
        assert!(e.contains("icons side by side"), "{e}");
    }

    #[test]
    fn magenta_is_transparent_in_24_bit() {
        let img = decode_bmp(&bmp(8, 8, 24, [255, 0, 255, 255])).unwrap();
        assert!(img.px.chunks_exact(4).all(|p| p[3] == 0));
    }
}
