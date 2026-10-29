//! The Appearance page (View > Appearance…): theme, accent colour and toolbar skin in one place.
//!
//! Toolbar skins come from the online gallery (one click installs *and* applies a skin), from the
//! packs already saved on this PC, or from a `.zip` the user picked or dropped onto the window.
use crate::{
    dialogs::Out,
    icons,
    skin_store::{Catalog, Job, SkinStore},
    theme::*,
    widgets::*,
};
use downcraft_core::{skinstore as net, Manager};
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, Context, FontId, Layout, RichText, Rounding, Sense, Stroke, TextureHandle, Ui,
};
use std::{path::PathBuf, sync::Arc};

pub struct AppearanceDlg {
    pub pinned: bool,
    /// (is an error, text) shown under the skin buttons.
    note: Option<(bool, String)>,
}

impl AppearanceDlg {
    pub fn new(store: &Arc<SkinStore>, ctx: &Context) -> Self {
        store.refresh_if_needed(ctx);
        AppearanceDlg { pinned: true, note: None }
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).size(13.0).strong().color(pal().strong));
    ui.add_space(2.0);
}

fn apply_theme(ctx: &Context, mgr: &Manager, theme: Theme, accent: Accent) {
    crate::theme::set(ctx, theme, accent);
    {
        let mut st = mgr.settings.lock();
        st.theme = theme.key().to_string();
        st.accent = accent.key().to_string();
    }
    mgr.save_settings();
}

/// Preview picture as a texture, uploaded once.
fn preview_texture(ctx: &Context, store: &SkinStore, id: &str) -> Option<TextureHandle> {
    let key = egui::Id::new(("skin-preview", id));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(key)) {
        return Some(t);
    }
    let img = store.take_preview(id)?;
    let tex = ctx.load_texture(format!("skin-preview-{id}"), img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}

fn small_button(ui: &mut Ui, label: &str, primary: bool) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(10.0, 3.0);
        let (fill, text) = if primary { (pal().accent, pal().on_accent) } else { (pal().btn_bg, pal().text2) };
        ui.add(egui::Button::new(RichText::new(label).color(text)).fill(fill).rounding(Rounding::same(7.0)).min_size(vec2(74.0, 26.0)))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

/// Returns the theme clicked this frame.
fn theme_tiles(ui: &mut Ui) -> Option<Theme> {
    let (cur_theme, cur_accent) = crate::theme::current();
    let gap = 8.0;
    let w = ((ui.available_width() - gap * 2.0) / 3.0).floor();
    let mut pick = None;
    for row in Theme::ALL.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &t in row {
                let sw = Palette::build(t, cur_accent);
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click());
                let p = ui.painter();
                p.rect_filled(rect, Rounding::same(8.0), sw.menu_bg);
                // a miniature of the window: a sidebar strip, two rows and a line of text
                let list = egui::Rect::from_min_max(rect.min + vec2(8.0, 8.0), pos2(rect.right() - 8.0, rect.top() + 24.0));
                p.rect_filled(list, Rounding::same(3.0), sw.row_bg);
                p.rect_filled(egui::Rect::from_min_size(list.min + vec2(4.0, 5.0), vec2(w * 0.35, 3.0)), Rounding::ZERO, sw.text);
                p.circle_filled(pos2(rect.right() - 16.0, list.center().y), 3.5, cur_accent.colors().0);
                p.text(pos2(rect.left() + 9.0, rect.bottom() - 11.0), Align2::LEFT_CENTER, t.name(), FontId::proportional(12.5), sw.text);
                let selected = t == cur_theme;
                let stroke = if selected {
                    Stroke::new(2.0_f32, pal().accent)
                } else if resp.hovered() {
                    Stroke::new(1.5_f32, pal().btn_hover_stroke)
                } else {
                    Stroke::new(1.0_f32, sw.grid)
                };
                p.rect_stroke(rect, Rounding::same(8.0), stroke);
                if selected {
                    let c = pos2(rect.right() - 13.0, rect.bottom() - 12.0);
                    let st = Stroke::new(1.7_f32, pal().accent);
                    p.line_segment([c + vec2(-3.5, 0.2), c + vec2(-1.0, 2.8)], st);
                    p.line_segment([c + vec2(-1.0, 2.8), c + vec2(3.8, -2.8)], st);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick = Some(t);
                }
            }
        });
    }
    pick
}

/// Returns the accent colour clicked this frame.
fn accent_dots(ui: &mut Ui) -> Option<Accent> {
    let cur_accent = crate::theme::current().1;
    let mut pick = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        for a in Accent::ALL {
            let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
            let p = ui.painter();
            p.circle_filled(rect.center(), if resp.hovered() { 12.5 } else { 11.5 }, a.colors().0);
            if a == cur_accent {
                p.circle_stroke(rect.center(), 13.5, Stroke::new(2.0_f32, pal().strong));
                let c = rect.center();
                let st = Stroke::new(1.8_f32, Color32::WHITE);
                p.line_segment([c + vec2(-4.0, 0.2), c + vec2(-1.2, 3.2)], st);
                p.line_segment([c + vec2(-1.2, 3.2), c + vec2(4.2, -3.0)], st);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(a.name()).clicked() {
                pick = Some(a);
            }
        }
    });
    pick
}

