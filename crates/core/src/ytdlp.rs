//! yt-dlp support: asks `yt-dlp -J` which formats a page offers (YouTube, Facebook, Instagram, X /
//! Twitter, TikTok, Vimeo, Reddit … - every site yt-dlp has an extractor for) and hands the browser
//! extension or the *Add download* dialog a compact list. Downloading itself still goes through the
//! normal engine (the chosen direct / HLS link is added like any other), so progress, resume and
//! segments all work as usual.
use crate::AudioPart;
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

/// Environment variable that overrides where yt-dlp is looked for.
pub const ENV_PATH: &str = "DOWNCRAFT_YTDLP";
const TIMEOUT: Duration = Duration::from_secs(90);

/// How yt-dlp is told to reach a site.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// Netscape cookie file (e.g. the cookies the browser extension sent along).
    pub cookies_file: Option<PathBuf>,
    /// Browser yt-dlp reads cookies from (`chrome`, `firefox` …); empty = none. Ignored when
    /// `cookies_file` is set.
    pub cookies_browser: String,
    /// `http://…` / `socks5h://…` proxy URL.
    pub proxy: Option<String>,
}

/// Browsers `--cookies-from-browser` understands (a fixed list, so a setting can't smuggle in an option).
pub const BROWSERS: [&str; 9] = ["brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale"];

pub fn valid_browser(b: &str) -> bool {
    BROWSERS.contains(&b)
}

/// Sites whose page addresses are video pages rather than files. A link on one of these goes to
/// yt-dlp automatically (any other address can still be sent there with the *Video* button).
const SITES: &[&str] = &[
    "youtube.com", "youtu.be", "youtube-nocookie.com", "facebook.com", "fb.watch", "fb.com", "instagram.com", "twitter.com", "x.com",
    "t.co", "threads.net", "tiktok.com", "vimeo.com", "twitch.tv", "dailymotion.com", "dai.ly", "reddit.com", "redd.it",
    "bilibili.com", "b23.tv", "rumble.com", "streamable.com", "snapchat.com", "pinterest.com", "pin.it", "linkedin.com",
    "soundcloud.com", "bandcamp.com", "odysee.com", "ok.ru", "vk.com", "9gag.com", "tumblr.com", "bsky.app", "kick.com",
];

fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or("");
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = auth.rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or("").to_ascii_lowercase()
}

/// `https://host` of an address (used as Referer / Origin for the links yt-dlp finds).
pub fn page_origin(url: &str) -> String {
    let scheme = if url.starts_with("http://") { "http" } else { "https" };
    let h = host_of(url);
    if h.is_empty() { String::new() } else { format!("{scheme}://{h}") }
}

/// True when `url` is a page on a site yt-dlp is used for by default (Facebook, Instagram, X …).
pub fn is_video_site(url: &str) -> bool {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return false;
    }
    let host = host_of(u);
    SITES.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Finds yt-dlp: `$DOWNCRAFT_YTDLP`, then DownCraft's tools folder, the data folder, the folder of
/// the executable and `PATH` (see `tools::locate`).
pub fn find(data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(ENV_PATH).map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    crate::tools::locate(data_dir, crate::tools::Tool::YtDlp).map(|i| i.path)
}

