use crate::{util, Category, Schedule};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ProxyMode {
    /// Honour `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`.
    #[default]
    System,
    None,
    /// `http://host:port` (HTTP CONNECT).
    Http,
    /// `socks5://host:port` (DNS resolved by the proxy).
    Socks5,
}

/// What DownCraft does when the main queue has finished.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum AfterQueue {
    #[default]
    Nothing,
    CloseApp,
    Sleep,
    Shutdown,
}

/// What a double click on a row of the download list does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum DoubleClick {
    /// Open a finished file, show the progress window of an unfinished one.
    #[default]
    Auto,
    /// Open the file (unfinished downloads show their progress window instead).
    Open,
    /// Open the folder the file is saved in.
    Folder,
    /// Always show the progress / details window.
    Progress,
}

/// What happens when a link that is already in the list is added again.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum DuplicateMode {
    #[default]
    Ask,
    AddAnyway,
    Skip,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum IpMode {
    #[default]
    Auto,
    V4,
    V6,
}

/// "Use at most `connections` connections to this server".
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ConnRule {
    /// Host name; `example.com` also covers its sub-domains, `*.example.com` and `cdn*.example.*` are patterns.
    pub host: String,
    pub connections: u32,
}

/// HTTP Basic credentials sent to a site (and, optionally, only below a path on it).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct SiteLogin {
    /// `host` or `host/path/prefix`.
    pub site: String,
    pub user: String,
    pub pass: String,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct SoundSlot {
    pub enabled: bool,
    /// Sound file; empty = the system's default sound.
    pub file: String,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Sounds {
    pub complete: SoundSlot,
    pub failed: SoundSlot,
    pub queue_done: SoundSlot,
}

pub const DEFAULT_CLIPBOARD_EXTS: &str = "7Z AAC APK APPIMAGE AVI BIN BZ2 DEB DMG EXE FLAC GZ IMG ISO JAR M3U8 M4A M4V MKV MOV MP3 MP4 MPEG MPG MSI OGG OPUS PDF PKG RAR RPM TAR TGZ WAV WEBM WMA WMV XZ ZIP R0* R1*";
pub const DEFAULT_CLIPBOARD_SKIP: &str = "*.update.microsoft.com download.windowsupdate.com *.windowsupdate.com";
pub const DEFAULT_PROXY_BYPASS: &str = "localhost,127.0.0.1,::1";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub connections: u32,
    pub download_root: PathBuf,
    pub remembered: HashMap<Category, PathBuf>,
    pub show_complete_dialog: bool,
    /// Default limit given to each new download (KB/s, 0 = off).
    pub speed_limit_kbps: u64,
    /// Limit shared by every download together (KB/s, 0 = off).
    pub global_speed_limit_kbps: u64,
    pub proxy_mode: ProxyMode,
    /// `host:port`, optionally with `user:pass@` in front.
    pub proxy_addr: String,
    /// How many times a download that failed for a network reason is retried automatically.
    pub auto_retries: u32,
    /// Closing the window keeps DownCraft (and its downloads) running from the system tray.
    pub close_to_tray: bool,
    /// The main window stays above every other window (the pin in the title bar).
    pub pin_main: bool,
    /// App theme chosen under View > Theme (key such as `dark`, `light`; unknown values mean `dark`).
    pub theme: String,
    /// Accent colour chosen under View > Accent color (key such as `blue`, `green`).
    pub accent: String,

    // ── General ──
    /// Start DownCraft when the user logs in.
    pub launch_on_startup: bool,
    /// When started at login, stay hidden in the tray (needs `close_to_tray`).
    pub start_hidden: bool,
    /// Offer to download links that are copied to the clipboard.
    pub clipboard_monitor: bool,
    /// Space-separated file extensions (wildcards allowed) the clipboard monitor reacts to.
    pub clipboard_exts: String,
    /// Space-separated hosts the clipboard monitor ignores.
    pub clipboard_skip_hosts: String,
    pub after_queue: AfterQueue,
    /// When the main queue starts / stops by itself (Tasks > Scheduler).
    pub schedule: Schedule,
    /// What a double click on a row does (right-click menu > On Double click).
    pub double_click: DoubleClick,

    // ── Save to ──
    /// Give the finished file the date the server reports (`Last-Modified`).
    pub set_file_date: bool,
    /// "Remember this folder" starts ticked in the Download File Info dialog.
    pub remember_folder_default: bool,

    // ── Downloads ──
    /// Show *Download File Info* for a link that was added by hand.
    pub show_start_dialog: bool,
    pub duplicate_mode: DuplicateMode,
    /// Empty = `DownCraft/<version>`.
    pub user_agent: String,
    /// Do not restart a resumed download just because `Last-Modified` changed (ETag and size still count).
    pub ignore_modified_on_resume: bool,
    /// Run `post_command` after every finished download.
    pub post_command_on: bool,
    /// Program and arguments; `%FILE%` is replaced by the downloaded file's path. Never run through a shell.
    pub post_command: String,

    // ── Connection ──
    pub conn_rules: Vec<ConnRule>,
    pub quota_on: bool,
    pub quota_mb: u64,
    pub quota_hours: u32,
    pub quota_warn: bool,
    pub connect_timeout_secs: u64,
    /// Seconds without data before a connection is declared dead and re-opened.
    pub stall_timeout_secs: u64,

    // ── Proxy ──
    pub proxy_user: String,
    pub proxy_pass: String,
    /// Comma-separated hosts that never use the proxy.
    pub proxy_bypass: String,

    // ── Site logins ──
    pub logins: Vec<SiteLogin>,

    // ── Video sites (yt-dlp) ──
    /// Browser yt-dlp reads its cookies from for pages that need a sign-in (Facebook, Instagram, X …);
    /// empty = none. The browser extension sends its own cookies, so this is for the *Add download* dialog.
    pub ytdlp_cookies_browser: String,

    // ── First-run setup window ──
    /// Show the setup window (browser extension + yt-dlp / ffmpeg / Deno) at start-up while something is missing.
    pub show_setup: bool,
    /// The browser extension has talked to DownCraft at least once.
    pub extension_seen: bool,

    // ── Network ──
    pub ip_mode: IpMode,
    /// Local IP address to send traffic from (pick the VPN or second adapter); empty = automatic.
    pub bind_addr: String,
    pub accept_invalid_certs: bool,

    // ── Sounds ──
    pub sounds: Sounds,
}

