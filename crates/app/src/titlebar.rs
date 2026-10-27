//! Custom window chrome (the window has no native title bar): a draggable title bar with the logo
//! and the minimise / maximise / close buttons, plus a hairline border and resizing from the
//! window's edges and corners. Same approach as the ArtCraft launcher.
use crate::theme::*;
use eframe::egui::{
    self, pos2, vec2, Color32, Context, CursorIcon, Frame, Margin, Rect, Response, Rounding, Sense, Stroke,
    TextureHandle, Ui,
};

pub const HEIGHT: f32 = 32.0;
/// How far in from the window's edges the window can be resized; the bar's controls leave it alone.
const EDGE: f32 = 5.0;
/// Corner radius of Windows 11 windows; 0 where the system cannot round them (the border then stays square).
fn radius() -> f32 {
    if ROUNDED.load(std::sync::atomic::Ordering::Relaxed) {
        8.0
    } else {
        0.0
    }
}
static ROUNDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks Windows 11 to round the corners of every window of this process (the main window and the
/// pop-ups have no native frame, so they would otherwise be sharp rectangles). Cheap enough to call
/// every frame: a window already rounded is left alone. A no-op elsewhere.
#[cfg(windows)]
pub fn round_windows() {
    use std::sync::atomic::Ordering::Relaxed;
    use windows_sys::Win32::{
        Foundation::{BOOL, HWND, LPARAM},
        Graphics::Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::EnumThreadWindows,
    };
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;
    unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut cur: i32 = 0;
        let got = DwmGetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &mut cur as *mut i32 as *mut core::ffi::c_void, 4);
        if got == 0 {
            ROUNDED.store(true, Relaxed); // Windows 11: the attribute exists
            if cur != DWMWCP_ROUND {
                let pref = DWMWCP_ROUND;
                DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const core::ffi::c_void, 4);
            }
        }
        1
    }
    unsafe {
        EnumThreadWindows(GetCurrentThreadId(), Some(each), 0);
    }
}

#[cfg(not(windows))]
pub fn round_windows() {}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Pin,
    Pinned,
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn maximized(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

static BAR_LOGO: std::sync::Mutex<Option<(u32, TextureHandle)>> = std::sync::Mutex::new(None);

/// The logo resized (Lanczos, premultiplied alpha so the rounded corners get no dark fringe) to exactly
/// `px` physical pixels. egui 0.27 has no mipmaps, so drawing the 256 px logo at 18 px looks blurry.
fn bar_logo(ctx: &Context, px: u32) -> TextureHandle {
    let mut g = BAR_LOGO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cur, t)) = g.as_ref() {
        if *cur == px {
            return t.clone();
        }
    }
    let mut img = image::load_from_memory(crate::LOGO).expect("logo").to_rgba8();
    for p in img.pixels_mut() {
        let a = p[3] as f32 / 255.0;
        for k in 0..3 {
            p[k] = (p[k] as f32 * a).round() as u8;
        }
    }
    let mut small = image::imageops::resize(&img, px, px, image::imageops::FilterType::Lanczos3);
    for p in small.pixels_mut() {
        let a = p[3] as f32 / 255.0;
        if a > 0.0 {
            for k in 0..3 {
                p[k] = (p[k] as f32 / a).round().min(255.0) as u8;
            }
        }
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], small.as_raw());
    let tex = ctx.load_texture(format!("bar-logo-{px}"), color, egui::TextureOptions::LINEAR);
    *g = Some((px, tex.clone()));
    tex
}

