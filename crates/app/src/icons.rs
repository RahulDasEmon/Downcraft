//! Vector icons drawn with the egui painter so they look identical on every OS.
use eframe::egui::{
    epaint::PathShape, pos2, vec2, Color32, ColorImage, Context, Painter, Pos2, Rect, Rounding, Shape, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use crate::skin::{self, Skin};
use downcraft_core::Category;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Add,
    Resume,
    Stop,
    StopAll,
    Delete,
    DeleteCompleted,
    Options,
    Scheduler,
    StartQueue,
    StopQueue,
    Friend,
}

fn poly(p: &Painter, pts: Vec<Pos2>, fill: Color32) {
    p.add(Shape::Path(PathShape::convex_polygon(pts, fill, Stroke::NONE)));
}

/// Toolbar artwork: 192px PNGs cut from the Full HD artwork (clean alpha edges, colours faded 25%) (`assets/tools/*.png`), embedded in the exe.
const ART: [(&str, &[u8]); 11] = [
    ("add", include_bytes!("../../../assets/tools/add.png")),
    ("resume", include_bytes!("../../../assets/tools/resume.png")),
    ("stop", include_bytes!("../../../assets/tools/stop.png")),
    ("stop_all", include_bytes!("../../../assets/tools/stop_all.png")),
    ("delete", include_bytes!("../../../assets/tools/delete.png")),
    ("delete_completed", include_bytes!("../../../assets/tools/delete_completed.png")),
    ("options", include_bytes!("../../../assets/tools/options.png")),
    ("scheduler", include_bytes!("../../../assets/tools/scheduler.png")),
    ("start_queue", include_bytes!("../../../assets/tools/start_queue.png")),
    ("stop_queue", include_bytes!("../../../assets/tools/stop_queue.png")),
    ("friend", include_bytes!("../../../assets/tools/friend.png")),
];

/// Toolbar icon size in points; the texture is pre-scaled to exactly this many *physical* pixels.
const TOOL_PT: f32 = 36.0;

/// Cached textures plus the physical pixel size they were built for.
static TEXTURES: Mutex<Option<(u32, Arc<Vec<TextureHandle>>)>> = Mutex::new(None);

/// egui 0.27 has no mipmaps, so squeezing a 192 px image into ~36 px with bilinear sampling skips most
/// source pixels and looks blurry/jagged. Instead each icon is resized once with Lanczos to the exact
/// on-screen pixel size (rebuilt if the DPI scale changes) and then drawn 1:1.
fn textures(ctx: &Context, px: u32) -> Arc<Vec<TextureHandle>> {
    let mut g = TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cur, t)) = g.as_ref() {
        if *cur == px {
            return t.clone();
        }
    }
    let t: Arc<Vec<TextureHandle>> = Arc::new(
        ART.iter()
            .map(|(name, bytes)| {
                let img = image::load_from_memory(bytes).expect("embedded icon").to_rgba8();
                let img = if img.width() != px {
                    image::imageops::resize(&img, px, px, image::imageops::FilterType::Lanczos3)
                } else {
                    img
                };
                let size = [img.width() as usize, img.height() as usize];
                let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                ctx.load_texture(format!("tool-{name}-{px}"), color, TextureOptions::LINEAR)
            })
            .collect(),
    );
    *g = Some((px, t.clone()));
    t
}

/// Toolbar skin chosen by the user (see `skin.rs`); `None` = the built-in art above.
struct SkinState {
    skin: Option<Skin>,
    /// GPU copy of `skin`, built on first use because it needs the egui context.
    tex: Option<Arc<SkinTex>>,
}

struct SkinTex {
    normal: Vec<(TextureHandle, Vec2)>,
    hot: Option<Vec<(TextureHandle, Vec2)>>,
}

static SKIN: Mutex<SkinState> = Mutex::new(SkinState { skin: None, tex: None });

fn skin_state() -> MutexGuard<'static, SkinState> {
    SKIN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs (or, with `None`, removes) the toolbar skin.
pub fn set_skin(skin: Option<Skin>) {
    let mut g = skin_state();
    g.skin = skin;
    g.tex = None;
}

/// Name of the skin in use, `None` for the default icons.
pub fn skin_name() -> Option<String> {
    skin_state().skin.as_ref().map(|s| s.name.clone())
}

/// True when the skin brings its own hover images (then the grey hover plate is not drawn).
pub fn skin_has_hot() -> bool {
    skin_state().skin.as_ref().map_or(false, |s| s.hot.is_some())
}

