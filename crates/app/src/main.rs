#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod clipboard;
mod dialogs;
mod extension;
mod icons;
mod options;
mod single;
mod skin;
mod sys;
mod theme;
mod titlebar;
mod tray;
mod widgets;

const LOGO: &[u8] = include_bytes!("../../../assets/logo-256.png");
/// White brand glyphs (simple-icons, CC0) for the About page buttons; tinted at draw time.
const ICON_GITHUB: &[u8] = include_bytes!("../../../assets/icon-github.png");
const ICON_DISCORD: &[u8] = include_bytes!("../../../assets/icon-discord.png");

fn main() -> eframe::Result<()> {
    // a second launch just raises the running window instead of adding another tray icon
    let listener = match single::acquire() {
        single::Guard::First(l) => l,
        single::Guard::Already => return Ok(()),
    };
    let img = image::load_from_memory(LOGO).expect("logo").to_rgba8();
    let (w, h) = img.dimensions();
    let icon = eframe::egui::IconData { rgba: img.into_raw(), width: w, height: h };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("DownCraft")
            .with_decorations(false) // no native title bar: see titlebar.rs
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([860.0, 420.0])
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("DownCraft", options, Box::new(move |cc| {
        single::listen(listener, cc.egui_ctx.clone());
        Box::new(app::App::new(cc))
    }))
}
