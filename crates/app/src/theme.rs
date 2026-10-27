//! Colours of the whole app. Everything is a runtime value so the user can switch the theme and the
//! accent colour from the View menu: call [`pal`] wherever a colour is needed and [`set`] to change it.
//!
//! A theme only lists the base colours of the IDM-style main window and of the pop-ups; hover,
//! pressed and similar shades are derived from them ([`lift`]), and everything that marks
//! "selected / primary / focus" comes from the accent colour.
use eframe::egui::{self, Color32, Rounding, Stroke};
use std::sync::{atomic::{AtomicBool, Ordering::Relaxed}, RwLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    /// The original IDM dark-mode look.
    Dark,
    Light,
    Midnight,
    /// Pure black, for OLED screens.
    Black,
    Nord,
    Mocha,
}

impl Theme {
    pub const ALL: [Theme; 6] = [Theme::Dark, Theme::Light, Theme::Midnight, Theme::Black, Theme::Nord, Theme::Mocha];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Midnight => "Midnight",
            Theme::Black => "Black (OLED)",
            Theme::Nord => "Nord",
            Theme::Mocha => "Mocha",
        }
    }

    /// Stable name kept in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Midnight => "midnight",
            Theme::Black => "black",
            Theme::Nord => "nord",
            Theme::Mocha => "mocha",
        }
    }

    /// Unknown or empty keys fall back to the original dark theme.
    pub fn from_key(k: &str) -> Theme {
        Theme::ALL.into_iter().find(|t| t.key() == k.trim().to_lowercase()).unwrap_or(Theme::Dark)
    }

    fn is_light(self) -> bool {
        self == Theme::Light
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    Blue,
    Teal,
    Green,
    Orange,
    Red,
    Pink,
    Purple,
    Yellow,
}

impl Accent {
    pub const ALL: [Accent; 8] =
        [Accent::Blue, Accent::Teal, Accent::Green, Accent::Orange, Accent::Red, Accent::Pink, Accent::Purple, Accent::Yellow];

    pub fn name(self) -> &'static str {
        match self {
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
            Accent::Green => "Green",
            Accent::Orange => "Orange",
            Accent::Red => "Red",
            Accent::Pink => "Pink",
            Accent::Purple => "Purple",
            Accent::Yellow => "Yellow",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Accent::Blue => "blue",
            Accent::Teal => "teal",
            Accent::Green => "green",
            Accent::Orange => "orange",
            Accent::Red => "red",
            Accent::Pink => "pink",
            Accent::Purple => "purple",
            Accent::Yellow => "yellow",
        }
    }

    pub fn from_key(k: &str) -> Accent {
        Accent::ALL.into_iter().find(|a| a.key() == k.trim().to_lowercase()).unwrap_or(Accent::Blue)
    }

    /// (normal, hovered) colour; both read well on light and on dark backgrounds.
    pub fn colors(self) -> (Color32, Color32) {
        let c = |r, g, b| Color32::from_rgb(r, g, b);
        match self {
            Accent::Blue => (c(0x2b, 0x7c, 0xd3), c(0x3a, 0x8c, 0xe4)),
            Accent::Teal => (c(0x16, 0x9c, 0x98), c(0x27, 0xb3, 0xae)),
            Accent::Green => (c(0x2e, 0x9e, 0x4f), c(0x3c, 0xb8, 0x62)),
            Accent::Orange => (c(0xe0, 0x7a, 0x1f), c(0xf0, 0x8c, 0x33)),
            Accent::Red => (c(0xd1, 0x3c, 0x3c), c(0xe6, 0x50, 0x50)),
            Accent::Pink => (c(0xd6, 0x45, 0x8f), c(0xe8, 0x5a, 0xa4)),
            Accent::Purple => (c(0x8a, 0x5c, 0xd6), c(0x9d, 0x72, 0xe6)),
            Accent::Yellow => (c(0xd9, 0xa4, 0x00), c(0xeb, 0xb8, 0x16)),
        }
    }
}

/// Every colour the app draws with.
#[derive(Clone, Copy)]
pub struct Palette {
    pub light: bool,

