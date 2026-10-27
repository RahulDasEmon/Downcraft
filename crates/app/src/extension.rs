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
    Err("Dragging out of the window is not supported on this system - use \"Save zip as…\" instead.".into())
}

pub const CAN_DRAG: bool = cfg!(any(windows, target_os = "macos"));
