use crate::{icons, theme::*, widgets::*};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Rect, RichText, Rounding, Sense, Stroke, TextureHandle};
use downcraft_core::tools::{self, Busy, ToolBox};
use downcraft_core::ytdlp::{Choice, Video};
use downcraft_core::{
    probe, util, AfterQueue, AudioPart, Category, Manager, NewDownload, Probe, Schedule, Status, Task, SEG_CONNECTING, SEG_DONE, SEG_ERROR,
    SEG_RECEIVING,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub enum Out<T> {
    Keep,
    Close,
    Done(T),
}

// ───────────────────────────── Add URL ─────────────────────────────

pub struct AddUrl {
    pub url: String,
    pub error: Option<String>,
    pub focused: bool,
    /// The user pressed *As video*: send every address to yt-dlp, even one on an unknown site.
    pub video: bool,
}

pub fn add_url_ui(ctx: &egui::Context, d: &mut AddUrl) -> Out<String> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "add-url", "Enter new address to download from", [520.0, 318.0], false, |ui| {
        heading(ui, "Add download", "Paste a link - or several, one per line", Badge::None);
        caption(ui, "Address");
        let r = input_multiline(ui, &mut d.url, 5);
        if !d.focused {
            r.request_focus();
            d.focused = true;
        }
        if let Some(e) = &d.error {
            ui.colored_label(pal().bad, e);
        }
        ui.add(
            egui::Label::new(
                RichText::new("Facebook, Instagram, X, TikTok, YouTube, Vimeo and other video pages are downloaded with yt-dlp. “As video” sends any other page there too.")
                    .size(11.5)
                    .color(pal().muted),
            )
            .wrap(true),
        );
        // Ctrl+Enter submits (plain Enter adds a new line)
        let mut ok = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
        match action_row(ui, &["Cancel", "As video", "OK"], Some(2)) {
            Some(2) => ok = true,
            Some(1) => {
                d.video = true;
                ok = true;
            }
            Some(_) => out = Out::Close,
            None => {}
        }
        if ok {
            out = Out::Done(d.url.clone());
        }
    });
    if closed {
        return Out::Close;
    }
    out
}

// ───────────────────── Video from a page (yt-dlp) ─────────────────────

#[derive(Clone)]
enum PickState {
    Loading,
    Ready(Vec<Video>),
    Failed(String),
}

/// What the user decided in the *Video download* window.
pub enum PickOut {
    /// A chosen row and the file name to save it under.
    Download(Choice, String),
    /// yt-dlp could not help: download the address as an ordinary link.
    PlainLink(String),
}

/// A page on Facebook, Instagram, X … : yt-dlp lists its videos and the user picks a quality.
/// The chosen link then goes through *Download File Info* like any other download.
pub struct VideoPick {
    key: u64,
    pub url: String,
    shared: Arc<Mutex<PickState>>,
    video: usize,
    sel: usize,
}

impl VideoPick {
    pub fn new(ctx: &egui::Context, mgr: &Arc<Manager>, key: u64, url: String) -> Self {
        let shared = Arc::new(Mutex::new(PickState::Loading));
        {
            let (s, m, u, ctx) = (shared.clone(), mgr.clone(), url.clone(), ctx.clone());
            std::thread::spawn(move || {
                let r = match m.ytdlp_fetch(&u) {
                    Ok(v) => PickState::Ready(v),
                    Err(e) => PickState::Failed(e),
                };
                *s.lock().unwrap() = r;
                ctx.request_repaint();
            });
        }
        VideoPick { key, url, shared, video: 0, sel: 0 }
    }
}

fn duration_text(secs: f64) -> String {
    let s = secs.round() as u64;
    if s == 0 {
        return String::new();
    }
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn video_pick_ui(ctx: &egui::Context, d: &mut VideoPick) -> Out<PickOut> {
    let state = d.shared.lock().unwrap().clone();
    let mut out = Out::Keep;
    let site = d.url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").trim_start_matches("www.").to_string();
    let closed = show_dialog_bare(ctx, ("video-pick", d.key), "Download video", [560.0, 400.0], true, |ui| {
        heading(ui, "Video download", &format!("Formats found by yt-dlp on {site}"), Badge::None);
        match &state {
            PickState::Loading => {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Getting the format list from yt-dlp…").color(pal().muted));
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                if action_row(ui, &["Cancel"], None).is_some() {
                    out = Out::Close;
                }
            }
            PickState::Failed(e) => {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(e).color(pal().bad)).wrap(true));
                match action_row(ui, &["Cancel", "Download link as is"], None) {
                    Some(0) => out = Out::Close,
                    Some(_) => out = Out::Done(PickOut::PlainLink(d.url.clone())),
                    None => {}
                }
            }
            PickState::Ready(videos) => {
                let vi = d.video.min(videos.len().saturating_sub(1));
                let v = &videos[vi];
                if videos.len() > 1 {
                    let label = |i: usize, v: &Video| {
                        let t = if v.title.is_empty() { &v.uploader } else { &v.title };
                        let dur = duration_text(v.duration);
                        format!("Video {} of {}: {}{}", i + 1, videos.len(), t.chars().take(60).collect::<String>(), if dur.is_empty() { String::new() } else { format!("  ({dur})") })
                    };
                    egui::ComboBox::from_id_source(("pick-video", d.key)).width(ui.available_width() - 4.0).selected_text(label(vi, v)).show_ui(ui, |ui| {
                        for (i, x) in videos.iter().enumerate() {
                            if ui.selectable_value(&mut d.video, i, label(i, x)).changed() {
                                d.sel = 0;
                            }
                        }
                    });
                } else {
                    let t = if v.title.is_empty() { &v.uploader } else { &v.title };
                    let dur = duration_text(v.duration);
                    ui.add(egui::Label::new(RichText::new(format!("{}{}", t, if dur.is_empty() { String::new() } else { format!("  ·  {dur}") })).color(pal().strong)).truncate(true));
                }
                ui.add_space(4.0);
                let sel = d.sel.min(v.choices.len().saturating_sub(1));
                egui::ScrollArea::vertical().id_source(("pick-rows", d.key)).auto_shrink([false, false]).max_height((ui.available_height() - 46.0).max(60.0)).show(ui, |ui| {
                    for (i, c) in v.choices.iter().enumerate() {
                        let size = if c.size > 0 { format!("   {}{}", if c.approx { "~" } else { "" }, util::fmt_size(c.size)) } else { String::new() };
                        let r = ui.selectable_label(sel == i, format!("{}{}", c.text, size));
                        let r = if c.tip.is_empty() { r } else { r.on_hover_text(&c.tip) };
                        if r.clicked() {
                            d.sel = i;
                        }
                        if r.double_clicked() {
                            out = Out::Done(PickOut::Download(c.clone(), v.file_name(c)));
                        }
                    }
                });
                if let Some(1) = action_row(ui, &["Cancel", "Download"], Some(1)) {
                    if let Some(c) = v.choices.get(sel) {
                        out = Out::Done(PickOut::Download(c.clone(), v.file_name(c)));
                    }
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    out = Out::Close;
                }
            }
        }
    });
    if closed {
        return Out::Close;
    }
    out
}

// ───────────────────────── Download File Info ─────────────────────────

#[derive(Clone)]
pub enum ProbeState {
    Pending,
    Done(Probe),
    Failed(String),
}

pub struct Confirm {
    pub key: u64,
    pub url: String,
    pub category: Category,
    pub save_as: String,
    pub description: String,
    pub remember: bool,
    pub shared: Arc<Mutex<ProbeState>>,
    pub state: ProbeState,
    pub applied: bool,
    pub path_edited: bool,
    pub headers: Vec<(String, String)>,
    /// File name suggested by the browser extension (kept even after the probe finishes).
    pub hint: Option<String>,
    /// Size the browser reported (0 = unknown); shown until the probe answers.
    pub size_hint: u64,
    /// Sound track to download and merge in when the link is a video-only stream.
    pub audio: Option<AudioPart>,
}

impl Confirm {
    pub fn new(
        ctx: &egui::Context,
        mgr: &Arc<Manager>,
        key: u64,
        url: String,
        headers: Vec<(String, String)>,
        hint: Option<String>,
        size_hint: u64,
    ) -> Self {
        let shared = Arc::new(Mutex::new(ProbeState::Pending));
        {
            let (s, c, u, ctx, h) = (shared.clone(), mgr.client(), url.clone(), ctx.clone(), mgr.with_auth(&url, &headers));
            std::thread::spawn(move || {
                let r = match probe(&c, &u, &h) {
                    Ok(p) => ProbeState::Done(p),
                    Err(e) => ProbeState::Failed(e),
                };
                *s.lock().unwrap() = r;
                ctx.request_repaint();
            });
        }
        let mut d = Confirm {
            key,
            url: url.clone(),
            category: Category::Documents,
            save_as: String::new(),
            description: String::new(),
            remember: mgr.settings.lock().remember_folder_default,
            shared,
            state: ProbeState::Pending,
            applied: false,
            path_edited: false,
            headers,
            hint: hint.map(|h| util::sanitize_file_name(&util::hls_name_if_playlist(&h))),
            size_hint,
            audio: None,
        };
        let first = d.hint.clone().unwrap_or_else(|| util::file_name_from_url(&url));
        d.set_name(mgr, &first);
        d
    }

    fn set_name(&mut self, mgr: &Manager, name: &str) {
        self.category = Category::from_file_name(name);
        let dir = mgr.settings.lock().dir_for(self.category);
        self.save_as = dir.join(name).to_string_lossy().into_owned();
    }
}