/// The title bar panel: logo, then whatever `menus` draws (the menu bar), then the window buttons.
/// Call first, before the toolbar. `pinned` is the current "always on top" state; returns `true`
/// when the pin button was clicked (the caller flips the state and sends the window-level command).
pub fn show(ctx: &Context, logo: &TextureHandle, pinned: bool, menus: impl FnOnce(&mut Ui)) -> bool {
    let mut pin_clicked = false;
    egui::TopBottomPanel::top("titlebar")
        .exact_height(HEIGHT)
        .frame(Frame::none().fill(pal().menu_bg).inner_margin(Margin::same(0.0)))
        .show(ctx, |ui| {
            drag_area(ui, EDGE);
            let r = ui.max_rect();
            ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0_f32, pal().hairline));

            // window buttons, flush right
            let buttons_w = 46.0 * 4.0;
            let right = Rect::from_min_max(pos2(r.right() - buttons_w, r.top()), r.max);
            ui.allocate_ui_at_rect(right, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ctx = ui.ctx().clone();
                    let max = maximized(&ctx);
                    if button(ui, Btn::Close, EDGE).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if button(ui, if max { Btn::Restore } else { Btn::Maximize }, EDGE).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if button(ui, Btn::Minimize, EDGE).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    if button(ui, if pinned { Btn::Pinned } else { Btn::Pin }, EDGE).clicked() {
                        pin_clicked = true;
                    }
                });
            });

            // logo, then the menus on one 21 px row centred in the bar
            let icon = Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(18.0, 18.0));
            let _ = logo; // the full-size texture is only used by the dialogs; the bar gets a crisp pre-scaled copy
            let ppp = ctx.pixels_per_point();
            let px = (18.0 * ppp).round().max(1.0) as u32;
            let tex = bar_logo(ctx, px);
            let side = px as f32 / ppp;
            let c = icon.center();
            // snap to whole physical pixels so the texture is drawn 1:1, never resampled
            let min = pos2(((c.x - side / 2.0) * ppp).round() / ppp, ((c.y - side / 2.0) * ppp).round() / ppp);
            ui.painter().image(
                tex.id(),
                Rect::from_min_size(min, vec2(side, side)),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let row_h = 21.0;
            let row = Rect::from_min_max(
                pos2(r.left() + 36.0, r.center().y - row_h / 2.0),
                pos2(right.left() - 4.0, r.center().y + row_h / 2.0),
            );
            ui.allocate_ui_at_rect(row, menus);
        });
    pin_clicked
}

/// Dragging the bar's empty space moves the window; double-click maximises or restores it.
/// Added before the controls so that they stay on top.
fn drag_area(ui: &mut Ui, edge: f32) {
    let r = ui.max_rect();
    let rect = Rect::from_min_max(r.min + vec2(edge, edge), r.max - vec2(edge, 0.0));
    let resp = ui.interact(rect, ui.id().with("titlebar-drag"), Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let max = maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
    }
}

