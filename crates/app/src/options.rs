//! The Options window: a side navigation with one page per topic, an *Apply* button and a
//! *Restore defaults* button for the page you are looking at. Everything is edited on a copy of the
//! settings and only written when OK / Apply is pressed.
use crate::{
    dialogs::Out,
    sys::{self, SoundKind},
    theme::*,
    widgets::*,
};
use downcraft_core::{
    util, AfterQueue, Category, ConnRule, DuplicateMode, IpMode, Manager, ProxyMode, Settings, SiteLogin, SoundSlot,
    DEFAULT_CLIPBOARD_EXTS, DEFAULT_CLIPBOARD_SKIP, DEFAULT_PROXY_BYPASS,
};
use eframe::egui::{self, vec2, Align, Align2, FontId, Layout, Response, RichText, Rounding, Sense, Ui};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const NAV_W: f32 = 148.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    FileTypes,
    SaveTo,
    Downloads,
    Connection,
    Proxy,
    Logins,
    Network,
    Sounds,
}

const TABS: [(Tab, &str, &str); 9] = [
    (Tab::General, "General", "Startup, tray, clipboard and what happens when the queue ends"),
    (Tab::FileTypes, "File types", "Which copied links the clipboard monitor offers"),
    (Tab::SaveTo, "Save to", "Folders for each category and how finished files are stamped"),
    (Tab::Downloads, "Downloads", "Dialogs, duplicates, identification and what runs afterwards"),
    (Tab::Connection, "Connection", "Connections, speed limits, retries and the download quota"),
    (Tab::Proxy, "Proxy / Socks", "Send downloads through a proxy server"),
    (Tab::Logins, "Site logins", "User names and passwords for sites that ask for them"),
    (Tab::Network, "Network", "IP version, outgoing address, timeouts and security"),
    (Tab::Sounds, "Sounds", "Play a sound when something finishes or fails"),
];

enum ConnTest {
    Idle,
    Running,
    Done(Result<String, String>),
}

pub struct OptionsDlg {
    s: Settings,
    tab: Tab,
    root: String,
    cat_dirs: Vec<(Category, String)>,
    error: Option<String>,
    /// Message in the footer after Apply ("Saved").
    applied: bool,
    autostart_was: bool,
    new_rule: (String, u32),
    /// `(index being edited, None = new entry)` and the form values.
    login_form: Option<(Option<usize>, SiteLogin)>,
    login_sel: Option<usize>,
    show_pass: bool,
    test: Arc<Mutex<ConnTest>>,
    clip_try: String,
}

impl OptionsDlg {
    pub fn new(mgr: &Manager) -> Self {
        let s = mgr.settings.lock().clone();
        OptionsDlg {
            root: s.download_root.to_string_lossy().into_owned(),
            cat_dirs: Category::ALL
                .iter()
                .map(|c| (*c, s.remembered.get(c).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()))
                .collect(),
            autostart_was: s.launch_on_startup,
            s,
            tab: Tab::General,
            error: None,
            applied: false,
            new_rule: (String::new(), 2),
            login_form: None,
            login_sel: None,
            show_pass: false,
            test: Arc::new(Mutex::new(ConnTest::Idle)),
            clip_try: String::new(),
        }
    }

    /// Checks the draft and writes it to the live settings. Settings that are changed elsewhere
    /// while the window is open (theme, accent, the window pin) are kept as they are now.
    fn commit(&mut self, mgr: &Arc<Manager>) -> Result<(), String> {
        let root = self.root.trim();
        if root.is_empty() {
            return Err("Choose a default download folder (Save to).".into());
        }
        if matches!(self.s.proxy_mode, ProxyMode::Http | ProxyMode::Socks5) && self.s.proxy_addr.trim().is_empty() {
            return Err("Enter the proxy address as host:port (Proxy / Socks).".into());
        }
        let bind = self.s.bind_addr.trim();
        if !bind.is_empty() && bind.parse::<std::net::IpAddr>().is_err() {
            return Err("The outgoing address must be an IP address such as 10.8.0.2 (Network).".into());
        }
        if self.s.quota_on && (self.s.quota_mb == 0 || self.s.quota_hours == 0) {
            return Err("The download quota needs a size and a number of hours (Connection).".into());
        }
        if self.s.post_command_on && self.s.post_command.trim().is_empty() {
            return Err("Enter the command to run after downloads, or switch it off (Downloads).".into());
        }
        let mut s = self.s.clone();
        s.download_root = PathBuf::from(root);
        s.remembered = self
            .cat_dirs
            .iter()
            .filter(|(_, t)| !t.trim().is_empty())
            .map(|(c, t)| (*c, PathBuf::from(t.trim())))
            .collect();
        s.proxy_addr = s.proxy_addr.trim().to_string();
        s.bind_addr = bind.to_string();
        s.user_agent = s.user_agent.trim().to_string();
        s.post_command = s.post_command.trim().to_string();
        s.conn_rules.retain(|r| !r.host.trim().is_empty());
        {
            let live = mgr.settings.lock();
            s.theme = live.theme.clone();
            s.accent = live.accent.clone();
            s.pin_main = live.pin_main;
        }
        *mgr.settings.lock() = s;
        mgr.apply_settings();
        mgr.save_settings();
        Ok(())
    }