pub fn confirm_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut Confirm) -> Out<Arc<Task>> {
    d.state = d.shared.lock().unwrap().clone();
    if !d.applied {
        match d.state.clone() {
            ProbeState::Done(p) => {
                d.applied = true;
                if !d.path_edited && d.hint.is_none() {
                    d.set_name(mgr, &p.file_name);
                }
            }
            ProbeState::Failed(_) => d.applied = true,
            ProbeState::Pending => {}
        }
    }

    let mut result = Out::Keep;
    let size_text = match &d.state {
        ProbeState::Pending if d.size_hint > 0 => util::fmt_size(d.size_hint),
        ProbeState::Pending => "…".to_string(),
        ProbeState::Done(p) if p.hls => "HLS stream".to_string(),
        ProbeState::Done(p) if p.total > 0 => util::fmt_size(p.total),
        ProbeState::Done(_) => "Unknown size".to_string(),
        ProbeState::Failed(_) => "Error".to_string(),
    };
    let failure = if let ProbeState::Failed(e) = &d.state { Some(e.clone()) } else { None };

    let closed = show_dialog_min(ctx, ("confirm", d.key), "Download File Info", [560.0, 276.0], true, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        heading_small(ui, "Download file info", &size_text);

        ui.horizontal(|ui| {
            field_label(ui, "Address");
            info_value(ui, &d.url);
        });
        ui.horizontal(|ui| {
            field_label(ui, "Category");
            let (rect, _) = ui.allocate_exact_size(vec2(28.0, FIELD_H), Sense::hover());
            icons::category(ui.painter(), rect.center(), d.category, 1.3);
            let before = d.category;
            let cw = (ui.available_width() - FIELD_H - ui.spacing().item_spacing.x).max(120.0);
            ui.spacing_mut().interact_size.y = FIELD_H;
            egui::ComboBox::from_id_source(("cat", d.key))
                .width(cw)
                .selected_text(d.category.name())
                .show_ui(ui, |ui| {
                    for c in Category::ALL {
                        ui.selectable_value(&mut d.category, c, c.name());
                    }
                });
            if before != d.category {
                let name = util::split_path(&d.save_as).1;
                let dir = mgr.settings.lock().dir_for(d.category);
                d.save_as = dir.join(name).to_string_lossy().into_owned();
                d.path_edited = true;
            }
            square_button(ui, "+", false).on_disabled_hover_text("Custom categories are coming soon");
        });
        ui.horizontal(|ui| {
            field_label(ui, "Save as");
            let w = ui.available_width() - FIELD_H - ui.spacing().item_spacing.x;
            if input(ui, &mut d.save_as, w).changed() {
                d.path_edited = true;
            }
            if square_button(ui, "…", true)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Choose folder and file name")
                .clicked()
            {
                let (dir, name) = util::split_path(&d.save_as);
                if let Some(p) = rfd::FileDialog::new().set_directory(dir).set_file_name(name).save_file() {
                    d.save_as = p.to_string_lossy().into_owned();
                    d.path_edited = true;
                }
            }
        });
        // remember check box and the chosen folder share one line
        let indent = LABEL_W + ui.spacing().item_spacing.x;
        let remember_label = format!("Remember this folder for \"{}\"", d.category.name());
        let dir = format!("{}{}", util::split_path(&d.save_as).0.display(), std::path::MAIN_SEPARATOR);
        ui.horizontal(|ui| {
            ui.add_space(indent);
            check_row(ui, &mut d.remember, &remember_label);
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(dir).size(11.5).color(pal().muted)).truncate(true));
        });
        ui.horizontal(|ui| {
            field_label(ui, "Description");
            let w = ui.available_width();
            input(ui, &mut d.description, w);
        });
        ui.add_space(4.0);

        if let Some(e) = &failure {
            ui.add(egui::Label::new(RichText::new(format!("Could not read file info: {e}")).color(pal().bad)).truncate(true))
                .on_hover_text(e);
        }
        match action_row(ui, &["Cancel", "Download later", "Start download"], Some(2)) {
            Some(i @ (1 | 2)) => {
                let (dir, name) = util::split_path(&d.save_as);
                if d.remember {
                    mgr.settings.lock().remembered.insert(d.category, dir.clone());
                    mgr.save_settings();
                }
                let probe = if let ProbeState::Done(p) = &d.state { Some(p.clone()) } else { None };
                let merge = d.audio.is_some();
                let task = mgr.add(NewDownload {
                    url: d.url.clone(),
                    dir: dir.clone(),
                    file_name: name.clone(),
                    category: d.category,
                    description: d.description.clone(),
                    probe,
                    queued: i == 1,
                    start: i == 2 && !merge,
                    headers: d.headers.clone(),
                });
                if let Some(a) = &d.audio {
                    // the sound track goes next to the video under a temporary name; it is merged
                    // into the video's file and removed when both are complete
                    let stem = util::split_path(&task.info.lock().file_name.clone()).1;
                    let stem = stem.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(stem);
                    let aname = format!("{stem}.audio.{}", a.ext);
                    let atask = mgr.add(NewDownload {
                        url: a.url.clone(),
                        dir,
                        file_name: aname.clone(),
                        category: Category::from_file_name(&aname),
                        description: String::new(),
                        probe: None,
                        queued: i == 1,
                        start: false,
                        headers: a.headers.clone(),
                    });
                    mgr.add_merge(&task, &atask);
                    if i == 2 {
                        mgr.start(&task);
                        mgr.start(&atask);
                    }
                }
                result = if i == 2 { Out::Done(task) } else { Out::Close };
            }
            Some(_) => result = Out::Close,
            None => {}
        }
    });
    if closed {
        return Out::Close;
    }
    result
}

// ───────────────────── Download progress ─────────────────────

pub struct Progress {
    pub task: Arc<Task>,
    pub details: bool,
    pub tab: usize,
    pub limit_on: bool,
    pub limit_kbps: u64,
    pub open_file: bool,
    pub open_folder: bool,
    pub exit_when_done: bool,
    shown_details: bool,
    last_title: String,
}

impl Progress {
    pub fn new(task: Arc<Task>) -> Self {
        let lim = task.limit_kbps.load(std::sync::atomic::Ordering::Relaxed);
        Progress {
            task,
            details: true,
            tab: 0,
            limit_on: lim > 0,
            limit_kbps: if lim > 0 { lim } else { 512 },
            open_file: false,
            open_folder: false,
            exit_when_done: false,
            shown_details: true,
            last_title: String::new(),
        }
    }
}

fn status_text(status: Status, err: &Option<String>) -> String {
    match status {
        Status::Connecting => "Connecting...".into(),
        Status::Downloading => "Receiving data...".into(),
        Status::Stopped => "Paused".into(),
        Status::Complete => "Download complete".into(),
        Status::Error => err.clone().unwrap_or_else(|| "Error".into()),
    }
}

pub fn progress_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut Progress) -> Out<()> {
    let t = d.task.clone();
    let (status, err, resumable, name, hls) = {
        let i = t.info.lock();
        let n = if i.file_name.is_empty() { util::file_name_from_url(&t.url) } else { i.file_name.clone() };
        (i.status, i.error.clone(), i.resumable, n, i.hls)
    };
    let seg_prog = (t.hls_done.load(std::sync::atomic::Ordering::Relaxed), t.hls_count.load(std::sync::atomic::Ordering::Relaxed));
    let total = t.total();
    let done = t.downloaded();
    let pct = t.percent();
    let speed = t.speed();
    let eta = t.eta();
    let size = if d.details { [560.0, 552.0] } else { [560.0, 376.0] };

    let mut action = 0; // 1 = pause/resume, 2 = cancel
    let mut want_title = format!("{}% \"{}\"", pct as u32, name);
    if want_title.chars().count() > 46 {
        want_title = format!("{}…", want_title.chars().take(45).collect::<String>());
    }
    let mut st = status_text(status, &err);
    if status == Status::Connecting {
        // an automatic retry: say which attempt this is and why the last one failed
        let n = t.retries.load(std::sync::atomic::Ordering::Relaxed);
        if n > 0 {
            let max = mgr.settings.lock().auto_retries;
            st = match &err {
                Some(e) => format!("Connecting... (retry {n}/{max}: {e})"),
                None => format!("Connecting... (retry {n}/{max})"),
            };
        }
    }
    if hls && status == Status::Downloading && seg_prog.1 > 0 {
        st = format!("{st} (segment {}/{})", seg_prog.0, seg_prog.1);
    }
    let badge = match status {
        Status::Complete => Badge::Done,
        Status::Error => Badge::Warn,
        _ => Badge::None,
    };
    let sub = format!("{}%  ·  {}", pct as u32, name);

    let closed = show_dialog_min(ctx, ("progress", t.id), &want_title, size, false, |ui| { // the task bar shows the live "42% name" title
        let ctx = ui.ctx().clone();
        if d.last_title != want_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want_title.clone()));
            d.last_title = want_title.clone();
        }
        if d.shown_details != d.details {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(560.0, if d.details { 552.0 } else { 376.0 })));
            d.shown_details = d.details;
        }

        heading(ui, &st, &sub, badge);

        ui.horizontal(|ui| {
            for (i, n) in ["Download status", "Speed limiter", "On completion"].iter().enumerate() {
                if ui.selectable_label(d.tab == i, *n).clicked() {
                    d.tab = i;
                }
            }
        });
        card(ui, |ui| {
            ui.set_min_height(84.0);
            ui.spacing_mut().interact_size.y = 20.0; // compact rows inside the card
            match d.tab {
                0 => {
                    ui.add(egui::Label::new(RichText::new(t.url.as_str()).color(pal().muted)).truncate(true));
                    // one label/value pair per row, like the reference
                    egui::Grid::new("pg").num_columns(2).min_col_width(130.0).spacing([14.0, 3.0]).show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, k: &str, v: String| {
                            ui.label(RichText::new(k).color(pal().muted));
                            ui.add(egui::Label::new(v).truncate(true));
                            ui.end_row();
                        };
                        row(ui, "Status", st.clone());
                        row(
                            ui,
                            "File size",
                            if total > 0 {
                                format!("{}{}", if hls && status != Status::Complete { "~" } else { "" }, util::fmt_size_prec(total, 3))
                            } else {
                                "Unknown".into()
                            },
                        );
                        row(ui, "Downloaded", format!("{}  ( {:.2} % )", util::fmt_size_prec(done, 3), pct));
                        row(ui, "Transfer rate", if speed > 0 { util::fmt_rate(speed, 3) } else { "-".into() });
                        row(ui, "Time left", eta.map(util::fmt_time).unwrap_or_else(|| "-".into()));
                        row(ui, "Resume capability", (if resumable { "Yes" } else { "No" }).to_string());
                    });
                }
                1 => {
                    if ui.checkbox(&mut d.limit_on, "Use speed limiter").changed() || d.limit_on {
                        t.limit_kbps
                            .store(if d.limit_on { d.limit_kbps } else { 0 }, std::sync::atomic::Ordering::Relaxed);
                    }
                    ui.horizontal(|ui| {
                        ui.label("Maximum download speed:");
                        ui.add_enabled(d.limit_on, egui::DragValue::new(&mut d.limit_kbps).clamp_range(8..=1_000_000).suffix(" KBytes/sec"));
                    });
                }
                _ => {
                    ui.checkbox(&mut d.open_file, "Open the file when done");
                    ui.checkbox(&mut d.open_folder, "Open the folder when done");
                    ui.checkbox(&mut d.exit_when_done, "Exit DownCraft when done");
                }
            }
        });
        ui.add_space(2.0);

        // main progress bar
        progress_bar(ui, (pct / 100.0) as f32, if status == Status::Complete { pal().good } else { pal().accent });
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            if action_button(ui, if d.details { "Hide details" } else { "Show details" }, false).clicked() {
                d.details = !d.details;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, "Cancel", false).clicked() {
                    action = 2;
                }
                let label = if status == Status::Stopped || status == Status::Error { "Resume" } else { "Pause" };
                if action_button(ui, label, true).clicked() {
                    action = 1;
                }
            });
        });

        if d.details {
            ui.add_space(4.0);
            caption(ui, "Start positions and download progress by connections");
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
            ui.painter().rect_filled(bar, Rounding::same(3.0), pal().track);
            let segs = t.segs.read().clone();
            if total > 0 {
                for s in &segs {
                    let x = |b: u64| bar.left() + bar.width() * (b as f32 / total as f32);
                    let end = s.end.load(std::sync::atomic::Ordering::Relaxed).min(total - 1);
                    let pos = s.pos.load(std::sync::atomic::Ordering::Relaxed).min(end + 1);
                    ui.painter().rect_filled(
                        Rect::from_min_max(pos2(x(s.start), bar.top()), pos2(x(pos), bar.bottom())),
                        Rounding::ZERO,
                        pal().accent,
                    );
                    ui.painter().line_segment(
                        [pos2(x(s.start), bar.top()), pos2(x(s.start), bar.bottom())],
                        Stroke::new(1.5_f32, pal().bare_bg),
                    );
                }
            }
            ui.add_space(4.0);
            let list_h = ui.available_height().clamp(60.0, 220.0);
            egui::Frame::none()
                .fill(pal().card_bg)
                .stroke(Stroke::new(1.0_f32, pal().card_stroke))
                .rounding(Rounding::same(9.0))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height((list_h - 18.0).max(40.0));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (w, h) in [(40.0, "N."), (140.0, "Downloaded"), (300.0, "Info")] {
                            let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                            ui.painter().text(r.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, h, FontId::proportional(11.5), pal().muted);
                        }
                    });
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for (i, s) in segs.iter().enumerate() {
                            let st = s.state.load(std::sync::atomic::Ordering::Relaxed);
                            let info = match st {
                                SEG_CONNECTING => "Connecting...",
                                SEG_RECEIVING => "Receiving data...",
                                SEG_DONE => "Download complete",
                                SEG_ERROR => "Error",
                                _ => "",
                            };
                            let dl = s.downloaded();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for (w, txt) in [
                                    (40.0, format!("{}", i + 1)),
                                    (140.0, if dl > 0 { util::fmt_size_prec(dl, 3) } else { String::new() }),
                                    (300.0, info.to_string()),
                                ] {
                                    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                                    ui.painter().text(r.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, txt, FontId::proportional(12.5), pal().text2);
                                }
                            });
                        }
                    });
                });
        }
    });

    if action == 2 {
        mgr.pause(&t);
        return Out::Close;
    }
    if action == 1 {
        if matches!(status, Status::Stopped | Status::Error) {
            mgr.start(&t);
        } else {
            mgr.pause(&t);
        }
    }
    if closed {
        Out::Close
    } else {
        Out::Keep
    }
}