fn button(ui: &mut Ui, b: Btn, edge: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(46.0, HEIGHT), Sense::hover());
    // clickable below the resize band along the top (and, for close, left of the right edge)
    let right = if b == Btn::Close { edge } else { 0.0 };
    let hit = Rect::from_min_max(rect.min + vec2(0.0, edge), rect.max - vec2(right, 0.0));
    let id = ui.id().with(("window-button", b as u8));
    let resp = ui.interact(hit, id, Sense::click());
    let (fill, ink) = match (b, resp.hovered()) {
        (Btn::Close, true) => (Color32::from_rgb(0xc4, 0x2b, 0x1c), Color32::WHITE),
        (_, true) => (pal().caption_hover, pal().strong),
        (_, false) => (Color32::TRANSPARENT, pal().caption_ink),
    };
    let p = ui.painter();
    p.rect_filled(rect, Rounding::ZERO, fill);
    let c = rect.center();
    let st = Stroke::new(1.0_f32, ink);
    let s = 5.0; // half the glyph: 10 px like Windows' caption buttons
    match b {
        Btn::Minimize => {
            p.hline(c.x - s..=c.x + s, c.y.floor() + 0.5, st);
        }
        Btn::Maximize => {
            p.rect_stroke(Rect::from_center_size(pos2(c.x.floor() + 0.5, c.y.floor() + 0.5), vec2(2.0 * s, 2.0 * s)), Rounding::ZERO, st);
        }
        Btn::Restore => {
            let front = Rect::from_min_size(c + vec2(-s, -s + 2.0), vec2(2.0 * s - 2.0, 2.0 * s - 2.0));
            p.rect_stroke(front, Rounding::ZERO, st);
            p.hline(c.x - s + 2.0..=c.x + s, c.y - s, st);
            p.vline(c.x + s, c.y - s..=c.y + s - 2.0, st);
        }
        Btn::Close => {
            p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], st);
            p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], st);
        }
        Btn::Pin | Btn::Pinned => {
            // a push-pin tilted 45 degrees (head up-right, needle down-left): filled while pinned
            let on = b == Btn::Pinned;
            let col = if on { pal().strong } else { ink };
            let st = Stroke::new(1.2_f32, col);
            let (sn, cs) = (std::f32::consts::FRAC_PI_4).sin_cos();
            let at = |x: f32, y: f32| c + vec2(x * cs - y * sn, x * sn + y * cs);
            let head = vec![at(-2.6, -7.0), at(2.6, -7.0), at(2.6, -1.5), at(-2.6, -1.5)];
            let flange = vec![at(-2.6, -1.5), at(2.6, -1.5), at(5.2, 1.2), at(-5.2, 1.2)];
            let outline = vec![at(-2.6, -7.0), at(2.6, -7.0), at(2.6, -1.5), at(5.2, 1.2), at(-5.2, 1.2), at(-2.6, -1.5)];
            if on {
                p.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
                p.add(egui::Shape::convex_polygon(flange, col, Stroke::NONE));
            } else {
                p.add(egui::Shape::closed_line(outline, st));
            }
            p.line_segment([at(0.0, 1.2), at(0.0, 7.5)], st);
        }
    }
    let tip = match b {
        Btn::Pin => "Always on top: off",
        Btn::Pinned => "Always on top: on",
        Btn::Minimize => "Minimize",
        Btn::Maximize => "Maximize",
        Btn::Restore => "Restore",
        Btn::Close => "Close",
    };
    resp.on_hover_text(tip)
}

/// A hairline round the window, and resizing it from its edges and corners. Call every frame, last.
pub fn edges(ctx: &Context) {
    use egui::viewport::ResizeDirection as D;
    use CursorIcon as C;
    let (max, full) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if max || full {
        return;
    }
    let r = ctx.screen_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("window-border")))
        .rect_stroke(r.shrink(0.5), Rounding::same(radius()), Stroke::new(1.0_f32, pal().win_border));
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()).filter(|p| r.contains(*p)) else { return };
    let near = |d: f32| d < EDGE;
    let corner = |d: f32| d < 3.0 * EDGE; // a corner takes a little of each edge beside it
    let (left, right, top, bottom) = (p.x - r.left(), r.right() - p.x, p.y - r.top(), r.bottom() - p.y);
    let hit = if (near(top) && corner(left)) || (near(left) && corner(top)) {
        Some((D::NorthWest, C::ResizeNorthWest))
    } else if (near(top) && corner(right)) || (near(right) && corner(top)) {
        Some((D::NorthEast, C::ResizeNorthEast))
    } else if (near(bottom) && corner(left)) || (near(left) && corner(bottom)) {
        Some((D::SouthWest, C::ResizeSouthWest))
    } else if (near(bottom) && corner(right)) || (near(right) && corner(bottom)) {
        Some((D::SouthEast, C::ResizeSouthEast))
    } else if near(top) {
        Some((D::North, C::ResizeNorth))
    } else if near(bottom) {
        Some((D::South, C::ResizeSouth))
    } else if near(left) {
        Some((D::West, C::ResizeWest))
    } else if near(right) {
        Some((D::East, C::ResizeEast))
    } else {
        None
    };
    if let Some((dir, cursor)) = hit {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
}

/// The hairline border of a pop-up (rounded where Windows rounds the window) and the OS rounding itself.
pub fn dialog_border(ctx: &Context) {
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("dialog-border")))
        .rect_stroke(ctx.screen_rect().shrink(0.5), Rounding::same(radius()), Stroke::new(1.0_f32, pal().win_border));
    round_windows();
}