/// One row of the gallery. Returns what the user asked for.
enum Ask {
    Install(net::SkinEntry),
    Use(String),
    Remove(String),
    Retry(String),
}

fn skin_card(ui: &mut Ui, store: &SkinStore, e: &net::SkinEntry, in_use: bool) -> Option<Ask> {
    let mut ask = None;
    let installed = store.is_installed(&e.id);
    let job = store.job(&e.id);
    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        // preview strip
        let w = ui.available_width();
        if let Some(tex) = preview_texture(ui.ctx(), store, &e.id) {
            let s = tex.size_vec2();
            let h = (w * s.y / s.x.max(1.0)).clamp(24.0, 96.0);
            let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            ui.painter().rect_filled(rect, Rounding::same(6.0), pal().bare_bg);
            let fit = (rect.width() / s.x).min(rect.height() / s.y);
            let img = egui::Rect::from_center_size(rect.center(), s * fit);
            ui.painter().image(tex.id(), img, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if !e.previews.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
            ui.painter().rect_filled(rect, Rounding::same(6.0), pal().bare_bg);
            let msg = if store.preview_failed(&e.id) { "No preview" } else { "Loading preview…" };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(12.0), pal().muted);
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(egui::Label::new(RichText::new(&e.name).size(14.0).strong().color(pal().strong)).truncate(true));
                let sub = match (e.author.is_empty(), installed, in_use) {
                    (_, _, true) => "In use".to_string(),
                    (true, true, _) => "On this PC".to_string(),
                    (true, false, _) => String::new(),
                    (false, true, _) => format!("by {} · on this PC", e.author),
                    (false, false, _) => format!("by {}", e.author),
                };
                if !sub.is_empty() {
                    let col = if in_use { pal().good } else { pal().muted };
                    ui.label(RichText::new(sub).size(11.5).color(col));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match &job {
                    Some(Job::Busy) => {
                        ui.add(egui::Spinner::new().size(16.0).color(pal().accent));
                        ui.label(RichText::new("Installing…").color(pal().muted));
                    }
                    _ if in_use => {
                        if installed && small_button(ui, "Remove", false).on_hover_text("Delete the saved copy (the skin stays active until you pick another)").clicked() {
                            ask = Some(Ask::Remove(e.id.clone()));
                        }
                    }
                    _ if installed => {
                        if small_button(ui, "Use", true).clicked() {
                            ask = Some(Ask::Use(e.id.clone()));
                        }
                        if small_button(ui, "Remove", false).on_hover_text("Delete the saved copy").clicked() {
                            ask = Some(Ask::Remove(e.id.clone()));
                        }
                    }
                    Some(Job::Failed(_)) => {
                        if small_button(ui, "Retry", true).clicked() {
                            ask = Some(Ask::Retry(e.id.clone()));
                        }
                    }
                    _ if !e.zip.is_empty() => {
                        if small_button(ui, "Install", true).on_hover_text("Download and apply this skin").clicked() {
                            ask = Some(Ask::Install(e.clone()));
                        }
                    }
                    _ => {}
                }
            });
        });
        if let Some(Job::Failed(m)) = &job {
            ui.add(egui::Label::new(RichText::new(m).size(11.5).color(pal().bad)));
        }
    });
    ui.add_space(6.0);
    ask
}