// ───────────────────── Download complete ─────────────────────

pub struct Complete {
    pub key: u64,
    pub url: String,
    pub path: PathBuf,
    pub size: u64,
    pub dont_show: bool,
    /// Set when the drag handle was grabbed; `App::update` starts the native drag (it needs the window handle).
    pub drag: bool,
}

pub fn complete_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut Complete) -> Out<()> {
    let mut out = Out::Keep;
    let path_s = d.path.to_string_lossy().into_owned();
    let closed = show_dialog_min(ctx, ("complete", d.key), "Download complete", [500.0, 312.0], true, |ui| {
        let file_name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        heading(ui, "Download complete", &format!("{}  ·  {} bytes", util::fmt_size(d.size), d.size), Badge::Done);

        if !file_name.is_empty() {
            ui.add(egui::Label::new(RichText::new(&file_name).size(13.5).color(pal().strong)).truncate(true));
            ui.add_space(2.0);
        }
        info_field(ui, "Address", &d.url);
        info_field(ui, "Saved to", &path_s);
        ui.add_space(8.0);

        // actions: one primary, two secondary, close on the right
        ui.horizontal(|ui| {
            if action_button(ui, "Open", true).clicked() {
                let _ = open::that(&d.path);
            }
            if action_button(ui, "Open folder", false).clicked() {
                if let Some(dir) = d.path.parent() {
                    let _ = open::that(dir);
                }
                out = Out::Close; // the folder is open: this pop-up has done its job
            }
            if action_button(ui, "Open with…", false).clicked() {
                if let Some(app) = rfd::FileDialog::new().set_title("Open with…").pick_file() {
                    let _ = std::process::Command::new(app).arg(&d.path).spawn();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, "Close", false).clicked() {
                    out = Out::Close;
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut d.dont_show, RichText::new("Don't show this dialog again").color(pal().muted));
            // drag handle: dragging it carries the downloaded file itself into a folder / another app
            if crate::extension::CAN_DRAG && d.path.is_file() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = file_drag_handle(ui)
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text("Drag the downloaded file into a folder or another app");
                    if r.drag_started_by(egui::PointerButton::Primary) {
                        d.drag = true;
                    }
                });
            }
        });
    });
    if d.dont_show {
        let mut s = mgr.settings.lock();
        if s.show_complete_dialog {
            s.show_complete_dialog = false;
            drop(s);
            mgr.save_settings();
        }
    }
    if closed {
        Out::Close
    } else {
        out
    }
}

// ───────────────────── Scheduler / Delete / About ─────────────────────

#[derive(Clone, Copy, PartialEq)]
enum SchedTab {
    Schedule,
    Files,
}

/// The Scheduler pop-up. It edits a copy; *Apply* writes it to the settings.
pub struct SchedulerDlg {
    tab: SchedTab,
    s: Schedule,
    after: AfterQueue,
    retries: u32,
    /// Date of the one-time start (kept as a date here, stored as text in the settings).
    date: chrono::NaiveDate,
    /// What is saved right now, to tell whether *Apply* has anything to do.
    saved: (Schedule, AfterQueue, u32),
}

const DATE_FMT: &str = "%Y-%m-%d";
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl SchedulerDlg {
    pub fn new(mgr: &Manager) -> Self {
        let st = mgr.settings.lock();
        let date = st.schedule.once_day().unwrap_or_else(|| chrono::Local::now().date_naive());
        let mut s = st.schedule.clone();
        s.once_date = date.format(DATE_FMT).to_string();
        let (after, retries) = (st.after_queue, st.auto_retries);
        SchedulerDlg { tab: SchedTab::Schedule, saved: (s.clone(), after, retries), s, after, retries, date }
    }

    /// The schedule as edited, with the date folded back in.
    fn current(&self) -> Schedule {
        let mut s = self.s.clone();
        s.once_date = self.date.format(DATE_FMT).to_string();
        s
    }

    fn dirty(&self) -> bool {
        (self.current(), self.after, self.retries) != self.saved
    }

    fn apply(&mut self, mgr: &Manager) {
        let s = self.current();
        {
            let mut st = mgr.settings.lock();
            st.schedule = s.clone();
            st.after_queue = self.after;
            st.auto_retries = self.retries;
        }
        mgr.save_settings();
        mgr.reset_schedule_clock();
        self.saved = (s, self.after, self.retries);
    }
}

/// Hour and minute boxes for a time of day given in minutes after midnight.
fn time_edit(ui: &mut egui::Ui, minutes: &mut u32, enabled: bool) {
    let (mut h, mut m) = (*minutes / 60, *minutes % 60);
    ui.add_enabled_ui(enabled, |ui| {
        let two = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:02}", v as u32);
        ui.add(egui::DragValue::new(&mut h).clamp_range(0..=23).custom_formatter(two));
        ui.label(":");
        ui.add(egui::DragValue::new(&mut m).clamp_range(0..=59).custom_formatter(two));
    });
    *minutes = h * 60 + m;
}

fn fmt_when(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map_or_else(String::new, |t| t.format("%a, %b %d, %H:%M").to_string())
}

/// The "Schedule" tab. `status` is the line under the cards (what runs next).
fn scheduler_schedule_tab(ui: &mut egui::Ui, d: &mut SchedulerDlg, status: &str) {
    card(ui, |ui| {
        check_row(ui, &mut d.s.on_launch, "Start the main queue when DownCraft starts");
    });
    card(ui, |ui| {
        ui.horizontal(|ui| {
            check_row(ui, &mut d.s.start_on, "Start download at");
            time_edit(ui, &mut d.s.start_min, d.s.start_on);
        });
        ui.add_enabled_ui(d.s.start_on || d.s.stop_on, |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.s.daily, false, "Once at");
                ui.add_enabled(!d.s.daily, egui_extras::DatePickerButton::new(&mut d.date).id_source("sched-date").calendar_week(false));
            });
            ui.radio_value(&mut d.s.daily, true, "Daily");
            ui.add_enabled_ui(d.s.daily, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in WEEKDAYS.iter().enumerate() {
                        check_row(ui, &mut d.s.days[i], name);
                    }
                });
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            check_row(ui, &mut d.s.stop_on, "Stop download at");
            time_edit(ui, &mut d.s.stop_min, d.s.stop_on);
        });
    });
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Retry a failed download up to");
            ui.add(egui::DragValue::new(&mut d.retries).clamp_range(0..=20));
            ui.label("times");
        });
        check_row(ui, &mut d.s.open_on, "Open the following file when done");
        ui.add_enabled_ui(d.s.open_on, |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - FIELD_H - ui.spacing().item_spacing.x;
                input(ui, &mut d.s.open_file, w);
                if square_button(ui, "…", true).on_hover_text("Choose a file").clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title("Open when the queue is done").pick_file() {
                        d.s.open_file = f.to_string_lossy().into_owned();
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("When the queue is done:");
            let label = |a: AfterQueue| match a {
                AfterQueue::Nothing => "Do nothing",
                AfterQueue::CloseApp => "Close DownCraft",
                AfterQueue::Sleep => "Put the computer to sleep",
                AfterQueue::Shutdown => "Shut down the computer",
            };
            egui::ComboBox::from_id_source("sched-after").width(210.0).selected_text(label(d.after)).show_ui(ui, |ui| {
                for a in [AfterQueue::Nothing, AfterQueue::CloseApp, AfterQueue::Sleep, AfterQueue::Shutdown] {
                    ui.selectable_value(&mut d.after, a, label(a));
                }
            });
        });
    });
    ui.label(RichText::new(status).size(12.0).color(pal().muted));
}

