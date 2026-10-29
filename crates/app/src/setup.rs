//! The Setup page (Tasks > Setup guide…, also shown on first start): the browser extension and the
//! three helper programs (yt-dlp, ffmpeg, Deno) in one place, with live progress, what each part is
//! for, where it lives, and install / update / remove buttons.
use crate::{
    dialogs::{tool_logo, Out},
    theme::*,
    widgets::*,
};
use downcraft_core::{
    tools::{self, Busy, Entry, Tool, ToolBox},
    util, Manager,
};
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, Context, FontId, Frame, Layout, Margin, Rect, RichText, Rounding, Sense, Shape, Stroke,
    TextureHandle, Ui,
};
use std::{path::PathBuf, sync::Arc};

pub const SIZE: [f32; 2] = [600.0, 690.0];

pub struct SetupDlg {
    pub pinned: bool,
    /// Set by the "Install…" button; `App::update` opens the *Browser extension* window.
    pub open_extension: bool,
    /// The extension zip on disk, for dragging it onto the browser's extensions page.
    pub path: Option<PathBuf>,
    /// Set when the logo was grabbed; `App::update` starts the native drag (it needs the window handle).
    pub drag: bool,
    pub message: Option<String>,
    /// The "don't open this at start-up" tick box (kept in sync with the saved setting).
    pub hide_at_start: bool,
    /// Set when the tick box was clicked; `App::update` saves the new value.
    pub changed: bool,
}

impl SetupDlg {
    pub fn new(mgr: &Manager) -> Self {
        let hide_at_start = !mgr.settings.lock().show_setup;
        let path = crate::extension::export(&mgr.data_dir().join("extension")).ok();
        SetupDlg { pinned: true, open_extension: false, path, drag: false, message: None, hide_at_start, changed: false }
    }
}

// ───────────────────────── small widgets ─────────────────────────

/// Rounded label with a tinted background. `dot`: `Some(true)` = a pulsing dot in front (busy).
fn pill(ui: &mut Ui, text: &str, fill: Color32, ink: Color32, dot: Option<bool>) {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.5), ink);
    let lead = if dot.is_some() { 12.0 } else { 0.0 };
    let size = vec2(g.size().x + 16.0 + lead, g.size().y + 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(size.y / 2.0), fill.linear_multiply(0.18));
    if let Some(pulse) = dot {
        let r = if pulse { 2.6 + 1.4 * ((ui.input(|i| i.time) * 6.0).sin() as f32 * 0.5 + 0.5) } else { 3.0 };
        p.circle_filled(pos2(rect.left() + 11.0, rect.center().y), r, ink);
    }
    p.galley(pos2(rect.left() + 8.0 + lead, rect.top() + 3.0), g, ink);
}

fn chip(ui: &mut Ui, text: &str) {
    pill(ui, text, pal().muted, pal().muted, None);
}

#[derive(Clone, Copy)]
enum Ico {
    Refresh,
    Trash,
    Folder,
}

