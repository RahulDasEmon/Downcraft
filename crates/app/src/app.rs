use crate::{
    appearance::{appearance_ui, AppearanceDlg},
    clipboard, dialogs::*, icons,
    icons::{Node, Tool},
    options::{options_ui, OptionsDlg},
    setup::{setup_ui, SetupDlg},
    skin_store::SkinStore,
    sys::{self, SoundKind},
    theme::*,
};
use eframe::egui::{
    self, epaint::PathShape, pos2, vec2, Align, Align2, Context, FontId, Frame, Layout, Margin, Painter, Pos2,
    Rect, Response, RichText, Rounding, Sense, Shape, Stroke, TextureHandle, Ui,
};
use egui_extras::{Column, TableBuilder};
use downcraft_core::{util, ytdlp, AfterQueue, Category, DoubleClick, DuplicateMode, Event, Manager, NewDownload, Status};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All(Option<Category>),
    Unfinished(Option<Category>),
    Finished(Option<Category>),
    Grabber,
    MainQueue,
    SyncQueue,
}

#[derive(Clone, Copy, PartialEq)]
enum Cmd {
    AddUrl,
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
    About,
    Setup,
    Appearance,
    ImportList,
    ExportList,
    Exit,
    OpenSelected,
    FolderSelected,
    MoveRename,
}

enum Act {
    Open(u64),
    OpenWith(u64),
    Folder(u64),
    Resume(u64),
    Stop(u64),
    Progress(u64),
    DeleteSel,
    MoveRename(u64),
    Redownload(u64),
    /// Put the selection into (`true`) or take it out of (`false`) the main queue.
    Queue(bool),
    SetDoubleClick(DoubleClick),
    Properties(u64),
}

struct Row {
    id: u64,
    cat: Category,
    name: String,
    size: String,
    status: String,
    eta: String,
    rate: String,
    last: String,
    desc: String,
    added: String,
    save_to: String,
    referer: String,
    page: String,
    queued: bool,
    done: bool,
    active: bool,
    // raw values, for sorting by column
    size_b: u64,
    eta_s: u64,
    rate_b: u64,
    last_ts: i64,
    added_ts: i64,
}

/// The user's home folder, so "Save To" can read `Downloads\Video\x.mp4` like IDM does.
fn home_dir() -> Option<&'static std::path::Path> {
    static HOME: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    HOME.get_or_init(|| std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from))
        .as_deref()
}