/// The "Files in the queue" tab: what the main queue will download, in order.
fn scheduler_files_tab(ui: &mut egui::Ui, mgr: &Manager) {
    let rows: Vec<(String, String, String)> = mgr
        .tasks
        .lock()
        .iter()
        .filter_map(|t| {
            let i = t.info.lock();
            if !i.queued || i.status == Status::Complete {
                return None;
            }
            let name = if i.file_name.is_empty() { util::file_name_from_url(&t.url) } else { i.file_name.clone() };
            let total = t.total.load(std::sync::atomic::Ordering::Relaxed);
            let size = if total == 0 || total == downcraft_core::UNKNOWN_END { "-".to_string() } else { util::fmt_size(total) };
            let status = match i.status {
                Status::Stopped => "Waiting",
                Status::Connecting => "Connecting",
                Status::Downloading => "Downloading",
                Status::Error => "Failed",
                Status::Complete => "Done",
            };
            Some((name, size, status.to_string()))
        })
        .collect();
    ui.label(RichText::new(format!("{} file(s) in the main download queue", rows.len())).size(12.0).color(pal().muted));
    let h = (ui.available_height() - 46.0).max(60.0);
    card(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(h - 26.0).min_scrolled_height(h - 26.0).show(ui, |ui| {
            if rows.is_empty() {
                ui.label(RichText::new("Nothing is waiting. Choose \"Download later\" when you add a download to put it in this queue.").color(pal().muted));
            }
            for (name, size, status) in &rows {
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    ui.add_sized([w - 150.0, 20.0], egui::Label::new(RichText::new(name).color(pal().text2)).truncate(true));
                    ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(size).color(pal().muted)));
                    ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(status).color(pal().muted)));
                });
            }
        });
    });
}

pub fn scheduler_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut SchedulerDlg) -> Out<()> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "scheduler", "Scheduler", SCHEDULER_SIZE, false, |ui| {
        heading(ui, "Scheduler", "Main download queue", Badge::None);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.tab, SchedTab::Schedule, "Schedule");
            ui.selectable_value(&mut d.tab, SchedTab::Files, "Files in the queue");
        });
        let running = mgr.queue_running.load(Relaxed);
        match d.tab {
            SchedTab::Schedule => {
                let now = util::now_ts();
                let cur = d.current();
                let mut status = String::from(if running { "The queue is running." } else { "The queue is stopped." });
                if let Some(t) = cur.next_start(now) {
                    status += &format!("  Next start: {}.", fmt_when(t));
                } else if cur.start_on {
                    status += "  No start time is left - pick a later date or time.";
                }
                if let Some(t) = cur.next_stop(now) {
                    status += &format!("  Next stop: {}.", fmt_when(t));
                }
                scheduler_schedule_tab(ui, d, &status);
            }
            SchedTab::Files => scheduler_files_tab(ui, mgr),
        }
        pin_bottom(ui, 30.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!running, |ui| {
                if action_button(ui, "Start now", false).clicked() {
                    mgr.start_queue();
                }
            });
            ui.add_enabled_ui(running, |ui| {
                if action_button(ui, "Stop", false).clicked() {
                    mgr.stop_queue();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, "Close", false).clicked() {
                    out = Out::Close;
                }
                ui.add_enabled_ui(d.dirty(), |ui| {
                    if action_button(ui, "Apply", true).clicked() {
                        d.apply(mgr);
                    }
                });
            });
        });
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

/// Window size of the Scheduler pop-up.
const SCHEDULER_SIZE: [f32; 2] = [560.0, 604.0];

// ───────────────────── Move / Rename and Properties (right-click menu) ─────────────────────

/// Move/Rename: one field with the full path the file should have.
pub struct MoveDlg {
    pub id: u64,
    pub path: String,
    error: Option<String>,
    focused: bool,
}

impl MoveDlg {
    pub fn new(task: &Task) -> Self {
        MoveDlg { id: task.id, path: task.final_path().display().to_string(), error: None, focused: false }
    }
}

const MOVE_SIZE: [f32; 2] = [540.0, 232.0];

fn move_content(ui: &mut egui::Ui, d: &mut MoveDlg) -> Option<usize> {
    heading(ui, "Move / Rename", "Change the file name, the folder, or both", Badge::None);
    caption(ui, "Save as");
    let mut enter = false;
    ui.horizontal(|ui| {
        let w = ui.available_width() - FIELD_H - ui.spacing().item_spacing.x;
        let r = input(ui, &mut d.path, w);
        if !d.focused {
            r.request_focus();
            d.focused = true;
        }
        enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if square_button(ui, "…", true).on_hover_text("Choose a folder and name").clicked() {
            let p = PathBuf::from(d.path.trim());
            let mut dlg = rfd::FileDialog::new().set_title("Move / Rename");
            if let Some(dir) = p.parent() {
                dlg = dlg.set_directory(dir);
            }
            if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                dlg = dlg.set_file_name(n);
            }
            if let Some(f) = dlg.save_file() {
                d.path = f.display().to_string();
                d.error = None;
            }
        }
    });
    match &d.error {
        Some(e) => {
            ui.add(egui::Label::new(RichText::new(e).color(pal().bad)).wrap(true));
        }
        None => {
            ui.label(RichText::new("A folder that does not exist yet is created.").size(12.0).color(pal().muted));
        }
    }
    let hit = action_row(ui, &["Cancel", "OK"], Some(1));
    if enter {
        Some(1)
    } else {
        hit
    }
}

pub fn move_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut MoveDlg) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "move-rename", "Move / Rename", MOVE_SIZE, false, |ui| match move_content(ui, d) {
        Some(1) => match mgr.find(d.id) {
            Some(t) => match mgr.move_task(&t, std::path::Path::new(d.path.trim())) {
                Ok(()) => out = Out::Close,
                Err(e) => d.error = Some(e),
            },
            None => out = Out::Close,
        },
        Some(_) => out = Out::Close,
        None => {}
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

/// Properties: everything the list knows about one download.
pub struct PropsDlg {
    pub task: Arc<Task>,
}

const PROPS_SIZE: [f32; 2] = [560.0, 520.0];

fn when(ts: i64) -> String {
    use chrono::TimeZone;
    if ts <= 0 {
        return String::new();
    }
    chrono::Local.timestamp_opt(ts, 0).single().map_or_else(String::new, |t| t.format("%b %d, %Y  %H:%M:%S").to_string())
}

/// (label, value) rows of the Properties pop-up; empty values are left out.
fn props_rows(t: &Task) -> Vec<(&'static str, String)> {
    let i = t.info.lock().clone();
    let total = t.total();
    let name = if i.file_name.is_empty() { util::file_name_from_url(&t.url) } else { i.file_name.clone() };
    let size = if total > 0 { format!("{}{}  ({} bytes)", if i.hls && i.status != Status::Complete { "~" } else { "" }, util::fmt_size(total), total) } else { "Unknown".into() };
    let status = match (i.status, &i.error) {
        (Status::Complete, _) => "Complete".to_string(),
        (Status::Error, Some(e)) => format!("Error - {e}"),
        (Status::Error, None) => "Error".to_string(),
        (Status::Connecting, _) => "Connecting".to_string(),
        (Status::Downloading, _) => format!("Downloading - {:.2}%", t.percent()),
        (Status::Stopped, _) if total > 0 => format!("Stopped - {:.2}%", t.percent()),
        (Status::Stopped, _) => "Stopped".to_string(),
    };
    vec![
        ("Name", name),
        ("Saved to", i.dir.display().to_string()),
        ("Address", t.url.clone()),
        ("Size", size),
        ("Status", status),
        ("Category", i.category.name().to_string()),
        ("In queue", if i.queued && i.status != Status::Complete { "Main download queue".into() } else { "No".into() }),
        ("Added", when(i.added)),
        ("Last try", when(i.last_try)),
        ("Completed", when(i.completed_at)),
        ("Description", i.description.clone()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .collect()
}

fn props_content(ui: &mut egui::Ui, rows: &[(&'static str, String)], title: &str) -> Option<usize> {
    heading(ui, "Properties", title, Badge::None);
    for (label, value) in rows {
        ui.horizontal(|ui| {
            field_label(ui, label);
            info_value(ui, value);
        });
    }
    action_row(ui, &["Close"], None)
}

pub fn props_ui(ctx: &egui::Context, d: &PropsDlg) -> Out<()> {
    let mut out = Out::Keep;
    let rows = props_rows(&d.task);
    let title = rows.first().map(|(_, v)| v.clone()).unwrap_or_default();
    let closed = show_dialog_bare(ctx, ("properties", d.task.id), "Properties", PROPS_SIZE, false, |ui| {
        if props_content(ui, &rows, &title).is_some() {
            out = Out::Close;
        }
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

pub struct DeleteDlg {
    pub ids: Vec<u64>,
    pub files: bool,
}

pub fn delete_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut DeleteDlg) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "delete", "Confirm", [440.0, 200.0], false, |ui| {
        heading(ui, &format!("Delete {} download(s)?", d.ids.len()), "They will be removed from the list", Badge::Warn);
        ui.checkbox(&mut d.files, "Also delete the file(s) from disk");
        match action_row(ui, &["No", "Yes"], Some(1)) {
            Some(1) => {
                for id in &d.ids {
                    if let Some(t) = mgr.find(*id) {
                        mgr.delete(&t, d.files);
                    }
                }
                out = Out::Close;
            }
            Some(_) => out = Out::Close,
            None => {}
        }
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

/// A small message pop-up with a single Close button.
pub fn notice_ui(ctx: &egui::Context, title: &str, message: &str) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "notice", title, [440.0, 190.0], true, |ui| {
        heading_small(ui, title, "");
        ui.add_space(6.0);
        ui.label(RichText::new(message).color(pal().text2));
        if action_row(ui, &["Close"], Some(0)) == Some(0) {
            out = Out::Close;
        }
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

// ───────────────────── About: developer card ─────────────────────

const DEV_NAME: &str = "Rahul Das Emon";
const DEV_HANDLE: &str = "@RahulDasEmon";
const DEV_URL: &str = "https://github.com/RahulDasEmon";
/// The portrait is the developer's live GitHub avatar: change it on GitHub and it changes here too.
const DEV_AVATAR_URL: &str = "https://github.com/RahulDasEmon.png?size=256";
/// Project links of the About page. An empty address shows that button dimmed, as "SOON".
const PROJECT_URL: &str = "https://github.com/RahulDasEmon/Downcraft";
const DISCORD_URL: &str = ""; // paste the Discord invite here, e.g. "https://discord.gg/xxxxxxx"

/// Pictures of the About page: the developer portrait (starts from the copy cached on disk, or the
/// app logo on a first, offline run, and refreshes itself from GitHub in the background) and
/// the brand icons of the link buttons.
pub struct AboutArt {
    pub texture: TextureHandle,
    github: TextureHandle,
    discord: TextureHandle,
    shown: Vec<u8>,
    cache: PathBuf,
    rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    next_try: std::time::Instant,
}

impl AboutArt {
    pub fn new(ctx: &egui::Context, cache: PathBuf, fallback: &[u8]) -> Self {
        let shown = std::fs::read(&cache)
            .ok()
            .filter(|b| image::load_from_memory(b).is_ok())
            .unwrap_or_else(|| fallback.to_vec());
        let texture = circular_texture(ctx, "developer", &shown, 256);
        let github = icon_texture(ctx, "icon-github", crate::ICON_GITHUB);
        let discord = icon_texture(ctx, "icon-discord", crate::ICON_DISCORD);
        AboutArt { texture, github, discord, shown, cache, rx: None, next_try: std::time::Instant::now() }
    }

    /// Fetches the current GitHub avatar in the background (at most once every 10 minutes; a failed
    /// attempt is retried after one). Uses the app's own HTTP client, so the proxy settings apply.
    pub fn refresh(&mut self, ctx: &egui::Context, mgr: &Manager) {
        if self.rx.is_some() || std::time::Instant::now() < self.next_try {
            return;
        }
        self.next_try = std::time::Instant::now() + std::time::Duration::from_secs(600);
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        let (client, cache, ctx) = (mgr.client(), self.cache.clone(), ctx.clone());
        std::thread::spawn(move || {
            let Ok(bytes) = util::fetch_small(&client, DEV_AVATAR_URL, 4 << 20) else { return };
            if image::load_from_memory(&bytes).is_err() {
                return; // not a picture (an error page, say): keep what we have
            }
            if let Some(dir) = cache.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = cache.with_extension("tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &cache);
            }
            let _ = tx.send(bytes);
            ctx.request_repaint();
        });
    }

    /// Call every frame: swaps in a freshly downloaded avatar once it has arrived.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(bytes) => {
                if bytes != self.shown {
                    self.texture = circular_texture(ctx, "developer", &bytes, 256);
                    self.shown = bytes;
                }
                self.rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None; // the download failed: try again a minute from now
                self.next_try = std::time::Instant::now() + std::time::Duration::from_secs(60);
            }
        }
    }
}

fn icon_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let img = image::load_from_memory(bytes).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let (w, h) = img.dimensions();
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()), egui::TextureOptions::LINEAR)
}

/// Decodes an embedded picture, centre-crops it to a square and cuts it into an anti-aliased circle.
/// A picture that cannot be decoded becomes a plain disc instead of crashing the About page.
pub fn circular_texture(ctx: &egui::Context, name: &str, bytes: &[u8], size: u32) -> TextureHandle {
    let src = image::load_from_memory(bytes)
        .map(|i| i.to_rgba8())
        .unwrap_or_else(|_| image::RgbaImage::from_pixel(size, size, image::Rgba([90, 70, 170, 255])));
    let (w, h) = src.dimensions();
    let side = w.min(h).max(1);
    let square = image::imageops::crop_imm(&src, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let mut img = image::imageops::resize(&square, size, size, image::imageops::FilterType::Lanczos3);
    let r = size as f32 / 2.0;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - r).powi(2) + (y as f32 + 0.5 - r).powi(2)).sqrt();
        px.0[3] = (px.0[3] as f32 * (r - d).clamp(0.0, 1.0)) as u8; // 1 px soft edge
    }
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], img.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

/// Linear blend of two colours (`t` = 0 gives `a`, 1 gives `b`).
fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    egui::Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// A rectangle with rounded top corners filled with a left-to-right gradient that fades towards the bottom.
fn gradient_banner(painter: &egui::Painter, rect: Rect, radius: f32, left: egui::Color32, right: egui::Color32) {
    use std::f32::consts::PI;
    let mut pts: Vec<egui::Pos2> = Vec::new();
    for (centre, from) in [
        (rect.left_top() + vec2(radius, radius), PI),
        (rect.right_top() + vec2(-radius, radius), 1.5 * PI),
    ] {
        for i in 0..=8 {
            let a = from + i as f32 * (PI / 2.0) / 8.0;
            pts.push(centre + radius * vec2(a.cos(), a.sin()));
        }
    }
    pts.push(rect.right_bottom());
    pts.push(rect.left_bottom());
    let colour = |p: egui::Pos2| {
        let tx = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let ty = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        mix(left, right, tx).gamma_multiply(1.0 - 0.55 * ty)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), colour(rect.center()));
    for p in &pts {
        mesh.colored_vertex(*p, colour(*p));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// `──── LABEL ────`
fn section_divider(ui: &mut egui::Ui, label: &str) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(10.5), p.muted);
    let (c, w) = (rect.center(), galley.size().x);
    let line = Stroke::new(1.0, p.hairline);
    ui.painter().line_segment([pos2(rect.left() + 4.0, c.y), pos2(c.x - w / 2.0 - 10.0, c.y)], line);
    ui.painter().line_segment([pos2(c.x + w / 2.0 + 10.0, c.y), pos2(rect.right() - 4.0, c.y)], line);
    ui.painter().galley(pos2(c.x - w / 2.0, c.y - galley.size().y / 2.0), galley, p.muted);
}