/// Small rounded square button with a line icon; the text only shows as a tooltip.
fn icon_btn(ui: &mut Ui, ico: Ico, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let hot = resp.hovered();
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(9.0), if hot { pal().btn_hover } else { pal().btn_bg });
    let ink = match (hot, ico) {
        (true, Ico::Trash) => pal().bad,
        (true, _) => pal().strong,
        _ => pal().muted,
    };
    let st = Stroke::new(1.4_f32, ink);
    let c = rect.center();
    let q = |x: f32, y: f32| c + vec2(x, y);
    match ico {
        Ico::Trash => {
            p.line_segment([q(-5.5, -3.5), q(5.5, -3.5)], st);
            p.add(Shape::line(vec![q(-1.8, -3.5), q(-1.8, -6.0), q(1.8, -6.0), q(1.8, -3.5)], st));
            p.add(Shape::line(vec![q(-4.2, -1.5), q(-3.5, 5.5), q(3.5, 5.5), q(4.2, -1.5)], st));
        }
        Ico::Folder => {
            p.add(Shape::closed_line(vec![q(-6.5, -4.5), q(-2.0, -4.5), q(-0.5, -2.5), q(6.5, -2.5), q(6.5, 5.0), q(-6.5, 5.0)], st));
        }
        Ico::Refresh => {
            // an open circle with an arrow head at its end
            let (r, t0, t1) = (5.2_f32, -2.2_f32, 3.3_f32);
            let pts: Vec<_> = (0..=20)
                .map(|i| {
                    let t = t0 + (t1 - t0) * i as f32 / 20.0;
                    q(r * t.cos(), r * t.sin())
                })
                .collect();
            p.add(Shape::line(pts, st));
            let end = q(r * t1.cos(), r * t1.sin());
            let d = vec2(-t1.sin(), t1.cos());
            let rot = |v: egui::Vec2, a: f32| vec2(v.x * a.cos() - v.y * a.sin(), v.x * a.sin() + v.y * a.cos());
            p.line_segment([end, end - rot(d, 0.55) * 4.0], st);
            p.line_segment([end, end - rot(d, -0.55) * 4.0], st);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A card with a coloured edge on the left; it lights up when the pointer is over it.
fn tile(ui: &mut Ui, edge: Color32, add: impl FnOnce(&mut Ui)) {
    let r = Frame::none()
        .fill(pal().card_bg)
        .stroke(Stroke::new(1.0_f32, pal().card_stroke))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin { left: 16.0, right: 14.0, top: 13.0, bottom: 13.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
        .rect;
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(r.left() + 1.0, r.top() + 14.0), pos2(r.left() + 4.0, r.bottom() - 14.0)), Rounding::same(2.0), edge);
    if ui.rect_contains_pointer(r) {
        p.rect_stroke(r, Rounding::same(12.0), Stroke::new(1.0_f32, pal().accent.linear_multiply(0.55)));
    }
}

/// Progress bar with a light sweep running over it; `None` = size unknown (the sweep crosses the whole bar).
fn live_bar(ui: &mut Ui, frac: Option<f32>) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(bar, Rounding::same(3.0), pal().track);
    let mut fill = bar;
    fill.set_width(bar.width() * frac.unwrap_or(1.0).max(0.03));
    p.rect_filled(fill, Rounding::same(3.0), pal().accent);
    let t = ui.input(|i| i.time) as f32;
    let sweep = 46.0;
    let x = fill.left() - sweep + ((t * 0.9) % 1.0) * (fill.width() + sweep);
    let clip = p.with_clip_rect(fill);
    clip.rect_filled(Rect::from_min_size(pos2(x, fill.top()), vec2(sweep, fill.height())), Rounding::same(3.0), Color32::WHITE.linear_multiply(if frac.is_some() { 0.28 } else { 0.18 }));
}

fn section_head(ui: &mut Ui, title: &str, right: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(11.5).strong().color(pal().muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(right).size(11.5).color(pal().muted));
        });
    });
    ui.add_space(2.0);
}

// ───────────────────────── the page ─────────────────────────