    // ── main window (IDM look) ──
    /// Primary text of the main window.
    pub text: Color32,
    /// Title bar + menu bar.
    pub menu_bg: Color32,
    /// Toolbar and the gaps between controls.
    pub client_bg: Color32,
    pub tree_bg: Color32,
    pub tree_sel: Color32,
    pub panel_border: Color32,
    pub list_border: Color32,
    pub header_bg: Color32,
    pub header_sep: Color32,
    pub row_bg: Color32,
    pub grid: Color32,
    /// Selected row: the accent mixed into the row colour.
    pub row_sel: Color32,
    pub row_sel_border: Color32,
    pub scroll_track: Color32,
    pub scroll_thumb: Color32,
    pub disabled: Color32,
    pub dots: Color32,
    /// Hover highlight painted over the toolbar and the tree.
    pub hover_tint: Color32,
    /// Sort arrow, tree chevrons.
    pub glyph: Color32,

    // ── window chrome ──
    pub hairline: Color32,
    pub win_border: Color32,
    pub caption_hover: Color32,
    pub caption_ink: Color32,

    // ── pop-ups ──
    pub bare_bg: Color32,
    pub card_bg: Color32,
    pub card_hot: Color32,
    pub card_stroke: Color32,
    pub btn_bg: Color32,
    pub btn_hover: Color32,
    pub btn_hover_stroke: Color32,
    pub btn_active: Color32,
    pub track: Color32,
    pub chip_bg: Color32,
    /// Headings and other emphasised text.
    pub strong: Color32,
    /// Body text of pop-ups.
    pub text2: Color32,
    /// Secondary text.
    pub muted: Color32,
    pub check_edge: Color32,
    pub check_edge_hot: Color32,
    pub good: Color32,
    pub good_text: Color32,
    pub bad: Color32,
    pub warn: Color32,

    // ── egui's own widgets ──
    pub panel_fill: Color32,
    pub input_bg: Color32,
    pub w_fill: Color32,
    pub w_stroke: Color32,
    pub w_hover_fill: Color32,
    pub w_hover_stroke: Color32,
    pub w_active_fill: Color32,

    // ── accent ──
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text / check marks drawn on top of the accent.
    pub on_accent: Color32,
}

/// The hand-picked base colours of a theme.
struct Base {
    text: Color32,
    menu_bg: Color32,
    client_bg: Color32,
    tree_bg: Color32,
    tree_sel: Color32,
    panel_border: Color32,
    list_border: Color32,
    header_bg: Color32,
    header_sep: Color32,
    row_bg: Color32,
    grid: Color32,
    scroll_track: Color32,
    disabled: Color32,
    dots: Color32,
    bare_bg: Color32,
    card_bg: Color32,
    card_stroke: Color32,
    btn_bg: Color32,
    muted: Color32,
    strong: Color32,
    text2: Color32,
}

fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn base(t: Theme) -> Base {
    match t {
        // sampled from Internet Download Manager 6.x in Windows dark mode
        Theme::Dark => Base {
            text: rgb(0xffffff),
            menu_bg: rgb(0x202020),
            client_bg: rgb(0x393939),
            tree_bg: rgb(0x202020),
            tree_sel: rgb(0x333333),
            panel_border: rgb(0xdcdcdc),
            list_border: rgb(0x828790),
            header_bg: rgb(0x191919),
            header_sep: rgb(0x636363),
            row_bg: rgb(0x393939),
            grid: rgb(0x555555),
            scroll_track: rgb(0x171717),
            disabled: rgb(0x8c8c8c),
            dots: rgb(0x8a8a8a),
            bare_bg: rgb(0x202020),
            card_bg: rgb(0x2a2a2a),
            card_stroke: rgb(0x3d3d3d),
            btn_bg: rgb(0x343434),
            muted: rgb(0xa9a9a9),
            strong: rgb(0xffffff),
            text2: rgb(0xe8e8e8),
        },
        Theme::Light => Base {
            text: rgb(0x1b1b1b),
            menu_bg: rgb(0xf3f3f3),
            client_bg: rgb(0xf0f0f0),
            tree_bg: rgb(0xffffff),
            tree_sel: rgb(0xdcdcdc),
            panel_border: rgb(0x828790),
            list_border: rgb(0xa0a0a0),
            header_bg: rgb(0xf7f7f7),
            header_sep: rgb(0xd5d5d5),
            row_bg: rgb(0xffffff),
            grid: rgb(0xe3e3e3),
            scroll_track: rgb(0xf0f0f0),
            disabled: rgb(0x9a9a9a),
            dots: rgb(0x7a7a7a),
            bare_bg: rgb(0xf5f5f5),
            card_bg: rgb(0xffffff),
            card_stroke: rgb(0xd6d6d6),
            btn_bg: rgb(0xececec),
            muted: rgb(0x5f5f5f),
            strong: rgb(0x000000),
            text2: rgb(0x202020),
        },
        Theme::Midnight => Base {
            text: rgb(0xe6edf3),
            menu_bg: rgb(0x0d1117),
            client_bg: rgb(0x161b22),
            tree_bg: rgb(0x0d1117),
            tree_sel: rgb(0x1f2a37),
            panel_border: rgb(0x3d444d),
            list_border: rgb(0x30363d),
            header_bg: rgb(0x010409),
            header_sep: rgb(0x30363d),
            row_bg: rgb(0x11161d),
            grid: rgb(0x252b33),
            scroll_track: rgb(0x0a0d12),
            disabled: rgb(0x6e7681),
            dots: rgb(0x6e7681),
            bare_bg: rgb(0x0d1117),
            card_bg: rgb(0x161b22),
            card_stroke: rgb(0x30363d),
            btn_bg: rgb(0x21262d),
            muted: rgb(0x8b949e),
            strong: rgb(0xffffff),
            text2: rgb(0xe6edf3),
        },
        Theme::Black => Base {
            text: rgb(0xf0f0f0),
            menu_bg: rgb(0x000000),
            client_bg: rgb(0x0b0b0b),
            tree_bg: rgb(0x000000),
            tree_sel: rgb(0x1c1c1c),
            panel_border: rgb(0x3a3a3a),
            list_border: rgb(0x3a3a3a),
            header_bg: rgb(0x0f0f0f),
            header_sep: rgb(0x2e2e2e),
            row_bg: rgb(0x000000),
            grid: rgb(0x222222),
            scroll_track: rgb(0x000000),
            disabled: rgb(0x6a6a6a),
            dots: rgb(0x6a6a6a),
            bare_bg: rgb(0x000000),
            card_bg: rgb(0x101010),
            card_stroke: rgb(0x2a2a2a),
            btn_bg: rgb(0x181818),
            muted: rgb(0x9a9a9a),
            strong: rgb(0xffffff),
            text2: rgb(0xe8e8e8),
        },
        Theme::Nord => Base {
            text: rgb(0xeceff4),
            menu_bg: rgb(0x2e3440),
            client_bg: rgb(0x3b4252),
            tree_bg: rgb(0x2e3440),
            tree_sel: rgb(0x434c5e),
            panel_border: rgb(0x616e88),
            list_border: rgb(0x4c566a),
            header_bg: rgb(0x272c36),
            header_sep: rgb(0x4c566a),
            row_bg: rgb(0x3b4252),
            grid: rgb(0x4c566a),
            scroll_track: rgb(0x2a2f39),
            disabled: rgb(0x7b88a1),
            dots: rgb(0x7b88a1),
            bare_bg: rgb(0x2e3440),
            card_bg: rgb(0x3b4252),
            card_stroke: rgb(0x4c566a),
            btn_bg: rgb(0x434c5e),
            muted: rgb(0xaab4c8),
            strong: rgb(0xffffff),
            text2: rgb(0xe5e9f0),
        },
        Theme::Mocha => Base {
            text: rgb(0xf2e9e0),
            menu_bg: rgb(0x1e1a17),
            client_bg: rgb(0x2d2824),
            tree_bg: rgb(0x1e1a17),
            tree_sel: rgb(0x3a332d),
            panel_border: rgb(0x6b5f55),
            list_border: rgb(0x5a4f46),
            header_bg: rgb(0x171412),
            header_sep: rgb(0x5a4f46),
            row_bg: rgb(0x2d2824),
            grid: rgb(0x4a413a),
            scroll_track: rgb(0x161311),
            disabled: rgb(0x8f8378),
            dots: rgb(0x8f8378),
            bare_bg: rgb(0x1e1a17),
            card_bg: rgb(0x2a2521),
            card_stroke: rgb(0x4a413a),
            btn_bg: rgb(0x38322c),
            muted: rgb(0xb3a699),
            strong: rgb(0xffffff),
            text2: rgb(0xeadfd5),
        },
    }
}