    fn reset_page(&mut self) {
        let d = Settings::default();
        let s = &mut self.s;
        match self.tab {
            Tab::General => {
                s.launch_on_startup = d.launch_on_startup;
                s.start_hidden = d.start_hidden;
                s.close_to_tray = d.close_to_tray;
                s.clipboard_monitor = d.clipboard_monitor;
                s.after_queue = d.after_queue;
            }
            Tab::FileTypes => {
                s.clipboard_exts = d.clipboard_exts;
                s.clipboard_skip_hosts = d.clipboard_skip_hosts;
            }
            Tab::SaveTo => {
                self.root = d.download_root.to_string_lossy().into_owned();
                for (_, t) in self.cat_dirs.iter_mut() {
                    t.clear();
                }
                s.set_file_date = d.set_file_date;
                s.remember_folder_default = d.remember_folder_default;
            }
            Tab::Downloads => {
                s.show_start_dialog = d.show_start_dialog;
                s.show_complete_dialog = d.show_complete_dialog;
                s.duplicate_mode = d.duplicate_mode;
                s.user_agent = d.user_agent;
                s.ignore_modified_on_resume = d.ignore_modified_on_resume;
                s.post_command_on = d.post_command_on;
                s.post_command = d.post_command;
            }
            Tab::Connection => {
                s.connections = d.connections;
                s.conn_rules = d.conn_rules;
                s.speed_limit_kbps = d.speed_limit_kbps;
                s.global_speed_limit_kbps = d.global_speed_limit_kbps;
                s.auto_retries = d.auto_retries;
                s.quota_on = d.quota_on;
                s.quota_mb = d.quota_mb;
                s.quota_hours = d.quota_hours;
                s.quota_warn = d.quota_warn;
            }
            Tab::Proxy => {
                s.proxy_mode = d.proxy_mode;
                s.proxy_addr = d.proxy_addr;
                s.proxy_user = d.proxy_user;
                s.proxy_pass = d.proxy_pass;
                s.proxy_bypass = d.proxy_bypass;
            }
            Tab::Logins => {} // never wipes saved passwords in one click
            Tab::Network => {
                s.ip_mode = d.ip_mode;
                s.bind_addr = d.bind_addr;
                s.connect_timeout_secs = d.connect_timeout_secs;
                s.stall_timeout_secs = d.stall_timeout_secs;
                s.accept_invalid_certs = d.accept_invalid_certs;
            }
            Tab::Sounds => s.sounds = d.sounds,
        }
    }
}

// ───────────────────────────── small building blocks ─────────────────────────────

fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, Rounding::same(7.0), pal().card_hot);
        let bar = egui::Rect::from_center_size(rect.left_center() + vec2(4.0, 0.0), vec2(3.0, 18.0));
        p.rect_filled(bar, Rounding::same(2.0), pal().accent);
    } else if resp.hovered() {
        p.rect_filled(rect, Rounding::same(7.0), pal().chip_bg);
    }
    let col = if selected { pal().strong } else { pal().text2 };
    p.text(rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), col);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A titled group of controls.
fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(8.0);
    ui.label(RichText::new(title).size(11.5).strong().color(pal().muted));
    ui.add_space(1.0);
    card(ui, add)
}

fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal().muted)).wrap(true));
}

fn warn_hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal().warn)).wrap(true));
}

/// Label on the left, control on the right, both centred on one line.
fn line(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([210.0, FIELD_H], egui::Label::new(RichText::new(label).color(pal().text2)).wrap(false));
        add(ui);
    });
}

fn text_field(ui: &mut Ui, text: &mut String, width: f32, hint_text: &str, password: bool) -> Response {
    ui.add_sized(
        [width.max(40.0), FIELD_H],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint_text).color(pal().muted))
            .password(password)
            .margin(vec2(10.0, 7.0))
            .vertical_align(Align::Center),
    )
}

fn small_button(ui: &mut Ui, label: &str) -> Response {
    ui.add(egui::Button::new(label).min_size(vec2(0.0, FIELD_H)))
}