/// Top banner: logo, what is going on, and a ring that fills as parts get ready.
fn hero(ui: &mut Ui, logo: &TextureHandle, ready: usize, total: usize, title: &str, sub: &str, busy: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0), Sense::hover());
    let all = ready >= total;
    let tint = if all { pal().good } else { pal().accent };
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(14.0), pal().card_bg);
    p.rect_filled(rect, Rounding::same(14.0), tint.linear_multiply(0.10));
    p.rect_stroke(rect, Rounding::same(14.0), Stroke::new(1.0_f32, tint.linear_multiply(0.45)));
    let logo_rect = Rect::from_min_size(rect.min + vec2(16.0, 16.0), vec2(56.0, 56.0));
    p.image(logo.id(), logo_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    p.text(pos2(logo_rect.right() + 14.0, rect.top() + 33.0), Align2::LEFT_CENTER, title, FontId::proportional(18.0), pal().strong);
    p.text(pos2(logo_rect.right() + 14.0, rect.top() + 56.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.5), pal().muted);

    // progress ring
    let target = if total == 0 { 0.0 } else { ready as f32 / total as f32 };
    let frac = ui.ctx().animate_value_with_time(egui::Id::new("setup-ring"), target, 0.6);
    let c = pos2(rect.right() - 46.0, rect.center().y);
    let r = 24.0_f32;
    let p = ui.painter();
    p.circle_stroke(c, r, Stroke::new(5.0_f32, pal().track));
    if frac > 0.003 {
        let n = ((frac * 64.0).ceil() as usize).max(2);
        let pts: Vec<_> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n as f32;
                c + vec2(r * a.cos(), r * a.sin())
            })
            .collect();
        p.add(Shape::line(pts, Stroke::new(5.0_f32, tint)));
    }
    if all {
        let st = Stroke::new(2.4_f32, pal().good);
        p.line_segment([c + vec2(-7.0, 0.5), c + vec2(-2.0, 5.5)], st);
        p.line_segment([c + vec2(-2.0, 5.5), c + vec2(8.0, -5.5)], st);
    } else {
        p.text(c, Align2::CENTER_CENTER, format!("{ready}/{total}"), FontId::proportional(13.5), pal().strong);
    }
    if busy {
        // a small orbiting dot on the ring while something is downloading
        let a = (ui.input(|i| i.time) as f32 * 3.0) % std::f32::consts::TAU;
        ui.painter().circle_filled(c + vec2((r + 8.0) * a.cos(), (r + 8.0) * a.sin()), 2.2, pal().accent);
    }
}

fn extension_card(ui: &mut Ui, d: &mut SetupDlg, seen: bool, logo: &TextureHandle) {
    let (fill, ink, label) = if seen { (pal().good, pal().good_text, "Connected") } else { (pal().disabled, pal().muted, "Not connected") };
    tile(ui, if seen { pal().good } else { pal().disabled }, |ui| {
        ui.horizontal(|ui| {
            let r = ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(46.0, 46.0)).sense(Sense::click_and_drag()));
            if d.path.is_some() {
                let r = r.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(if crate::extension::CAN_DRAG {
                    "Drag me onto the browser's extensions page (chrome://extensions, Developer mode on)"
                } else {
                    "Use Install… to add the extension with Load unpacked"
                });
                if r.drag_started() && crate::extension::CAN_DRAG {
                    d.drag = true;
                }
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 130.0).max(80.0));
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Browser extension").size(15.0).strong().color(pal().strong));
                    pill(ui, label, fill, ink, None);
                });
                ui.label(RichText::new("Adds a Download button to videos and sends links, with cookies, straight to DownCraft.").size(12.5).color(pal().muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if action_button(ui, if seen { "Reinstall…" } else { "Install…" }, !seen).clicked() {
                    d.open_extension = true;
                }
            });
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            for b in ["Chrome", "Edge", "Brave", "Opera"] {
                chip(ui, b);
            }
            chip(ui, "Talks to this PC only");
        });
        let hint = match (&d.message, crate::extension::CAN_DRAG) {
            (Some(m), _) => m.clone(),
            (None, true) if seen => "Drag the logo onto chrome://extensions to reinstall.".to_string(),
            (None, true) => "Drag the logo onto chrome://extensions (Developer mode on) and it installs itself.".to_string(),
            (None, false) => "Press Install… for the steps: Load unpacked with DownCraft's extension folder.".to_string(),
        };
        let col = if d.message.is_some() { pal().bad } else { pal().muted };
        ui.add(egui::Label::new(RichText::new(hint.as_str()).size(12.0).color(col)).truncate(true)).on_hover_text(hint);
    });
}