/// True for pages where yt-dlp needs a JavaScript runtime (Deno) to read the video.
fn needs_deno(url: &str) -> bool {
    let host = host_of(url);
    ["youtube.com", "youtu.be", "youtube-nocookie.com"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// The helper programs `url` needs that are not installed yet, in the order they are shown:
/// yt-dlp always, Deno on YouTube, ffmpeg (joins video and audio) always.
pub fn missing_tools(data_dir: &Path, url: &str) -> Vec<crate::tools::Tool> {
    use crate::tools::{locate, Tool};
    let mut missing = Vec::new();
    if find(data_dir).is_none() {
        missing.push(Tool::YtDlp);
    }
    if needs_deno(url) && locate(data_dir, Tool::Deno).is_none() {
        missing.push(Tool::Deno);
    }
    if locate(data_dir, Tool::Ffmpeg).is_none() {
        missing.push(Tool::Ffmpeg);
    }
    missing
}

/// The text of the "Download this video" popup when `missing` isn't empty, e.g.
/// `yt-dlp isn't installed. In DownCraft open Tasks → Tools and press Get next to yt-dlp.`
pub fn missing_message(missing: &[crate::tools::Tool]) -> String {
    let names: Vec<&str> = missing.iter().map(|t| t.name()).collect();
    let list = match names.as_slice() {
        [] => return String::new(),
        [one] => one.to_string(),
        [init @ .., last] => format!("{} and {}", init.join(", "), last),
    };
    let verb = if names.len() == 1 { "isn't" } else { "aren't" };
    let mut msg = format!("{list} {verb} installed. In DownCraft open Tasks → Tools and press Get next to {list}.");
    // DownCraft has no download for some tools on some systems (ffmpeg on macOS): say what to do instead
    for t in missing.iter().filter(|t| t.source().is_none()) {
        msg += &format!(" {}: {}", t.name(), t.manual_hint());
    }
    msg
}

/// Runs `yt-dlp -J --no-playlist <url>` and returns the parsed JSON.
fn run(bin: &Path, data_dir: &Path, url: &str, opts: &Opts) -> Result<Value, String> {
    let mut cmd = Command::new(bin);
    cmd.args(["-J", "--no-playlist", "--no-warnings", "--no-colors", "--socket-timeout", "20"]);
    // Translated auto-captions are the slowest part of a YouTube answer and DownCraft never uses them.
    cmd.args(["--extractor-args", "youtube:skip=translated_subs"]);
    if let Some(f) = &opts.cookies_file {
        cmd.arg("--cookies").arg(f);
    } else if valid_browser(&opts.cookies_browser) {
        cmd.arg("--cookies-from-browser").arg(&opts.cookies_browser);
    }
    if let Some(p) = opts.proxy.as_deref().filter(|p| !p.is_empty()) {
        cmd.arg("--proxy").arg(p);
    }
    // `--` keeps a hostile "URL" from being read as an option.
    cmd.arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Tools installed from DownCraft's Tools window (ffmpeg, Deno) are found by yt-dlp through PATH.
    let mut paths = vec![crate::tools::tools_dir(data_dir)];
    paths.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("PATH", joined);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    // Both pipes are drained on their own threads, otherwise a big JSON blocks yt-dlp on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let t_err = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("yt-dlp timed out".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned();
    if !status.success() {
        let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("yt-dlp failed");
        let mut msg: String = line.trim().trim_start_matches("ERROR: ").chars().take(300).collect();
        if needs_login(&msg) {
            msg += " - sign in to the site in your browser. The browser extension passes your cookies on by itself; in DownCraft choose the browser under Options → Video sites.";
        }
        return Err(msg);
    }
    serde_json::from_slice(&stdout).map_err(|e| format!("unreadable yt-dlp output: {e}"))
}

fn is_none(v: &Value) -> bool {
    v.as_str().map_or(true, |s| s.is_empty() || s == "none")
}

fn codec_family(c: &str) -> &'static str {
    let c = c.to_ascii_lowercase();
    if c.starts_with("avc") || c.starts_with("h264") {
        "H.264"
    } else if c.starts_with("vp9") || c.starts_with("vp09") {
        "VP9"
    } else if c.starts_with("av01") || c.starts_with("av1") {
        "AV1"
    } else if c.starts_with("hev") || c.starts_with("hvc") || c.starts_with("h265") {
        "H.265"
    } else if c.starts_with("mp4a") || c.starts_with("aac") {
        "AAC"
    } else if c.starts_with("opus") {
        "Opus"
    } else {
        ""
    }
}

/// Turns yt-dlp's `-J` output into the small structure the extension shows.
///
/// Every format is `kind` = `muxed` (picture + sound), `video` (picture only) or `audio`.
/// Storyboards, DRM-protected and segment-list (DASH/ISM/F4M) formats are dropped because the
/// engine can't download them. Near-duplicates (same size/fps/codec) keep only the best bitrate.
pub fn normalize(info: &Value) -> Result<Value, String> {
    // A post with several videos (an X thread, an Instagram carousel) comes back as a playlist.
    if let Some(entries) = info["entries"].as_array().filter(|e| !e.is_empty()) {
        let mut ok: Vec<Value> = Vec::new();
        let mut first_err = None;
        for e in entries.iter().filter(|e| e.is_object()) {
            match normalize(e) {
                Ok(mut v) => {
                    if v["title"].as_str().unwrap_or("").is_empty() {
                        v["title"] = info["title"].clone();
                    }
                    ok.push(v)
                }
                Err(e) => first_err = first_err.or(Some(e)),
            }
        }
        return match ok.len() {
            0 => Err(first_err.unwrap_or_else(|| "yt-dlp found no downloadable formats for this page.".into())),
            1 => Ok(ok.remove(0)),
            _ => {
                let mut head = ok[0].clone();
                head["entries"] = Value::Array(ok);
                Ok(head)
            }
        };
    }
    let live = info["is_live"].as_bool().unwrap_or(false) || info["live_status"].as_str() == Some("is_live");
    if live {
        return Err("Live streams can't be downloaded.".into());
    }
    let raw = info["formats"].as_array().cloned().unwrap_or_else(|| vec![info.clone()]);
    let mut best: HashMap<String, Value> = HashMap::new();
    for f in &raw {
        let url = f["url"].as_str().unwrap_or("");
        if !(url.starts_with("http://") || url.starts_with("https://")) || f["has_drm"].as_bool() == Some(true) {
            continue;
        }
        let proto = f["protocol"].as_str().unwrap_or("https");
        let hls = proto.starts_with("m3u8");
        if !(hls || proto == "https" || proto == "http") || f["ext"].as_str() == Some("mhtml") {
            continue;
        }
        let vcodec = f["vcodec"].as_str().unwrap_or("");
        let acodec = f["acodec"].as_str().unwrap_or("");
        let has_v = if vcodec.is_empty() { !is_none(&f["video_ext"]) } else { vcodec != "none" };
        let has_a = if acodec.is_empty() { !is_none(&f["audio_ext"]) } else { acodec != "none" };
        let kind = match (has_v, has_a) {
            (true, true) => "muxed",
            (true, false) => "video",
            (false, true) => "audio",
            _ => continue,
        };
        let height = f["height"].as_u64().unwrap_or(0);
        let fps = f["fps"].as_f64().unwrap_or(0.0).round() as u64;
        let tbr = f["tbr"].as_f64().or_else(|| f["abr"].as_f64()).unwrap_or(0.0);
        let (size, approx) = match (f["filesize"].as_u64(), f["filesize_approx"].as_u64()) {
            (Some(s), _) if s > 0 => (s, false),
            (_, Some(s)) if s > 0 => (s, true),
            _ => (0, false),
        };
        let ext = f["ext"].as_str().unwrap_or("mp4");
        let lang = f["language"].as_str().unwrap_or("");
        let codec = codec_family(if has_v { vcodec } else { acodec });
        let headers: Map<String, Value> = f["http_headers"]
            .as_object()
            .map(|o| o.iter().filter(|(_, v)| v.is_string()).map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let entry = json!({
            "id": f["format_id"].as_str().unwrap_or(""),
            "kind": kind,
            "ext": ext,
            "height": height,
            "fps": fps,
            "codec": codec,
            "lang": lang,
            "note": f["format_note"].as_str().unwrap_or(""),
            "hdr": f["dynamic_range"].as_str().map_or(false, |d| d != "SDR"),
            "hls": hls,
            "size": size,
            "approx": approx,
            "tbr": tbr,
            "url": url,
            "headers": headers,
        });
        let key = format!("{kind}|{height}|{}|{codec}|{ext}|{lang}|{hls}", if fps > 30 { 60 } else { 30 });
        let better = best.get(&key).map_or(true, |o| o["tbr"].as_f64().unwrap_or(0.0) < tbr);
        if better {
            best.insert(key, entry);
        }
    }
    let mut formats: Vec<Value> = best.into_values().collect();
    let rank = |k: &str| match k {
        "muxed" => 0,
        "video" => 1,
        _ => 2,
    };
    formats.sort_by(|a, b| {
        rank(a["kind"].as_str().unwrap_or(""))
            .cmp(&rank(b["kind"].as_str().unwrap_or("")))
            .then(b["height"].as_u64().cmp(&a["height"].as_u64()))
            .then(b["fps"].as_u64().cmp(&a["fps"].as_u64()))
            // a plain file beats an HLS stream of the same quality (faster, resumable)
            .then(a["hls"].as_bool().cmp(&b["hls"].as_bool()))
            .then(b["tbr"].as_f64().partial_cmp(&a["tbr"].as_f64()).unwrap_or(std::cmp::Ordering::Equal))
    });
    // Keep the audio list bounded: several dubbed tracks x codecs add up (the extension picks from these).
    let mut audio = 0;
    formats.retain(|f| f["kind"] != "audio" || {
        audio += 1;
        audio <= 12
    });
    if formats.is_empty() {
        return Err("yt-dlp found no downloadable formats for this page.".into());
    }
    Ok(json!({
        "ok": true,
        "id": info["id"].as_str().unwrap_or(""),
        "title": info["title"].as_str().unwrap_or(""),
        "uploader": info["uploader"].as_str().or(info["channel"].as_str()).unwrap_or(""),
        "duration": info["duration"].as_f64().unwrap_or(0.0),
        "formats": formats,
    }))
}

fn needs_login(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["log in", "login", "sign in", "cookies", "private", "age-restricted", "rate-limit"].iter().any(|k| m.contains(k))
}

const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_MAX: usize = 32;

type Cache = Mutex<Vec<(String, Instant, Value)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// One lock per address, so the same page asked twice at once (the extension's early lookup and the
/// click, or two windows) runs yt-dlp once and the second asker gets the first one's answer.
fn url_lock(url: &str) -> Arc<Mutex<()>> {
    static L: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut m = L.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 256 {
        m.retain(|_, l| Arc::strong_count(l) > 1);
    }
    m.entry(url.to_string()).or_default().clone()
}

/// Lists the formats of `url`. The error text is shown to the user as is. Answers are kept for five
/// minutes (the links in them stay valid for hours), failures are never kept.
pub fn formats(data_dir: &Path, url: &str, opts: &Opts) -> Result<Value, String> {
    let lock = url_lock(url);
    let _turn = lock.lock().unwrap_or_else(|e| e.into_inner());
    let lookup = |url: &str| {
        let c = cache().lock().unwrap_or_else(|e| e.into_inner());
        c.iter().find(|(u, t, _)| u == url && t.elapsed() < CACHE_TTL).map(|(_, _, v)| v.clone())
    };
    if let Some(v) = lookup(url) {
        return Ok(v);
    }
    let bin = find(data_dir).ok_or_else(|| missing_message(&[crate::tools::Tool::YtDlp]))?;
    let v = normalize(&run(&bin, data_dir, url, opts)?)?;
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|(u, t, _)| u != url && t.elapsed() < CACHE_TTL);
    if c.len() >= CACHE_MAX {
        c.remove(0);
    }
    c.push((url.to_string(), Instant::now(), v.clone()));
    Ok(v)
}

/// A cookie file that is deleted again when this goes out of scope.
pub struct TempCookies(pub PathBuf);

impl Drop for TempCookies {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes the cookies the browser extension sent (`[{domain, path, secure, expires, name, value}]`)
/// as a Netscape cookie file for `--cookies`. `None` when there is nothing usable.
pub fn write_cookie_file(data_dir: &Path, cookies: &Value) -> Option<TempCookies> {
    let list = cookies.as_array()?;
    let clean = |s: &str| !s.is_empty() && !s.contains(['\t', '\r', '\n']);
    let far = crate::util::now_ts() + 86_400;
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    let mut n = 0;
    for c in list.iter().take(500) {
        let (domain, name, value) = (c["domain"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""), c["value"].as_str().unwrap_or(""));
        let path = c["path"].as_str().filter(|p| p.starts_with('/')).unwrap_or("/");
        if !clean(domain) || !clean(name) || !clean(path) || value.contains(['\t', '\r', '\n']) {
            continue;
        }
        let exp = c["expires"].as_f64().filter(|e| *e > 0.0).map_or(far, |e| e as i64);
        text += &format!(
            "{domain}\t{}\t{path}\t{}\t{exp}\t{name}\t{value}\n",
            if domain.starts_with('.') { "TRUE" } else { "FALSE" },
            if c["secure"].as_bool().unwrap_or(false) { "TRUE" } else { "FALSE" },
        );
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let dir = data_dir.join("tmp");
    std::fs::create_dir_all(&dir).ok()?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = dir.join(format!("cookies-{}-{}.txt", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::write(&path, text).ok()?;
    Some(TempCookies(path))
}

// ───────────────────── choices: what the user picks from ─────────────────────

/// One row of the format list: a quality (picture + sound, joined by ffmpeg when they come as two
/// files) or an audio-only track. Mirrors `dcChoices` in the browser extension's `formats.js`.
#[derive(Clone, Debug)]
pub struct Choice {
    pub text: String,
    /// Short tag used in the file name (`720p`, `audio`).
    pub label: String,
    pub height: u64,
    pub fps: u64,
    pub audio_only: bool,
    pub ext: String,
    pub size: u64,
    pub approx: bool,
    pub tip: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Sound track to download and merge in (set when `url` is a video-only link).
    pub audio: Option<AudioPart>,
}

fn st<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn tbr_of(v: &Value) -> f64 {
    v["tbr"].as_f64().unwrap_or(0.0)
}

fn header_list(f: &Value, page: &str) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = f["headers"]
        .as_object()
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
        .unwrap_or_default();
    let origin = page_origin(page);
    if !origin.is_empty() {
        h.retain(|(k, _)| !k.eq_ignore_ascii_case("referer") && !k.eq_ignore_ascii_case("origin"));
        h.push(("Referer".into(), format!("{origin}/")));
        h.push(("Origin".into(), origin));
    }
    h
}

fn merged_ext(v: &str, a: &str) -> &'static str {
    match (v, a) {
        ("mp4", "m4a") => "mp4",
        ("webm", "webm") => "webm",
        _ => "mkv",
    }
}

fn pick_audio<'a>(v: &Value, audios: &[&'a Value]) -> Option<&'a Value> {
    let want = match st(v, "ext") {
        "mp4" => "m4a",
        "webm" => "webm",
        _ => "",
    };
    let direct: Vec<&&Value> = audios.iter().filter(|a| !a["hls"].as_bool().unwrap_or(false)).collect();
    let score = |a: &Value| {
        let note = st(a, "note").to_ascii_lowercase();
        (if st(a, "ext") == want { 2e6 } else { 0.0 })
            + (if note.contains("default") || note.contains("original") { 1e6 } else { 0.0 })
            + (if st(a, "lang").is_empty() { 1e5 } else { 0.0 })
            + tbr_of(a)
    };
    let pool: Vec<&Value> = if direct.is_empty() { audios.to_vec() } else { direct.into_iter().copied().collect() };
    pool.into_iter().max_by(|x, y| score(x).partial_cmp(&score(y)).unwrap_or(std::cmp::Ordering::Equal))
}