/// One row of a table: full-width hover/selection background with text in fixed columns
/// (a width of 0 takes whatever is left).
fn table_row(ui: &mut Ui, selected: bool, cols: &[(&str, f32)]) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, Rounding::same(5.0), pal().row_sel);
    } else if resp.hovered() {
        p.rect_filled(rect, Rounding::same(5.0), pal().chip_bg);
    }
    let mut x = rect.left() + 8.0;
    for (text, w) in cols {
        let w = if *w <= 0.0 { (rect.right() - x - 6.0).max(10.0) } else { *w };
        let clip = egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(w - 6.0, rect.height()));
        ui.painter().with_clip_rect(clip).text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            *text,
            FontId::proportional(13.0),
            pal().text2,
        );
        x += w;
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn table_head(ui: &mut Ui, cols: &[(&str, f32)]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let mut x = rect.left() + 8.0;
    for (text, w) in cols {
        let w = if *w <= 0.0 { (rect.right() - x).max(10.0) } else { *w };
        ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, *text, FontId::proportional(11.5), pal().muted);
        x += w;
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0_f32, pal().card_stroke));
}

fn pick_folder_into(target: &mut String) {
    let mut dlg = rfd::FileDialog::new();
    if !target.trim().is_empty() {
        dlg = dlg.set_directory(target.trim());
    }
    if let Some(p) = dlg.pick_folder() {
        *target = p.to_string_lossy().into_owned();
    }
}

fn cat_color_hint(c: Category) -> &'static str {
    match c {
        Category::Compressed => "zip, rar, 7z, iso …",
        Category::Documents => "everything else",
        Category::Music => "mp3, flac, wav …",
        Category::Programs => "exe, msi, dmg, deb …",
        Category::Video => "mp4, mkv, avi …",
    }
}

// ───────────────────────────── the window ─────────────────────────────

pub fn options_ui(ctx: &egui::Context, mgr: &Arc<Manager>, d: &mut OptionsDlg) -> Out<()> {
    let mut out = Out::Keep;
    let closed = show_dialog_bare(ctx, "options", "Options", [800.0, 650.0], false, |ui| {
        heading(ui, "Options", "Downloads, connections, network and notifications", Badge::None);

        let err_h = if d.error.is_some() { 24.0 } else { 0.0 };
        let body_h = (ui.available_height() - 46.0 - err_h).max(140.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), body_h), Layout::left_to_right(Align::Min), |ui| {
            ui.allocate_ui_with_layout(vec2(NAV_W, body_h), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (tab, name, _) in TABS {
                    if nav_item(ui, name, d.tab == tab).clicked() {
                        d.tab = tab;
                    }
                }
            });
            ui.add_space(6.0);
            let (line_rect, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
            ui.painter().rect_filled(line_rect, Rounding::ZERO, pal().card_stroke);
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), body_h), Layout::top_down(Align::Min), |ui| {
                let (_, name, sub) = TABS.iter().find(|t| t.0 == d.tab).copied().unwrap_or(TABS[0]);
                ui.label(RichText::new(name).size(16.0).strong().color(pal().strong));
                ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(pal().muted)).truncate(true));
                ui.add_space(2.0);
                egui::ScrollArea::vertical().id_source(("opt-page", d.tab as u8)).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match d.tab {
                        Tab::General => d.page_general(ui),
                        Tab::FileTypes => d.page_file_types(ui),
                        Tab::SaveTo => d.page_save_to(ui),
                        Tab::Downloads => d.page_downloads(ui),
                        Tab::Connection => d.page_connection(ui, mgr),
                        Tab::Proxy => d.page_proxy(ui),
                        Tab::Logins => d.page_logins(ui),
                        Tab::Network => d.page_network(ui),
                        Tab::Sounds => d.page_sounds(ui),
                    }
                    ui.add_space(6.0);
                });
            });
        });

        if let Some(e) = &d.error {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(e).color(pal().bad)).truncate(true)).on_hover_text(e);
        } else if d.applied {
            ui.add_space(4.0);
            ui.colored_label(pal().good, "Saved.");
        }

        pin_bottom(ui, 30.0);
        let mut act = None;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(d.tab != Tab::Logins, |ui| {
                if action_button(ui, "Restore page defaults", false).clicked() {
                    act = Some(3);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if action_button(ui, "OK", true).clicked() {
                    act = Some(2);
                }
                if action_button(ui, "Apply", false).clicked() {
                    act = Some(1);
                }
                if action_button(ui, "Cancel", false).clicked() {
                    act = Some(0);
                }
            });
        });
        match act {
            Some(0) => out = Out::Close,
            Some(3) => {
                d.reset_page();
                d.applied = false;
            }
            Some(a @ (1 | 2)) => match d.commit(mgr) {
                Ok(()) => {
                    d.error = None;
                    d.applied = true;
                    // start-at-login is an OS setting: only touch it when the box was changed
                    if d.s.launch_on_startup != d.autostart_was {
                        match sys::set_autostart(d.s.launch_on_startup) {
                            Ok(()) => d.autostart_was = d.s.launch_on_startup,
                            Err(e) => {
                                d.s.launch_on_startup = d.autostart_was;
                                mgr.settings.lock().launch_on_startup = d.autostart_was;
                                mgr.save_settings();
                                d.error = Some(format!("Could not change the start-at-login entry: {e}"));
                                d.applied = false;
                            }
                        }
                    }
                    if a == 2 && d.error.is_none() {
                        out = Out::Close;
                    }
                }
                Err(e) => {
                    d.error = Some(e);
                    d.applied = false;
                }
            },
            _ => {}
        }
    });
    if closed {
        Out::Close
    } else {
        out
    }
}