/// The developer's business card: gradient banner, a portrait inside an orbiting comet ring, name,
/// a pulsing "DEVELOPER" chip and a link to the GitHub profile.
fn developer_card(ui: &mut egui::Ui, photo: &TextureHandle) {
    use std::f32::consts::PI;
    let p = pal();
    let violet = egui::Color32::from_rgb(150, 96, 255);
    let pink = egui::Color32::from_rgb(244, 98, 170);
    let white = egui::Color32::WHITE;
    let time = ui.input(|i| i.time) as f32;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16)); // keeps the ring turning

    let (card, _) = ui.allocate_exact_size(vec2(ui.available_width(), 282.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(card, Rounding::same(14.0), p.card_bg, Stroke::new(1.0, p.card_stroke));

    // ── banner: gradient, dot grid, two soft bubbles ──
    let banner = Rect::from_min_size(card.min + vec2(1.0, 1.0), vec2(card.width() - 2.0, 77.0));
    gradient_banner(&painter, banner, 13.0, p.accent.gamma_multiply(0.80), violet.gamma_multiply(0.80));
    let clip = painter.with_clip_rect(banner);
    clip.circle_filled(banner.left_top() + vec2(62.0, 30.0), 34.0, white.gamma_multiply(0.07));
    clip.circle_filled(banner.right_top() + vec2(-66.0, 54.0), 46.0, white.gamma_multiply(0.06));
    let mut y = banner.top() + 12.0;
    while y < banner.bottom() - 6.0 {
        let mut x = banner.left() + 20.0;
        while x < banner.right() - 16.0 {
            clip.circle_filled(pos2(x, y), 1.1, white.gamma_multiply(0.22));
            x += 14.0;
        }
        y += 14.0;
    }

    // ── portrait ──
    let centre = pos2(card.center().x, card.top() + 78.0);
    let avatar_hit = Rect::from_center_size(centre, vec2(104.0, 104.0));
    let av = ui.interact(avatar_hit, ui.make_persistent_id("dev-avatar"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let lift = ui.ctx().animate_bool_with_time(av.id, av.hovered(), 0.18);

    for k in 0..7 {
        // soft halo
        painter.circle_filled(centre, 82.0 - k as f32 * 3.5, p.accent.gamma_multiply(0.035));
    }
    painter.circle_filled(centre, 60.0, p.card_bg); // cut-out into the banner
    painter.circle_stroke(centre, 60.0, Stroke::new(1.0, p.accent.gamma_multiply(0.22))); // satellite orbit
    let ring_r = 52.0;
    painter.circle_stroke(centre, ring_r, Stroke::new(3.5, p.accent.gamma_multiply(0.16))); // ring track
    let photo_r = 44.0 + 2.5 * lift;
    painter.image(
        photo.id(),
        Rect::from_center_size(centre, vec2(photo_r * 2.0, photo_r * 2.0)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        white,
    );
    // comet: a tail that fades in over 270° and ends in a bright head
    let head = time * (2.2 + 2.0 * lift);
    let span = 1.5 * PI;
    let steps = 96;
    let at = |a: f32, r: f32| centre + r * vec2(a.cos(), a.sin());
    let mut prev = at(head - span, ring_r);
    for i in 1..=steps {
        let f = i as f32 / steps as f32;
        let pt = at(head - span * (1.0 - f), ring_r);
        let col = mix(violet, p.accent, f).gamma_multiply(f.powf(1.3));
        painter.line_segment([prev, pt], Stroke::new(4.0, col));
        prev = pt;
    }
    painter.circle_filled(prev, 10.0, p.accent.gamma_multiply(0.20));
    painter.circle_filled(prev, 5.0, white);
    // satellite on the outer orbit, going the other way
    let sat = at(1.0 - time * 1.1, 60.0);
    painter.circle_filled(sat, 6.0, pink.gamma_multiply(0.25));
    painter.circle_filled(sat, 3.0, pink);

    // ── name, handle ──
    painter.text(pos2(card.center().x, card.top() + 144.0), Align2::CENTER_TOP, DEV_NAME, FontId::proportional(22.0), p.strong);
    painter.text(pos2(card.center().x, card.top() + 174.0), Align2::CENTER_TOP, DEV_HANDLE, FontId::proportional(13.0), p.muted);

    // ── "DEVELOPER" chip with a pulsing dot ──
    let g = painter.layout_no_wrap("DEVELOPER".to_owned(), FontId::proportional(10.5), p.accent);
    let chip = Rect::from_center_size(pos2(card.center().x, card.top() + 211.0), vec2(g.size().x + 34.0, 22.0));
    painter.rect(chip, Rounding::same(11.0), p.accent.gamma_multiply(0.14), Stroke::new(1.0, p.accent.gamma_multiply(0.45)));
    let dot = pos2(chip.left() + 12.0, chip.center().y);
    let pulse = 0.5 + 0.5 * (time * 3.0).sin();
    painter.circle_filled(dot, 3.0 + 3.0 * pulse, p.accent.gamma_multiply(0.30 * (1.0 - pulse)));
    painter.circle_filled(dot, 2.6, p.accent);
    painter.galley(pos2(chip.left() + 22.0, chip.center().y - g.size().y / 2.0), g, p.accent);

    // ── GitHub button ──
    let btn = Rect::from_center_size(pos2(card.center().x, card.top() + 250.0), vec2(184.0, 32.0));
    let br = ui.interact(btn, ui.make_persistent_id("dev-github"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let br = br.on_hover_text(DEV_URL);
    let hot = ui.ctx().animate_bool_with_time(br.id, br.hovered(), 0.12);
    painter.rect_filled(btn, Rounding::same(16.0), mix(p.accent, p.accent_hover, hot));
    let label = painter.layout_no_wrap("View GitHub profile".to_owned(), FontId::proportional(13.0), p.on_accent);
    let total = label.size().x + 6.0 + 9.0;
    let x0 = btn.center().x - total / 2.0;
    painter.galley(pos2(x0, btn.center().y - label.size().y / 2.0), label.clone(), p.on_accent);
    // little "↗" arrow that nudges on hover (drawn, so it does not depend on a font glyph)
    let a = pos2(x0 + label.size().x + 6.0 + 1.0 * hot, btn.center().y);
    let ink = Stroke::new(1.6, p.on_accent);
    painter.line_segment([a + vec2(0.0, 4.0), a + vec2(8.0, -4.0)], ink);
    painter.line_segment([a + vec2(2.0, -4.0), a + vec2(8.0, -4.0)], ink);
    painter.line_segment([a + vec2(8.0, -4.0), a + vec2(8.0, 2.0)], ink);

    if av.clicked() || br.clicked() {
        let _ = open::that(DEV_URL);
    }
}

/// A pill with a brand icon and a label. On hover it floods with the brand colour. Without an
/// address (`url` empty) it is dimmed, carries a "SOON" tag and does nothing.
fn social_button(ui: &mut egui::Ui, rect: Rect, icon: &TextureHandle, label: &str, url: &str, brand: egui::Color32, on_brand: egui::Color32) {
    let p = pal();
    let live = !url.is_empty();
    let id = ui.make_persistent_id(("social", label));
    let r = ui.interact(rect, id, if live { Sense::click() } else { Sense::hover() });
    let r = if live {
        r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(url)
    } else {
        r.on_hover_text(format!("{label} link coming soon"))
    };
    let hot = ui.ctx().animate_bool_with_time(id, live && r.hovered(), 0.14);
    let painter = ui.painter();
    let fill = mix(p.btn_bg, brand, hot);
    let edge = mix(p.card_stroke, brand, hot);
    let ink = if live { mix(p.text2, on_brand, hot) } else { p.muted.gamma_multiply(0.75) };
    painter.rect(rect, Rounding::same(rect.height() / 2.0), fill, Stroke::new(1.0, edge));

    let text = painter.layout_no_wrap(label.to_owned(), FontId::proportional(13.0), ink);
    let soon = (!live).then(|| painter.layout_no_wrap("SOON".to_owned(), FontId::proportional(8.5), p.muted));
    let soon_w = soon.as_ref().map_or(0.0, |g| g.size().x + 12.0 + 6.0);
    let total = 16.0 + 8.0 + text.size().x + soon_w;
    let mut x = rect.center().x - total / 2.0 - 0.0;
    let lift = -1.0 * hot; // the icon hops up a pixel on hover
    painter.image(
        icon.id(),
        Rect::from_min_size(pos2(x, rect.center().y - 8.0 + lift), vec2(16.0, 16.0)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        ink,
    );
    x += 16.0 + 8.0;
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text.clone(), ink);
    if let Some(g) = soon {
        let chip = Rect::from_min_size(pos2(x + text.size().x + 6.0, rect.center().y - 7.0), vec2(g.size().x + 12.0, 14.0));
        painter.rect(chip, Rounding::same(7.0), p.muted.gamma_multiply(0.12), Stroke::new(1.0, p.muted.gamma_multiply(0.30)));
        painter.galley(pos2(chip.left() + 6.0, chip.center().y - g.size().y / 2.0), g, p.muted);
    }
    if r.clicked() {
        let _ = open::that(url);
    }
}

pub fn about_ui(ctx: &egui::Context, logo: &TextureHandle, art: &AboutArt) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "about", "About DownCraft", [440.0, 572.0], false, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(60.0, 60.0)));
            ui.add_space(2.0);
            ui.label(RichText::new(format!("DownCraft {}", env!("CARGO_PKG_VERSION"))).size(18.0).strong().color(pal().strong));
            ui.label(RichText::new("A free, open-source, multi-connection download manager.").color(pal().muted));
            ui.label(RichText::new("Pure Rust. Windows · macOS · Linux.").color(pal().muted));
        });
        ui.add_space(10.0);
        section_divider(ui, "CRAFTED BY");
        ui.add_space(8.0);
        developer_card(ui, &art.texture);

        // links, pinned to the bottom (the window's own × and Esc close the page)
        pin_bottom(ui, 16.0 + 10.0 + 36.0);
        section_divider(ui, "CONNECT");
        ui.add_space(10.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        let (w, gap) = (168.0, 12.0);
        let left = Rect::from_min_size(pos2(row.center().x - w - gap / 2.0, row.top()), vec2(w, 36.0));
        let right = Rect::from_min_size(pos2(row.center().x + gap / 2.0, row.top()), vec2(w, 36.0));
        let (gh_brand, gh_ink) = if pal().light {
            (egui::Color32::from_rgb(36, 41, 47), egui::Color32::WHITE)
        } else {
            (egui::Color32::from_rgb(240, 246, 252), egui::Color32::from_rgb(13, 17, 23))
        };
        social_button(ui, left, &art.github, "GitHub", PROJECT_URL, gh_brand, gh_ink);
        social_button(ui, right, &art.discord, "Discord", DISCORD_URL, egui::Color32::from_rgb(88, 101, 242), egui::Color32::WHITE);
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

// ───────────────────── Browser extension page ─────────────────────

pub struct ExtensionDlg {
    pub path: Option<PathBuf>,
    pub message: Option<(bool, String)>, // (is_error, text)
    /// Set when the logo was grabbed; `App::update` starts the native drag (it needs the window handle).
    pub drag: bool,
    /// Keep the pop-up above every other window (on by default).
    pub pinned: bool,
}

impl ExtensionDlg {
    pub fn new(mgr: &Manager) -> Self {
        match crate::extension::export(&mgr.data_dir().join("extension")) {
            Ok(path) => ExtensionDlg { path: Some(path), message: None, drag: false, pinned: true },
            Err(e) => ExtensionDlg { path: None, message: Some((true, format!("Could not prepare the extension: {e}"))), drag: false, pinned: true },
        }
    }
}

pub fn extension_ui(ctx: &egui::Context, d: &mut ExtensionDlg, logo: &TextureHandle) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "extension", "Browser extension", [520.0, 500.0], true, |ui| {
        if pin_button(ui, d.pinned) {
            d.pinned = !d.pinned;
            let level = if d.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("DownCraft browser extension").size(17.0).strong().color(pal().strong));
            ui.label(RichText::new("Chrome · Edge · Brave · Opera").color(pal().muted));
            ui.add_space(10.0);

            let r = ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(96.0, 96.0)).sense(Sense::click_and_drag()));
            if d.path.is_some() {
                let r = r.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(if crate::extension::CAN_DRAG {
                    "Drag me onto the chrome://extensions page"
                } else {
                    "Use \"Save zip as…\" below"
                });
                if r.drag_started() && crate::extension::CAN_DRAG {
                    d.drag = true;
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new(crate::extension::FILE_NAME).strong().color(pal().strong));
            if crate::extension::CAN_DRAG {
                ui.label("Drag this logo onto the browser's extensions page to install.");
            } else {
                ui.label("Save the zip, then drag it onto the browser's extensions page.");
            }
        });
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.label("1.  Open  chrome://extensions  (Edge: edge://extensions) and switch on Developer mode.");
            ui.label("2.  Drag the logo above into that page - the extension installs itself.");
            ui.label("3.  Keep DownCraft running; downloads from the browser then open here.");
        });
        ui.add_space(6.0);
        if let Some((err, text)) = &d.message {
            let col = if *err { pal().bad } else { pal().good_text };
            ui.add(egui::Label::new(RichText::new(text).color(col)).truncate(true)).on_hover_text(text);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if action_button(ui, "Copy chrome://extensions", false).clicked() {
                ui.output_mut(|o| o.copied_text = "chrome://extensions".into());
                d.message = Some((false, "Address copied - paste it into the browser's address bar.".into()));
            }
            if ui.add_enabled(d.path.is_some(), egui::Button::new("Save zip as…").min_size(vec2(88.0, 30.0))).clicked() {
                if let Some(src) = &d.path {
                    if let Some(dest) = rfd::FileDialog::new().set_file_name(crate::extension::FILE_NAME).add_filter("Zip", &["zip"]).save_file() {
                        d.message = Some(match std::fs::copy(src, &dest) {
                            Ok(_) => (false, format!("Saved to {}", dest.display())),
                            Err(e) => (true, format!("Could not save: {e}")),
                        });
                    }
                }
            }
            if ui.add_enabled(d.path.is_some(), egui::Button::new("Show in folder").min_size(vec2(88.0, 30.0))).clicked() {
                if let Some(dir) = d.path.as_ref().and_then(|p| p.parent()) {
                    let _ = open::that(dir);
                }
            }
        });
        pin_bottom(ui, 30.0);
        ui.vertical_centered(|ui| {
            if action_button(ui, "Close", true).clicked() {
                out = Out::Close;
            }
        });
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