fn quality_tag(h: u64) -> &'static str {
    if h >= 2160 { " 4K" } else if h >= 720 { " HD" } else { "" }
}

/// Rows for the *Add download* dialog, best quality first. `res` is one result of `normalize`
/// (pick an entry of `entries` first for a post with several videos); `page` is the address that
/// was asked about (its origin becomes the Referer of the downloads).
pub fn choices(res: &Value, page: &str) -> Vec<Choice> {
    let all: Vec<&Value> = res["formats"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let audios: Vec<&Value> = all.iter().copied().filter(|f| st(f, "kind") == "audio").collect();
    let codec_rank = |c: &str| match c {
        "H.264" => 3,
        "VP9" => 2,
        "AV1" => 1,
        _ => 0,
    };
    let mut groups: BTreeMap<(u64, u64), Vec<&Value>> = BTreeMap::new();
    for f in all.iter().copied().filter(|f| st(f, "kind") != "audio") {
        let fps = f["fps"].as_u64().unwrap_or(0);
        groups.entry((f["height"].as_u64().unwrap_or(0), if fps > 30 { 60 } else { 30 })).or_default().push(f);
    }
    fn cmp_tbr(x: &Value, y: &Value) -> std::cmp::Ordering {
        tbr_of(x).partial_cmp(&tbr_of(y)).unwrap_or(std::cmp::Ordering::Equal)
    }
    let mut out: Vec<Choice> = Vec::new();
    for list in groups.values() {
        let direct: Vec<&Value> = list.iter().copied().filter(|f| !f["hls"].as_bool().unwrap_or(false)).collect();
        let mut v = direct.iter().filter(|f| st(f, "kind") == "muxed").max_by(|x, y| cmp_tbr(x, y)).copied();
        let mut a: Option<&Value> = None;
        if v.is_none() {
            v = direct
                .iter()
                .filter(|f| st(f, "kind") == "video")
                .max_by(|x, y| codec_rank(st(x, "codec")).cmp(&codec_rank(st(y, "codec"))).then(cmp_tbr(x, y)))
                .copied();
            if let Some(vv) = v {
                a = pick_audio(vv, &audios);
                if a.is_none() {
                    v = None; // never offer a silent file
                }
            }
        }
        if v.is_none() {
            v = list.iter().filter(|f| st(f, "kind") == "muxed").max_by(|x, y| cmp_tbr(x, y)).copied(); // HLS with sound
        }
        let Some(v) = v else { continue };
        let hls = v["hls"].as_bool().unwrap_or(false);
        let ext = match a {
            Some(a) => merged_ext(st(v, "ext"), st(a, "ext")).to_string(),
            None if hls => "ts".to_string(),
            None => if st(v, "ext").is_empty() { "mp4".into() } else { st(v, "ext").to_string() },
        };
        let (height, fps) = (v["height"].as_u64().unwrap_or(0), v["fps"].as_u64().unwrap_or(0));
        let fps_s = if fps > 30 { fps.to_string() } else { String::new() };
        let q = if height > 0 {
            format!("{height}p{fps_s}{}", quality_tag(height))
        } else if !st(v, "note").is_empty() {
            st(v, "note").to_string()
        } else {
            "video".to_string()
        };
        let label = if height > 0 { format!("{height}p{fps_s}") } else if !st(v, "note").is_empty() { st(v, "note").to_string() } else { "video".into() };
        let tip = [
            st(v, "codec").to_string(),
            if v["hdr"].as_bool().unwrap_or(false) { "HDR".into() } else { String::new() },
            a.map(|a| format!("+ {} audio (merged)", st(a, "ext").to_ascii_uppercase())).unwrap_or_default(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        out.push(Choice {
            text: format!("{} file, quality {q}", ext.to_ascii_uppercase()),
            label,
            height,
            fps,
            audio_only: false,
            ext,
            size: v["size"].as_u64().unwrap_or(0) + a.map_or(0, |a| a["size"].as_u64().unwrap_or(0)),
            approx: v["approx"].as_bool().unwrap_or(false) || a.is_some(),
            tip,
            url: st(v, "url").to_string(),
            headers: header_list(v, page),
            audio: a.map(|a| AudioPart {
                url: st(a, "url").to_string(),
                headers: header_list(a, page),
                size: a["size"].as_u64().unwrap_or(0),
                ext: if st(a, "ext").is_empty() { "m4a".into() } else { st(a, "ext").chars().filter(|c| c.is_ascii_alphanumeric()).take(5).collect() },
            }),
        });
    }
    out.sort_by(|x, y| y.height.cmp(&x.height).then(y.fps.cmp(&x.fps)));

    // audio only: the best track of each type (and language), highest bitrate first
    let mut seen: Vec<&Value> = Vec::new();
    for f in audios.iter().copied().filter(|f| !f["hls"].as_bool().unwrap_or(false)) {
        let same = |o: &&Value| st(o, "ext") == st(f, "ext") && st(o, "lang") == st(f, "lang");
        let rank = |x: &Value| {
            let n = st(x, "note").to_ascii_lowercase();
            (if n.contains("default") || n.contains("original") { 1e6 } else { 0.0 }) + tbr_of(x)
        };
        match seen.iter().position(same) {
            Some(i) if rank(f) > rank(seen[i]) => seen[i] = f,
            Some(_) => {}
            None => seen.push(f),
        }
    }
    seen.sort_by(|x, y| tbr_of(y).partial_cmp(&tbr_of(x)).unwrap_or(std::cmp::Ordering::Equal));
    for f in seen.into_iter().take(4) {
        let lang = st(f, "lang");
        out.push(Choice {
            text: format!(
                "{} audio{}{}",
                st(f, "ext").to_ascii_uppercase(),
                if lang.is_empty() { String::new() } else { format!(" ({lang})") },
                if tbr_of(f) > 0.0 { format!(", {}k", tbr_of(f).round() as u64) } else { String::new() }
            ),
            label: "audio".into(),
            height: 0,
            fps: 0,
            audio_only: true,
            ext: st(f, "ext").to_string(),
            size: f["size"].as_u64().unwrap_or(0),
            approx: f["approx"].as_bool().unwrap_or(false),
            tip: st(f, "codec").to_string(),
            url: st(f, "url").to_string(),
            headers: header_list(f, page),
            audio: None,
        });
    }
    out
}

/// `Title [720p].mp4` - the name a chosen row is saved under.
pub fn file_name(title: &str, uploader: &str, c: &Choice) -> String {
    let clean = |t: &str| -> String {
        let t: String = t.chars().map(|ch| if ch.is_control() || "<>:\"/\\|?*".contains(ch) { ' ' } else { ch }).collect();
        t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(100).collect::<String>().trim().to_string()
    };
    let mut base = clean(title);
    if base.is_empty() {
        base = clean(uploader);
    }
    if base.is_empty() {
        base = "video".into();
    }
    format!("{base} [{}].{}", c.label, if c.ext.is_empty() { "mp4" } else { &c.ext })
}

// ───────────────────── for the desktop app ─────────────────────

/// One video of a page (a post can hold several) with the rows to pick from.
#[derive(Clone, Debug)]
pub struct Video {
    pub title: String,
    pub uploader: String,
    pub duration: f64,
    pub choices: Vec<Choice>,
}

impl Video {
    /// The name a chosen row is saved under (`Title [720p].mp4`).
    pub fn file_name(&self, c: &Choice) -> String {
        file_name(&self.title, &self.uploader, c)
    }
}

/// Turns a `normalize` result into the videos it holds, dropping any without a downloadable row.
pub fn videos(res: &Value, page: &str) -> Vec<Video> {
    let one = |v: &Value| Video {
        title: st(v, "title").to_string(),
        uploader: st(v, "uploader").to_string(),
        duration: v["duration"].as_f64().unwrap_or(0.0),
        choices: choices(v, page),
    };
    let list: Vec<Video> = match res["entries"].as_array().filter(|e| !e.is_empty()) {
        Some(e) => e.iter().map(one).collect(),
        None => vec![one(res)],
    };
    list.into_iter().filter(|v| !v.choices.is_empty()).collect()
}

/// Asks yt-dlp about `url` and returns the videos found. Blocks for up to ~90 s: call it from a thread.
pub fn fetch(data_dir: &Path, url: &str, opts: &Opts) -> Result<Vec<Video>, String> {
    let res = formats(data_dir, url, opts)?;
    let v = videos(&res, url);
    if v.is_empty() {
        Err("yt-dlp found no downloadable formats for this page.".into())
    } else {
        Ok(v)
    }
}
