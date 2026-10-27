use eframe::egui::{self, vec2, Color32, Frame, Response, RichText, Rounding, Sense, Stroke, TextureHandle, Ui};
use crate::theme::pal;
use std::{
    collections::HashSet,
    sync::{Mutex, MutexGuard, OnceLock},
};

/// Dialog windows drawn in the previous / the current frame. A pop-up that was drawn last frame but
/// not this one has been dismissed; it is closed explicitly instead of waiting for the main window
/// to notice (a minimised or tray-hidden main window may repaint late, leaving a "stuck" pop-up).
type Seen = (HashSet<egui::ViewportId>, HashSet<egui::ViewportId>);
static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();

fn seen() -> MutexGuard<'static, Seen> {
    SEEN.get_or_init(|| Mutex::new((HashSet::new(), HashSet::new()))).lock().unwrap_or_else(|e| e.into_inner())
}

/// Call once at the end of every frame, after all dialogs were drawn. Returns true while any
/// dialog window exists.
pub fn finish_dialog_frame(ctx: &egui::Context) -> bool {
    let mut s = seen();
    let gone: Vec<_> = s.0.difference(&s.1).copied().collect();
    for id in &gone {
        // hide first: even if the OS window lingers a moment, it is never visible and clickable-but-dead
        ctx.send_viewport_cmd_to(*id, egui::ViewportCommand::Visible(false));
        ctx.send_viewport_cmd_to(*id, egui::ViewportCommand::Close);
    }
    if !gone.is_empty() {
        ctx.request_repaint();
    }
    s.0 = std::mem::take(&mut s.1);
    !s.0.is_empty()
}


/// Opens a real OS window (egui immediate viewport) with the DownCraft pop-up look: no title bar,
/// the whole window drags, a cross in the corner (and Esc) closes it, and the content carries its
/// own heading (see `heading`). `size` is the full window; `top` keeps it above every other window
/// (used for pop-ups that must be seen, such as "Download File Info" arriving from the browser).
/// Returns `true` when the user closed it with the [x] / Esc.
pub fn show_dialog_bare(
    ctx: &egui::Context,
    key: impl std::hash::Hash,
    title: &str,
    size: [f32; 2],
    top: bool,
    add: impl FnMut(&mut Ui),
) -> bool {
    show_dialog_impl(ctx, key, title, size, top, false, add)
}

/// Same as `show_dialog_bare`, plus a minimize button left of the close cross (used by the download
/// pop-ups, which people want to get out of the way while they keep running).
pub fn show_dialog_min(
    ctx: &egui::Context,
    key: impl std::hash::Hash,
    title: &str,
    size: [f32; 2],
    top: bool,
    add: impl FnMut(&mut Ui),
) -> bool {
    show_dialog_impl(ctx, key, title, size, top, true, add)
}

