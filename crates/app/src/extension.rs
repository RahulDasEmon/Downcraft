//! The browser extension travels inside the app as a zip (built by `build.rs` from `extension/`).
//! The Extension page writes it to disk and lets the user drag it straight onto `chrome://extensions`.
use std::path::{Path, PathBuf};

pub const ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/downcraft-extension.zip"));
pub const FILE_NAME: &str = "downcraft-extension.zip";

/// Writes the zip into `dir` (only when it is missing or out of date) and returns its path.
pub fn export(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join(FILE_NAME);
    if std::fs::read(&p).map_or(true, |b| b != ZIP) {
        std::fs::write(&p, ZIP)?;
    }
    Ok(p)
}

pub const FOLDER_NAME: &str = "downcraft-extension";

/// Unpacks the extension into `dir/downcraft-extension/` (only when missing or out of date) and
/// returns that folder. Where a file cannot be dragged onto the browser (Linux), the extension is
/// installed with the browser's "Load unpacked" button, which needs exactly this folder.
pub fn unpack(dir: &Path) -> std::io::Result<PathBuf> {
    let io_err = |e: zip::result::ZipError| std::io::Error::new(std::io::ErrorKind::Other, e.to_string());
    let out = dir.join(FOLDER_NAME);
    let stamp = dir.join(".downcraft-extension.stamp");
    // cheap fingerprint of the embedded zip (FNV-1a)
    let fp = ZIP.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let want = format!("{fp:016x}");
    if out.join("manifest.json").is_file() && std::fs::read_to_string(&stamp).map_or(false, |s| s == want) {
        return Ok(out);
    }
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(ZIP)).map_err(io_err)?;
    for i in 0..ar.len() {
        let mut f = ar.by_index(i).map_err(io_err)?;
        let rel = match f.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let dest = out.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut f, &mut std::fs::File::create(&dest)?)?;
    }
    std::fs::write(&stamp, want)?;
    Ok(out)
}

/// Small PNG shown under the cursor while dragging.
#[cfg(any(windows, target_os = "macos"))]
fn preview() -> Vec<u8> {
    use image::{imageops::FilterType, ImageOutputFormat};
    let mut out = std::io::Cursor::new(Vec::new());
    if let Ok(img) = image::load_from_memory(crate::LOGO) {
        let _ = img.resize(72, 72, FilterType::Lanczos3).write_to(&mut out, ImageOutputFormat::Png);
    }
    out.into_inner()
}

/// Starts a native file drag (Windows / macOS) so a file (the extension zip, or a finished download)
/// can be dropped on the browser, a folder or another app.
#[cfg(any(windows, target_os = "macos"))]
pub fn start_drag(frame: &eframe::Frame, file: &Path) -> Result<(), String> {
    drag::start_drag(
        frame,
        drag::DragItem::Files(vec![file.to_path_buf()]),
        drag::Image::Raw(preview()),
        |_result, _pos| {},
        drag::Options::default(),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn start_drag(_frame: &eframe::Frame, _file: &Path) -> Result<(), String> {
    Err("Dragging out of the window is not supported on this system - use \"Load unpacked\" instead.".into())
}

pub const CAN_DRAG: bool = cfg!(any(windows, target_os = "macos"));