/// Value of a stored request header (name compared without case).
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// One entry of the list's right-click menu; clicking it closes the menu.
fn menu_item(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let clicked = ui.add_enabled(enabled, egui::Button::new(label).frame(false)).clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

pub struct App {
    mgr: Arc<Manager>,
    logo: TextureHandle,
    /// Developer portrait for the About page: the live GitHub avatar.
    dev: crate::dialogs::AboutArt,
    filter: Filter,
    open: [bool; 4],
    show_sidebar: bool,
    selected: Vec<u64>,
    /// (column, ascending) of the list sort, set by clicking a column header
    sort: Option<(usize, bool)>,
    next_key: u64,
    add_url: Option<AddUrl>,
    video_picks: Vec<VideoPick>,
    confirms: Vec<Confirm>,
    rehide_frames: u32,
    progresses: Vec<Progress>,
    completes: Vec<Complete>,
    options: Option<OptionsDlg>,
    scheduler: Option<SchedulerDlg>,
    delete: Option<DeleteDlg>,
    about: bool,
    /// The Appearance page (theme, accent colour, toolbar skin).
    appearance: Option<AppearanceDlg>,
    /// Online skin gallery + skins saved on this PC.
    skin_store: Arc<SkinStore>,
    extension: Option<ExtensionDlg>,
    toolbox: Arc<downcraft_core::tools::ToolBox>,
    /// Main window kept above every other window (title-bar pin); `pin_sent` = level already applied at start.
    pinned: bool,
    pin_sent: bool,
    /// First-run setup window (extension + helper programs) and whether the start-up check has run (0 = waiting for the tool scan, 2 = done).
    setup: Option<SetupDlg>,
    setup_check: u8,
    move_dlg: Option<MoveDlg>,
    props: Option<PropsDlg>,
    tray: Option<crate::tray::Tray>,
    quitting: Arc<AtomicBool>,
    clip: clipboard::Monitor,
    /// Set by the engine thread when the queue finished and Options asks for an action afterwards.
    after_pending: Arc<Mutex<Option<AfterQueue>>>,
    /// Waiting for the last running download before the action is carried out.
    after_wait: Option<AfterQueue>,
    power: Option<PowerDlg>,
    dup: Option<DupDlg>,
    /// Generic message pop-up (title, text).
    notice: Option<(String, String)>,
    /// Started by the OS at login: may start minimized (decided once, on the first frame).
    autostart_launch: bool,
    start_checked: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::apply(&cc.egui_ctx);
        let img = image::load_from_memory(crate::LOGO).expect("logo").to_rgba8();
        let (w, h) = img.dimensions();
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        // pop-up windows draw their own title bar and fetch the logo from here
        cc.egui_ctx.data_mut(|d| d.insert_temp(egui::Id::new("app-logo"), logo.clone()));
        let mgr = Manager::new();
        // toolbar skin the user loaded in an earlier session
        icons::set_skin(crate::skin::load_saved(mgr.data_dir()));
        {
            let c = cc.egui_ctx.clone();
            mgr.set_repaint(move || c.request_repaint());
            mgr.set_incoming_hook(crate::tray::wake_for_dialog);
        }
        mgr.start_bridge();
        if mgr.settings.lock().schedule.on_launch {
            mgr.start_queue(); // Scheduler: "Start the main queue when DownCraft starts"
        }
        let after_pending = Arc::new(Mutex::new(None));
        Self::install_event_hook(&mgr, after_pending.clone());
        let toolbox = downcraft_core::tools::ToolBox::new(mgr.clone());
        let skin_store = SkinStore::new(mgr.clone());
        let pinned = mgr.settings.lock().pin_main;
        let dev = crate::dialogs::AboutArt::new(&cc.egui_ctx, mgr.data_dir().join("developer.png"), crate::LOGO);
        {
            // theme and accent colour chosen in an earlier session
            let st = mgr.settings.lock();
            crate::theme::set(&cc.egui_ctx, Theme::from_key(&st.theme), Accent::from_key(&st.accent));
        }
        App {
            mgr,
            logo,
            dev,
            filter: Filter::All(None),
            open: [true, false, false, false], // like IDM: only "All Downloads" starts expanded,
            show_sidebar: true,
            selected: vec![],
            sort: None,
            next_key: 1,
            add_url: None,
            video_picks: vec![],
            confirms: vec![],
            rehide_frames: 0,
            progresses: vec![],
            completes: vec![],
            options: None,
            scheduler: None,
            delete: None,
            about: false,
            appearance: None,
            skin_store,
            extension: None,
            toolbox,
            pinned,
            pin_sent: false,
            setup: None,
            setup_check: 0,
            move_dlg: None,
            props: None,
            tray: None,
            quitting: Arc::new(AtomicBool::new(false)),
            clip: clipboard::Monitor::new(),
            after_pending,
            after_wait: None,
            power: None,
            dup: None,
            notice: None,
            autostart_launch: std::env::args().any(|a| a == "--autostart"),
            start_checked: false,
        }
    }

    /// Sounds, the command after a download and the queue-finished action must work while the
    /// window is hidden in the tray, so they run from the engine's event hook, not from a frame.
    fn install_event_hook(mgr: &Arc<Manager>, pending: Arc<Mutex<Option<AfterQueue>>>) {
        let weak = Arc::downgrade(mgr);
        mgr.set_event_hook(move |ev| {
            let Some(m) = weak.upgrade() else { return };
            match ev {
                Event::Completed(id) => {
                    let (snd, cmd) = {
                        let s = m.settings.lock();
                        (
                            s.sounds.complete.enabled.then(|| s.sounds.complete.file.clone()),
                            (s.post_command_on && !s.post_command.trim().is_empty()).then(|| s.post_command.clone()),
                        )
                    };
                    if let Some(f) = snd {
                        sys::play_sound(SoundKind::Complete, &f);
                    }
                    if let (Some(cmd), Some(t)) = (cmd, m.find(id)) {
                        sys::run_after_download(&cmd, &t.final_path());
                    }
                }
                Event::Failed(id) => {
                    let (snd, max) = {
                        let s = m.settings.lock();
                        (s.sounds.failed.enabled.then(|| s.sounds.failed.file.clone()), s.auto_retries)
                    };
                    // only the last failure counts: an automatic retry is still coming otherwise
                    let last = m.find(id).map_or(false, |t| !t.transient.load(Relaxed) || t.retries.load(Relaxed) >= max);
                    if let (Some(f), true) = (snd, last) {
                        sys::play_sound(SoundKind::Failed, &f);
                    }
                }
                Event::QueueFinished => {
                    let (snd, after, open_file) = {
                        let s = m.settings.lock();
                        (
                            s.sounds.queue_done.enabled.then(|| s.sounds.queue_done.file.clone()),
                            s.after_queue,
                            (s.schedule.open_on && !s.schedule.open_file.trim().is_empty()).then(|| s.schedule.open_file.clone()),
                        )
                    };
                    if let Some(f) = open_file {
                        let _ = open::that_detached(f.trim());
                    }
                    if let Some(f) = snd {
                        sys::play_sound(SoundKind::QueueDone, &f);
                    }
                    if after != AfterQueue::Nothing {
                        *pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(after);
                        crate::tray::wake_for_dialog();
                    }
                }
            }
        });
    }

    fn add_single_unchecked(&mut self, ctx: &Context, url: String) {
        self.add_single(ctx, url, false);
    }

    fn already_listed(&self, url: &str) -> Option<String> {
        let t = self.mgr.tasks.lock().iter().find(|t| t.url == url).cloned()?;
        let name = t.info.lock().file_name.clone();
        Some(if name.is_empty() { util::file_name_from_url(url) } else { name })
    }

    /// One link from Add URL or the clipboard: duplicate check, then either the Download File Info
    /// dialog or (when that is switched off) straight into its category folder.
    fn add_single(&mut self, ctx: &Context, url: String, check_dup: bool) {
        if check_dup {
            if let Some(existing) = self.already_listed(&url) {
                let mode = self.mgr.settings.lock().duplicate_mode;
                match mode {
                    DuplicateMode::Ask => {
                        self.dup = Some(DupDlg { url, existing });
                        return;
                    }
                    DuplicateMode::Skip => {
                        self.notice = Some(("Already in the list".into(), format!("{existing} is already in the list, so it was not added again.")));
                        return;
                    }
                    DuplicateMode::AddAnyway => {}
                }
            }
        }
        if self.mgr.settings.lock().show_start_dialog {
            let key = self.key();
            self.confirms.push(Confirm::new(ctx, &self.mgr, key, url, vec![], None, 0));
        } else {
            let name = util::file_name_from_url(&url);
            let category = Category::from_file_name(&name);
            let dir = self.mgr.settings.lock().dir_for(category);
            self.mgr.add(NewDownload {
                url,
                dir,
                file_name: String::new(),
                category,
                description: String::new(),
                probe: None,
                queued: false,
                start: true,
                headers: vec![],
            });
        }
    }

    /// What Add URL does with its lines: addresses on video sites (or all of them, with *As video*) go to
    /// yt-dlp and a quality picker; the rest are ordinary downloads.
    fn add_links(&mut self, ctx: &Context, urls: Vec<String>, as_video: bool) {
        const MAX_VIDEO_PAGES: usize = 6; // one yt-dlp run each, so a pasted list can't start dozens at once
        let (videos, plain): (Vec<String>, Vec<String>) = urls.into_iter().partition(|u| as_video || ytdlp::is_video_site(u));
        if videos.len() > MAX_VIDEO_PAGES {
            self.notice = Some((
                "Too many video pages".into(),
                format!("Only the first {MAX_VIDEO_PAGES} video pages were opened. Add the rest again afterwards."),
            ));
        }
        for url in videos.into_iter().take(MAX_VIDEO_PAGES) {
            let key = self.key();
            self.video_picks.push(VideoPick::new(ctx, &self.mgr, key, url));
        }
        match plain.len() {
            0 => {}
            1 => self.add_single(ctx, plain.into_iter().next().unwrap_or_default(), true),
            _ => self.add_batch(plain),
        }
    }

    /// Several addresses at once: each goes to its category folder and into the main queue, which
    /// is started, so they download one after another without a dialog per file.
    fn add_batch(&mut self, urls: Vec<String>) {
        let mut skipped = 0;
        let anyway = self.mgr.settings.lock().duplicate_mode == DuplicateMode::AddAnyway;
        for url in urls {
            if !anyway && self.already_listed(&url).is_some() {
                skipped += 1;
                continue;
            }
            let name = util::file_name_from_url(&url);
            let category = Category::from_file_name(&name);
            let dir = self.mgr.settings.lock().dir_for(category);
            self.mgr.add(NewDownload {
                url,
                dir,
                file_name: String::new(),
                category,
                description: String::new(),
                probe: None,
                queued: true,
                start: false,
                headers: vec![],
            });
        }
        if skipped > 0 {
            self.notice = Some(("Already in the list".into(), format!("{skipped} link(s) were already in the list and were not added again.")));
        }
        self.mgr.start_queue();
    }

    fn key(&mut self) -> u64 {
        self.next_key += 1;
        self.next_key
    }

    // ───────────── state helpers ─────────────

    fn matches(&self, info: &downcraft_core::TaskInfo) -> bool {
        let cat_ok = |c: &Option<Category>| c.map_or(true, |c| c == info.category);
        let done = info.status == Status::Complete;
        match &self.filter {
            Filter::All(c) => cat_ok(c),
            Filter::Unfinished(c) => !done && cat_ok(c),
            Filter::Finished(c) => done && cat_ok(c),
            Filter::MainQueue => info.queued && !done,
            Filter::Grabber | Filter::SyncQueue => false,
        }
    }

    fn rows(&self) -> Vec<Row> {
        use chrono::TimeZone;
        let tasks = self.mgr.tasks.lock().clone();
        tasks
            .iter()
            .filter_map(|t| {
                let i = t.info.lock().clone();
                if !self.matches(&i) {
                    return None;
                }
                let total = t.total();
                let status = match i.status {
                    Status::Complete => "Complete".to_string(),
                    Status::Error => "Error".to_string(),
                    Status::Connecting => "Connecting".to_string(),
                    _ if total > 0 => format!("{:.2}%", t.percent()),
                    _ => String::new(),
                };
                let downloading = i.status == Status::Downloading;
                let eta = if downloading { t.eta() } else { None };
                let speed = if downloading { t.speed() } else { 0 };
                let name = if i.file_name.is_empty() { util::file_name_from_url(&t.url) } else { i.file_name.clone() };
                let save_to = {
                    let full = i.dir.join(&name);
                    let shown = home_dir().and_then(|h| full.strip_prefix(h).ok()).unwrap_or(&full);
                    shown.display().to_string()
                };
                Some(Row {
                    id: t.id,
                    cat: i.category,
                    name,
                    size: if total > 0 && i.status != Status::Error {
                        format!("{}{}", if i.hls && i.status != Status::Complete { "~" } else { "" }, util::fmt_size(total))
                    } else {
                        String::new()
                    },
                    status,
                    eta: eta.map(util::fmt_time).unwrap_or_default(),
                    rate: if downloading { util::fmt_rate(speed, 2) } else { String::new() },
                    size_b: total,
                    eta_s: eta.unwrap_or(0),
                    rate_b: speed,
                    last_ts: i.last_try,
                    last: chrono::Local
                        .timestamp_opt(i.last_try, 0)
                        .single()
                        .map(|d| d.format("%b %d %H:%M:%S").to_string())
                        .unwrap_or_default(),
                    desc: match (&i.status, &i.error) {
                        (Status::Error, Some(e)) => e.clone(),
                        _ => i.description.clone(),
                    },
                    added_ts: i.added,
                    added: chrono::Local
                        .timestamp_opt(i.added, 0)
                        .single()
                        .map(|d| d.format("%b %d %H:%M:%S %Y").to_string())
                        .unwrap_or_default(),
                    save_to,
                    referer: header(&i.headers, "referer").unwrap_or_default().to_string(),
                    // the page the download was started from; older entries only know the referer
                    page: header(&i.headers, "x-downcraft-page").or_else(|| header(&i.headers, "referer")).unwrap_or_default().to_string(),
                    queued: i.queued && i.status != Status::Complete,
                    done: i.status == Status::Complete,
                    active: matches!(i.status, Status::Connecting | Status::Downloading),
                })
            })
            .collect()
    }

    fn selected_tasks(&self) -> Vec<Arc<downcraft_core::Task>> {
        self.selected.iter().filter_map(|id| self.mgr.find(*id)).collect()
    }

    // ───────────── commands ─────────────

    fn run(&mut self, ctx: &Context, cmd: Cmd) {
        match cmd {
            Cmd::AddUrl => {
                if self.add_url.is_none() {
                    self.add_url = Some(AddUrl { url: String::new(), error: None, focused: false, video: false });
                }
            }
            Cmd::Resume => {
                for t in self.selected_tasks() {
                    self.mgr.start(&t);
                }
            }
            Cmd::Stop => {
                for t in self.selected_tasks() {
                    self.mgr.pause(&t);
                }
            }
            Cmd::StopAll => self.mgr.pause_all(),
            Cmd::Delete => {
                if !self.selected.is_empty() && self.delete.is_none() {
                    self.delete = Some(DeleteDlg { ids: self.selected.clone(), files: false });
                }
            }
            Cmd::DeleteCompleted => {
                self.mgr.delete_completed();
                let mgr = self.mgr.clone();
                self.selected.retain(|id| mgr.find(*id).is_some());
            }
            Cmd::Options => self.options = Some(OptionsDlg::new(&self.mgr)),
            Cmd::Scheduler => self.scheduler = Some(SchedulerDlg::new(&self.mgr)),
            Cmd::StartQueue => self.mgr.start_queue(),
            Cmd::StopQueue => self.mgr.stop_queue(),
            Cmd::Friend => {
                let _ = open::that("mailto:?subject=DownCraft%20download%20manager&body=Try%20DownCraft%2C%20a%20free%20open-source%20download%20manager%20written%20in%20Rust.");
            }
            Cmd::About => self.about = true,
            Cmd::Appearance => {
                if self.appearance.is_none() {
                    self.appearance = Some(AppearanceDlg::new(&self.skin_store, ctx));
                }
            }
            Cmd::Setup => {
                self.toolbox.refresh();
                if self.setup.is_none() {
                    self.setup = Some(SetupDlg::new(&self.mgr));
                }
            }
            Cmd::ExportList => {
                if let Some(dest) = rfd::FileDialog::new()
                    .set_title("Export downloads")
                    .set_file_name("downcraft-export.json")
                    .add_filter("DownCraft export", &["json"])
                    .save_file()
                {
                    let (title, text) = match self.mgr.export_to(&dest) {
                        Ok(n) => ("Export", format!("{n} download(s) exported to\n{}", dest.display())),
                        Err(e) => ("Export failed", format!("Could not write the file: {e}")),
                    };
                    rfd::MessageDialog::new().set_title(title).set_description(text).set_buttons(rfd::MessageButtons::Ok).show();
                }
            }
            Cmd::ImportList => {
                if let Some(src) = rfd::FileDialog::new()
                    .set_title("Import downloads")
                    .add_filter("DownCraft export or list of links", &["json", "txt"])
                    .pick_file()
                {
                    let (title, text) = match self.mgr.import_from(&src) {
                        Ok(r) => (
                            "Import",
                            format!(
                                "{} download(s) added (stopped - select them and press Resume).{}",
                                r.added,
                                if r.skipped > 0 { format!("\n{} skipped (already in the list or not an http/https link).", r.skipped) } else { String::new() }
                            ),
                        ),
                        Err(e) => ("Import failed", format!("Could not read the file: {e}")),
                    };
                    ctx.request_repaint();
                    rfd::MessageDialog::new().set_title(title).set_description(text).set_buttons(rfd::MessageButtons::Ok).show();
                }
            }
            Cmd::MoveRename => {
                if let (Some(id), None) = (self.selected.first().copied(), &self.move_dlg) {
                    if let Some(t) = self.mgr.find(id) {
                        self.move_dlg = Some(MoveDlg::new(&t));
                    }
                }
            }
            Cmd::Exit => {
                self.quitting.store(true, Relaxed); // a real exit, not "hide to tray"
                ctx.send_viewport_cmd(egui::ViewportCommand::Close)
            }
            Cmd::OpenSelected => {
                if let Some(t) = self.selected_tasks().first() {
                    if t.info.lock().status == Status::Complete {
                        let _ = open::that(t.final_path());
                    }
                }
            }
            Cmd::FolderSelected => {
                if let Some(t) = self.selected_tasks().first() {
                    let _ = open::that(t.info.lock().dir.clone());
                }
            }
        }
    }

    fn process_events(&mut self, ctx: &Context) {
        for ev in self.mgr.take_events() {
            let Event::Completed(id) = ev else { continue };
            let flags = self
                .progresses
                .iter()
                .find(|p| p.task.id == id)
                .map(|p| (p.open_file, p.open_folder, p.exit_when_done))
                .unwrap_or_default();
            self.progresses.retain(|p| p.task.id != id);
            let Some(t) = self.mgr.find(id) else { continue };
            let path = t.final_path();
            if self.mgr.settings.lock().show_complete_dialog {
                let key = self.key();
                self.completes.push(Complete { key, url: t.url.clone(), path: path.clone(), size: t.total(), dont_show: false, drag: false });
            }
            if flags.0 {
                let _ = open::that(&path);
            }
            if flags.1 {
                if let Some(d) = path.parent() {
                    let _ = open::that(d);
                }
            }
            if flags.2 {
                self.quitting.store(true, Relaxed);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Clipboard monitor, the action after the queue and the quota warning.
    fn background_features(&mut self, ctx: &Context) {
        // links that were just copied
        let found = {
            let st = self.mgr.settings.lock();
            self.clip.poll(&st)
        };
        let copied: Vec<String> = found
            .into_iter()
            .filter(|u| self.already_listed(u).is_none() && !self.confirms.iter().any(|c| &c.url == u))
            .collect();
        if copied.len() == 1 {
            crate::tray::wake_for_dialog();
            let url = copied.into_iter().next().unwrap_or_default();
            self.add_single(ctx, url, false);
        } else if copied.len() > 1 {
            self.add_batch(copied);
        }

        // what to do now that the queue is finished - once nothing else is still downloading
        if let Some(a) = self.after_pending.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.after_wait = Some(a);
        }
        if let Some(a) = self.after_wait {
            if !self.mgr.any_active() {
                self.after_wait = None;
                match a {
                    AfterQueue::CloseApp => {
                        self.quitting.store(true, Relaxed);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    AfterQueue::Sleep | AfterQueue::Shutdown => {
                        crate::tray::wake_for_dialog();
                        self.power = Some(PowerDlg { action: a, deadline: Instant::now() + Duration::from_secs(POWER_COUNTDOWN_SECS) });
                    }
                    AfterQueue::Nothing => {}
                }
            }
        }

        if self.mgr.take_quota_notice() {
            crate::tray::wake_for_dialog();
            let left = self.mgr.quota_status().map_or(0, |q| q.resets_in);
            self.notice = Some((
                "Download quota reached".into(),
                format!("The limit set under Options > Connection was reached, so downloads were stopped. They can be started again in {}.", util::fmt_time(left)),
            ));
        }
    }

    // ───────────── panels ─────────────

    fn menu_bar(&mut self, ctx: &Context, cmd: &mut Option<Cmd>) {
        let has_sel = !self.selected.is_empty();
        let any_active = self.mgr.any_active();
        // the menus live in the title bar itself: logo, menus, then the window buttons
        let logo = self.logo.clone();
        let pin_clicked = crate::titlebar::show(ctx, &logo, self.pinned, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().button_padding = vec2(7.0, 2.0);
                ui.spacing_mut().interact_size.y = 19.0;
                egui::menu::bar(ui, |ui| {
                    let mut item = |ui: &mut Ui, label: &str, enabled: bool, c: Cmd| {
                        if ui.add_enabled(enabled, egui::Button::new(label).frame(false)).clicked() {
                            *cmd = Some(c);
                            ui.close_menu();
                        }
                    };
                    ui.menu_button(mnemonic("Tasks"), |ui| {
                        item(ui, "Add new download…", true, Cmd::AddUrl);
                        item(ui, "Setup guide…", true, Cmd::Setup);
                        ui.separator();
                        item(ui, "Import downloads…", true, Cmd::ImportList);
                        item(ui, "Export downloads…", true, Cmd::ExportList);
                        ui.separator();
                        item(ui, "Exit", true, Cmd::Exit);
                    });
                    ui.menu_button(mnemonic("File"), |ui| {
                        item(ui, "Open", has_sel, Cmd::OpenSelected);
                        item(ui, "Open folder", has_sel, Cmd::FolderSelected);
                        ui.separator();
                        item(ui, "Delete", has_sel, Cmd::Delete);
                    });
                    ui.menu_button(mnemonic("Downloads"), |ui| {
                        item(ui, "Resume", has_sel, Cmd::Resume);
                        item(ui, "Stop", has_sel, Cmd::Stop);
                        item(ui, "Stop All", any_active, Cmd::StopAll);
                        ui.separator();
                        item(ui, "Delete Completed", true, Cmd::DeleteCompleted);
                        ui.separator();
                        item(ui, "Start Queue", true, Cmd::StartQueue);
                        item(ui, "Stop Queue", true, Cmd::StopQueue);
                        item(ui, "Scheduler…", true, Cmd::Scheduler);
                        ui.separator();
                        item(ui, "Options…", true, Cmd::Options);
                    });
                    ui.menu_button(mnemonic("View"), |ui| {
                        ui.checkbox(&mut self.show_sidebar, "Categories");
                        ui.separator();
                        item(ui, "Appearance…", true, Cmd::Appearance);
                    });
                    // "About" is a single entry: clicking it opens the About page
                    let bg = ui.painter().add(Shape::Noop);
                    let r = ui.add(egui::Button::new(mnemonic("About")).frame(false));
                    if r.hovered() || r.is_pointer_button_down_on() {
                        ui.painter().set(bg, Shape::rect_filled(r.rect, Rounding::same(2.0), ui.visuals().widgets.hovered.weak_bg_fill));
                    }
                    if r.clicked() {
                        *cmd = Some(Cmd::About);
                    }
                });
        });
        if pin_clicked {
            self.pinned = !self.pinned;
            self.set_level(ctx);
            self.mgr.settings.lock().pin_main = self.pinned;
            self.mgr.save_settings();
        }
    }

    /// Applies the "always on top" state to the main window.
    fn set_level(&self, ctx: &Context) {
        let level = if self.pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
    }

    fn toolbar(&mut self, ctx: &Context, cmd: &mut Option<Cmd>) {
        let tasks = self.selected_tasks();
        let any_resumable = tasks.iter().any(|t| !matches!(t.info.lock().status, Status::Complete | Status::Connecting | Status::Downloading));
        let any_stoppable = tasks.iter().any(|t| t.is_active());
        let any_active = self.mgr.any_active();
        let any_complete = self.mgr.tasks.lock().iter().any(|t| t.info.lock().status == Status::Complete);
        let qrun = self.mgr.queue_running.load(std::sync::atomic::Ordering::Relaxed);
        let any_queued = self.mgr.tasks.lock().iter().any(|t| {
            let i = t.info.lock();
            i.queued && i.status != Status::Complete
        });
        let has_sel = !self.selected.is_empty();

        egui::TopBottomPanel::top("toolbar")
            .exact_height(70.0)
            .frame(Frame::none().fill(pal().client_bg).inner_margin(Margin::symmetric(1.0, 0.0)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.horizontal(|ui| {
                    let first: [(Tool, &str, &str, bool, Cmd); 8] = [
                        (Tool::Add, "Add URL", "Add a new download", true, Cmd::AddUrl),
                        (Tool::Resume, "Resume", "Resume the selected downloads", any_resumable, Cmd::Resume),
                        (Tool::Stop, "Stop", "Stop the selected downloads", any_stoppable, Cmd::Stop),
                        (Tool::StopAll, "Stop All", "Stop all active downloads", any_active, Cmd::StopAll),
                        (Tool::Delete, "Delete", "Delete the selected downloads", has_sel, Cmd::Delete),
                        (Tool::DeleteCompleted, "Delete Co...", "Delete completed downloads from the list", any_complete, Cmd::DeleteCompleted),
                        (Tool::Options, "Options", "Options", true, Cmd::Options),
                        (Tool::Scheduler, "Scheduler", "Scheduler", true, Cmd::Scheduler),
                    ];
                    for (icon, label, tip, en, c) in first {
                        if tool_button(ui, icon, label, tip, en, false).0 {
                            *cmd = Some(c);
                        }
                    }
                    // the two queue buttons carry a drop-down arrow, as in IDM
                    let en = any_queued && !qrun;
                    let (main, arrow) = tool_button(ui, Tool::StartQueue, "Start Qu...", "Start the main download queue", en, true);
                    if main {
                        *cmd = Some(Cmd::StartQueue);
                    }
                    if let Some(a) = arrow {
                        queue_dropdown(ui, a, "start_queue_menu", cmd, Cmd::StartQueue, en);
                    }
                    let (main, arrow) = tool_button(ui, Tool::StopQueue, "Stop Qu...", "Stop the main download queue", qrun, true);
                    if main {
                        *cmd = Some(Cmd::StopQueue);
                    }
                    if let Some(a) = arrow {
                        queue_dropdown(ui, a, "stop_queue_menu", cmd, Cmd::StopQueue, qrun);
                    }
                    if tool_button(ui, Tool::Friend, "Tell a Friend", "Tell a friend about DownCraft", true, false).0 {
                        *cmd = Some(Cmd::Friend);
                    }
                });
            });
    }

    fn sidebar(&mut self, ctx: &Context) {
        let mut hide = false;
        // new id ("side_v2"): older builds saved a collapsed width under "side" that would be restored on start
        egui::SidePanel::left("side_v2")
            .resizable(true)
            .default_width(220.0)
            .width_range(150.0..=420.0)
            .show_separator_line(false)
            .frame(Frame::none().fill(pal().client_bg).inner_margin(Margin { left: 6.0, right: 2.0, top: 0.0, bottom: 6.0 }))
            .show(ctx, |ui| {
                // egui stores the panel's width from what its content occupies; the tree is drawn into a
                // manually placed rect, so without this the panel collapses to its minimum width every frame
                ui.set_min_width(ui.available_width());
                let full = ui.available_rect_before_wrap();
                let p = ui.painter().clone();
                p.rect_filled(full, Rounding::ZERO, pal().tree_bg);
                let head = Rect::from_min_size(full.min, vec2(full.width(), 18.0));
                p.rect_filled(head, Rounding::ZERO, pal().client_bg);
                p.rect_stroke(full.shrink(0.5), Rounding::ZERO, Stroke::new(1.0_f32, pal().panel_border));
                p.text(head.left_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER, "Categories", FontId::proportional(ui_size()), pal().text);
                // close [x]
                let xr = Rect::from_center_size(pos2(head.right() - 12.0, head.center().y), vec2(16.0, 16.0));
                let xresp = ui.interact(xr, ui.id().with("hide_categories"), Sense::click());
                let xc = xr.center();
                let xs = Stroke::new(1.5_f32, pal().text);
                p.line_segment([xc + vec2(-4.0, -4.0), xc + vec2(4.0, 4.0)], xs);
                p.line_segment([xc + vec2(-4.0, 4.0), xc + vec2(4.0, -4.0)], xs);
                if xresp.clicked() {
                    hide = true;
                }

                let body = Rect::from_min_max(pos2(full.left() + 1.0, head.bottom()), pos2(full.right() - 1.0, full.bottom() - 1.0));
                ui.allocate_ui_at_rect(body, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        let sel = self.filter;
                        let mut new_filter = None;
                        let mut open = self.open;
                        let cats = |mk: fn(Option<Category>) -> Filter| -> Vec<(Filter, Option<Category>, &'static str)> {
                            Category::ALL.iter().map(|c| (mk(Some(*c)), Some(*c), c.name())).collect()
                        };
                        // (label, icon, filter, children, index into `open`)
                        let roots: Vec<(&str, Node, Filter, Vec<(Filter, Option<Category>, &str)>, Option<usize>)> = vec![
                            ("All Downloads", Node::All, Filter::All(None), cats(Filter::All), Some(0)),
                            ("Unfinished", Node::Unfinished, Filter::Unfinished(None), cats(Filter::Unfinished), Some(1)),
                            ("Finished", Node::Finished, Filter::Finished(None), cats(Filter::Finished), Some(2)),
                            ("Grabber projects", Node::Grabber, Filter::Grabber, vec![], None),
                            (
                                "Queues",
                                Node::Queues,
                                Filter::MainQueue,
                                vec![(Filter::MainQueue, None, "Main download queue"), (Filter::SyncQueue, None, "Synchronization queue")],
                                Some(3),
                            ),
                        ];

                        // geometry gathered while the rows are laid out, for the dotted connector lines
                        let mut root_geo: Vec<(f32, bool, f32)> = vec![]; // (y, has chevron, icon left)
                        let mut kid_geo: Vec<(f32, f32, Vec<(f32, f32)>)> = vec![]; // (parent icon x, parent y, [(y, icon left)])
                        let x0 = ui.max_rect().left();
                        for (label, node, f, kids, idx) in &roots {
                            let is_open = idx.map(|i| open[i]);
                            let hit = tree_row(ui, 0, label, is_open, sel == *f, |p, c| icons::node(p, c, *node));
                            if hit.clicked {
                                new_filter = Some(*f);
                            }
                            if hit.toggled {
                                if let Some(i) = idx {
                                    open[*i] = !open[*i];
                                }
                            }
                            root_geo.push((hit.y, idx.is_some(), hit.icon_left));
                            if is_open == Some(true) {
                                let mut ks = vec![];
                                for (kf, cat, name) in kids {
                                    let k = tree_row(ui, 1, name, None, sel == *kf, |p, c| match cat {
                                        Some(cat) => icons::category(p, c, *cat, 1.0),
                                        None => icons::node(p, c, Node::Queues),
                                    });
                                    if k.clicked {
                                        new_filter = Some(*kf);
                                    }
                                    ks.push((k.y, k.icon_left));
                                }
                                kid_geo.push((hit.icon_x, hit.y, ks));
                            }
                        }

                        // dotted connectors, like the Windows tree view
                        let px = |v: f32| v.floor() + 0.5;
                        let dots = |a: Pos2, b: Pos2| {
                            ui.painter().extend(Shape::dotted_line(&[pos2(px(a.x), px(a.y)), pos2(px(b.x), px(b.y))], pal().dots, 2.0, 0.5));
                        };
                        let rx = x0 + 11.0;
                        for w in root_geo.windows(2) {
                            let (a, b) = (w[0], w[1]);
                            dots(pos2(rx, a.0 + if a.1 { 7.0 } else { 0.0 }), pos2(rx, b.0 - if b.1 { 7.0 } else { 0.0 }));
                        }
                        for (y, chevron, left) in &root_geo {
                            dots(pos2(rx + if *chevron { 5.0 } else { 0.0 }, *y), pos2(*left - 2.0, *y));
                        }
                        for (ix, py, ks) in &kid_geo {
                            if let Some(last) = ks.last() {
                                dots(pos2(*ix, py + 8.0), pos2(*ix, last.0));
                            }
                            for (y, left) in ks {
                                dots(pos2(*ix, *y), pos2(*left - 2.0, *y));
                            }
                        }

                        self.open = open;
                        if let Some(f) = new_filter {
                            self.filter = f;
                        }
                    });
                });
            });
        if hide {
            self.show_sidebar = false;
        }
    }

    fn table(&mut self, ctx: &Context) {
        const HEAD: [&str; 12] = [
            "File Name", "Q", "Size", "Status", "Time left", "Transfer rate", "Last Try...", "Description", "Date Added", "Save To", "Referer",
            "Parent web page",
        ];
        let dbl = self.mgr.settings.lock().double_click;
        let mut rows = self.rows();
        if let Some((col, asc)) = self.sort {
            rows.sort_by(|a, b| {
                let o = match col {
                    0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    1 => a.queued.cmp(&b.queued),
                    2 => a.size_b.cmp(&b.size_b),
                    3 => a.status.cmp(&b.status),
                    4 => a.eta_s.cmp(&b.eta_s),
                    5 => a.rate_b.cmp(&b.rate_b),
                    6 => a.last_ts.cmp(&b.last_ts),
                    7 => a.desc.to_lowercase().cmp(&b.desc.to_lowercase()),
                    8 => a.added_ts.cmp(&b.added_ts),
                    9 => a.save_to.to_lowercase().cmp(&b.save_to.to_lowercase()),
                    10 => a.referer.to_lowercase().cmp(&b.referer.to_lowercase()),
                    _ => a.page.to_lowercase().cmp(&b.page.to_lowercase()),
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            });
        }
        let sort = self.sort;
        let mut sort_click: Option<usize> = None;
        let mut acts: Vec<Act> = Vec::new();
        // rows drawn this frame (id + screen rect): the drag-select works out which ones the drag covers
        let mut row_rects: Vec<(u64, Rect)> = Vec::new();
        let drag_id = egui::Id::new("list-drag-select");
        let selected = &mut self.selected;
        let multi = ctx.input(|i| i.modifiers.command || i.modifiers.shift);

        egui::CentralPanel::default()
            .frame(Frame::none().fill(pal().client_bg).inner_margin(Margin { left: 5.0, right: 3.0, top: 0.0, bottom: 6.0 }))
            .show(ctx, |ui| {
                // the list control: 1px frame, grey rows, dark scroll-bar track
                let full = ui.available_rect_before_wrap();
                let inner = full.shrink(1.0);
                ui.painter().rect_filled(full, Rounding::ZERO, pal().row_bg);
                // like a Windows list view: the dark scroll-bar track (and the space it takes) exists only
                // while the list actually scrolls; otherwise the columns run right up to the frame
                let overflow = rows.len() as f32 * 17.0 > inner.height() - 24.0;
                // like IDM: the grid (row lines + column dividers) fills the whole list box, entries or not,
                // so when nothing scrolls the real rows are followed by empty ones down to the bottom edge
                let fit = ((inner.height() - 24.0) / 17.0).ceil().max(0.0) as usize;
                let total_rows = if overflow { rows.len() } else { rows.len().max(fit) };
                if overflow {
                    let track = Rect::from_min_max(pos2(inner.right() - 17.0, inner.top()), inner.max);
                    ui.painter().rect_filled(track, Rounding::ZERO, pal().scroll_track);
                }
                ui.painter().rect_stroke(full.shrink(0.5), Rounding::ZERO, Stroke::new(1.0_f32, pal().list_border));

                ui.allocate_ui_at_rect(inner, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    ui.spacing_mut().scroll = egui::style::ScrollStyle {
                        floating: false,
                        bar_width: if overflow { 3.0 } else { 0.0 },
                        bar_inner_margin: if overflow { 7.0 } else { 0.0 },
                        bar_outer_margin: if overflow { 7.0 } else { 0.0 },
                        handle_min_length: 24.0,
                        foreground_color: true,
                        dormant_background_opacity: 0.0,
                        active_background_opacity: 0.0,
                        interact_background_opacity: 0.0,
                        dormant_handle_opacity: 1.0,
                        active_handle_opacity: 1.0,
                        interact_handle_opacity: 1.0,
                        ..egui::style::ScrollStyle::solid()
                    };
                    ui.visuals_mut().selection.bg_fill = pal().row_sel;
                    ui.visuals_mut().widgets.inactive.fg_stroke.color = pal().scroll_thumb;

                    TableBuilder::new(ui)
                        .striped(false)
                        .vscroll(overflow)
                        .resizable(true)
                        .sense(Sense::click_and_drag())
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .column(Column::initial(180.0).at_least(40.0).clip(true))
                        .column(Column::initial(36.0).at_least(24.0).clip(true))
                        .column(Column::initial(78.0).at_least(40.0).clip(true))
                        .column(Column::initial(62.0).at_least(40.0).clip(true))
                        .column(Column::initial(76.0).at_least(40.0).clip(true))
                        .column(Column::initial(88.0).at_least(40.0).clip(true))
                        .column(Column::initial(72.0).at_least(40.0).clip(true))
                        .column(Column::initial(200.0).at_least(40.0).clip(true))
                        .column(Column::initial(184.0).at_least(40.0).clip(true))
                        .column(Column::initial(196.0).at_least(40.0).clip(true))
                        .column(Column::initial(196.0).at_least(40.0).clip(true))
                        .column(Column::remainder().at_least(60.0).clip(true))
                        .header(24.0, |mut h| {
                            for (i, t) in HEAD.iter().enumerate() {
                                h.col(|ui| {
                                    let r = ui.max_rect();
                                    let resp = ui.interact(r, ui.id().with(("column-header", i)), Sense::click());
                                    let p = ui.painter();
                                    p.rect_filled(r, Rounding::ZERO, pal().header_bg);
                                    if i + 1 < HEAD.len() {
                                        let x = r.right() - 0.5;
                                        p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, pal().header_sep));
                                    }
                                    let galley = p.layout_no_wrap(t.to_string(), FontId::proportional(ui_size()), pal().text);
                                    let tw = galley.size().x;
                                    p.galley(pos2(r.left() + 6.0, r.center().y - galley.size().y / 2.0), galley, pal().text);
                                    if let Some((col, asc)) = sort {
                                        if col == i {
                                            let c = pos2((r.left() + 6.0 + tw + 14.0).min(r.right() - 8.0), r.center().y);
                                            let s = if asc { -1.0 } else { 1.0 };
                                            p.add(Shape::line(
                                                vec![c + vec2(-3.5, 1.5 * s), c + vec2(0.0, -2.0 * s), c + vec2(3.5, 1.5 * s)],
                                                Stroke::new(1.3_f32, pal().glyph),
                                            ));
                                        }
                                    }
                                    if resp.clicked() {
                                        sort_click = Some(i);
                                    }
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(17.0, total_rows, |mut row| {
                                let Some(r) = rows.get(row.index()) else {
                                    // empty filler row: only the grid lines
                                    for c in 0..HEAD.len() {
                                        row.col(|ui| {
                                            // no hover highlight on filler rows
                                            ui.painter().rect_filled(ui.max_rect(), Rounding::ZERO, pal().row_bg);
                                            grid(ui, false, c == 0, c + 1 == HEAD.len());
                                        });
                                    }
                                    return;
                                };
                                let sel = selected.contains(&r.id);
                                row.set_selected(sel);
                                row.col(|ui| {
                                    grid(ui, sel, true, false);
                                    ui.add_space(3.0);
                                    let (rc, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                    icons::category(ui.painter(), rc.center(), r.cat, 1.0);
                                    cell_text(ui, &r.name, 3.0);
                                });
                                row.col(|ui| {
                                    grid(ui, sel, false, false);
                                    if r.queued {
                                        ui.add_space(10.0);
                                        let (rc, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                        icons::node(ui.painter(), rc.center(), Node::Queues);
                                    }
                                });
                                let texts = [&r.size, &r.status, &r.eta, &r.rate, &r.last, &r.desc, &r.added, &r.save_to, &r.referer, &r.page];
                                for (n, txt) in texts.into_iter().enumerate() {
                                    row.col(|ui| {
                                        grid(ui, sel, false, n == 9);
                                        cell_text(ui, txt, 5.0);
                                    });
                                }
                                let resp = row.response();
                                row_rects.push((r.id, resp.rect));
                                // pressing on a row and dragging selects every row between the press and the pointer
                                if resp.drag_started_by(egui::PointerButton::Primary) {
                                    let y = ctx.input(|i| i.pointer.press_origin().map(|p| p.y)).unwrap_or(resp.rect.center().y);
                                    let base = if multi { selected.clone() } else { Vec::new() };
                                    ctx.data_mut(|d| d.insert_temp(drag_id, (y, base)));
                                }
                                if resp.clicked() {
                                    if multi {
                                        if let Some(p) = selected.iter().position(|i| *i == r.id) {
                                            selected.remove(p);
                                        } else {
                                            selected.push(r.id);
                                        }
                                    } else {
                                        *selected = vec![r.id];
                                    }
                                }
                                if resp.secondary_clicked() && !selected.contains(&r.id) {
                                    *selected = vec![r.id];
                                }
                                if resp.double_clicked() {
                                    acts.push(match dbl {
                                        DoubleClick::Folder => Act::Folder(r.id),
                                        DoubleClick::Progress => Act::Progress(r.id),
                                        DoubleClick::Auto | DoubleClick::Open if r.done => Act::Open(r.id),
                                        DoubleClick::Auto | DoubleClick::Open => Act::Progress(r.id),
                                    });
                                }
                                resp.context_menu(|ui| {
                                    ui.set_min_width(210.0);
                                    if menu_item(ui, "Open", r.done) {
                                        acts.push(Act::Open(r.id));
                                    }
                                    if menu_item(ui, "Open with...", r.done) {
                                        acts.push(Act::OpenWith(r.id));
                                    }
                                    if menu_item(ui, "Open folder", true) {
                                        acts.push(Act::Folder(r.id));
                                    }
                                    ui.separator();
                                    if menu_item(ui, "Move/Rename (Ctrl+M)", !r.active) {
                                        acts.push(Act::MoveRename(r.id));
                                    }
                                    if menu_item(ui, "Redownload", !r.active) {
                                        acts.push(Act::Redownload(r.id));
                                    }
                                    ui.separator();
                                    if menu_item(ui, "Resume Download", !r.done && !r.active) {
                                        acts.push(Act::Resume(r.id));
                                    }
                                    if menu_item(ui, "Stop Download", r.active) {
                                        acts.push(Act::Stop(r.id));
                                    }
                                    ui.separator();
                                    if menu_item(ui, "Remove", true) {
                                        acts.push(Act::DeleteSel);
                                    }
                                    ui.separator();
                                    ui.add_enabled_ui(!r.done, |ui| {
                                        ui.menu_button("Add to queue", |ui| {
                                            if menu_item(ui, "Main download queue", !r.queued) {
                                                acts.push(Act::Queue(true));
                                            }
                                        });
                                    });
                                    if menu_item(ui, "Delete from queue", r.queued) {
                                        acts.push(Act::Queue(false));
                                    }
                                    ui.separator();
                                    ui.menu_button("On Double click", |ui| {
                                        for (mode, label) in [
                                            (DoubleClick::Auto, "Open file, or show progress"),
                                            (DoubleClick::Open, "Open file"),
                                            (DoubleClick::Folder, "Open folder"),
                                            (DoubleClick::Progress, "Show progress / details"),
                                        ] {
                                            if ui.radio(dbl == mode, label).clicked() {
                                                acts.push(Act::SetDoubleClick(mode));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                    ui.separator();
                                    if menu_item(ui, "Properties", true) {
                                        acts.push(Act::Properties(r.id));
                                    }
                                });
                            });
                        });
                });
            });

        // drag-select: while the button is held, the selection is the starting selection (kept with
        // Ctrl/Shift) plus every visible row between where the drag began and the pointer
        if let Some((start_y, base)) = ctx.data(|d| d.get_temp::<(f32, Vec<u64>)>(drag_id)) {
            if ctx.input(|i| i.pointer.primary_down()) {
                if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
                    let (lo, hi) = if p.y < start_y { (p.y, start_y) } else { (start_y, p.y) };
                    let mut sel = base;
                    for (id, rect) in &row_rects {
                        if rect.bottom() >= lo && rect.top() <= hi && !sel.contains(id) {
                            sel.push(*id);
                        }
                    }
                    self.selected = sel;
                }
            } else {
                ctx.data_mut(|d| d.remove_temp::<(f32, Vec<u64>)>(drag_id));
            }
        }

        // a click on a column header sorts by it; a second click reverses the order
        if let Some(c) = sort_click {
            self.sort = match self.sort {
                Some((k, asc)) if k == c => Some((c, !asc)),
                _ => Some((c, true)),
            };
        }

        for a in acts {
            match a {
                Act::Open(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        let _ = open::that(t.final_path());
                    }
                }
                Act::Folder(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        let _ = open::that(t.info.lock().dir.clone());
                    }
                }
                Act::Resume(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        self.mgr.start(&t);
                    }
                }
                Act::Stop(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        self.mgr.pause(&t);
                    }
                }
                Act::Progress(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        if !self.progresses.iter().any(|p| p.task.id == id) {
                            self.progresses.push(Progress::new(t));
                        }
                    }
                }
                Act::DeleteSel => self.run(ctx, Cmd::Delete),
                Act::OpenWith(id) => {
                    if let (Some(t), Some(app)) = (self.mgr.find(id), rfd::FileDialog::new().set_title("Open with...").pick_file()) {
                        let _ = std::process::Command::new(app).arg(t.final_path()).spawn();
                    }
                }
                Act::MoveRename(id) => {
                    if self.move_dlg.is_none() {
                        if let Some(t) = self.mgr.find(id) {
                            self.move_dlg = Some(MoveDlg::new(&t));
                        }
                    }
                }
                Act::Redownload(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        let name = t.info.lock().file_name.clone();
                        let ok = rfd::MessageDialog::new()
                            .set_level(rfd::MessageLevel::Warning)
                            .set_title("Redownload")
                            .set_description(format!("Delete what was downloaded of \"{name}\" and download it again from the start?"))
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show();
                        if matches!(ok, rfd::MessageDialogResult::Yes) {
                            let fresh = self.mgr.redownload(&t);
                            for s in self.selected.iter_mut().filter(|s| **s == id) {
                                *s = fresh.id;
                            }
                        }
                    }
                }
                Act::Queue(on) => {
                    for t in self.selected_tasks() {
                        if t.info.lock().status != Status::Complete {
                            self.mgr.set_queued(&t, on);
                        }
                    }
                }
                Act::SetDoubleClick(mode) => {
                    self.mgr.settings.lock().double_click = mode;
                    self.mgr.save_settings();
                }
                Act::Properties(id) => {
                    if let Some(t) = self.mgr.find(id) {
                        self.props = Some(PropsDlg { task: t });
                    }
                }
            }
        }
    }

    // ───────────── dialogs ─────────────

    fn dialogs(&mut self, ctx: &Context) {
        if let Some(mut d) = self.add_url.take() {
            match add_url_ui(ctx, &mut d) {
                Out::Keep => self.add_url = Some(d),
                Out::Close => {}
                Out::Done(u) => match normalize_url(&u) {
                    Ok(urls) => self.add_links(ctx, urls, d.video),
                    Err(e) => {
                        d.video = false;
                        d.error = Some(e);
                        self.add_url = Some(d);
                    }
                },
            }
        }

        // pages on Facebook, Instagram, X … : pick a quality from the formats yt-dlp lists
        let mut keep = Vec::new();
        for mut d in std::mem::take(&mut self.video_picks) {
            match video_pick_ui(ctx, &mut d) {
                Out::Keep => keep.push(d),
                Out::Close => {}
                Out::Done(PickOut::PlainLink(url)) => self.add_single(ctx, url, false),
                Out::Done(PickOut::Download(c, name)) => {
                    let key = self.key();
                    let size = c.size + c.audio.as_ref().map_or(0, |a| a.size);
                    let mut conf = Confirm::new(ctx, &self.mgr, key, c.url, c.headers, Some(name), size);
                    conf.audio = c.audio;
                    self.confirms.push(conf);
                }
            }
        }
        self.video_picks = keep;

        let mut keep = Vec::new();
        let mut started = Vec::new();
        for mut d in std::mem::take(&mut self.confirms) {
            match confirm_ui(ctx, &self.mgr, &mut d) {
                Out::Keep => keep.push(d),
                Out::Close => {}
                Out::Done(task) => started.push(Progress::new(task)),
            }
        }
        self.confirms = keep;
        self.progresses.extend(started);

        let mut keep = Vec::new();
        for mut d in std::mem::take(&mut self.progresses) {
            if let Out::Keep = progress_ui(ctx, &self.mgr, &mut d) {
                keep.push(d);
            }
        }
        // a progress window may have been re-added meanwhile (e.g. by an event); keep unique
        for d in keep {
            if !self.progresses.iter().any(|p| p.task.id == d.task.id) {
                self.progresses.push(d);
            }
        }

        let mut keep = Vec::new();
        for mut d in std::mem::take(&mut self.completes) {
            if let Out::Keep = complete_ui(ctx, &self.mgr, &mut d) {
                keep.push(d);
            }
        }
        self.completes = keep;

        if let Some(mut d) = self.options.take() {
            if let Out::Keep = options_ui(ctx, &self.mgr, &mut d) {
                self.options = Some(d);
            }
        }
        if let Some(mut d) = self.move_dlg.take() {
            if let Out::Keep = move_ui(ctx, &self.mgr, &mut d) {
                self.move_dlg = Some(d);
            }
        }
        if let Some(d) = self.props.take() {
            if let Out::Keep = props_ui(ctx, &d) {
                self.props = Some(d);
            }
        }
        if let Some(mut d) = self.scheduler.take() {
            if let Out::Keep = scheduler_ui(ctx, &self.mgr, &mut d) {
                self.scheduler = Some(d);
            }
        }
        if let Some(mut d) = self.delete.take() {
            match delete_ui(ctx, &self.mgr, &mut d) {
                Out::Keep => self.delete = Some(d),
                _ => {
                    let mgr = self.mgr.clone();
                    self.selected.retain(|id| mgr.find(*id).is_some());
                }
            }
        }
        if let Some((title, msg)) = self.notice.clone() {
            if let Out::Close = notice_ui(ctx, &title, &msg) {
                self.notice = None;
            }
        }
        if let Some(d) = self.dup.take() {
            match dup_ui(ctx, &d) {
                Out::Keep => self.dup = Some(d),
                Out::Done(_) => {
                    // add it again: no second duplicate check
                    self.add_single_unchecked(ctx, d.url);
                }
                Out::Close => {}
            }
        }
        if let Some(d) = self.power.take() {
            match power_ui(ctx, &d) {
                Out::Keep => self.power = Some(d),
                Out::Close => {}
                Out::Done(()) => {
                    self.mgr.pause_all();
                    self.mgr.save();
                    self.mgr.save_settings();
                    sys::power(d.action);
                }
            }
        }
        if self.about {
            if let Out::Close = about_ui(ctx, &self.logo, &self.dev) {
                self.about = false;
            }
        }
        if let Some(mut d) = self.extension.take() {
            if let Out::Keep = extension_ui(ctx, &mut d, &self.logo) {
                self.extension = Some(d);
            }
        }
        if let Some(mut d) = self.appearance.take() {
            if let Out::Keep = appearance_ui(ctx, &mut d, &self.mgr, &self.skin_store) {
                self.appearance = Some(d);
            }
        }
        if let Some(mut d) = self.setup.take() {
            let out = setup_ui(ctx, &mut d, &self.toolbox, self.mgr.extension_seen(), &self.logo);
            if std::mem::take(&mut d.changed) {
                self.mgr.settings.lock().show_setup = !d.hide_at_start;
                self.mgr.save_settings();
            }
            if std::mem::take(&mut d.open_extension) && self.extension.is_none() {
                self.extension = Some(ExtensionDlg::new(&self.mgr));
            }
            match out {
                Out::Keep => self.setup = Some(d),
                Out::Done(()) | Out::Close => {}
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        // developer portrait on the About page: refresh from GitHub when the page is open
        self.dev.poll(ctx);
        if self.about {
            self.dev.refresh(ctx, &self.mgr);
        }
        // restore the pinned state saved by the last run
        if !self.pin_sent {
            self.pin_sent = true;
            if self.pinned {
                self.set_level(ctx);
            }
        }
        // tray icon follows the setting; closing the window hides it instead of quitting
        let want_tray = self.mgr.settings.lock().close_to_tray && crate::tray::SUPPORTED;
        if want_tray && self.tray.is_none() {
            self.tray = crate::tray::create(ctx, self.quitting.clone(), self.mgr.clone());
        }
        if let Some(t) = &self.tray {
            t.set_visible(want_tray);
        }
        if ctx.input(|i| i.viewport().close_requested()) && want_tray && self.tray.is_some() && !self.quitting.load(Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let popups = !self.confirms.is_empty()
                || !self.progresses.is_empty()
                || !self.completes.is_empty()
                || crate::widgets::any_dialog_open();
            if popups {
                // Hiding the window would stop this update loop and freeze the open pop-ups
                // (they could no longer be moved, minimised or closed). Park it off-screen
                // instead; it is hidden for real once the last pop-up has been closed.
                crate::tray::park_for_dialogs();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }
        // launched by the OS at login with "Start minimized": go to the tray (or the taskbar)
        if !self.start_checked {
            self.start_checked = true;
            let (hide, tray_on) = {
                let st = self.mgr.settings.lock();
                (self.autostart_launch && st.launch_on_startup && st.start_hidden, st.close_to_tray && crate::tray::SUPPORTED)
            };
            if hide {
                if tray_on && self.tray.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
        }
        // First run / something missing: offer the setup window once the tools have been looked for
        if self.setup_check == 0 && !self.toolbox.is_busy() {
            self.setup_check = 2;
            let (wanted, hidden_start) = {
                let st = self.mgr.settings.lock();
                (st.show_setup, self.autostart_launch && st.launch_on_startup && st.start_hidden)
            };
            let tool_missing = self.toolbox.snapshot().iter().any(|(t, e)| e.installed.is_none() && t.source().is_some());
            if wanted && !hidden_start && (tool_missing || !self.mgr.extension_seen()) {
                crate::tray::wake_for_dialog();
                self.setup = Some(SetupDlg::new(&self.mgr));
            }
        }
        self.process_events(ctx);
        self.background_features(ctx);
        // links sent by the browser extension -> "Download File Info"
        let incoming = self.mgr.take_incoming();
        if !incoming.is_empty() {
            // Keep the loop running for the dialog, but never show or raise the main window.
            crate::tray::wake_for_dialog();
        }
        for inc in incoming {
            let key = self.key();
            let mut c = Confirm::new(ctx, &self.mgr, key, inc.url, inc.headers, inc.file_name, inc.size + inc.audio.as_ref().map_or(0, |a| a.size));
            c.audio = inc.audio;
            self.confirms.push(c);
        }

        // yt-dlp, Deno or ffmpeg is missing for a video the browser extension asked about: open Tools
        if self.mgr.take_tools_request() {
            crate::tray::wake_for_dialog();
            if self.setup.is_none() {
                self.toolbox.refresh();
                self.setup = Some(SetupDlg::new(&self.mgr));
            }
        }

        let mut cmd = None;
        self.menu_bar(ctx, &mut cmd);
        self.toolbar(ctx, &mut cmd);
        if self.show_sidebar {
            self.sidebar(ctx);
        }
        self.table(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && cmd.is_none() {
            cmd = Some(Cmd::Delete);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::M)) && cmd.is_none() {
            cmd = Some(Cmd::MoveRename);
        }
        if let Some(c) = cmd {
            self.run(ctx, c);
        }
        self.dialogs(ctx);
        // the Extension page's logo was grabbed: hand the zip to the OS drag-and-drop
        if let Some(d) = self.extension.as_mut() {
            if std::mem::take(&mut d.drag) {
                if let Some(path) = d.path.clone() {
                    if let Err(e) = crate::extension::start_drag(frame, &path) {
                        d.message = Some((true, e));
                    }
                }
            }
        }

        // the logo of the setup window was grabbed: same thing
        if let Some(d) = self.setup.as_mut() {
            if std::mem::take(&mut d.drag) {
                if let Some(path) = d.path.clone() {
                    d.message = crate::extension::start_drag(frame, &path).err();
                }
            }
        }

        // the drag handle of a "Download complete" pop-up was grabbed: drag the downloaded file itself out
        for d in self.completes.iter_mut() {
            if std::mem::take(&mut d.drag) {
                if let Err(e) = crate::extension::start_drag(frame, &d.path) {
                    eprintln!("could not start dragging {}: {e}", d.path.display());
                }
            }
        }

        crate::titlebar::edges(ctx);
        crate::titlebar::round_windows(); // Windows 11: rounded corners for the main window and the pop-ups
        let dialogs_open = crate::widgets::finish_dialog_frame(ctx);

        // the hidden window was only parked off-screen for a browser pop-up: tuck it back into the tray,
        // but only after the pop-ups have really been closed and a few frames have passed
        let woken = crate::tray::WOKEN_HIDDEN.load(Relaxed);
        if woken {
            if self.confirms.is_empty() && self.progresses.is_empty() && self.completes.is_empty() && !dialogs_open {
                self.rehide_frames += 1;
                if self.rehide_frames >= 6 {
                    self.rehide_frames = 0;
                    // the window was parked off-screen only to keep the loop alive: hide it again
                    crate::tray::rehide();
                }
            } else {
                self.rehide_frames = 0;
            }
        }

        let busy = self.mgr.any_active() || !self.confirms.is_empty() || !self.video_picks.is_empty();
        // pop-ups are drawn by this loop, so keep it quick while one is open or being torn down
        let ms = if dialogs_open || woken || !self.confirms.is_empty() || !self.video_picks.is_empty() { 33 } else if busy { 100 } else { 500 };
        ctx.request_repaint_after(Duration::from_millis(ms));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.mgr.pause_all();
        self.mgr.save();
        self.mgr.save_settings();
    }
}

/// One address per line. Anything that is not http(s) is reported instead of silently dropped.
fn normalize_url(u: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for line in u.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let full = if line.contains("://") { line.to_string() } else { format!("https://{line}") };
        if !(full.starts_with("http://") || full.starts_with("https://")) {
            return Err(format!("Only http:// and https:// addresses are supported: {line}"));
        }
        if reqwest_url_invalid(&full) {
            return Err(format!("Not a valid address: {line}"));
        }
        if !out.contains(&full) {
            out.push(full);
        }
    }
    if out.is_empty() {
        return Err("Enter an address first.".into());
    }
    Ok(out)
}

fn reqwest_url_invalid(u: &str) -> bool {
    // host must exist and contain no spaces; the engine does the strict parsing
    let rest = u.split("://").nth(1).unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.is_empty() || host.contains(' ')
}

/// Menu-bar title with its first letter underlined, like the Windows access keys in IDM.
fn mnemonic(label: &str) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let fmt = TextFormat {
        font_id: FontId::proportional(ui_size()),
        color: pal().text,
        valign: egui::Align::Center, // keep the text in the middle of the bar
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    // frameless menu-bar buttons have no padding of their own, so it is part of the title
    job.append(label, 7.0, fmt.clone());
    job.append(" ", 0.0, fmt);
    job
}

/// One toolbar button: 70 px wide, icon 24 px below the top edge, label 54 px below it. The queue
/// buttons are 16 px wider and carry a small drop-down arrow; its response is returned as well.
fn tool_button(ui: &mut Ui, icon: Tool, label: &str, tip: &str, enabled: bool, dropdown: bool) -> (bool, Option<Response>) {
    let w = if dropdown { 86.0 } else { 70.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(w, 70.0), Sense::hover());
    let main = Rect::from_min_size(rect.min, vec2(70.0, 70.0));
    let resp = ui.interact(main, ui.id().with((label, "main")), Sense::click());
    let p = ui.painter();
    let hover = resp.hovered();
    if hover && enabled && !icons::skin_has_hot() {
        p.rect_filled(main.shrink2(vec2(2.0, 4.0)), Rounding::same(3.0), pal().hover_tint);
    }
    icons::tool(ui.ctx(), p, pos2(main.center().x, main.top() + 24.0), icon, enabled, hover);
    p.text(
        pos2(main.center().x, main.top() + 54.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(ui_size()),
        if enabled { pal().text } else { pal().disabled },
    );
    let clicked = resp.on_hover_text(tip).clicked() && enabled;
    let arrow = dropdown.then(|| {
        let ar = Rect::from_min_size(pos2(main.right(), main.top()), vec2(16.0, 70.0));
        let r = ui.interact(ar, ui.id().with((label, "arrow")), Sense::click());
        let c = pos2(ar.center().x - 1.0, ar.top() + 32.0);
        ui.painter().add(Shape::Path(PathShape::convex_polygon(
            vec![c + vec2(-3.5, -1.5), c + vec2(3.5, -1.5), c + vec2(0.0, 2.5)],
            pal().disabled,
            Stroke::NONE,
        )));
        r
    });
    (clicked, arrow)
}

/// The pop-up behind the arrow of "Start Qu..." / "Stop Qu...".
fn queue_dropdown(ui: &mut Ui, arrow: Response, id: &str, cmd: &mut Option<Cmd>, c: Cmd, enabled: bool) {
    let pid = ui.make_persistent_id(id);
    if arrow.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    egui::popup::popup_below_widget(ui, pid, &arrow, |ui| {
        ui.set_min_width(170.0);
        if ui.add_enabled(enabled, egui::Button::new("Main download queue").frame(false)).clicked() {
            *cmd = Some(c);
            ui.memory_mut(|m| m.close_popup());
        }
        ui.add_enabled(false, egui::Button::new("Synchronization queue").frame(false));
    });
}

/// Grid lines of one list cell: a 1px line on the right and bottom edge, or the blue selection
/// frame (top and bottom on every cell, left on the first one, right on the last one).
fn grid(ui: &Ui, selected: bool, first: bool, last: bool) {
    let r = ui.max_rect();
    let p = ui.painter();
    if selected {
        let s = Stroke::new(1.0_f32, pal().row_sel_border);
        p.line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], s);
        p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], s);
        if first {
            p.line_segment([pos2(r.left() + 0.5, r.top()), pos2(r.left() + 0.5, r.bottom())], s);
        }
        if last {
            p.line_segment([pos2(r.right() - 0.5, r.top()), pos2(r.right() - 0.5, r.bottom())], s);
        }
    } else {
        let s = Stroke::new(1.0_f32, pal().grid);
        p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], s);
        if !last {
            p.line_segment([pos2(r.right() - 0.5, r.top()), pos2(r.right() - 0.5, r.bottom())], s);
        }
    }
}