fn skin_textures(ctx: &Context) -> Option<Arc<SkinTex>> {
    let mut g = skin_state();
    if g.tex.is_none() {
        let upload = |name: &str, imgs: &[ColorImage]| -> Vec<(TextureHandle, Vec2)> {
            imgs.iter()
                .enumerate()
                .map(|(i, img)| {
                    let size = vec2(img.size[0] as f32, img.size[1] as f32);
                    (ctx.load_texture(format!("skin-{name}-{i}"), img.clone(), TextureOptions::LINEAR), size)
                })
                .collect()
        };
        g.tex = g.skin.as_ref().map(|s| {
            Arc::new(SkinTex {
                normal: upload("n", &s.normal),
                hot: s.hot.as_ref().map(|h| upload("h", h)),
            })
        });
    }
    g.tex.clone()
}

/// Draws a toolbar icon centred on `c`; disabled icons are drawn faint. `hover` swaps in the skin's
/// "hot" image when it has one.
pub fn tool(ctx: &Context, p: &Painter, c: Pos2, icon: Tool, enabled: bool, hover: bool) {
    let tint = if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if let Some(sk) = skin_textures(ctx) {
        // a skin strip has the Grabber in slot 10, which DownCraft has no button for
        let slot = if icon == Tool::Friend { 11 } else { icon as usize }.min(skin::SLOTS - 1);
        let set = match (&sk.hot, hover && enabled) {
            (Some(h), true) => h,
            _ => &sk.normal,
        };
        let (tex, size) = &set[slot];
        // native size, shrunk only when it would not fit the 70 x 46 px the button leaves for it
        let k = (64.0 / size.x).min(46.0 / size.y).min(1.0);
        p.image(tex.id(), Rect::from_center_size(c, *size * k), uv, tint);
        return;
    }
    let ppp = ctx.pixels_per_point();
    let px = (TOOL_PT * ppp).round().max(1.0) as u32;
    let texs = textures(ctx, px);
    let tex = &texs[icon as usize];
    // snap to the physical pixel grid so the 1:1 texture is never resampled between pixels
    let side = px as f32 / ppp;
    let min = pos2(((c.x - side / 2.0) * ppp).round() / ppp, ((c.y - side / 2.0) * ppp).round() / ppp);
    p.image(tex.id(), Rect::from_min_size(min, vec2(side, side)), uv, tint);
}

pub fn category(p: &Painter, c: Pos2, cat: Category, k: f32) {
    let q = |x: f32, y: f32| pos2(c.x + x * k, c.y + y * k);
    let v = |w: f32, h: f32| vec2(w * k, h * k);
    let r = |x: f32| Rounding::same(x * k);
    match cat {
        Category::Compressed => {
            let y = Color32::from_rgb(0xe8, 0xb8, 0x4a);
            p.rect_filled(Rect::from_center_size(c, v(14.0, 11.0)), r(2.0), y);
            p.rect_filled(Rect::from_min_size(q(-7.0, -7.0), v(6.0, 3.0)), r(1.0), y);
            for i in 0..3 {
                p.rect_filled(
                    Rect::from_center_size(q(0.0, -3.0 + i as f32 * 3.5), v(3.0, 1.8)),
                    Rounding::ZERO,
                    Color32::from_rgb(0x6b, 0x4a, 0x10),
                );
            }
        }
        Category::Documents => {
            p.rect_filled(Rect::from_center_size(c, v(10.0, 14.0)), r(1.5), Color32::from_rgb(0xe8, 0xe8, 0xe8));
            for i in 0..3 {
                let y = -3.5 + i as f32 * 3.2;
                p.line_segment([q(-3.0, y), q(3.0, y)], Stroke::new(1.2 * k, Color32::from_rgb(0x4a, 0x7a, 0xd0)));
            }
        }
        Category::Music => {
            let b = Color32::from_rgb(0x4a, 0x9a, 0xf0);
            p.circle_filled(q(-2.0, 4.0), 3.6 * k, b);
            p.line_segment([q(1.2, 4.0), q(1.2, -6.0)], Stroke::new(1.8 * k, b));
            p.line_segment([q(1.2, -6.0), q(5.0, -4.0)], Stroke::new(1.8 * k, b));
        }
        Category::Programs => {
            p.rect_filled(Rect::from_center_size(c, v(14.0, 12.0)), r(1.5), Color32::from_rgb(0x3a, 0x7a, 0xd8));
            p.rect_filled(Rect::from_min_size(q(-7.0, -6.0), v(14.0, 3.0)), r(1.0), Color32::from_rgb(0xdd, 0xe8, 0xff));
        }
        Category::Video => {
            // traffic-cone, as in the reference screenshots
            poly(p, vec![q(0.0, -7.0), q(7.0, 6.0), q(-7.0, 6.0)], Color32::from_rgb(0xf2, 0x8a, 0x1a));
            p.line_segment([q(-3.0, 0.0), q(3.0, 0.0)], Stroke::new(1.8 * k, Color32::WHITE));
        }
    }
}