fn tool_card(ui: &mut Ui, tb: &Arc<ToolBox>, tool: Tool, e: &Entry) {
    let installing = e.busy == Busy::Installing;
    let ready = e.installed.is_some();
    let managed = e.installed.as_ref().map_or(false, |i| i.managed);
    let err = e.message.as_ref().filter(|(bad, _)| *bad).map(|(_, t)| t.clone());
    let frac = if e.total > 0 { (e.done as f32 / e.total as f32).clamp(0.0, 1.0) } else { 0.0 };
    let edge = if err.is_some() {
        pal().bad
    } else if installing {
        pal().accent
    } else if ready {
        pal().good
    } else if tool.source().is_none() {
        pal().warn
    } else {
        pal().disabled
    };
    tile(ui, edge, |ui| {
        ui.horizontal(|ui| {
            let tl = tool_logo(ui.ctx(), tool);
            ui.add(egui::Image::new(&tl).fit_to_exact_size(vec2(46.0, 46.0)));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 130.0).max(80.0));
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tool.name()).size(15.0).strong().color(pal().strong));
                    if err.is_some() {
                        pill(ui, "Failed", pal().bad, pal().bad, None);
                    } else if installing {
                        let t = if e.extracting { "Unpacking".to_string() } else if e.total > 0 { format!("Downloading {}%", (frac * 100.0) as u32) } else { "Downloading".to_string() };
                        pill(ui, &t, pal().accent, pal().accent, Some(true));
                    } else if e.busy == Busy::Checking {
                        pill(ui, "Checking…", pal().muted, pal().muted, Some(true));
                    } else if e.busy == Busy::Removing {
                        pill(ui, "Removing…", pal().muted, pal().muted, Some(true));
                    } else if ready {
                        pill(ui, "Ready", pal().good, pal().good_text, None);
                    } else if tool.source().is_none() {
                        pill(ui, "Install manually", pal().warn, pal().warn, None);
                    } else {
                        pill(ui, "Not installed", pal().muted, pal().muted, None);
                    }
                });
                ui.label(RichText::new(tool.purpose()).size(12.5).color(pal().muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match e.busy {
                    Busy::Installing => {
                        if action_button(ui, "Cancel", false).clicked() {
                            tb.cancel(tool);
                        }
                    }
                    Busy::Idle if tool.source().is_some() => {
                        if managed {
                            if icon_btn(ui, Ico::Trash, "Remove").clicked() {
                                tb.remove(tool);
                            }
                            if icon_btn(ui, Ico::Refresh, "Update to the latest version").clicked() {
                                tb.install(tool);
                            }
                        } else if !ready {
                            if action_button(ui, &format!("Get · {} MB", tool.approx_mb()), true).clicked() {
                                tb.install(tool);
                            }
                        } else if action_button(ui, "Install", false).on_hover_text("Keep and update DownCraft's own copy, and use that one").clicked() {
                            tb.install(tool);
                        }
                    }
                    _ => {}
                }
            });
        });

        ui.add_space(3.0);
        if installing {
            live_bar(ui, if e.extracting || e.total == 0 { None } else { Some(frac) });
            let t = if e.extracting {
                "Unpacking…".to_string()
            } else if e.total > 0 {
                format!("{} of {}", util::fmt_size(e.done), util::fmt_size(e.total))
            } else {
                format!("{} downloaded", util::fmt_size(e.done))
            };
            ui.label(RichText::new(t).size(11.5).color(pal().muted));
        } else if let Some(i) = &e.installed {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                if let Some(v) = &e.version {
                    let short = if v.chars().count() > 18 { format!("{}…", v.chars().take(17).collect::<String>()) } else { v.clone() };
                    let txt = if short.starts_with(|c: char| c.is_ascii_digit()) { format!("v{short}") } else { short };
                    chip(ui, &txt);
                }
                chip(ui, if managed { "Managed by DownCraft" } else { "Found on this PC" });
            });
            let path = i.path.display().to_string();
            ui.add(egui::Label::new(RichText::new(&path).size(11.5).color(pal().disabled)).truncate(true)).on_hover_text(path);
        } else if e.busy == Busy::Idle {
            if tool.source().is_none() {
                ui.label(RichText::new(tool.manual_hint()).size(12.0).color(pal().warn));
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    chip(ui, &format!("{} MB download", tool.approx_mb()));
                    chip(ui, "Kept in DownCraft's folder");
                });
            }
        }
        if let Some(t) = &err {
            ui.add_space(2.0);
            ui.label(RichText::new(t).size(12.0).color(pal().bad));
        }
    });
    ui.add_space(8.0);
}