fn show_dialog_impl(
    ctx: &egui::Context,
    key: impl std::hash::Hash,
    title: &str,
    size: [f32; 2],
    top: bool,
    minimizable: bool,
    mut add: impl FnMut(&mut Ui),
) -> bool {
    let id = egui::ViewportId::from_hash_of(key);
    seen().1.insert(id);
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(false)
        .with_decorations(false) // drawn by the pop-up itself
        .with_minimize_button(false)
        .with_maximize_button(false);
    // Open centred on the main window (the position only matters when the window is created).
    // A minimised window reports a nonsense position, so fall back to the middle of the screen.
    let centre = ctx.input(|i| {
        let vp = i.viewport();
        match vp.outer_rect {
            Some(r) if r.min.x > -10_000.0 && r.min.y > -10_000.0 && r.width() > 0.0 => Some(r.center()),
            _ => vp.monitor_size.map(|m| egui::pos2(m.x / 2.0, m.y / 2.0)),
        }
    });
    if let Some(c) = centre {
        builder = builder.with_position(egui::pos2((c.x - size[0] / 2.0).max(0.0), (c.y - size[1] / 2.0).max(0.0)));
    }
    if top {
        builder = builder.with_always_on_top().with_active(true);
    }
    ctx.show_viewport_immediate(id, builder, |ctx, class| {
        if class == egui::ViewportClass::Embedded {
            let mut open = true;
            egui::Window::new(title)
                .id(egui::Id::new(id))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| add(ui));
            !open
        } else {
            egui::CentralPanel::default().frame(Frame::none().fill(pal().bare_bg)).show(ctx, |ui| {
                let full = ui.max_rect();
                // dragging anywhere on the background moves the window (added first: widgets sit on top)
                let drag = ui.interact(full, ui.id().with("bare-drag"), egui::Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.allocate_ui_at_rect(full.shrink2(vec2(22.0, 18.0)), |ui| {
                    modern_style(ui);
                    add(ui)
                });
                if close_cross(ui, full).clicked() {
                    dismiss(ui.ctx());
                }
                if minimizable && minimize_button(ui, full).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                dismiss(ctx);
            }
            crate::titlebar::dialog_border(ctx);
            ctx.input(|i| i.viewport().close_requested())
        }
    })
}

/// Closes the calling pop-up right away: hidden at once, then asked to close, and a repaint is
/// requested so the close is processed immediately instead of waiting for the next input event.
pub fn dismiss(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    ctx.request_repaint();
}

/// Look of every widget inside a pop-up: rounded, dark, flat, with the accent colour for focus.
fn modern_style(ui: &mut Ui) {
    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
    ui.spacing_mut().button_padding = vec2(12.0, 4.0);
    ui.spacing_mut().interact_size.y = 28.0;
    ui.style_mut().interaction.selectable_labels = false; // so dragging text moves the window
    let v = ui.visuals_mut();
    v.extreme_bg_color = pal().card_bg;
    v.selection.bg_fill = pal().accent;
    v.selection.stroke = Stroke::new(1.0_f32, pal().accent);
    let r = Rounding::same(7.0);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.widgets.inactive.bg_fill = pal().btn_bg;
    v.widgets.inactive.weak_bg_fill = pal().btn_bg;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal().card_stroke);
    v.widgets.hovered.bg_fill = pal().btn_hover;
    v.widgets.hovered.weak_bg_fill = pal().btn_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal().btn_hover_stroke);
    v.widgets.active.bg_fill = pal().btn_active;
    v.widgets.active.weak_bg_fill = pal().btn_active;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal().accent);
}

/// Small marker drawn on the corner of the heading logo.
#[derive(Clone, Copy, PartialEq)]
pub enum Badge {
    None,
    Done,
    Warn,
}

/// Heading of a pop-up: the logo (optionally with a status badge), a title and a muted line below.
pub fn heading(ui: &mut Ui, title: &str, sub: &str, badge: Badge) {
    let logo: Option<TextureHandle> = ui.ctx().data(|d| d.get_temp(egui::Id::new("app-logo")));
    ui.horizontal(|ui| {
        let (rc, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
        if let Some(logo) = &logo {
            ui.put(rc, egui::Image::new(logo).fit_to_exact_size(vec2(46.0, 46.0)));
        }
        if badge != Badge::None {
            let b = rc.right_bottom() + vec2(-3.0, -3.0);
            let p = ui.painter();
            p.circle_filled(b, 9.5, pal().bare_bg);
            let (col, mark) = if badge == Badge::Done { (pal().good, 0) } else { (pal().warn, 1) };
            p.circle_filled(b, 8.0, col);
            let st = Stroke::new(1.6_f32, Color32::WHITE);
            if mark == 0 {
                p.line_segment([b + vec2(-3.5, 0.2), b + vec2(-1.0, 2.8)], st);
                p.line_segment([b + vec2(-1.0, 2.8), b + vec2(3.8, -2.6)], st);
            } else {
                p.line_segment([b + vec2(0.0, -4.0), b + vec2(0.0, 1.0)], st);
                p.circle_filled(b + vec2(0.0, 3.8), 1.0, Color32::WHITE);
            }
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.set_max_width((ui.available_width() - 70.0).max(40.0)); // leave room for the minimize button and the close cross
            ui.add(egui::Label::new(RichText::new(title).size(17.0).strong().color(pal().strong)).truncate(true));
            if !sub.is_empty() {
                ui.add(egui::Label::new(RichText::new(sub).color(pal().muted)).truncate(true));
            }
        });
    });
    ui.add_space(10.0);
}