/// `t` (0..=1) of the way from `a` to `b` (opacity included).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let (x, y) = (a.to_srgba_unmultiplied(), b.to_srgba_unmultiplied());
    let f = |i: usize| (x[i] as f32 + (y[i] as f32 - x[i] as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(0), f(1), f(2), f(3))
}

/// A shade `amt` steps away from `c`: lighter on dark themes, darker (a bit more gently) on light ones.
fn lift(c: Color32, amt: i32, light: bool) -> Color32 {
    let d = if light { -((amt as f32 * 0.6) as i32) } else { amt };
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let f = |v: u8| (v as i32 + d).clamp(0, 255) as u8;
    Color32::from_rgba_unmultiplied(f(r), f(g), f(b), a)
}

fn luma(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

impl Palette {
    pub fn build(theme: Theme, accent: Accent) -> Palette {
        let b = base(theme);
        let l = theme.is_light();
        let (acc, acc_hover) = accent.colors();
        let edge = if accent == Accent::Blue { rgb(0x0078d4) } else { acc }; // IDM's own selection blue
        Palette {
            light: l,
            text: b.text,
            menu_bg: b.menu_bg,
            client_bg: b.client_bg,
            tree_bg: b.tree_bg,
            tree_sel: b.tree_sel,
            panel_border: b.panel_border,
            list_border: b.list_border,
            header_bg: b.header_bg,
            header_sep: b.header_sep,
            row_bg: b.row_bg,
            grid: b.grid,
            row_sel: mix(b.row_bg, edge, 0.25),
            row_sel_border: edge,
            scroll_track: b.scroll_track,
            scroll_thumb: b.disabled,
            disabled: b.disabled,
            dots: b.dots,
            hover_tint: if l { Color32::from_black_alpha(14) } else { Color32::from_white_alpha(14) },
            glyph: mix(b.text, b.menu_bg, 0.22),

            hairline: lift(b.menu_bg, 0x1a, l),
            win_border: b.grid,
            caption_hover: lift(b.menu_bg, 0x1a, l),
            caption_ink: mix(b.text, b.menu_bg, 0.22),

            bare_bg: b.bare_bg,
            card_bg: b.card_bg,
            card_hot: lift(b.card_bg, 6, l),
            card_stroke: b.card_stroke,
            btn_bg: b.btn_bg,
            btn_hover: lift(b.btn_bg, 14, l),
            btn_hover_stroke: lift(b.card_stroke, 29, l),
            btn_active: lift(b.btn_bg, 24, l),
            track: lift(b.bare_bg, 0x1a, l),
            chip_bg: lift(b.bare_bg, 0x1c, l),
            strong: b.strong,
            text2: b.text2,
            muted: b.muted,
            check_edge: mix(b.text2, b.bare_bg, 0.55),
            check_edge_hot: mix(b.text2, b.bare_bg, 0.35),
            good: rgb(0x2ea043),
            good_text: if l { rgb(0x1e7a2e) } else { rgb(0x7fd87f) },
            bad: if l { rgb(0xd13438) } else { rgb(0xff6b6b) },
            warn: if l { rgb(0xb36b00) } else { rgb(0xe09a1f) },

            panel_fill: lift(b.bare_bg, 0x0b, l),
            input_bg: if l { rgb(0xffffff) } else { b.header_bg },
            w_fill: lift(b.bare_bg, 0x18, l),
            w_stroke: lift(b.bare_bg, 0x4a, l),
            w_hover_fill: lift(b.bare_bg, 0x26, l),
            w_hover_stroke: lift(b.bare_bg, 0x6a, l),
            w_active_fill: lift(b.bare_bg, 0x32, l),

            accent: acc,
            accent_hover: acc_hover,
            on_accent: if luma(acc) > 0.6 { rgb(0x1b1b1b) } else { Color32::WHITE },
        }
    }
}

#[derive(Clone, Copy)]
struct State {
    theme: Theme,
    accent: Accent,
    pal: Palette,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

fn state() -> State {
    if let Some(s) = *STATE.read().unwrap_or_else(|e| e.into_inner()) {
        return s;
    }
    State { theme: Theme::Dark, accent: Accent::Blue, pal: Palette::build(Theme::Dark, Accent::Blue) }
}

/// The colours currently in use. Cheap (a small `Copy` struct); call it where a colour is needed.
pub fn pal() -> Palette {
    state().pal
}

/// Theme and accent currently in use.
pub fn current() -> (Theme, Accent) {
    let s = state();
    (s.theme, s.accent)
}

/// Switches theme and accent colour; the next frame is drawn with them everywhere (pop-ups included).
pub fn set(ctx: &egui::Context, theme: Theme, accent: Accent) {
    *STATE.write().unwrap_or_else(|e| e.into_inner()) = Some(State { theme, accent, pal: Palette::build(theme, accent) });
    apply_visuals(ctx);
    ctx.request_repaint();
}

/// Set when Segoe UI (the font IDM uses) was found on this machine.
static SEGOE: AtomicBool = AtomicBool::new(false);

/// Font size of the main window: 9 pt Segoe UI = 12 px; the bundled fallback font is a bit
/// narrower per point so it gets half a pixel more.
pub fn ui_size() -> f32 {
    if SEGOE.load(Relaxed) {
        12.0
    } else {
        12.5
    }
}

/// IDM is drawn in Segoe UI. It cannot be bundled (it is Microsoft's), so it is loaded from the
/// system when present; everywhere else egui's built-in font stays in use.
fn load_fonts(ctx: &egui::Context) {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for var in ["WINDIR", "SystemRoot"] {
        if let Some(w) = std::env::var_os(var) {
            paths.push(std::path::PathBuf::from(w).join("Fonts").join("segoeui.ttf"));
        }
    }
    paths.push(std::path::PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf"));
    for p in paths {
        if let Ok(bytes) = std::fs::read(&p) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("segoe-ui".into(), egui::FontData::from_owned(bytes));
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                f.insert(0, "segoe-ui".into());
            }
            ctx.set_fonts(fonts);
            SEGOE.store(true, Relaxed);
            return;
        }
    }
}

/// One-time setup: fonts and spacing. The colours come from [`set`].
pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    apply_visuals(ctx);
    let mut s = (*ctx.style()).clone();
    s.spacing.item_spacing = egui::vec2(6.0, 5.0);
    s.spacing.button_padding = egui::vec2(10.0, 3.0);
    s.spacing.interact_size.y = 22.0;
    s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(ui_size() + 0.5));
    s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(ui_size() + 0.5));
    s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(11.5));
    ctx.set_style(s);
}