pub fn appearance_ui(ctx: &Context, d: &mut AppearanceDlg, mgr: &Arc<Manager>, store: &Arc<SkinStore>) -> Out<()> {
    let mut out = Out::Keep;
    if matches!(store.catalog(), Catalog::Loading) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    let closed = show_dialog_bare(ctx, "appearance", "Appearance", [560.0, 680.0], true, |ui| {
        if pin_button(ui, d.pinned) {
            d.pinned = !d.pinned;
            let level = if d.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        heading(ui, "Appearance", "Theme, accent colour and toolbar skin", Badge::None);

        // a skin pack dropped onto the window is applied at once
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(p) = dropped.iter().find(|p| p.extension().map_or(false, |e| e.eq_ignore_ascii_case("zip"))) {
            d.note = Some(match crate::skin::import_zip(p, mgr.data_dir()) {
                Ok(s) => {
                    let name = s.name.clone();
                    icons::set_skin(Some(s));
                    (false, format!("{name} applied."))
                }
                Err(e) => (true, format!("Could not load the skin: {e}")),
            });
        }

        let h = (ui.available_height() - 44.0).max(120.0);
        egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(0.0);
            ui.set_width(ui.available_width() - 6.0);

            section(ui, "Theme");
            if let Some(t) = theme_tiles(ui) {
                apply_theme(ui.ctx(), mgr, t, crate::theme::current().1);
            }

            section(ui, "Accent color");
            if let Some(a) = accent_dots(ui) {
                apply_theme(ui.ctx(), mgr, crate::theme::current().0, a);
            }

            section(ui, "Toolbar skin");
            let skin_now = icons::skin_name();
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new("In use").size(11.5).color(pal().muted));
                        let name = skin_now.clone().unwrap_or_else(|| "Default icons".to_string());
                        ui.add(egui::Label::new(RichText::new(name).size(14.0).strong().color(pal().strong)).truncate(true));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if ui.add_enabled_ui(skin_now.is_some(), |ui| small_button(ui, "Default", false)).inner.on_hover_text("Back to the built-in icons").clicked() {
                            crate::skin::clear(mgr.data_dir());
                            icons::set_skin(None);
                            d.note = None;
                        }
                        if small_button(ui, "Load .zip…", false).on_hover_text("Pick a skin pack from your PC (or drop it on this window)").clicked() {
                            if let Some(p) = rfd::FileDialog::new().add_filter("Toolbar skin", &["zip"]).pick_file() {
                                d.note = Some(match crate::skin::import_zip(&p, mgr.data_dir()) {
                                    Ok(s) => {
                                        let name = s.name.clone();
                                        icons::set_skin(Some(s));
                                        (false, format!("{name} applied."))
                                    }
                                    Err(e) => (true, format!("Could not load the skin: {e}")),
                                });
                            }
                        }
                    });
                });
                if let Some((bad, msg)) = &d.note {
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(msg).size(12.0).color(if *bad { pal().bad } else { pal().good })));
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Skin gallery").size(12.5).strong().color(pal().strong));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if small_button(ui, "Refresh", false).clicked() {
                        store.refresh(ui.ctx());
                    }
                    if small_button(ui, "On GitHub", false).on_hover_text(net::PAGE).clicked() {
                        let _ = open::that(net::PAGE);
                    }
                });
            });
            ui.add_space(2.0);

            // gallery entries, then packs on this PC the gallery does not list (so they work offline)
            let catalog = store.catalog();
            let mut rows: Vec<net::SkinEntry> = match &catalog {
                Catalog::Ready(l) => l.clone(),
                _ => Vec::new(),
            };
            for id in store.installed_ids() {
                if !rows.iter().any(|r| r.id == id) {
                    rows.push(net::SkinEntry { id: id.clone(), name: id, author: String::new(), zip: String::new(), previews: Vec::new() });
                }
            }
            match &catalog {
                Catalog::Loading if rows.is_empty() => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(pal().accent));
                        ui.label(RichText::new("Loading the skin list…").color(pal().muted));
                    });
                }
                Catalog::Failed(e) => {
                    ui.add(egui::Label::new(RichText::new(format!("Could not load the gallery: {e}")).size(12.0).color(pal().bad)));
                }
                Catalog::Ready(l) if l.is_empty() && rows.is_empty() => {
                    ui.label(RichText::new("No skins are published yet.").color(pal().muted));
                }
                _ => {}
            }
            ui.add_space(4.0);
            let mut ask = None;
            for e in &rows {
                let in_use = skin_now.as_deref().map_or(false, |n| n.eq_ignore_ascii_case(&e.name) || n.eq_ignore_ascii_case(&e.id));
                if let Some(a) = skin_card(ui, store, e, in_use) {
                    ask = Some(a);
                }
            }
            match ask {
                Some(Ask::Install(e)) => store.install(e, ui.ctx()),
                Some(Ask::Retry(id)) => {
                    store.clear_job(&id);
                    if let Some(e) = rows.iter().find(|r| r.id == id) {
                        store.install(e.clone(), ui.ctx());
                    }
                }
                Some(Ask::Use(id)) => {
                    d.note = Some(match store.use_installed(&id) {
                        Ok(()) => (false, "Skin applied.".to_string()),
                        Err(e) => (true, format!("Could not load the skin: {e}")),
                    });
                }
                Some(Ask::Remove(id)) => store.remove(&id),
                None => {}
            }
        });
        pin_bottom(ui, 32.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if action_button(ui, "Close", true).clicked() {
                out = Out::Close;
                dismiss(ui.ctx());
            }
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

    /// Runs `add` in a pop-up-sized panel (a few frames, like the real thing) and returns the height it used.
    fn used_height(add: impl Fn(&mut Ui)) -> f32 {
        let ctx = Context::default();
        crate::theme::set(&ctx, Theme::Dark, Accent::Blue);
        let mut used = 0.0_f32;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(560.0, 680.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let area = ui.max_rect().shrink2(vec2(22.0, 18.0));
                    ui.allocate_ui_at_rect(area, |ui| {
                        add(ui);
                        used = ui.min_rect().height();
                    });
                });
            });
        }
        used
    }

    #[test]
    fn theme_and_accent_pickers_fit_the_page() {
        // 6 themes in two rows of three, 8 accent dots in one row; nothing is picked without a click
        let h = used_height(|ui| {
            assert!(theme_tiles(ui).is_none());
            assert!(accent_dots(ui).is_none());
        });
        assert!(h > 100.0 && h < 220.0, "pickers use {h:.0} px");
    }
}