/// Compact heading for small pop-ups: 34 px logo, title and a muted line beside it.
pub fn heading_small(ui: &mut Ui, title: &str, sub: &str) {
    let logo: Option<TextureHandle> = ui.ctx().data(|d| d.get_temp(egui::Id::new("app-logo")));
    ui.horizontal(|ui| {
        let (rc, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
        if let Some(logo) = &logo {
            ui.put(rc, egui::Image::new(logo).fit_to_exact_size(vec2(34.0, 34.0)));
        }
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_max_width((ui.available_width() - 70.0).max(40.0)); // room for minimize + close
            ui.add(egui::Label::new(RichText::new(title).size(15.0).strong().color(pal().strong)).truncate(true));
            if !sub.is_empty() {
                ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(pal().muted)).truncate(true));
            }
        });
    });
    ui.add_space(4.0);
}

/// Small muted caption above a field.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(11.5).color(pal().muted));
}

/// Rounded dark card to group controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::none()
        .fill(pal().card_bg)
        .stroke(Stroke::new(1.0_f32, pal().card_stroke))
        .rounding(Rounding::same(9.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Rounded dark text input.
pub fn input(ui: &mut Ui, text: &mut String, width: f32) -> Response {
    ui.add_sized(
        [width.max(40.0), FIELD_H],
        egui::TextEdit::singleline(text).margin(vec2(10.0, 7.0)).vertical_align(egui::Align::Center),
    )
}

/// Height shared by every single-line control of a pop-up, so rows line up.
pub const FIELD_H: f32 = 28.0;

/// Fixed-size square button (e.g. "+" / "…") that never grows past FIELD_H: the normal button padding
/// would make it wider than its slot and push the row's right edge out of line.
pub fn square_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(2.0, 2.0);
        ui.add_enabled(enabled, egui::Button::new(label).min_size(vec2(FIELD_H, FIELD_H)))
    })
    .inner
}

/// Square, rounded check box with a label (the stock egui one turns round under the pop-up style).
pub fn check_row(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::TextStyle::Body.resolve(ui.style()), pal().text2);
    let size = vec2(20.0 + 8.0 + galley.size().x, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *checked = !*checked;
        resp.mark_changed();
    }
    let bx = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 8.0), vec2(16.0, 16.0));
    let p = ui.painter();
    if *checked {
        p.rect_filled(bx, Rounding::same(3.0), pal().accent);
        let st = Stroke::new(1.8_f32, pal().on_accent);
        let c = bx.center();
        p.line_segment([c + vec2(-3.6, 0.2), c + vec2(-1.1, 2.8)], st);
        p.line_segment([c + vec2(-1.1, 2.8), c + vec2(3.8, -2.8)], st);
    } else {
        let edge = if resp.hovered() { pal().check_edge_hot } else { pal().check_edge };
        p.rect_filled(bx, Rounding::same(3.0), pal().card_bg);
        p.rect_stroke(bx, Rounding::same(3.0), Stroke::new(1.2_f32, edge));
    }
    p.galley(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, pal().text2);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Several-line variant of `input`.
pub fn input_multiline(ui: &mut Ui, text: &mut String, rows: usize) -> Response {
    ui.add(
        egui::TextEdit::multiline(text)
            .desired_width(ui.available_width())
            .desired_rows(rows)
            .margin(vec2(8.0, 6.0)),
    )
}

