use crate::{download, ratelimit::RateLimiter, task::*, util, Category, IpMode, ProxyMode, Probe, Settings, Status};
use parking_lot::Mutex;
use reqwest::blocking::Client;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering::Relaxed},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Completed(u64),
    Failed(u64),
    /// The main queue ran out of downloads to start (not raised when the user stops it).
    QueueFinished,
}

/// Called (from an engine thread - keep it quick) for every event, even while the window is hidden.
pub type EventHook = Arc<dyn Fn(Event) + Send + Sync>;

/// How much of the download quota is used (see *Options > Connection*).
#[derive(Clone, Copy, Debug)]
pub struct QuotaStatus {
    pub used: u64,
    pub limit: u64,
    /// Seconds until the counter starts again from zero.
    pub resets_in: u64,
    pub blocked: bool,
}

pub struct NewDownload {
    pub url: String,
    pub dir: PathBuf,
    pub file_name: String,
    pub category: Category,
    pub description: String,
    pub probe: Option<Probe>,
    pub queued: bool,
    pub start: bool,
    pub headers: Vec<(String, String)>,
}

/// A link sent by the browser extension, waiting for the UI to show *Download File Info*.
pub struct Incoming {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub file_name: Option<String>,
    /// Size the browser already knows (0 = unknown) – lets the dialog show it before the probe returns.
    pub size: u64,
    /// Where the link came from: "media" (video button / sniffed list), "download" (a download the
    /// browser started and the extension took over) or "menu" (right-click).
    pub source: String,
    /// Set when `url` is a video-only link: its sound track, to be downloaded and merged in.
    pub audio: Option<AudioPart>,
}

/// The sound track that goes with a video-only link (YouTube serves the best pictures without sound).
/// The two files are downloaded separately and joined by ffmpeg when both are complete.
#[derive(Clone, Debug)]
pub struct AudioPart {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub size: u64,
    /// File extension of the audio file (`m4a`, `webm`…).
    pub ext: String,
}

pub struct Manager {
    pub tasks: Mutex<Vec<Arc<Task>>>,
    pub settings: Mutex<Settings>,
    pub queue_running: AtomicBool,
    /// When the scheduler last looked at the clock (unix time); start / stop times that fall after
    /// it and not after now fire on the next tick.
    sched_cursor: AtomicI64,
    next_id: AtomicU64,
    client: Mutex<Client>,
    /// Pacing shared by every download (the global speed limit).
    pub global_bucket: RateLimiter,
    global_bps: AtomicU64,
    /// Seconds without a single byte before a connection is declared dead and re-opened.
    pub stall_secs: AtomicU64,
    /// Hosts that refused parallel connections, with the connection count they tolerated.
    host_caps: Mutex<HashMap<String, (u32, i64)>>,
    events: Mutex<Vec<Event>>,
    incoming: Mutex<Vec<Incoming>>,
    repaint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Runs when a link arrives from the browser (the UI uses it to wake a hidden window).
    on_incoming: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The browser extension found a helper program missing: the UI should open its Tools window.
    tools_wanted: AtomicBool,
    /// The browser extension has been heard from in this session (so the settings are written only once).
    ext_seen: AtomicBool,
    /// When that was last asked for (the window is offered at most once a minute).
    tools_asked: Mutex<Option<Instant>>,
    on_event: Mutex<Option<EventHook>>,
    /// Bytes received in the current quota window, when it began (unix time, 0 = not yet), and
    /// whether the quota has been used up.
    quota_used: AtomicU64,
    quota_since: AtomicI64,
    quota_blocked: AtomicBool,
    quota_notice: AtomicBool,
    /// The queue started something since it was last (re)started - so running dry means "finished".
    queue_worked: AtomicBool,
    data_dir: PathBuf,
    save_lock: Mutex<()>,
    /// Video + audio pairs waiting for both halves (see `merge.rs`).
    pub(crate) merges: Mutex<Vec<crate::merge::MergeJob>>,
}