impl Default for Settings {
    fn default() -> Self {
        let root = directories::UserDirs::new()
            .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("Downloads"));
        Self {
            connections: 8,
            download_root: root,
            remembered: HashMap::new(),
            show_complete_dialog: true,
            speed_limit_kbps: 0,
            global_speed_limit_kbps: 0,
            proxy_mode: ProxyMode::System,
            proxy_addr: String::new(),
            auto_retries: 3,
            close_to_tray: false,
            pin_main: false,
            theme: "dark".into(),
            accent: "blue".into(),

            launch_on_startup: false,
            start_hidden: false,
            clipboard_monitor: false,
            clipboard_exts: DEFAULT_CLIPBOARD_EXTS.into(),
            clipboard_skip_hosts: DEFAULT_CLIPBOARD_SKIP.into(),
            after_queue: AfterQueue::Nothing,
            schedule: Schedule::default(),
            double_click: DoubleClick::Auto,

            set_file_date: false,
            remember_folder_default: false,

            show_start_dialog: true,
            duplicate_mode: DuplicateMode::Ask,
            user_agent: String::new(),
            ignore_modified_on_resume: false,
            post_command_on: false,
            post_command: String::new(),

            conn_rules: Vec::new(),
            quota_on: false,
            quota_mb: 5000,
            quota_hours: 24,
            quota_warn: true,
            connect_timeout_secs: 20,
            stall_timeout_secs: 30,

            proxy_user: String::new(),
            proxy_pass: String::new(),
            proxy_bypass: DEFAULT_PROXY_BYPASS.into(),
            ytdlp_cookies_browser: String::new(),

            show_setup: true,
            extension_seen: false,

            logins: Vec::new(),

            ip_mode: IpMode::Auto,
            bind_addr: String::new(),
            accept_invalid_certs: false,

            sounds: Sounds::default(),
        }
    }
}

fn host_and_path(url: &str) -> Option<(String, String)> {
    let u = reqwest::Url::parse(url).ok()?;
    Some((u.host_str()?.to_ascii_lowercase(), u.path().to_string()))
}

impl Settings {
    /// The proxy URL reqwest should use, or `None` for "no explicit proxy".
    /// The user name and password of the Proxy page are put into it (percent-encoded).
    pub fn proxy_url(&self) -> Option<String> {
        let a = self.proxy_addr.trim();
        if a.is_empty() {
            return None;
        }
        let scheme = match self.proxy_mode {
            ProxyMode::Http => "http",
            ProxyMode::Socks5 => "socks5h",
            _ => return None,
        };
        let (scheme, rest) = match a.split_once("://") {
            Some((s, r)) => (s.to_string(), r.to_string()),
            None => (scheme.to_string(), a.to_string()),
        };
        // credentials typed into the address itself win over the separate fields
        if rest.contains('@') || self.proxy_user.is_empty() {
            return Some(format!("{scheme}://{rest}"));
        }
        let auth = if self.proxy_pass.is_empty() {
            util::percent_encode(&self.proxy_user)
        } else {
            format!("{}:{}", util::percent_encode(&self.proxy_user), util::percent_encode(&self.proxy_pass))
        };
        Some(format!("{scheme}://{auth}@{rest}"))
    }

    /// `Downloads/Video`, or the path the user asked us to remember for that category.
    pub fn dir_for(&self, cat: Category) -> PathBuf {
        self.remembered
            .get(&cat)
            .cloned()
            .unwrap_or_else(|| self.download_root.join(cat.name()))
    }