/// Pushes the next row (normally the buttons) down to the bottom edge of the pop-up.
pub fn pin_bottom(ui: &mut Ui, row_height: f32) {
    let rest = ui.available_height() - row_height;
    if rest > 0.0 {
        ui.add_space(rest);
    }
}

/// A right-aligned row of buttons, pinned to the bottom of the pop-up; the one at `primary` gets the
/// accent colour. Returns the index (in the order given, left to right) that was clicked.
pub fn action_row(ui: &mut Ui, labels: &[&str], primary: Option<usize>) -> Option<usize> {
    pin_bottom(ui, 30.0);
    let mut hit = None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (i, l) in labels.iter().enumerate().rev() {
            if action_button(ui, l, primary == Some(i)).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// A caption and a read-only value in a rounded dark card with light text; the whole card copies
/// the value when clicked.
pub fn info_field(ui: &mut Ui, caption_text: &str, value: &str) {
    caption(ui, caption_text);
    info_value(ui, value);
}

/// Width of the left label column in the wide, label-beside-field pop-up layout.
pub const LABEL_W: f32 = 76.0;

/// Label drawn at the left of a field row (vertically centred on the field).
pub fn field_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(LABEL_W, FIELD_H), Sense::hover());
    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), pal().muted);
}

/// Read-only value box (click copies), filling the remaining width of the row.
pub fn info_value(ui: &mut Ui, value: &str) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect_filled(rect, Rounding::same(7.0), if hot { pal().card_hot } else { pal().card_bg });
    ui.painter().rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0_f32, pal().card_stroke));
    let text_rect = egui::Rect::from_min_max(rect.min + vec2(10.0, 0.0), rect.max - vec2(10.0, 0.0));
    ui.allocate_ui_at_rect(text_rect, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).color(pal().text2)).truncate(true));
        });
    });
    if resp.clicked() {
        ui.output_mut(|o| o.copied_text = value.to_string());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{value}\n\nClick to copy"));
}

/// Rounded flat button; `primary` is the accent-coloured main action.
pub fn action_button(ui: &mut Ui, label: &str, primary: bool) -> Response {
    let (fill, text) = if primary {
        (pal().accent, pal().on_accent)
    } else {
        (pal().btn_bg, pal().text2)
    };
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        let hover = if primary { pal().accent_hover } else { pal().btn_hover };
        v.widgets.hovered.weak_bg_fill = hover;
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.active.weak_bg_fill = hover;
        v.widgets.active.bg_stroke = Stroke::NONE;
        ui.add(
            egui::Button::new(RichText::new(label).color(text))
                .fill(fill)
                .rounding(Rounding::same(7.0))
                .min_size(vec2(88.0, 30.0)),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

/// "Drag the file out" handle: a page with a folded corner and an arrow leaving it.
pub fn file_drag_handle(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 28.0), Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    if hot {
        ui.painter().rect_filled(rect, Rounding::same(7.0), pal().chip_bg);
    }
    let col = if hot { pal().strong } else { pal().glyph };
    let st = Stroke::new(1.4_f32, col);
    let c = rect.center();
    let at = |x: f32, y: f32| c + vec2(x, y);
    let p = ui.painter();
    // page with a folded top-right corner
    p.add(egui::Shape::closed_line(
        vec![at(-11.0, -8.0), at(-4.0, -8.0), at(1.0, -3.0), at(1.0, 8.0), at(-11.0, 8.0)],
        st,
    ));
    p.line_segment([at(-4.0, -8.0), at(-4.0, -3.0)], st);
    p.line_segment([at(-4.0, -3.0), at(1.0, -3.0)], st);
    // arrow pointing out of the page
    p.line_segment([at(3.0, 1.0), at(12.0, 1.0)], st);
    p.line_segment([at(8.5, -2.5), at(12.0, 1.0)], st);
    p.line_segment([at(8.5, 4.5), at(12.0, 1.0)], st);
    resp
}

/// Slim rounded progress bar.
pub fn progress_bar(ui: &mut Ui, frac: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(5.0), pal().track);
    let mut fill = rect;
    fill.set_width((rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { 10.0 } else { 0.0 }));
    ui.painter().rect_filled(fill, Rounding::same(5.0), color);
}