impl Manager {
    /// Uses `./downcraft-data` next to the executable when it exists (portable mode),
    /// otherwise the platform config directory.
    pub fn new() -> Arc<Self> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        // Renamed from FetchCraft: carry the old data folder over once so nothing is lost.
        let migrate = |old: PathBuf, new: &PathBuf| {
            if old.exists() && !new.exists() {
                let _ = std::fs::rename(&old, new);
            }
        };
        let portable = exe_dir.as_ref().map(|d| d.join("downcraft-data"));
        if let (Some(exe), Some(new)) = (&exe_dir, &portable) {
            migrate(exe.join("fetchcraft-data"), new);
        }
        let portable = portable.filter(|d| d.exists());
        let dir = portable.unwrap_or_else(|| {
            match directories::ProjectDirs::from("", "", "downcraft") {
                Some(d) => {
                    let new = d.config_dir().to_path_buf();
                    if let Some(old) = directories::ProjectDirs::from("", "", "fetchcraft") {
                        migrate(old.config_dir().to_path_buf(), &new);
                    }
                    new
                }
                None => PathBuf::from(".downcraft"),
            }
        });
        Self::with_dir(dir)
    }

    pub fn with_dir(data_dir: PathBuf) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&data_dir);
        let settings: Settings = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let records: Vec<Record> = std::fs::read(data_dir.join("downloads.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let next = records.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let tasks = records.into_iter().map(|r| Arc::new(Task::from_record(r))).collect();
        let global_bps = settings.global_speed_limit_kbps.saturating_mul(1024);
        let stall = settings.stall_timeout_secs.clamp(5, 600);
        let client = Self::build_client(&settings);
        let m = Arc::new(Manager {
            tasks: Mutex::new(tasks),
            settings: Mutex::new(settings),
            queue_running: AtomicBool::new(false),
            sched_cursor: AtomicI64::new(util::now_ts()),
            next_id: AtomicU64::new(next),
            client: Mutex::new(client),
            global_bucket: RateLimiter::default(),
            global_bps: AtomicU64::new(global_bps),
            stall_secs: AtomicU64::new(stall),
            host_caps: Mutex::new(HashMap::new()),
            events: Mutex::new(Vec::new()),
            incoming: Mutex::new(Vec::new()),
            repaint: Mutex::new(None),
            on_incoming: Mutex::new(None),
            tools_wanted: AtomicBool::new(false),
            ext_seen: AtomicBool::new(false),
            tools_asked: Mutex::new(None),
            on_event: Mutex::new(None),
            quota_used: AtomicU64::new(0),
            quota_since: AtomicI64::new(0),
            quota_blocked: AtomicBool::new(false),
            quota_notice: AtomicBool::new(false),
            queue_worked: AtomicBool::new(false),
            data_dir,
            save_lock: Mutex::new(()),
            merges: Mutex::new(Vec::new()),
        });
        // The queue, the scheduler and automatic retries must keep running while the window is
        // minimised or hidden (a UI that is not repainting is not calling anything), so they get
        // their own heartbeat. It ends by itself once the manager is dropped.
        let weak = Arc::downgrade(&m);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            match weak.upgrade() {
                Some(m) => m.tick(),
                None => break,
            }
        });
        m
    }

    fn build_client(s: &Settings) -> Client {
        let ua = s.user_agent.trim();
        let ua = if ua.is_empty() { concat!("DownCraft/", env!("CARGO_PKG_VERSION")) } else { ua };
        let mut b = Client::builder()
            .user_agent(ua)
            .connect_timeout(Duration::from_secs(s.connect_timeout_secs.clamp(3, 300)))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .timeout(None);
        // Binding to the unspecified address of one family forces that family; a concrete address
        // sends the traffic out of that adapter (e.g. the VPN).
        match s.ip_mode {
            IpMode::Auto => {}
            IpMode::V4 => b = b.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            IpMode::V6 => b = b.local_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        }
        if let Ok(ip) = s.bind_addr.trim().parse::<IpAddr>() {
            b = b.local_address(ip);
        }
        if s.accept_invalid_certs {
            b = b.danger_accept_invalid_certs(true);
        }
        match s.proxy_mode {
            ProxyMode::None => b = b.no_proxy(),
            ProxyMode::System => {}
            ProxyMode::Http | ProxyMode::Socks5 => {
                if let Some(mut p) = s.proxy_url().and_then(|u| reqwest::Proxy::all(u).ok()) {
                    let bypass = s.proxy_bypass.trim();
                    if !bypass.is_empty() {
                        p = p.no_proxy(reqwest::NoProxy::from_string(bypass));
                    }
                    b = b.proxy(p);
                }
            }
        }
        // A broken proxy string must never leave the app without an HTTP client.
        b.build().unwrap_or_else(|_| Client::new())
    }

    /// Fetches a small page through a client built from `s` (not the live one), so the Options
    /// dialog can test proxy / network settings before they are saved. Returns a one-line result.
    pub fn test_connection(s: &Settings) -> Result<String, String> {
        let client = Self::build_client(s);
        let t0 = std::time::Instant::now();
        let r = client
            .get("https://www.gstatic.com/generate_204")
            .timeout(Duration::from_secs(12))
            .send()
            .map_err(|e| {
                let mut m = e.to_string();
                let mut src = std::error::Error::source(&e);
                while let Some(x) = src {
                    m = format!("{m}: {x}");
                    src = x.source();
                }
                m
            })?;
        Ok(format!("Connected - HTTP {} in {} ms", r.status().as_u16(), t0.elapsed().as_millis()))
    }

    /// Call after changing `settings`: rebuilds the HTTP client (proxy) and the global limit.
    pub fn apply_settings(&self) {
        let s = self.settings.lock().clone();
        *self.client.lock() = Self::build_client(&s);
        self.global_bps.store(s.global_speed_limit_kbps.saturating_mul(1024), Relaxed);
        self.stall_secs.store(s.stall_timeout_secs.clamp(5, 600), Relaxed);
    }

    pub fn global_limit_bps(&self) -> u64 {
        self.global_bps.load(Relaxed)
    }

    /// `headers` plus `Authorization: Basic …` when a saved site login matches `url`. Only for the
    /// requests we make: the login is never written into the download's saved headers.
    pub fn with_auth(&self, url: &str, headers: &[(String, String)]) -> Vec<(String, String)> {
        let mut h = headers.to_vec();
        if !h.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
            if let Some(a) = self.settings.lock().auth_header(url) {
                h.push(a);
            }
        }
        h
    }

    /// Books `n` received bytes against the download quota.
    pub(crate) fn account(&self, n: u64) {
        self.quota_used.fetch_add(n, Relaxed);
    }

    pub fn quota_blocked(&self) -> bool {
        self.quota_blocked.load(Relaxed)
    }

    pub fn quota_status(&self) -> Option<QuotaStatus> {
        let s = self.settings.lock();
        if !s.quota_on {
            return None;
        }
        let window = s.quota_hours.max(1) as i64 * 3600;
        let since = self.quota_since.load(Relaxed);
        let left = if since == 0 { window } else { (since + window - util::now_ts()).max(0) };
        Some(QuotaStatus {
            used: self.quota_used.load(Relaxed),
            limit: s.quota_mb.saturating_mul(1024 * 1024),
            resets_in: left as u64,
            blocked: self.quota_blocked(),
        })
    }

    /// True once after the quota stopped the downloads (the UI shows its warning then).
    pub fn take_quota_notice(&self) -> bool {
        self.quota_notice.swap(false, Relaxed)
    }

    /// Starts the quota window over (the *Reset* button of the Options page).
    pub fn reset_quota(&self) {
        self.quota_used.store(0, Relaxed);
        self.quota_since.store(util::now_ts(), Relaxed);
        self.quota_blocked.store(false, Relaxed);
    }

    fn check_quota(&self) {
        let (on, limit, window, warn) = {
            let s = self.settings.lock();
            (s.quota_on, s.quota_mb.saturating_mul(1024 * 1024), s.quota_hours.max(1) as i64 * 3600, s.quota_warn)
        };
        if !on {
            self.quota_blocked.store(false, Relaxed);
            self.quota_since.store(0, Relaxed);
            self.quota_used.store(0, Relaxed);
            return;
        }
        let now = util::now_ts();
        let since = self.quota_since.load(Relaxed);
        if since == 0 {
            self.quota_since.store(now, Relaxed);
        } else if now - since >= window {
            self.quota_used.store(0, Relaxed);
            self.quota_since.store(now, Relaxed);
            self.quota_blocked.store(false, Relaxed);
        }
        if limit > 0 && self.quota_used.load(Relaxed) >= limit && !self.quota_blocked.swap(true, Relaxed) {
            self.stop_queue();
            self.pause_all();
            if warn {
                self.quota_notice.store(true, Relaxed);
                if let Some(f) = self.repaint.lock().clone() {
                    f();
                }
            }
        }
    }

    pub fn set_event_hook(&self, f: impl Fn(Event) + Send + Sync + 'static) {
        *self.on_event.lock() = Some(Arc::new(f));
    }

    fn fire(&self, e: Event) {
        let f = self.on_event.lock().clone();
        if let Some(f) = f {
            f(e);
        }
    }

    /// Remembers that `host` only tolerates `cap` parallel connections (for 24 h).
    pub fn note_host_cap(&self, host: &str, cap: u32) {
        if host.is_empty() {
            return;
        }
        let mut m = self.host_caps.lock();
        let e = m.entry(host.to_string()).or_insert((cap, 0));
        e.0 = e.0.min(cap).max(1);
        e.1 = util::now_ts();
    }

    pub fn host_cap(&self, host: &str) -> Option<u32> {
        let mut m = self.host_caps.lock();
        match m.get(host) {
            Some((c, ts)) if util::now_ts() - ts < 24 * 3600 => Some(*c),
            Some(_) => {
                m.remove(host);
                None
            }
            None => None,
        }
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.repaint.lock() = Some(Arc::new(f));
    }

    /// How yt-dlp should reach sites: the proxy of Options > Connection and the chosen cookie browser.
    pub fn ytdlp_opts(&self) -> crate::ytdlp::Opts {
        let s = self.settings.lock();
        crate::ytdlp::Opts { cookies_file: None, cookies_browser: s.ytdlp_cookies_browser.clone(), proxy: s.proxy_url() }
    }

    /// Lists the videos on a page through yt-dlp (blocks - run it on a thread).
    pub fn ytdlp_fetch(&self, url: &str) -> Result<Vec<crate::ytdlp::Video>, String> {
        crate::ytdlp::fetch(&self.data_dir, url, &self.ytdlp_opts())
    }

    pub fn set_incoming_hook(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_incoming.lock() = Some(Arc::new(f));
    }

    pub fn push_incoming(&self, i: Incoming) {
        self.incoming.lock().push(i);
        if let Some(f) = self.on_incoming.lock().clone() {
            f();
        }
        if let Some(f) = self.repaint.lock().clone() {
            f();
        }
    }

    /// Asks the UI to open its Tools window because yt-dlp, Deno or ffmpeg is missing. Asked again
    /// within a minute (every hover over a video asks) it does nothing, so closing the window sticks.
    pub fn request_tools_window(&self) {
        {
            let mut last = self.tools_asked.lock();
            if last.map_or(false, |t| t.elapsed() < Duration::from_secs(60)) {
                return;
            }
            *last = Some(Instant::now());
        }
        self.tools_wanted.store(true, Relaxed);
        if let Some(f) = self.on_incoming.lock().clone() {
            f();
        }
        if let Some(f) = self.repaint.lock().clone() {
            f();
        }
    }

    /// Called for every request that comes from the browser extension; remembers (once, for good) that it is installed.
    pub fn note_extension(&self) {
        if self.ext_seen.swap(true, Relaxed) {
            return;
        }
        let first = {
            let mut s = self.settings.lock();
            !std::mem::replace(&mut s.extension_seen, true)
        };
        if first {
            self.save_settings();
        }
    }

    /// True once the browser extension has contacted DownCraft (now or in an earlier session).
    pub fn extension_seen(&self) -> bool {
        self.settings.lock().extension_seen
    }

    /// True once after `request_tools_window`.
    pub fn take_tools_request(&self) -> bool {
        self.tools_wanted.swap(false, Relaxed)
    }

    pub fn take_incoming(&self) -> Vec<Incoming> {
        std::mem::take(&mut *self.incoming.lock())
    }

    /// Starts the localhost listener the browser extension talks to. Returns false if the port is taken
    /// (e.g. a second DownCraft instance is already running).
    pub fn start_bridge(self: &Arc<Self>) -> bool {
        crate::bridge::spawn(self.clone())
    }

    pub fn client(&self) -> Client {
        self.client.lock().clone()
    }

    pub fn push_event(self: &Arc<Self>, e: Event) {
        // the halves of a video + audio pair are held back; the pair reports once it is merged
        if let Event::Completed(id) = e {
            if self.merge_gate(id) {
                return;
            }
        }
        self.events.lock().push(e);
        self.fire(e);
    }

    /// Queues an event without the merge check (used by the merge itself) and wakes the UI.
    pub(crate) fn push_event_raw(&self, e: Event) {
        self.events.lock().push(e);
        self.fire(e);
        let f = self.repaint.lock().clone();
        if let Some(f) = f {
            f();
        }
    }

    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock())
    }

    pub fn find(&self, id: u64) -> Option<Arc<Task>> {
        self.tasks.lock().iter().find(|t| t.id == id).cloned()
    }

    pub fn any_active(&self) -> bool {
        self.tasks.lock().iter().any(|t| t.is_active())
    }

    pub fn add(self: &Arc<Self>, req: NewDownload) -> Arc<Task> {
        // With no probe and no explicit name, leave it empty: the engine names the file from
        // Content-Disposition after probing (the UI shows the URL's file name meanwhile).
        let mut name = req.file_name.clone();
        if name.is_empty() {
            if let Some(p) = &req.probe {
                name = p.file_name.clone();
            }
        }
        let is_hls = req.probe.as_ref().map_or(false, |p| p.hls) || name.to_ascii_lowercase().ends_with(".m3u8");
        let name = if is_hls && !name.is_empty() { util::hls_name(&name) } else { name };
        let name = if name.is_empty() { name } else { util::unique_name(&req.dir, &name) };
        let now = util::now_ts();
        let (total, resumable, probed) = match &req.probe {
            Some(p) => (p.total, p.resumable, true),
            None => (0, false, false),
        };
        let info = TaskInfo {
            file_name: name,
            dir: req.dir,
            category: req.category,
            description: req.description,
            status: Status::Stopped,
            error: None,
            resumable,
            probed,
            queued: req.queued,
            added: now,
            last_try: now,
            completed_at: 0,
            headers: req.headers,
            hls: is_hls,
            etag: req.probe.as_ref().and_then(|p| p.etag.clone()),
            last_modified: req.probe.as_ref().and_then(|p| p.last_modified.clone()),
        };
        let id = self.next_id.fetch_add(1, Relaxed);
        let task = Arc::new(Task::new(id, req.url, info, total));
        task.limit_kbps.store(self.settings.lock().speed_limit_kbps, Relaxed);
        self.tasks.lock().push(task.clone());
        self.save();
        if req.start {
            self.start(&task);
        }
        task
    }

    /// Starts or resumes a download (a manual start also refills its automatic-retry budget).
    pub fn start(self: &Arc<Self>, task: &Arc<Task>) {
        task.retries.store(0, Relaxed);
        task.info.lock().error = None;
        self.start_inner(task);
    }

    fn start_inner(self: &Arc<Self>, task: &Arc<Task>) {
        if self.quota_blocked() {
            let left = self.quota_status().map_or(0, |q| q.resets_in);
            task.info.lock().error = Some(format!("Download quota reached - available again in {}", util::fmt_time(left)));
            return;
        }
        {
            let mut i = task.info.lock();
            if matches!(i.status, Status::Connecting | Status::Downloading | Status::Complete) {
                return;
            }
            i.status = Status::Connecting;
            // `i.error` is kept: an automatic retry shows why the previous attempt failed
            i.last_try = util::now_ts();
        }
        task.ctl.store(CTL_RUN, Relaxed);
        let epoch = task.epoch.fetch_add(1, Relaxed) + 1;
        let (m, t) = (self.clone(), task.clone());
        std::thread::spawn(move || download::run(m, t, epoch));
    }

    pub fn pause(&self, task: &Arc<Task>) {
        task.ctl.store(CTL_STOP, Relaxed);
        task.epoch.fetch_add(1, Relaxed); // supersedes supervisor + segment threads
        task.speed.store(0, Relaxed);
        {
            let mut i = task.info.lock();
            if matches!(i.status, Status::Connecting | Status::Downloading) {
                i.status = Status::Stopped;
            }
        }
        self.save();
    }

    pub fn pause_all(&self) {
        let tasks = self.tasks.lock().clone();
        for t in tasks.iter().filter(|t| t.is_active()) {
            self.pause(t);
        }
    }

    pub fn delete(&self, task: &Arc<Task>, remove_files: bool) {
        self.pause(task);
        self.tasks.lock().retain(|t| t.id != task.id);
        if remove_files {
            // let the workers of a just-stopped run let go of the file first (matters on Windows)
            let t0 = std::time::Instant::now();
            while task.live_workers.load(Relaxed) > 0 && t0.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(30));
            }
            let _ = std::fs::remove_file(task.final_path());
            let _ = std::fs::remove_file(task.part_path());
            let _ = std::fs::remove_dir_all(task.hls_dir());
        }
        self.save();
    }

    /// Puts a download into the main queue or takes it out again.
    pub fn set_queued(&self, task: &Arc<Task>, queued: bool) {
        task.info.lock().queued = queued;
        self.save();
    }

    /// Throws away what was downloaded (finished file or partial data) and starts the same link
    /// again from the beginning. The new entry keeps the name, folder, category, description,
    /// request headers and queue membership; it gets a new id, which is returned.
    pub fn redownload(self: &Arc<Self>, task: &Arc<Task>) -> Arc<Task> {
        let (dir, name, category, description, queued, headers, hls) = {
            let i = task.info.lock();
            (i.dir.clone(), i.file_name.clone(), i.category, i.description.clone(), i.queued, i.headers.clone(), i.hls)
        };
        let url = task.url.clone();
        self.delete(task, true);
        let fresh = self.add(NewDownload { url, dir, file_name: name, category, description, probe: None, queued, start: false, headers });
        // an HLS download is stored under its final name, which `add` would not recognise as HLS
        fresh.info.lock().hls = hls;
        self.start(&fresh);
        fresh
    }

    /// Renames and / or moves a download that is not running: the finished file, or the partial
    /// data of an unfinished one, goes to `new_path` and the entry follows it.
    pub fn move_task(&self, task: &Arc<Task>, new_path: &std::path::Path) -> Result<(), String> {
        use std::path::{Path, PathBuf};
        if task.is_active() {
            return Err("Stop the download first.".into());
        }
        let name = new_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or("Enter a file name.")?
            .to_string();
        if name.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c)) {
            return Err("A file name cannot contain  < > : \" | ? *".into());
        }
        let (old_dir, old_name, done) = {
            let i = task.info.lock();
            (i.dir.clone(), i.file_name.clone(), i.status == Status::Complete)
        };
        let dir = match new_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => old_dir.clone(),
        };
        if dir == old_dir && name == old_name {
            return Ok(());
        }
        // (from, to) for everything that exists on disk for this download
        let pairs = |d1: &Path, n1: &str, d2: &Path, n2: &str| -> Vec<(PathBuf, PathBuf)> {
            let mut v = vec![(d1.join(n1), d2.join(n2))];
            if !done {
                v.push((d1.join(format!("{n1}.fcpart")), d2.join(format!("{n2}.fcpart"))));
                v.push((d1.join(format!("{n1}.fcparts")), d2.join(format!("{n2}.fcparts"))));
            }
            v.into_iter().filter(|(from, _)| from.exists()).collect()
        };
        let moves = pairs(&old_dir, &old_name, &dir, &name);
        for (from, to) in &moves {
            // a rename that only changes the letter case is the same file on Windows
            let same = from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
            if to.exists() && !same {
                return Err(format!("\"{}\" already exists in that folder.", to.file_name().and_then(|n| n.to_str()).unwrap_or("")));
            }
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create the folder: {e}"))?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (from, to) in moves {
            if let Err(e) = move_path(&from, &to) {
                for (f, t) in moved.iter().rev() {
                    let _ = move_path(t, f); // put back what was already moved
                }
                return Err(format!("Could not move the file: {e}"));
            }
            moved.push((from, to));
        }
        {
            let mut i = task.info.lock();
            i.dir = dir;
            i.file_name = name;
        }
        self.save();
        Ok(())
    }

    pub fn delete_completed(&self) {
        self.tasks.lock().retain(|t| t.info.lock().status != Status::Complete);
        self.save();
    }

    pub fn start_queue(&self) {
        self.queue_running.store(true, Relaxed);
    }

    pub fn stop_queue(&self) {
        self.queue_running.store(false, Relaxed);
        self.queue_worked.store(false, Relaxed);
        let tasks = self.tasks.lock().clone();
        for t in tasks.iter().filter(|t| t.is_active() && t.info.lock().queued) {
            self.pause(t);
        }
    }

    /// Forget the time that has passed so far: call after the schedule was changed, so that a start
    /// time earlier today does not fire retroactively.
    pub fn reset_schedule_clock(&self) {
        self.sched_cursor.store(util::now_ts(), Relaxed);
    }

    /// Starts / stops the main queue when a time of the schedule has passed since the last call.
    fn run_schedule(&self) {
        let now = util::now_ts();
        let prev = self.sched_cursor.swap(now, Relaxed);
        let (start, stop, once) = {
            let s = self.settings.lock();
            (s.schedule.start_due(prev, now), s.schedule.stop_due(prev, now), !s.schedule.daily)
        };
        if start && stop {
            return; // both passed while nobody was looking: the window is over
        }
        if start {
            if once {
                // a one-time start is used up
                self.settings.lock().schedule.start_on = false;
                self.save_settings();
            }
            self.start_queue();
        }
        if stop {
            self.stop_queue();
        }
    }

    /// Call regularly (the UI does it every frame): runs the scheduler and advances the main queue.
    pub fn tick(self: &Arc<Self>) {
        self.run_schedule();
        self.check_quota();
        self.retry_failed();
        if !self.queue_running.load(Relaxed) {
            return;
        }
        let tasks = self.tasks.lock().clone();
        let running = tasks.iter().any(|t| {
            let i = t.info.lock();
            i.queued && matches!(i.status, Status::Connecting | Status::Downloading)
        });
        if running {
            return;
        }
        let next = tasks.iter().find(|t| {
            let i = t.info.lock();
            i.queued && i.status == Status::Stopped
        });
        match next {
            Some(t) => {
                self.queue_worked.store(true, Relaxed);
                self.start(t)
            }
            None => {
                self.queue_running.store(false, Relaxed);
                if self.queue_worked.swap(false, Relaxed) {
                    self.push_event_raw(Event::QueueFinished);
                }
            }
        }
    }

    /// Restarts downloads that failed for a network reason, with growing delays (5 s, 10 s, 20 s…).
    fn retry_failed(self: &Arc<Self>) {
        let max = self.settings.lock().auto_retries;
        if max == 0 {
            return;
        }
        let now = util::now_ts();
        let tasks = self.tasks.lock().clone();
        for t in tasks {
            let failed = t.info.lock().status == Status::Error;
            if failed && t.transient.load(Relaxed) && t.retries.load(Relaxed) < max && now >= t.retry_at.load(Relaxed) {
                t.retries.fetch_add(1, Relaxed);
                self.start_inner(&t);
            }
        }
    }

    pub fn save(&self) {
        let _g = self.save_lock.lock();
        let recs: Vec<Record> = self.tasks.lock().iter().map(|t| t.to_record()).collect();
        Self::write_atomic(&self.data_dir.join("downloads.json"), &recs);
    }

    pub fn save_settings(&self) {
        let _g = self.save_lock.lock();
        let s = self.settings.lock().clone();
        Self::write_atomic(&self.data_dir.join("settings.json"), &s);
    }

    fn write_atomic<T: serde::Serialize>(path: &std::path::Path, v: &T) {
        if let Ok(bytes) = serde_json::to_vec_pretty(v) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

/// Renames a file or folder; across drives a file is copied and the original removed.
fn move_path(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "a folder of partial data cannot be moved to another drive"));
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}