    /// Connections to use for `host`: the first matching server exception, else the default.
    pub fn connections_for(&self, host: &str) -> u32 {
        self.conn_rules
            .iter()
            .find(|r| util::host_matches(&r.host, host))
            .map_or(self.connections, |r| r.connections)
            .clamp(1, 32)
    }

    /// The saved login for `url`: the most specific matching entry wins (longest site string).
    pub fn login_for(&self, url: &str) -> Option<&SiteLogin> {
        let (host, path) = host_and_path(url)?;
        self.logins
            .iter()
            .filter(|l| !l.user.is_empty())
            .filter(|l| {
                let site = l.site.trim().trim_start_matches("http://").trim_start_matches("https://");
                let (h, p) = match site.split_once('/') {
                    Some((h, p)) => (h, format!("/{p}")),
                    None => (site, String::new()),
                };
                util::host_matches(h, &host) && path.starts_with(&p)
            })
            .max_by_key(|l| l.site.len())
    }

    /// `Authorization: Basic …` for `url` when a saved login matches.
    pub fn auth_header(&self, url: &str) -> Option<(String, String)> {
        let l = self.login_for(url)?;
        let token = util::base64_encode(format!("{}:{}", l.user, l.pass).as_bytes());
        Some(("authorization".into(), format!("Basic {token}")))
    }

    /// Should the clipboard monitor offer this text? Only http(s) links to one of the listed file
    /// types on a host that is not excluded.
    pub fn wants_clipboard_url(&self, url: &str) -> bool {
        let Ok(u) = reqwest::Url::parse(url) else { return false };
        if !matches!(u.scheme(), "http" | "https") {
            return false;
        }
        let host = u.host_str().unwrap_or("");
        if self.clipboard_skip_hosts.split_whitespace().any(|p| util::host_matches(p, host)) {
            return false;
        }
        let name = u.path().rsplit('/').next().unwrap_or("");
        let Some((_, ext)) = name.rsplit_once('.') else { return false };
        self.clipboard_exts.split_whitespace().any(|p| util::glob_match(p, ext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_files_still_load() {
        let s: Settings = serde_json::from_str(r#"{"connections": 4, "theme": "light"}"#).unwrap();
        assert_eq!(s.connections, 4);
        assert_eq!(s.theme, "light");
        assert_eq!(s.stall_timeout_secs, 30);
        assert!(s.show_start_dialog);
    }

    #[test]
    fn server_exceptions_override_the_default() {
        let mut s = Settings::default();
        s.conn_rules.push(ConnRule { host: "slow.example.com".into(), connections: 2 });
        s.conn_rules.push(ConnRule { host: "example.com".into(), connections: 99 });
        assert_eq!(s.connections_for("slow.example.com"), 2);
        assert_eq!(s.connections_for("www.example.com"), 32); // clamped
        assert_eq!(s.connections_for("other.org"), 8);
    }

    #[test]
    fn the_most_specific_login_wins() {
        let mut s = Settings::default();
        s.logins.push(SiteLogin { site: "example.com".into(), user: "a".into(), pass: "1".into() });
        s.logins.push(SiteLogin { site: "example.com/private".into(), user: "b".into(), pass: "2".into() });
        assert_eq!(s.login_for("https://example.com/pub/x.zip").unwrap().user, "a");
        assert_eq!(s.login_for("https://cdn.example.com/private/x.zip").unwrap().user, "b");
        assert!(s.login_for("https://elsewhere.org/x.zip").is_none());
        let (k, v) = s.auth_header("https://example.com/f").unwrap();
        assert_eq!(k, "authorization");
        assert_eq!(v, "Basic YTox"); // a:1
    }

    #[test]
    fn clipboard_filter() {
        let s = Settings::default();
        assert!(s.wants_clipboard_url("https://example.com/files/setup.EXE?x=1"));
        assert!(s.wants_clipboard_url("http://example.com/a.r01"));
        assert!(!s.wants_clipboard_url("https://example.com/page.html"));
        assert!(!s.wants_clipboard_url("https://example.com/no-extension"));
        assert!(!s.wants_clipboard_url("ftp://example.com/a.zip"));
        assert!(!s.wants_clipboard_url("https://download.windowsupdate.com/a.cab.exe"));
        assert!(!s.wants_clipboard_url("just some text.zip"));
    }

    #[test]
    fn proxy_credentials_go_into_the_url() {
        let mut s = Settings { proxy_mode: ProxyMode::Socks5, proxy_addr: "10.0.0.1:1080".into(), ..Default::default() };
        assert_eq!(s.proxy_url().unwrap(), "socks5h://10.0.0.1:1080");
        s.proxy_user = "me".into();
        s.proxy_pass = "p@ss".into();
        assert_eq!(s.proxy_url().unwrap(), "socks5h://me:p%40ss@10.0.0.1:1080");
        s.proxy_addr = "http://x:y@proxy:3128".into();
        s.proxy_mode = ProxyMode::Http;
        assert_eq!(s.proxy_url().unwrap(), "http://x:y@proxy:3128");
        s.proxy_mode = ProxyMode::System;
        assert!(s.proxy_url().is_none());
    }
}