// ───────────────────── Tools: yt-dlp, ffmpeg, Deno ─────────────────────

pub struct ToolsDlg {
    pub pinned: bool,
}

#[derive(Clone, Copy)]
enum Ico {
    Refresh,
    Trash,
    Folder,
    Cross,
}

/// Small rounded square button with a line icon; the text only shows as a tooltip.
fn icon_btn(ui: &mut egui::Ui, ico: Ico, tip: &str) -> egui::Response {
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
        Ico::Cross => {
            p.line_segment([q(-4.5, -4.5), q(4.5, 4.5)], st);
            p.line_segment([q(-4.5, 4.5), q(4.5, -4.5)], st);
        }
        Ico::Trash => {
            p.line_segment([q(-5.5, -3.5), q(5.5, -3.5)], st);
            p.add(egui::Shape::line(vec![q(-1.8, -3.5), q(-1.8, -6.0), q(1.8, -6.0), q(1.8, -3.5)], st));
            p.add(egui::Shape::line(vec![q(-4.2, -1.5), q(-3.5, 5.5), q(3.5, 5.5), q(4.2, -1.5)], st));
        }
        Ico::Folder => {
            p.add(egui::Shape::closed_line(vec![q(-6.5, -4.5), q(-2.0, -4.5), q(-0.5, -2.5), q(6.5, -2.5), q(6.5, 5.0), q(-6.5, 5.0)], st));
        }
        Ico::Refresh => {
            // an open circle with an arrow head at its end
            let (r, t0, t1) = (5.2_f32, -2.2_f32, 3.3_f32);
            let pts: Vec<_> = (0..=20).map(|i| {
                let t = t0 + (t1 - t0) * i as f32 / 20.0;
                q(r * t.cos(), r * t.sin())
            }).collect();
            p.add(egui::Shape::line(pts, st));
            let end = q(r * t1.cos(), r * t1.sin());
            let d = vec2(-t1.sin(), t1.cos()); // direction of travel at the end
            let rot = |v: egui::Vec2, a: f32| vec2(v.x * a.cos() - v.y * a.sin(), v.x * a.sin() + v.y * a.cos());
            p.line_segment([end, end - rot(d, 0.55) * 4.0], st);
            p.line_segment([end, end - rot(d, -0.55) * 4.0], st);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// The 128 px logo of a tool, decoded once and kept in the egui context.
fn tool_logo(ctx: &egui::Context, tool: tools::Tool) -> TextureHandle {
    let (key, bytes): (&str, &[u8]) = match tool {
        tools::Tool::YtDlp => ("tool-logo-ytdlp", include_bytes!("../../../assets/tools/logos/ytdlp.png")),
        tools::Tool::Ffmpeg => ("tool-logo-ffmpeg", include_bytes!("../../../assets/tools/logos/ffmpeg.png")),
        tools::Tool::Deno => ("tool-logo-deno", include_bytes!("../../../assets/tools/logos/deno.png")),
    };
    let id = egui::Id::new(key);
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(bytes).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let (w, h) = img.dimensions();
    let tex = ctx.load_texture(key, egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

pub fn tools_ui(ctx: &egui::Context, d: &mut ToolsDlg, tb: &Arc<ToolBox>) -> Out<()> {
    let mut out = Out::Keep;
    let entries = tb.snapshot();
    if tb.is_busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100)); // keeps the progress bars moving
    }
    let dir = tools::tools_dir(&tb.data_dir());
    let closed = show_dialog_bare(ctx, "tools", "Tools", [480.0, 412.0], true, |ui| {
        if pin_button(ui, d.pinned) {
            d.pinned = !d.pinned;
            let level = if d.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        heading(ui, "Tools", "For video site downloads", Badge::None);
        for (tool, e) in &entries {
            let tool = *tool;
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::hover());
            let installing = e.busy == Busy::Installing;
            let managed = e.installed.as_ref().map_or(false, |i| i.managed);
            let ver = e.version.clone().unwrap_or_default();
            let short = if ver.chars().count() > 16 { format!("{}…", ver.chars().take(15).collect::<String>()) } else { ver.clone() };
            let err = e.message.as_ref().filter(|(is_err, _)| *is_err).map(|(_, t)| t.clone());
            let frac = if e.total > 0 { (e.done as f32 / e.total as f32).clamp(0.0, 1.0) } else { 0.0 };

            // (dot colour, status text, text colour)
            let (dot, status, status_col) = if let Some(t) = &err {
                (pal().bad, t.clone(), pal().bad)
            } else if installing {
                let t = if e.extracting {
                    "Unpacking…".to_string()
                } else if e.total > 0 {
                    format!("Downloading {}%", (frac * 100.0) as u32)
                } else {
                    format!("{} downloaded", util::fmt_size(e.done))
                };
                (pal().accent, t, pal().muted)
            } else if e.busy == Busy::Checking {
                (pal().disabled, "Checking…".to_string(), pal().muted)
            } else if e.busy == Busy::Removing {
                (pal().disabled, "Removing…".to_string(), pal().muted)
            } else if e.installed.is_none() && tool.source().is_none() {
                (pal().warn, "Install manually".to_string(), pal().warn)
            } else {
                match &e.installed {
                    Some(_) if managed => (pal().good, short.clone(), pal().muted),
                    Some(_) => (pal().good, format!("On this PC · {short}"), pal().muted),
                    None => (pal().disabled, "Not installed".to_string(), pal().muted),
                }
            };

            // card, icon tile, name and status line
            let p = ui.painter();
            p.rect_filled(rect, Rounding::same(12.0), pal().card_bg);
            p.rect_stroke(rect, Rounding::same(12.0), Stroke::new(1.0_f32, pal().card_stroke));
            let tile = egui::Rect::from_min_size(rect.min + vec2(16.0, 16.0), vec2(44.0, 44.0));
            let logo = tool_logo(ui.ctx(), tool);
            p.image(logo.id(), tile, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            let tx = tile.right() + 14.0;
            p.text(pos2(tx, rect.top() + 26.0), Align2::LEFT_CENTER, tool.name(), FontId::proportional(15.5), pal().strong);
            p.circle_filled(pos2(tx + 4.0, rect.top() + 49.0), 3.5, dot);
            let status_rect = egui::Rect::from_min_size(pos2(tx + 14.0, rect.top() + 41.0), vec2((rect.right() - 130.0 - (tx + 14.0)).max(40.0), 16.0));
            // left-aligned (ui.put would centre the text inside the rect)
            let mut status_ui = ui.child_ui(status_rect, egui::Layout::left_to_right(egui::Align::Center));
            status_ui.add(egui::Label::new(RichText::new(&status).size(12.5).color(status_col)).truncate(true));

            if installing {
                let bar = egui::Rect::from_min_size(pos2(tx, rect.bottom() - 11.0), vec2(rect.right() - 16.0 - tx, 4.0));
                let p = ui.painter();
                p.rect_filled(bar, Rounding::same(2.0), pal().track);
                let mut fill = bar;
                fill.set_width(bar.width() * if e.extracting { 1.0 } else { frac }.max(0.02));
                p.rect_filled(fill, Rounding::same(2.0), pal().accent);
            }

            // what used to be a paragraph and a path line now lives in the tooltip
            let hover_rect = egui::Rect::from_min_max(rect.min, pos2(rect.right() - 130.0, rect.bottom()));
            let mut tip = tool.purpose().to_string();
            if let Some(i) = &e.installed {
                tip += &format!("\n{}", i.path.display());
            }
            if tool.source().is_none() {
                tip += &format!("\n{}", tool.manual_hint());
            }
            if ver.chars().count() > 16 {
                tip += &format!("\nVersion {ver}");
            }
            ui.interact(hover_rect, ui.id().with(("tool-row", tool as u8)), Sense::hover()).on_hover_text(tip);

            // actions, flush right
            let btns = egui::Rect::from_min_max(pos2(rect.right() - 220.0, rect.top()), pos2(rect.right() - 16.0, rect.bottom()));
            // a child ui never moves the parent's cursor: with no buttons (while checking) the
            // empty area must not pull the next card up into this one
            let mut btn_ui = ui.child_ui(btns, egui::Layout::right_to_left(egui::Align::Center));
            {
                btn_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match e.busy {
                        Busy::Installing => {
                            if icon_btn(ui, Ico::Cross, "Cancel").clicked() {
                                tb.cancel(tool);
                            }
                        }
                        Busy::Removing | Busy::Checking => {}
                        Busy::Idle => {
                            if tool.source().is_none() {
                                return;
                            }
                            if managed {
                                if icon_btn(ui, Ico::Trash, "Remove").clicked() {
                                    tb.remove(tool);
                                }
                                if icon_btn(ui, Ico::Refresh, "Update to the latest version").clicked() {
                                    tb.install(tool);
                                }
                            } else if e.installed.is_none() {
                                if action_button(ui, &format!("Get · {} MB", tool.approx_mb()), true).clicked() {
                                    tb.install(tool);
                                }
                            } else if action_button(ui, "Install", false).on_hover_text("Keep and update DownCraft's own copy, and use that one").clicked() {
                                tb.install(tool);
                            }
                        }
                    }
                });
            }
            ui.add_space(4.0);
        }
        pin_bottom(ui, 32.0);
        ui.horizontal(|ui| {
            if icon_btn(ui, Ico::Folder, &format!("Open folder\n{}", dir.display())).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(&dir);
            }
            if icon_btn(ui, Ico::Refresh, "Check again").clicked() {
                tb.refresh();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, "Close", true).clicked() {
                    out = Out::Close;
                    crate::widgets::dismiss(ui.ctx()); // runs inside the pop-up, so it targets the pop-up window
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


// ───────────────────── First-run setup: browser extension + helper programs ─────────────────────

pub struct SetupDlg {
    pub pinned: bool,
    /// Set by the "Install…" button; `App::update` opens the *Browser extension* window.
    pub open_extension: bool,
    /// The extension zip on disk, for dragging it onto the browser's extensions page.
    pub path: Option<PathBuf>,
    /// Set when the logo was grabbed; `App::update` starts the native drag (it needs the window handle).
    pub drag: bool,
    pub message: Option<String>,
}

impl SetupDlg {
    pub fn new(mgr: &Manager) -> Self {
        let path = crate::extension::export(&mgr.data_dir().join("extension")).ok();
        SetupDlg { pinned: true, open_extension: false, path, drag: false, message: None }
    }
}

/// `Done(())` = "Don't show this again"; `Close` = just close it (it comes back next start while something is missing).
pub fn setup_ui(ctx: &egui::Context, d: &mut SetupDlg, tb: &Arc<ToolBox>, extension_seen: bool, logo: &TextureHandle) -> Out<()> {
    let mut out = Out::Keep;
    let entries = tb.snapshot();
    if tb.is_busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100)); // keeps the progress numbers moving
    }
    let missing: Vec<tools::Tool> = entries
        .iter()
        .filter(|(t, e)| e.installed.is_none() && t.source().is_some() && e.busy == Busy::Idle)
        .map(|(t, _)| *t)
        .collect();
    let all_installed = entries.iter().all(|(t, e)| e.installed.is_some() || t.source().is_none());
    let ready = all_installed && extension_seen;

    let closed = show_dialog_bare(ctx, "setup", "Set up DownCraft", [500.0, 524.0], true, |ui| {
        if pin_button(ui, d.pinned) {
            d.pinned = !d.pinned;
            let level = if d.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        if ready {
            heading(ui, "Everything is ready", "The extension and the helper programs are set up", Badge::Done);
        } else {
            heading(ui, "Set up DownCraft", "Two quick steps for YouTube, Facebook, Instagram, X and other video sites", Badge::Warn);
        }

        // ── 1. browser extension (the logo can be dragged onto the browser's extensions page) ──
        ui.label(RichText::new("1 · BROWSER EXTENSION").size(11.5).strong().color(pal().muted));
        ui.add_space(2.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(44.0, 44.0)).sense(Sense::click_and_drag()));
                if d.path.is_some() {
                    let r = r.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(if crate::extension::CAN_DRAG {
                        "Drag me onto the browser's extensions page (chrome://extensions, Developer mode on)"
                    } else {
                        "Use Install… to save the extension zip"
                    });
                    if r.drag_started() && crate::extension::CAN_DRAG {
                        d.drag = true;
                    }
                }
                ui.add_space(4.0);
                let (dot, status) = match (extension_seen, crate::extension::CAN_DRAG) {
                    (true, true) => (pal().good, "Connected · drag the logo to reinstall"),
                    (true, false) => (pal().good, "Connected"),
                    (false, true) => (pal().disabled, "Not connected · drag the logo to your browser"),
                    (false, false) => (pal().disabled, "Not connected · use Install… to save the zip"),
                };
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 120.0).max(60.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new("Browser extension").size(14.5).strong().color(pal().strong));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.label(RichText::new("●").size(10.0).color(dot));
                        let text = d.message.as_deref().unwrap_or(status);
                        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal().muted)).truncate(true)).on_hover_text(text);
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if action_button(ui, if extension_seen { "Reinstall…" } else { "Install…" }, !extension_seen).clicked() {
                        d.open_extension = true;
                    }
                });
            });
        });
        ui.add_space(10.0);

        // ── 2. helper programs ──
        ui.label(RichText::new("2 · DOWNLOAD HELPERS").size(11.5).strong().color(pal().muted));
        ui.add_space(2.0);
        for (tool, e) in &entries {
            let tool = *tool;
            let installing = e.busy == Busy::Installing;
            let err = e.message.as_ref().filter(|(is_err, _)| *is_err).map(|(_, t)| t.clone());
            let frac = if e.total > 0 { (e.done as f32 / e.total as f32).clamp(0.0, 1.0) } else { 0.0 };
            let (dot, status, col) = if let Some(t) = &err {
                (pal().bad, t.clone(), pal().bad)
            } else if installing {
                let t = if e.extracting {
                    "Unpacking…".to_string()
                } else if e.total > 0 {
                    format!("Downloading {}%", (frac * 100.0) as u32)
                } else {
                    format!("{} downloaded", util::fmt_size(e.done))
                };
                (pal().accent, t, pal().muted)
            } else if e.busy != Busy::Idle {
                (pal().disabled, "Checking…".to_string(), pal().muted)
            } else if e.installed.is_some() {
                (pal().good, "Installed".to_string(), pal().muted)
            } else if tool.source().is_none() {
                (pal().warn, tool.manual_hint().to_string(), pal().warn)
            } else {
                (pal().disabled, "Not installed".to_string(), pal().muted)
            };
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    let tl = tool_logo(ui.ctx(), tool);
                    ui.add(egui::Image::new(&tl).fit_to_exact_size(vec2(44.0, 44.0)));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 120.0).max(60.0));
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(tool.name()).size(14.5).strong().color(pal().strong));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            ui.label(RichText::new("●").size(10.0).color(dot));
                            ui.add(egui::Label::new(RichText::new(&status).size(12.5).color(col)).truncate(true)).on_hover_text(tool.purpose());
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if installing {
                            if action_button(ui, "Cancel", false).clicked() {
                                tb.cancel(tool);
                            }
                        } else if e.busy == Busy::Idle && e.installed.is_none() && tool.source().is_some() {
                            if action_button(ui, &format!("Get · {} MB", tool.approx_mb()), true).clicked() {
                                tb.install(tool);
                            }
                        }
                    });
                });
                if installing {
                    let bar = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover()).0;
                    let p = ui.painter();
                    p.rect_filled(bar, Rounding::same(2.0), pal().track);
                    let mut fill = bar;
                    fill.set_width(bar.width() * if e.extracting { 1.0 } else { frac }.max(0.02));
                    p.rect_filled(fill, Rounding::same(2.0), pal().accent);
                }
            });
            ui.add_space(4.0);
        }

        // footer: the buttons never move the content above them (the pop-up has a fixed size)
        pin_bottom(ui, 32.0);
        ui.horizontal(|ui| {
            if action_button(ui, "Don't show again", false)
                .on_hover_text("Never open this window at start-up. It stays under Tasks > Setup guide.")
                .clicked()
            {
                out = Out::Done(());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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


// ───────────────────── Duplicate link / Queue-finished countdown ─────────────────────

/// A link that is already in the list was added again (Options > Downloads decides whether to ask).
pub struct DupDlg {
    pub url: String,
    /// What the existing entry is called (its file name, or the link when it has none yet).
    pub existing: String,
}

/// `Done(true)` = add it again anyway.
pub fn dup_ui(ctx: &egui::Context, d: &DupDlg) -> Out<bool> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "duplicate", "Link already in the list", [500.0, 250.0], true, |ui| {
        heading(ui, "This link is already in the list", "Do you want to download it a second time?", Badge::Warn);
        info_field(ui, "Existing download", &d.existing);
        info_field(ui, "Link", &d.url);
        match action_row(ui, &["Don't add", "Add anyway"], Some(1)) {
            Some(1) => out = Out::Done(true),
            Some(_) => out = Out::Close,
            None => {}
        }
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

/// Countdown before the computer is put to sleep or shut down after the queue finished.
pub struct PowerDlg {
    pub action: AfterQueue,
    pub deadline: std::time::Instant,
}

pub const POWER_COUNTDOWN_SECS: u64 = 60;

/// `Done(())` = do it now (also when the countdown ran out); `Close` = cancelled.
pub fn power_ui(ctx: &egui::Context, d: &PowerDlg) -> Out<()> {
    let left = d.deadline.saturating_duration_since(std::time::Instant::now()).as_secs();
    let (verb, noun) = match d.action {
        AfterQueue::Shutdown => ("Shut down", "shut down"),
        _ => ("Sleep", "go to sleep"),
    };
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "power", "Queue finished", [460.0, 230.0], true, |ui| {
        heading(ui, "The download queue is finished", &format!("The computer will {noun} in {left} s"), Badge::Done);
        progress_bar(ui, left as f32 / POWER_COUNTDOWN_SECS as f32, pal().accent);
        ui.add_space(4.0);
        ui.colored_label(pal().muted, "Downloads and the list are saved first.");
        let now = format!("{verb} now");
        match action_row(ui, &["Cancel", now.as_str()], Some(0)) {
            Some(1) => out = Out::Done(()),
            Some(_) => out = Out::Close,
            None => {}
        }
    });
    if closed {
        return Out::Close;
    }
    if left == 0 {
        return Out::Done(());
    }
    out
}

#[cfg(test)]
mod scheduler_layout_tests {
    use super::*;

    /// Height the Scheduler's content needs (heading, tabs, cards, status line, button row) must be
    /// close to the window's inner height: no clipped buttons, no big empty gap at the bottom.
    #[test]
    fn scheduler_content_fits_its_window() {
        let ctx = egui::Context::default();
        crate::theme::set(&ctx, Theme::from_key("dark"), Accent::from_key("blue"));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let s = Schedule { start_on: true, stop_on: true, open_on: true, daily: true, ..Schedule::default() };
        let mut d = SchedulerDlg { tab: SchedTab::Schedule, saved: (s.clone(), AfterQueue::Nothing, 3), s, after: AfterQueue::Nothing, retries: 3, date };
        let status = "The queue is stopped.  Next start: Sun, Oct 25, 23:00.  Next stop: Mon, Oct 26, 07:30.";
        let mut used = 0.0_f32;
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(SCHEDULER_SIZE[0], SCHEDULER_SIZE[1]))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let area = ui.max_rect().shrink2(vec2(22.0, 18.0));
                    ui.allocate_ui_at_rect(area, |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                        ui.spacing_mut().button_padding = vec2(12.0, 4.0);
                        ui.spacing_mut().interact_size.y = 28.0;
                        heading(ui, "Scheduler", "Main download queue", Badge::None);
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.tab, SchedTab::Schedule, "Schedule");
                            ui.selectable_value(&mut d.tab, SchedTab::Files, "Files in the queue");
                        });
                        scheduler_schedule_tab(ui, &mut d, status);
                        used = ui.min_rect().height() + 6.0 + 30.0; // + the button row
                    });
                });
            });
        }
        let avail = SCHEDULER_SIZE[1] - 36.0;
        println!("content needs {used:.0} px of {avail:.0} px");
        assert!(used <= avail, "content ({used:.0}) is taller than the window ({avail:.0})");
        assert!(avail - used < 40.0, "{:.0} px of empty space at the bottom", avail - used);
    }
}