/// `Close` = the window was closed. The start-up tick box is saved by `App::update` (see `SetupDlg::changed`).
pub fn setup_ui(ctx: &Context, d: &mut SetupDlg, tb: &Arc<ToolBox>, extension_seen: bool, logo: &TextureHandle) -> Out<()> {
    let mut out = Out::Keep;
    let entries = tb.snapshot();
    let busy = tb.is_busy();
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(40)); // progress numbers, sweeps and pulses
    }
    let missing: Vec<Tool> = entries.iter().filter(|(t, e)| e.installed.is_none() && t.source().is_some() && e.busy == Busy::Idle).map(|(t, _)| *t).collect();
    let installed = entries.iter().filter(|(_, e)| e.installed.is_some()).count();
    let total = 1 + entries.len();
    let ready_n = installed + extension_seen as usize;
    let left = total - ready_n;
    let installing: Vec<&str> = entries.iter().filter(|(_, e)| e.busy == Busy::Installing).map(|(t, _)| t.name()).collect();
    let mb: u32 = missing.iter().map(|t| t.approx_mb()).sum();
    let (title, sub) = if !installing.is_empty() {
        ("Setting things up…".to_string(), format!("Downloading {}", installing.join(", ")))
    } else if left == 0 {
        ("Everything is ready".to_string(), "The extension and the helper programs are set up.".to_string())
    } else {
        let dl = if mb > 0 { format!(" · about {mb} MB to download") } else { String::new() };
        ("Set up DownCraft".to_string(), format!("{left} step{} left{dl}", if left == 1 { "" } else { "s" }))
    };
    let dir = tools::tools_dir(&tb.data_dir());

    let closed = show_dialog_bare(ctx, "setup", "Set up DownCraft", SIZE, true, |ui| {
        if pin_button(ui, d.pinned) {
            d.pinned = !d.pinned;
            let level = if d.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        let h = (ui.available_height() - 44.0).max(120.0);
        egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width() - 8.0);
            hero(ui, logo, ready_n, total, &title, &sub, busy);
            ui.add_space(8.0);

            section_head(ui, "BROWSER EXTENSION", if extension_seen { "connected" } else { "not connected yet" });
            extension_card(ui, d, extension_seen, logo);
            ui.add_space(10.0);

            section_head(ui, "DOWNLOAD HELPERS", &format!("{installed} of {} installed", entries.len()));
            ui.label(RichText::new("Only needed for video sites such as YouTube, Facebook, Instagram and X. Ordinary file downloads work without them.").size(12.0).color(pal().muted));
            ui.add_space(6.0);
            for (tool, e) in &entries {
                tool_card(ui, tb, *tool, e);
            }
        });

        // footer: the buttons never move the content above them (the pop-up has a fixed size)
        pin_bottom(ui, 34.0);
        ui.horizontal(|ui| {
            let tick = ui
                .checkbox(&mut d.hide_at_start, RichText::new("Don't show at start-up").color(pal().text2))
                .on_hover_text("Tick to stop this window opening when DownCraft starts. Untick to get it back. It always stays under Tasks > Setup guide.");
            if tick.changed() {
                d.changed = true;
            }
            if icon_btn(ui, Ico::Folder, &format!("Open the helpers folder\n{}", dir.display())).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(&dir);
            }
            if icon_btn(ui, Ico::Refresh, "Check again").clicked() {
                tb.refresh();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let many = missing.len() > 1;
                if action_button(ui, "Close", !many).clicked() {
                    out = Out::Close;
                }
                if many && action_button(ui, &format!("Get all {} missing", missing.len()), true).clicked() {
                    for t in &missing {
                        tb.install(*t);
                    }
                }
            });
        });
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pills_and_chips_have_a_sane_size() {
        let ctx = Context::default();
        crate::theme::set(&ctx, Theme::Dark, Accent::Blue);
        let mut w = 0.0;
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(SIZE[0], SIZE[1]))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        pill(ui, "Downloading 42%", pal().accent, pal().accent, Some(true));
                        chip(ui, "Managed by DownCraft");
                        live_bar(ui, Some(0.4));
                        w = ui.min_rect().width();
                    });
                });
            });
        }
        assert!(w > 100.0, "row is {w:.0} px wide");
    }
}