/// Pin toggle in the top-right corner of a pop-up, left of the close cross ("always on top").
/// Call at the start of the pop-up's content closure; it positions itself from the content area.
/// Returns true when it was clicked; the caller flips its flag and sends the window-level command.
pub fn pin_button(ui: &mut Ui, pinned: bool) -> bool {
    let window = ui.max_rect().expand2(vec2(22.0, 18.0)); // content area + the pop-up's margins
    let r = egui::Rect::from_min_size(egui::pos2(window.right() - 70.0, window.top() + 10.0), vec2(28.0, 28.0));
    let resp = ui.interact(r, ui.id().with("bare-pin"), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(r, Rounding::same(7.0), pal().chip_bg);
    }
    let col = if pinned { pal().strong } else if hot { pal().glyph } else { pal().muted };
    let st = Stroke::new(1.3_f32, col);
    let c = r.center();
    // a push-pin tilted 45 degrees (head up-right, needle down-left), like the Windows 11 pin
    let (sn, cs) = (std::f32::consts::FRAC_PI_4).sin_cos();
    let at = |x: f32, y: f32| c + vec2(x * cs - y * sn, x * sn + y * cs);
    let head = vec![at(-2.6, -7.0), at(2.6, -7.0), at(2.6, -1.5), at(-2.6, -1.5)];
    let flange = vec![at(-2.6, -1.5), at(2.6, -1.5), at(5.2, 1.2), at(-5.2, 1.2)];
    let outline = vec![at(-2.6, -7.0), at(2.6, -7.0), at(2.6, -1.5), at(5.2, 1.2), at(-5.2, 1.2), at(-2.6, -1.5)];
    let p = ui.painter();
    if pinned {
        p.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
        p.add(egui::Shape::convex_polygon(flange, col, Stroke::NONE));
    } else {
        p.add(egui::Shape::closed_line(outline, st));
    }
    p.line_segment([at(0.0, 1.2), at(0.0, 7.5)], st);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if pinned { "Always on top: on" } else { "Always on top: off" })
        .clicked()
}

/// The small "minimize" dash left of the close cross of a title-bar-less pop-up.
fn minimize_button(ui: &mut Ui, window: egui::Rect) -> Response {
    let r = egui::Rect::from_min_size(egui::pos2(window.right() - 70.0, window.top() + 10.0), vec2(28.0, 28.0));
    let resp = ui.interact(r, ui.id().with("bare-minimize"), egui::Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(r, egui::Rounding::same(7.0), pal().chip_bg);
    }
    let c = r.center();
    let st = egui::Stroke::new(1.4_f32, if hot { pal().strong } else { pal().muted });
    ui.painter().line_segment([c + vec2(-5.0, 3.0), c + vec2(5.0, 3.0)], st);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Minimize")
}

/// The small cross in the top-right corner of a title-bar-less pop-up.
fn close_cross(ui: &mut Ui, window: egui::Rect) -> Response {
    let r = egui::Rect::from_min_size(egui::pos2(window.right() - 38.0, window.top() + 10.0), vec2(28.0, 28.0));
    let resp = ui.interact(r, ui.id().with("bare-close"), egui::Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(r, egui::Rounding::same(7.0), pal().chip_bg);
    }
    let c = r.center();
    let st = egui::Stroke::new(1.4_f32, if hot { pal().strong } else { pal().muted });
    ui.painter().line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], st);
    ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], st);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close")
}