#[cfg(test)]
mod menu_dialog_layout_tests {
    use super::*;

    /// Height `content` needs inside a pop-up of `size`, with the pop-up's margins and spacing.
    fn needs(size: [f32; 2], mut content: impl FnMut(&mut egui::Ui)) -> f32 {
        let ctx = egui::Context::default();
        crate::theme::set(&ctx, Theme::from_key("dark"), Accent::from_key("blue"));
        let mut used = 0.0_f32;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0], size[1]))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let area = ui.max_rect().shrink2(vec2(22.0, 18.0));
                    ui.allocate_ui_at_rect(area, |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                        ui.spacing_mut().button_padding = vec2(12.0, 4.0);
                        ui.spacing_mut().interact_size.y = 28.0;
                        content(ui);
                        used = ui.min_rect().height();
                    });
                });
            });
        }
        used
    }

    #[test]
    fn move_dialog_fits() {
        let mut d = MoveDlg { id: 1, path: "C:\\Users\\x\\Downloads\\Video\\some-file.mp4".into(), error: Some("A file with that name already exists.".into()), focused: true };
        let used = needs(MOVE_SIZE, |ui| {
            let _ = move_content(ui, &mut d);
        });
        let avail = MOVE_SIZE[1] - 36.0;
        println!("move needs {used:.0} of {avail:.0}");
        assert!(used <= avail, "clipped: {used:.0} > {avail:.0}");
        assert!(avail - used < 40.0, "{:.0} px empty at the bottom", avail - used);
    }

    #[test]
    fn properties_dialog_fits_with_every_row() {
        let rows: Vec<(&'static str, String)> = ["Name", "Saved to", "Address", "Size", "Status", "Category", "In queue", "Added", "Last try", "Completed", "Description"]
            .iter()
            .map(|l| (*l, "value".to_string()))
            .collect();
        let used = needs(PROPS_SIZE, |ui| {
            let _ = props_content(ui, &rows, "file.mp4");
        });
        let avail = PROPS_SIZE[1] - 36.0;
        println!("properties needs {used:.0} of {avail:.0}");
        assert!(used <= avail, "clipped: {used:.0} > {avail:.0}");
        assert!(avail - used < 40.0, "{:.0} px empty at the bottom", avail - used);
    }
}