/// The folder-like nodes of IDM's category tree.
#[derive(Clone, Copy, PartialEq)]
pub enum Node {
    /// open folder – "All Downloads"
    All,
    /// folder with a green arrow – "Unfinished"
    Unfinished,
    /// folder with a blue tick – "Finished"
    Finished,
    /// folder with a globe – "Grabber projects"
    Grabber,
    /// stacked folders – "Queues" and the queue column of the list
    Queues,
}

const FOLDER_FILL: Color32 = Color32::from_rgb(0xf7, 0xe7, 0xa6);
const FOLDER_BACK: Color32 = Color32::from_rgb(0xe3, 0xc0, 0x5a);
const FOLDER_LINE: Color32 = Color32::from_rgb(0xb5, 0x8d, 0x24);

/// One pale-yellow Explorer-style folder, 16 x 13 px, centred on `c`.
fn idm_folder(p: &Painter, c: Pos2, open: bool) {
    let q = |x: f32, y: f32| pos2(c.x + x, c.y + y);
    let line = Stroke::new(1.0_f32, FOLDER_LINE);
    // back plate with the tab
    p.rect_filled(Rect::from_min_size(q(-8.0, -6.0), vec2(7.0, 3.0)), Rounding::same(1.0), FOLDER_BACK);
    p.rect_filled(Rect::from_min_size(q(-8.0, -4.0), vec2(16.0, 11.0)), Rounding::same(1.0), FOLDER_BACK);
    if open {
        // the front flap swings open to the right
        poly(p, vec![q(-8.0, 7.0), q(-5.0, -1.0), q(8.0, -1.0), q(5.0, 7.0)], FOLDER_FILL);
        p.add(Shape::closed_line(vec![q(-8.0, 7.0), q(-5.0, -1.0), q(8.0, -1.0), q(5.0, 7.0)], line));
    } else {
        p.rect_filled(Rect::from_min_size(q(-8.0, -2.0), vec2(16.0, 9.0)), Rounding::same(1.0), FOLDER_FILL);
        p.rect_stroke(Rect::from_min_size(q(-8.0, -4.0), vec2(16.0, 11.0)), Rounding::same(1.0), line);
    }
}

pub fn node(p: &Painter, c: Pos2, n: Node) {
    let q = |x: f32, y: f32| pos2(c.x + x, c.y + y);
    match n {
        Node::All => idm_folder(p, c, true),
        Node::Queues => {
            // three folders fanned out behind each other
            idm_folder(p, q(-2.0, -1.5), false);
            idm_folder(p, q(0.0, 0.0), false);
            idm_folder(p, q(2.0, 1.5), false);
        }
        Node::Unfinished => {
            idm_folder(p, c, false);
            let g = Color32::from_rgb(0x2a, 0xa0, 0x2a);
            p.rect_filled(Rect::from_min_size(q(3.0, -1.0), vec2(4.0, 4.0)), Rounding::ZERO, g);
            poly(p, vec![q(1.0, 3.0), q(9.0, 3.0), q(5.0, 8.0)], g);
        }
        Node::Finished => {
            idm_folder(p, c, false);
            p.add(Shape::line(
                vec![q(0.0, 3.0), q(3.0, 6.5), q(9.0, -1.0)],
                Stroke::new(2.6_f32, Color32::from_rgb(0x2b, 0x62, 0xd0)),
            ));
        }
        Node::Grabber => {
            idm_folder(p, c, false);
            let blue = Color32::from_rgb(0x1f, 0x8c, 0xe6);
            p.circle_filled(q(4.5, 3.5), 4.6, blue);
            p.circle_stroke(q(4.5, 3.5), 4.6, Stroke::new(0.8_f32, Color32::from_rgb(0xff, 0xff, 0xff)));
            p.circle_stroke(q(4.5, 3.5), 2.2, Stroke::new(0.9_f32, Color32::from_rgb(0xbf, 0xe3, 0xff)));
            p.line_segment([q(2.3, 3.5), q(6.7, 3.5)], Stroke::new(0.8_f32, Color32::from_rgb(0xbf, 0xe3, 0xff)));
        }
    }
}