// ───────────────────────────── pages ─────────────────────────────

impl OptionsDlg {
    fn page_general(&mut self, ui: &mut Ui) {
        let tray_ok = crate::tray::SUPPORTED;
        section(ui, "STARTUP", |ui| {
            check_row(ui, &mut self.s.launch_on_startup, "Start DownCraft when I log in");
            ui.add_enabled_ui(self.s.launch_on_startup, |ui| {
                check_row(ui, &mut self.s.start_hidden, "Start minimized when launched that way");
            });
            hint(ui, "Downloads that were running are listed as stopped; use the queue or the scheduler to continue them automatically.");
        });
        section(ui, "WINDOW", |ui| {
            ui.add_enabled_ui(tray_ok, |ui| {
                check_row(ui, &mut self.s.close_to_tray, "Keep running in the system tray when the window is closed");
            })
            .response
            .on_disabled_hover_text("The system tray is available on Windows and macOS");
            if !tray_ok {
                hint(ui, "The system tray is not available on this platform; closing the window quits DownCraft.");
            }
        });
        section(ui, "CLIPBOARD MONITOR", |ui| {
            check_row(ui, &mut self.s.clipboard_monitor, "Offer to download links I copy");
            hint(ui, "When you copy a link to one of the file types on the next page, the Download File Info window opens. Links that are already in the list are ignored.");
        });
        section(ui, "WHEN THE MAIN QUEUE FINISHES", |ui| {
            line(ui, "Then", |ui| {
                let label = |a: AfterQueue| match a {
                    AfterQueue::Nothing => "Do nothing",
                    AfterQueue::CloseApp => "Close DownCraft",
                    AfterQueue::Sleep => "Put the computer to sleep",
                    AfterQueue::Shutdown => "Shut down the computer",
                };
                egui::ComboBox::from_id_source("after-queue").width(240.0).selected_text(label(self.s.after_queue)).show_ui(ui, |ui| {
                    for a in [AfterQueue::Nothing, AfterQueue::CloseApp, AfterQueue::Sleep, AfterQueue::Shutdown] {
                        ui.selectable_value(&mut self.s.after_queue, a, label(a));
                    }
                });
            });
            hint(ui, "Sleep and shut down show a 60-second countdown with a Cancel button first. Nothing happens while another download is still running.");
        });
    }

    fn page_file_types(&mut self, ui: &mut Ui) {
        section(ui, "OFFER THESE FILE TYPES", |ui| {
            hint(ui, "Extensions separated by spaces. * and ? work as wildcards (R0* catches .r00 to .r09).");
            input_multiline(ui, &mut self.s.clipboard_exts, 5);
            if small_button(ui, "Default").clicked() {
                self.s.clipboard_exts = DEFAULT_CLIPBOARD_EXTS.into();
            }
        });
        section(ui, "NEVER OFFER LINKS FROM THESE SITES", |ui| {
            hint(ui, "Host names separated by spaces; example.com also covers its sub-domains.");
            input_multiline(ui, &mut self.s.clipboard_skip_hosts, 3);
            if small_button(ui, "Default").clicked() {
                self.s.clipboard_skip_hosts = DEFAULT_CLIPBOARD_SKIP.into();
            }
        });
        section(ui, "TRY A LINK", |ui| {
            let w = ui.available_width();
            text_field(ui, &mut self.clip_try, w, "Paste a link to see whether it would be offered", false);
            let t = self.clip_try.trim();
            if !t.is_empty() {
                if self.s.wants_clipboard_url(t) {
                    ui.colored_label(pal().good, "Would be offered");
                } else {
                    ui.colored_label(pal().muted, "Would be ignored (not a link to a listed file type, or the site is excluded)");
                }
            }
        });
    }