fn cell_text(ui: &mut Ui, s: &str, pad: f32) {
    ui.add_space(pad);
    ui.add(egui::Label::new(RichText::new(s).size(ui_size()).color(pal().text)).truncate(true).selectable(false));
}

struct TreeHit {
    clicked: bool,
    toggled: bool,
    /// vertical centre of the row
    y: f32,
    /// horizontal centre and left edge of the icon (anchors for the dotted lines)
    icon_x: f32,
    icon_left: f32,
}

/// One row of the category tree (16 px high, like the Windows tree view).
fn tree_row(ui: &mut Ui, depth: usize, label: &str, chevron: Option<bool>, selected: bool, icon: impl FnOnce(&Painter, Pos2)) -> TreeHit {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
    let p = ui.painter();
    let x0 = rect.left();
    let cy = rect.center().y;
    let icon_x = x0 + if depth == 0 { 30.0 } else { 47.0 };
    let text_x = icon_x + 14.0;
    let galley = p.layout_no_wrap(label.to_string(), FontId::proportional(ui_size()), pal().text);
    let hug = Rect::from_min_max(pos2(icon_x - 10.0, rect.top()), pos2(text_x + galley.size().x + 3.0, rect.bottom()));
    if selected {
        p.rect_filled(hug, Rounding::ZERO, pal().tree_sel);
    } else if resp.hovered() {
        p.rect_filled(hug, Rounding::ZERO, pal().hover_tint);
    }
    let mut toggled = false;
    if let Some(o) = chevron {
        let c = pos2(x0 + 10.0, cy);
        let pts = if o {
            vec![c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)]
        } else {
            vec![c + vec2(-1.5, -3.5), c + vec2(2.0, 0.0), c + vec2(-1.5, 3.5)]
        };
        p.add(Shape::line(pts, Stroke::new(1.2_f32, pal().glyph)));
        if resp.clicked() {
            toggled = resp.interact_pointer_pos().map_or(false, |pp| pp.x < x0 + 20.0);
        }
    }
    icon(p, pos2(icon_x, cy));
    p.galley(pos2(text_x, cy - galley.size().y / 2.0), galley, pal().text);
    TreeHit { clicked: resp.clicked() && !toggled, toggled, y: cy, icon_x, icon_left: icon_x - 8.0 }
}