/// egui's own widgets (menus, buttons, text fields, scroll bars) in the current palette.
fn apply_visuals(ctx: &egui::Context) {
    let p = pal();
    let mut v = if p.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = p.panel_fill;
    v.window_fill = p.panel_fill;
    v.extreme_bg_color = p.input_bg;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.row_sel;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.hyperlink_color = p.accent;
    v.window_stroke = Stroke::new(1.0_f32, p.win_border);
    let r = Rounding::same(3.0);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive] {
        w.rounding = r;
    }
    v.widgets.inactive.bg_fill = p.w_fill;
    v.widgets.inactive.weak_bg_fill = p.w_fill;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.w_stroke);
    v.widgets.hovered.bg_fill = p.w_hover_fill;
    v.widgets.hovered.weak_bg_fill = p.w_hover_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.w_hover_stroke);
    v.widgets.active.bg_fill = p.w_active_fill;
    v.widgets.active.weak_bg_fill = p.w_active_fill;
    ctx.set_visuals(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_unknown_falls_back() {
        for t in Theme::ALL {
            assert_eq!(Theme::from_key(t.key()), t);
        }
        for a in Accent::ALL {
            assert_eq!(Accent::from_key(a.key()), a);
        }
        assert_eq!(Theme::from_key("nope"), Theme::Dark);
        assert_eq!(Accent::from_key(""), Accent::Blue);
    }

    #[test]
    fn dark_blue_keeps_the_original_idm_colours() {
        let p = Palette::build(Theme::Dark, Accent::Blue);
        assert_eq!(p.menu_bg, Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(p.row_sel_border, Color32::from_rgb(0x00, 0x78, 0xd4));
        assert_eq!(p.btn_hover, Color32::from_rgb(0x42, 0x42, 0x42));
        assert_eq!(p.accent, Color32::from_rgb(0x2b, 0x7c, 0xd3));
    }

    #[test]
    fn accent_text_is_readable() {
        assert_eq!(Palette::build(Theme::Dark, Accent::Yellow).on_accent, Color32::from_rgb(0x1b, 0x1b, 0x1b));
        assert_eq!(Palette::build(Theme::Dark, Accent::Blue).on_accent, Color32::WHITE);
    }
}