    fn page_save_to(&mut self, ui: &mut Ui) {
        section(ui, "DEFAULT DOWNLOAD FOLDER", |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 96.0;
                text_field(ui, &mut self.root, w, "", false);
                if small_button(ui, "Browse…").clicked() {
                    pick_folder_into(&mut self.root);
                }
            });
            hint(ui, "Each category gets its own sub-folder here unless you choose another folder below.");
        });
        section(ui, "FOLDER FOR EACH CATEGORY", |ui| {
            let root = PathBuf::from(self.root.trim());
            for (cat, text) in self.cat_dirs.iter_mut() {
                ui.horizontal(|ui| {
                    ui.add_sized([86.0, FIELD_H], egui::Label::new(RichText::new(cat.name()).color(pal().text2)).wrap(false))
                        .on_hover_text(cat_color_hint(*cat));
                    let w = ui.available_width() - 170.0;
                    let default = root.join(cat.name()).to_string_lossy().into_owned();
                    text_field(ui, text, w, &default, false);
                    if small_button(ui, "Browse…").clicked() {
                        pick_folder_into(text);
                    }
                    if ui.add_enabled(!text.is_empty(), egui::Button::new("Reset").min_size(vec2(0.0, FIELD_H))).clicked() {
                        text.clear();
                    }
                });
            }
            hint(ui, "An empty field uses the default folder shown in grey.");
        });
        section(ui, "FILES", |ui| {
            check_row(ui, &mut self.s.remember_folder_default, "Tick \"Remember this folder\" in Download File Info by default");
            check_row(ui, &mut self.s.set_file_date, "Give finished files the date reported by the server");
            hint(ui, "The server's Last-Modified date is used when it sends one; otherwise the file keeps today's date.");
        });
    }

    fn page_downloads(&mut self, ui: &mut Ui) {
        section(ui, "DIALOGS", |ui| {
            check_row(ui, &mut self.s.show_start_dialog, "Show Download File Info for links I add with Add URL");
            hint(ui, "Off: the download starts at once into its category folder. Links from the browser extension always show the dialog.");
            check_row(ui, &mut self.s.show_complete_dialog, "Show the Download complete window");
        });
        section(ui, "LINKS ALREADY IN THE LIST", |ui| {
            let label = |m: DuplicateMode| match m {
                DuplicateMode::Ask => "Show a dialog and ask what to do",
                DuplicateMode::AddAnyway => "Add it again (the file gets a new name)",
                DuplicateMode::Skip => "Skip it",
            };
            egui::ComboBox::from_id_source("dup-mode").width(320.0).selected_text(label(self.s.duplicate_mode)).show_ui(ui, |ui| {
                for m in [DuplicateMode::Ask, DuplicateMode::AddAnyway, DuplicateMode::Skip] {
                    ui.selectable_value(&mut self.s.duplicate_mode, m, label(m));
                }
            });
        });
        section(ui, "VIDEO SITES (YT-DLP)", |ui| {
            hint(ui, "Facebook, Instagram, X, TikTok, Vimeo and over a thousand other sites are downloaded through yt-dlp (install it under Tasks → Tools).");
            let label = |b: &str| -> String {
                let mut c = b.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => "None".to_string(),
                }
            };
            line(ui, "Use the cookies of", |ui| {
                egui::ComboBox::from_id_source("ytdlp-cookies").width(180.0).selected_text(label(&self.s.ytdlp_cookies_browser)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.s.ytdlp_cookies_browser, String::new(), "None");
                    for b in downcraft_core::ytdlp::BROWSERS {
                        ui.selectable_value(&mut self.s.ytdlp_cookies_browser, b.to_string(), label(b));
                    }
                });
            });
            hint(ui, "Choose the browser you are signed in with when a post only plays for logged-in users (private accounts, some Instagram and Facebook videos). The browser extension sends its own cookies, so this is only for links typed into Add URL.");
        });
        section(ui, "IDENTIFICATION", |ui| {
            hint(ui, "User-Agent sent to servers. Leave empty for DownCraft's own. Links from the browser keep the browser's own.");
            ui.horizontal(|ui| {
                let w = ui.available_width() - 80.0;
                text_field(ui, &mut self.s.user_agent, w, concat!("DownCraft/", env!("CARGO_PKG_VERSION")), false);
                if small_button(ui, "Default").clicked() {
                    self.s.user_agent.clear();
                }
            });
        });
        section(ui, "RESUMING", |ui| {
            check_row(ui, &mut self.s.ignore_modified_on_resume, "Ignore file modification time changes when resuming");
            hint(ui, "Some servers change the date on every request. A different size or ETag still restarts the download, so a changed file is never glued together.");
        });
        section(ui, "AFTER EACH DOWNLOAD", |ui| {
            check_row(ui, &mut self.s.post_command_on, "Run a program on the finished file");
            ui.add_enabled_ui(self.s.post_command_on, |ui| {
                let w = ui.available_width();
                text_field(ui, &mut self.s.post_command, w, "program  arguments  %FILE%", false);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Examples:").size(11.5).color(pal().muted));
                    #[cfg(windows)]
                    if small_button(ui, "Windows Defender").clicked() {
                        self.s.post_command = "\"C:\\Program Files\\Windows Defender\\MpCmdRun.exe\" -Scan -ScanType 3 -File \"%FILE%\"".into();
                    }
                    if small_button(ui, "ClamAV").clicked() {
                        self.s.post_command = "clamscan %FILE%".into();
                    }
                });
            });
            hint(ui, "Use it for a virus scan, moving files or notifications. %FILE% becomes the file's full path (it is added at the end if you leave it out). The program is started directly, not through a shell, so use a full path or something on your PATH.");
        });
    }

    fn page_connection(&mut self, ui: &mut Ui, mgr: &Arc<Manager>) {
        section(ui, "CONNECTIONS PER DOWNLOAD", |ui| {
            line(ui, "Default maximum", |ui| {
                ui.add(egui::Slider::new(&mut self.s.connections, 1..=32));
            });
            hint(ui, "Servers that refuse extra connections are detected automatically and get fewer for the next 24 hours.");
        });
        section(ui, "EXCEPTIONS FOR SPECIFIC SERVERS", |ui| {
            let cols = [("Server", 330.0), ("Connections", 0.0)];
            table_head(ui, &cols);
            let mut remove = None;
            if self.s.conn_rules.is_empty() {
                ui.add_space(4.0);
                hint(ui, "No exceptions. Add a server that needs fewer (or more) connections than the default.");
            }
            for (i, r) in self.s.conn_rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_sized([326.0, 26.0], egui::Label::new(RichText::new(&r.host).color(pal().text2)).truncate(true));
                    ui.add(egui::DragValue::new(&mut r.connections).clamp_range(1..=32));
                    if small_button(ui, "Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.s.conn_rules.remove(i);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                text_field(ui, &mut self.new_rule.0, 322.0, "example.com or *.example.com", false);
                ui.add(egui::DragValue::new(&mut self.new_rule.1).clamp_range(1..=32));
                let ok = !self.new_rule.0.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new("Add").min_size(vec2(0.0, FIELD_H))).clicked() {
                    let host = self.new_rule.0.trim().to_ascii_lowercase();
                    self.s.conn_rules.retain(|r| r.host != host);
                    self.s.conn_rules.push(ConnRule { host, connections: self.new_rule.1 });
                    self.new_rule.0.clear();
                }
            });
        });
        section(ui, "SPEED LIMITS", |ui| {
            line(ui, "Each new download", |ui| {
                ui.add(egui::DragValue::new(&mut self.s.speed_limit_kbps).clamp_range(0..=1_000_000).suffix(" KB/s  (0 = no limit)"));
            });
            line(ui, "All downloads together", |ui| {
                ui.add(egui::DragValue::new(&mut self.s.global_speed_limit_kbps).clamp_range(0..=1_000_000).suffix(" KB/s  (0 = no limit)"));
            });
        });
        section(ui, "RETRIES", |ui| {
            line(ui, "Retry after a network error", |ui| {
                ui.add(egui::DragValue::new(&mut self.s.auto_retries).clamp_range(0..=20).suffix(" times"));
            });
            hint(ui, "The wait grows each time: 5 s, 10 s, 20 s … up to two minutes. Errors such as 404 are never retried.");
        });
        section(ui, "DOWNLOAD QUOTA", |ui| {
            check_row(ui, &mut self.s.quota_on, "Limit how much DownCraft downloads");
            ui.add_enabled_ui(self.s.quota_on, |ui| {
                ui.horizontal(|ui| {
                    ui.label("No more than");
                    ui.add(egui::DragValue::new(&mut self.s.quota_mb).clamp_range(1..=100_000_000).suffix(" MB"));
                    ui.label("every");
                    ui.add(egui::DragValue::new(&mut self.s.quota_hours).clamp_range(1..=8760).suffix(" hours"));
                });
                check_row(ui, &mut self.s.quota_warn, "Show a warning when downloads are stopped");
            });
            if let Some(q) = mgr.quota_status() {
                ui.add_space(2.0);
                let frac = if q.limit == 0 { 0.0 } else { q.used as f32 / q.limit as f32 };
                progress_bar(ui, frac, if q.blocked { pal().bad } else { pal().accent });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} of {} used - counter restarts in {}{}",
                            util::fmt_size(q.used),
                            util::fmt_size(q.limit),
                            util::fmt_time(q.resets_in),
                            if q.blocked { " (downloads are paused)" } else { "" }
                        ))
                        .size(11.5)
                        .color(if q.blocked { pal().bad } else { pal().muted }),
                    );
                    if small_button(ui, "Reset counter").clicked() {
                        mgr.reset_quota();
                    }
                });
            }
            hint(ui, "When the amount is reached, running downloads stop and new ones are refused until the period has passed. The usage shown is from the last time you pressed OK or Apply.");
        });
    }

    fn connection_test(&mut self, ui: &mut Ui) {
        let running = matches!(*self.test.lock().unwrap_or_else(|e| e.into_inner()), ConnTest::Running);
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("Test connection").min_size(vec2(0.0, FIELD_H))).clicked() {
                let mut s = self.s.clone();
                s.proxy_addr = s.proxy_addr.trim().to_string();
                s.bind_addr = s.bind_addr.trim().to_string();
                *self.test.lock().unwrap_or_else(|e| e.into_inner()) = ConnTest::Running;
                let (shared, ctx) = (self.test.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    let r = Manager::test_connection(&s);
                    *shared.lock().unwrap_or_else(|e| e.into_inner()) = ConnTest::Done(r);
                    ctx.request_repaint();
                });
            }
            match &*self.test.lock().unwrap_or_else(|e| e.into_inner()) {
                ConnTest::Idle => {}
                ConnTest::Running => {
                    ui.spinner();
                    ui.label(RichText::new("Connecting…").color(pal().muted));
                }
                ConnTest::Done(Ok(m)) => {
                    ui.colored_label(pal().good, m);
                }
                ConnTest::Done(Err(m)) => {
                    ui.add(egui::Label::new(RichText::new(m).color(pal().bad)).truncate(true)).on_hover_text(m);
                }
            }
        });
    }

    fn page_proxy(&mut self, ui: &mut Ui) {
        section(ui, "PROXY", |ui| {
            ui.radio_value(&mut self.s.proxy_mode, ProxyMode::None, "No proxy");
            ui.radio_value(&mut self.s.proxy_mode, ProxyMode::System, "Use the system / environment settings (HTTP_PROXY, HTTPS_PROXY, ALL_PROXY)");
            ui.radio_value(&mut self.s.proxy_mode, ProxyMode::Http, "HTTP proxy");
            ui.radio_value(&mut self.s.proxy_mode, ProxyMode::Socks5, "SOCKS5 proxy (names are resolved by the proxy)");
        });
        let manual = matches!(self.s.proxy_mode, ProxyMode::Http | ProxyMode::Socks5);
        section(ui, "PROXY SERVER", |ui| {
            ui.add_enabled_ui(manual, |ui| {
                line(ui, "Address (host:port)", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut self.s.proxy_addr, w, "127.0.0.1:1080", false);
                });
                line(ui, "User name", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut self.s.proxy_user, w, "optional", false);
                });
                line(ui, "Password", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut self.s.proxy_pass, w, "optional", true);
                });
                line(ui, "Never use it for", |ui| {
                    let w = ui.available_width() - 80.0;
                    text_field(ui, &mut self.s.proxy_bypass, w, "comma-separated hosts", false);
                    if small_button(ui, "Default").clicked() {
                        self.s.proxy_bypass = DEFAULT_PROXY_BYPASS.into();
                    }
                });
            });
            hint(ui, "A user name typed into the address (user:pass@host:port) is used instead of the fields above.");
            if !self.s.proxy_pass.is_empty() {
                warn_hint(ui, "The proxy password is stored unencrypted in settings.json.");
            }
        });
        section(ui, "TEST", |ui| {
            self.connection_test(ui);
            hint(ui, "Fetches a tiny page through the settings on this page and the Network page, without saving them.");
        });
    }

    fn page_logins(&mut self, ui: &mut Ui) {
        section(ui, "SAVED LOGINS", |ui| {
            hint(ui, "Sent as HTTP Basic authentication to the site, and never to other sites a download is redirected to. The most specific entry wins - use host/path to limit one to a folder.");
            let cols = [("Site / path", 280.0), ("User", 150.0), ("Password", 0.0)];
            table_head(ui, &cols);
            egui::ScrollArea::vertical().id_source("login-rows").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
                if self.s.logins.is_empty() {
                    ui.add_space(6.0);
                    hint(ui, "Nothing saved yet.");
                }
                for (i, l) in self.s.logins.iter().enumerate() {
                    let dots = "•".repeat(l.pass.chars().count().min(12));
                    let r = table_row(ui, self.login_sel == Some(i), &[(l.site.as_str(), 280.0), (l.user.as_str(), 150.0), (dots.as_str(), 0.0)]);
                    if r.clicked() {
                        self.login_sel = Some(i);
                    }
                    if r.double_clicked() {
                        self.login_form = Some((Some(i), l.clone()));
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if small_button(ui, "New").clicked() {
                    self.login_form = Some((None, SiteLogin::default()));
                    self.show_pass = false;
                }
                let sel = self.login_sel.filter(|i| *i < self.s.logins.len());
                if ui.add_enabled(sel.is_some(), egui::Button::new("Edit").min_size(vec2(0.0, FIELD_H))).clicked() {
                    if let Some(i) = sel {
                        self.login_form = Some((Some(i), self.s.logins[i].clone()));
                        self.show_pass = false;
                    }
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new("Remove").min_size(vec2(0.0, FIELD_H))).clicked() {
                    if let Some(i) = sel {
                        self.s.logins.remove(i);
                        self.login_sel = None;
                        self.login_form = None;
                    }
                }
            });
            warn_hint(ui, "Passwords are stored unencrypted in settings.json on this computer.");
        });
        let mut done = None;
        if let Some((idx, form)) = self.login_form.as_mut() {
            let editing = idx.is_some();
            section(ui, if editing { "EDIT LOGIN" } else { "NEW LOGIN" }, |ui| {
                line(ui, "Site or path", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut form.site, w, "files.example.com  or  example.com/private", false);
                });
                line(ui, "User name", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut form.user, w, "", false);
                });
                line(ui, "Password", |ui| {
                    let w = ui.available_width();
                    text_field(ui, &mut form.pass, w, "", !self.show_pass);
                });
                check_row(ui, &mut self.show_pass, "Show password");
                let site = form.site.trim().trim_start_matches("http://").trim_start_matches("https://");
                let valid = !site.is_empty() && !form.user.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid, egui::Button::new("Save").min_size(vec2(0.0, FIELD_H))).clicked() {
                        done = Some(true);
                    }
                    if small_button(ui, "Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        }
        match (done, self.login_form.take()) {
            (Some(true), Some((idx, mut f))) => {
                f.site = f.site.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/').to_string();
                f.user = f.user.trim().to_string();
                match idx {
                    Some(i) if i < self.s.logins.len() => self.s.logins[i] = f,
                    _ => {
                        self.s.logins.push(f);
                        self.login_sel = Some(self.s.logins.len() - 1);
                    }
                }
            }
            (None, form) => self.login_form = form,
            _ => {}
        }
    }

    fn page_network(&mut self, ui: &mut Ui) {
        section(ui, "IP VERSION", |ui| {
            ui.radio_value(&mut self.s.ip_mode, IpMode::Auto, "Automatic");
            ui.radio_value(&mut self.s.ip_mode, IpMode::V4, "IPv4 only (helps when a broken IPv6 route makes connections hang)");
            ui.radio_value(&mut self.s.ip_mode, IpMode::V6, "IPv6 only");
        });
        section(ui, "OUTGOING ADDRESS", |ui| {
            let w = ui.available_width();
            text_field(ui, &mut self.s.bind_addr, w, "automatic", false);
            hint(ui, "Send all downloads out of one network adapter by giving its IP address - for example the address of your VPN or a second connection. Leave empty to let the system choose.");
        });
        section(ui, "TIMEOUTS", |ui| {
            line(ui, "Connecting to a server", |ui| {
                ui.add(egui::DragValue::new(&mut self.s.connect_timeout_secs).clamp_range(3..=300).suffix(" seconds"));
            });
            line(ui, "Reconnect after no data for", |ui| {
                ui.add(egui::DragValue::new(&mut self.s.stall_timeout_secs).clamp_range(5..=600).suffix(" seconds"));
            });
            hint(ui, "Raise these on a very slow or unstable connection.");
        });
        section(ui, "SECURITY", |ui| {
            check_row(ui, &mut self.s.accept_invalid_certs, "Accept invalid or self-signed HTTPS certificates");
            if self.s.accept_invalid_certs {
                warn_hint(ui, "Anyone on the network path can then read and alter your downloads. Only use this for a server you run yourself.");
            }
        });
        section(ui, "TEST", |ui| {
            self.connection_test(ui);
        });
    }

    fn page_sounds(&mut self, ui: &mut Ui) {
        let slots: [(&str, SoundKind); 3] = [
            ("Download complete", SoundKind::Complete),
            ("Download failed (after the last retry)", SoundKind::Failed),
            ("Queue finished", SoundKind::QueueDone),
        ];
        for (title, kind) in slots {
            let slot: &mut SoundSlot = match kind {
                SoundKind::Complete => &mut self.s.sounds.complete,
                SoundKind::Failed => &mut self.s.sounds.failed,
                SoundKind::QueueDone => &mut self.s.sounds.queue_done,
            };
            card(ui, |ui| {
                check_row(ui, &mut slot.enabled, title);
                ui.add_enabled_ui(slot.enabled, |ui| {
                    ui.horizontal(|ui| {
                        let shown = if slot.file.is_empty() { "System default sound".to_string() } else { slot.file.clone() };
                        let w = ui.available_width() - 230.0;
                        ui.add_sized(
                            [w.max(60.0), FIELD_H],
                            egui::Label::new(RichText::new(shown).color(if slot.file.is_empty() { pal().muted } else { pal().text2 })).truncate(true),
                        );
                        if small_button(ui, "Browse…").clicked() {
                            let exts: &[&str] = if cfg!(windows) { &["wav"] } else { &["wav", "mp3", "ogg", "oga", "flac", "aiff", "m4a"] };
                            if let Some(p) = rfd::FileDialog::new().add_filter("Sound", exts).pick_file() {
                                slot.file = p.to_string_lossy().into_owned();
                            }
                        }
                        if ui.add_enabled(!slot.file.is_empty(), egui::Button::new("Default").min_size(vec2(0.0, FIELD_H))).clicked() {
                            slot.file.clear();
                        }
                        if small_button(ui, "Play").clicked() {
                            sys::play_sound(kind, &slot.file);
                        }
                    });
                });
            });
        }
        if cfg!(windows) {
            hint(ui, "Windows plays .wav files. Leave a sound on \"System default\" to use the Windows sound for that kind of event.");
        }
    }
}
