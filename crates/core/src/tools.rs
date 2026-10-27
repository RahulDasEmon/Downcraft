//! Helper programs DownCraft can set up for the user: **yt-dlp** (lists the formats of YouTube, Facebook, Instagram, X … pages), **ffmpeg**
//! (merging / converting) and **Deno** (the JavaScript runtime recent yt-dlp needs for YouTube).
//!
//! Managed copies live in `<data dir>/tools` and are downloaded from the projects' own GitHub
//! releases. Copies already on the system (`PATH`, next to the executable) are found and used too,
//! but only managed ones can be updated or removed from here - DownCraft never touches the rest.
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::Manager;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Deno,
}

pub const ALL: [Tool; 3] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Deno];

/// How a tool's release asset is packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pack {
    /// The download is the program itself.
    File,
    Zip,
    /// `.tar.xz` (Linux builds of ffmpeg).
    TarXz,
}

#[derive(Clone, Debug)]
pub struct Source {
    pub url: String,
    pub pack: Pack,
}

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Deno => "Deno",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Tool::YtDlp => "Finds the video and audio formats of YouTube, Facebook, Instagram, X and over a thousand other sites.",
            Tool::Ffmpeg => "Joins separate video and audio, converts formats. Needed for video-site downloads.",
            Tool::Deno => "JavaScript runtime that recent yt-dlp versions need to read YouTube.",
        }
    }

    /// Rough download size, shown before installing.
    pub fn approx_mb(self) -> u32 {
        match (self, cfg!(windows)) {
            (Tool::YtDlp, true) => 18,
            (Tool::YtDlp, false) => 40,
            (Tool::Ffmpeg, _) => 160,
            (Tool::Deno, _) => 42,
        }
    }

    /// Program names this tool installs (without `.exe`).
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Tool::YtDlp => &["yt-dlp"],
            Tool::Ffmpeg => &["ffmpeg", "ffprobe"],
            Tool::Deno => &["deno"],
        }
    }

    pub fn exe(self) -> &'static str {
        self.files()[0]
    }

    /// Where to get it on this system, or `None` when DownCraft can't install it here.
    pub fn source(self) -> Option<Source> {
        let (win, linux, mac) = (cfg!(windows), cfg!(target_os = "linux"), cfg!(target_os = "macos"));
        let (x64, arm) = (cfg!(target_arch = "x86_64"), cfg!(target_arch = "aarch64"));
        let gh = |repo: &str, file: &str| format!("https://github.com/{repo}/releases/latest/download/{file}");
        match self {
            Tool::YtDlp => {
                let asset = match (win, linux, mac, x64, arm) {
                    (true, _, _, true, _) => "yt-dlp.exe",
                    (true, _, _, _, true) => "yt-dlp_arm64.exe",
                    (_, true, _, true, _) => "yt-dlp_linux",
                    (_, true, _, _, true) => "yt-dlp_linux_aarch64",
                    (_, _, true, _, _) => "yt-dlp_macos",
                    _ => return None,
                };
                Some(Source { url: gh("yt-dlp/yt-dlp", asset), pack: Pack::File })
            }
            Tool::Deno => {
                let os = match (win, linux, mac) {
                    (true, _, _) => "pc-windows-msvc",
                    (_, true, _) => "unknown-linux-gnu",
                    (_, _, true) => "apple-darwin",
                    _ => return None,
                };
                let arch = if x64 {
                    "x86_64"
                } else if arm {
                    "aarch64"
                } else {
                    return None;
                };
                Some(Source { url: gh("denoland/deno", &format!("deno-{arch}-{os}.zip")), pack: Pack::Zip })
            }
            // Static builds published by the yt-dlp project. None exist for macOS: `brew install ffmpeg`.
            Tool::Ffmpeg => {
                let (plat, pack, ext) = match (win, linux, x64, arm) {
                    (true, _, true, _) => ("win64", Pack::Zip, "zip"),
                    (true, _, _, true) => ("winarm64", Pack::Zip, "zip"),
                    (_, true, true, _) => ("linux64", Pack::TarXz, "tar.xz"),
                    (_, true, _, true) => ("linuxarm64", Pack::TarXz, "tar.xz"),
                    _ => return None,
                };
                Some(Source { url: gh("yt-dlp/FFmpeg-Builds", &format!("ffmpeg-master-latest-{plat}-gpl.{ext}")), pack })
            }
        }
    }

    /// Shown when `source()` is `None`.
    pub fn manual_hint(self) -> &'static str {
        if cfg!(target_os = "macos") && self == Tool::Ffmpeg {
            "No ready-made download for macOS - install it with Homebrew:  brew install ffmpeg"
        } else {
            "No download is available for this system - install it yourself and put it on your PATH."
        }
    }
}

pub fn tools_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub path: PathBuf,
    /// Lives in DownCraft's tools folder, so it can be updated and removed from the app.
    pub managed: bool,
}

/// Finds a tool: DownCraft's tools folder first, then the data folder, the folder of the
/// executable and finally `PATH`.
pub fn locate(data_dir: &Path, tool: Tool) -> Option<Installed> {
    let managed_dir = tools_dir(data_dir);
    let mut dirs: Vec<PathBuf> = vec![managed_dir.clone(), data_dir.to_path_buf()];
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        dirs.push(d);
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    let names: Vec<String> = if tool == Tool::YtDlp && !cfg!(windows) {
        vec!["yt-dlp".into(), "yt-dlp_linux".into(), "yt-dlp_macos".into()]
    } else {
        vec![format!("{}{EXE}", tool.exe())]
    };
    for d in dirs {
        for n in &names {
            let p = d.join(n);
            if p.is_file() {
                return Some(Installed { managed: d == managed_dir, path: p });
            }
        }
    }
    None
}

fn hide_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

/// Asks the program for its version (short text), or `None` when it doesn't run.
pub fn version(path: &Path, tool: Tool) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg(if tool == Tool::Ffmpeg { "-version" } else { "--version" })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_window(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = out.by_ref().take(64 * 1024).read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > Duration::from_secs(15) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(30)),
        }
    }
    let text = reader.join().ok()?;
    let line = text.lines().next()?.trim();
    let v = match tool {
        Tool::YtDlp => line.to_string(),
        // "ffmpeg version N-123-gabc Copyright ..." / "deno 2.1.4 (stable, ...)"
        Tool::Ffmpeg => line.split_whitespace().nth(2)?.to_string(),
        Tool::Deno => line.split_whitespace().nth(1)?.to_string(),
    };
    Some(v.chars().take(40).collect())
}

// ───────────────────────── installing ─────────────────────────

fn replace(tmp: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() && fs::remove_file(dest).is_err() {
        // Windows won't delete a running program, but it lets it be renamed out of the way.
        let old = dest.with_extension("old");
        let _ = fs::remove_file(&old);
        fs::rename(dest, &old).map_err(|e| format!("{} is in use - close it and try again ({e})", dest.display()))?;
    }
    fs::rename(tmp, dest).map_err(|e| format!("could not place {}: {e}", dest.display()))
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_executable(_: &Path) {}

fn file_name_lower(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn extract_zip(archive: &Path, dir: &Path, tool: Tool) -> Result<(), String> {
    let f = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("the download is not a valid zip: {e}"))?;
    let mut found = 0;
    for name in tool.files() {
        let want = format!("{name}{EXE}");
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).map_err(|e| e.to_string())?;
            let inner = match e.enclosed_name() {
                Some(p) if !e.is_dir() => p.to_path_buf(),
                _ => continue,
            };
            if file_name_lower(&inner) != want {
                continue;
            }
            let tmp = dir.join(format!("{want}.part"));
            let mut out = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut out).map_err(|e| e.to_string())?;
            out.flush().ok();
            drop(out);
            make_executable(&tmp);
            replace(&tmp, &dir.join(&want))?;
            found += 1;
            break;
        }
    }
    // ffprobe is a bonus; the main program must be there.
    if found == 0 {
        return Err(format!("{} was not found inside the download", tool.exe()));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_tar_xz(archive: &Path, dir: &Path, tool: Tool) -> Result<(), String> {
    let f = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(xz2::read::XzDecoder::new(std::io::BufReader::new(f)));
    let mut found = 0;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let name = file_name_lower(&path);
        if !tool.files().contains(&name.as_str()) || path.parent().and_then(|p| p.file_name()).map_or(true, |d| d != "bin") {
            continue;
        }
        let tmp = dir.join(format!("{name}.part"));
        let mut out = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        drop(out);
        make_executable(&tmp);
        replace(&tmp, &dir.join(&name))?;
        found += 1;
    }
    if found == 0 {
        return Err(format!("{} was not found inside the download", tool.exe()));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn extract_tar_xz(_: &Path, _: &Path, _: Tool) -> Result<(), String> {
    Err("this archive type is not supported on this system".into())
}

/// Live numbers of a running install, polled by the UI.
#[derive(Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicU64,
    pub total: std::sync::atomic::AtomicU64,
    pub extracting: AtomicBool,
    pub cancel: AtomicBool,
}

/// An error with its causes ("error sending request: ... certificate ...") so the user sees why.
fn describe(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let t = c.to_string();
        if !out.contains(&t) {
            out.push_str(": ");
            out.push_str(&t);
        }
        cur = c.source();
    }
    out
}

/// Downloads and unpacks `tool` into the tools folder, then checks that it really runs.
pub fn install(client: &reqwest::blocking::Client, data_dir: &Path, tool: Tool, p: &Progress) -> Result<(), String> {
    let src = tool.source().ok_or_else(|| tool.manual_hint().to_string())?;
    let dir = tools_dir(data_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{}.download", tool.exe()));
    let result = (|| -> Result<(), String> {
        let mut resp = client.get(&src.url).send().map_err(|e| format!("download failed: {}", describe(&e)))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: server answered {}", resp.status()));
        }
        p.total.store(resp.content_length().unwrap_or(0), Relaxed);
        let mut out = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if p.cancel.load(Relaxed) {
                return Err("cancelled".into());
            }
            let n = resp.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            p.done.fetch_add(n as u64, Relaxed);
        }
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        let total = p.total.load(Relaxed);
        if total > 0 && p.done.load(Relaxed) != total {
            return Err("download ended early".into());
        }
        p.extracting.store(true, Relaxed);
        match src.pack {
            Pack::File => {
                make_executable(&tmp);
                let dest = dir.join(format!("{}{EXE}", tool.exe()));
                replace(&tmp, &dest)
            }
            Pack::Zip => extract_zip(&tmp, &dir, tool),
            Pack::TarXz => extract_tar_xz(&tmp, &dir, tool),
        }
    })();
    let _ = fs::remove_file(&tmp);
    result?;
    // A wrong-architecture or truncated file must not be left behind looking installed.
    let installed = locate_managed(data_dir, tool).ok_or("installed file not found")?;
    if version(&installed, tool).is_none() {
        let _ = remove(data_dir, tool);
        return Err("the downloaded program doesn't run on this system".into());
    }
    Ok(())
}

fn locate_managed(data_dir: &Path, tool: Tool) -> Option<PathBuf> {
    let p = tools_dir(data_dir).join(format!("{}{EXE}", tool.exe()));
    p.is_file().then_some(p)
}

/// Deletes DownCraft's own copy (never one found elsewhere on the system).
pub fn remove(data_dir: &Path, tool: Tool) -> Result<(), String> {
    let dir = tools_dir(data_dir);
    let mut err = None;
    for name in tool.files() {
        let exe = dir.join(format!("{name}{EXE}"));
        let part = dir.join(format!("{name}{EXE}.part"));
        let old = exe.with_extension("old");
        for p in [exe, part, old] {
            if p.exists() {
                if let Err(e) = fs::remove_file(&p) {
                    err = Some(format!("could not delete {}: {e} (is it running?)", p.display()));
                }
            }
        }
    }
    let _ = fs::remove_file(dir.join(format!("{}.download", tool.exe())));
    err.map_or(Ok(()), Err)
}

// ───────────────────────── state for the UI ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Idle,
    Checking,
    Installing,
    Removing,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub installed: Option<Installed>,
    pub version: Option<String>,
    pub busy: Busy,
    pub done: u64,
    pub total: u64,
    pub extracting: bool,
    /// Last result: `(is_error, text)`.
    pub message: Option<(bool, String)>,
}

impl Entry {
    fn new() -> Self {
        Entry { installed: None, version: None, busy: Busy::Idle, done: 0, total: 0, extracting: false, message: None }
    }
}

/// Runs installs/removals on background threads and lets the UI poll the result.
pub struct ToolBox {
    mgr: Arc<Manager>,
    entries: Mutex<HashMap<Tool, Entry>>,
    progress: Mutex<HashMap<Tool, Arc<Progress>>>,
}

impl ToolBox {
    pub fn new(mgr: Arc<Manager>) -> Arc<Self> {
        let tb = Arc::new(ToolBox {
            mgr,
            entries: Mutex::new(ALL.iter().map(|t| (*t, Entry::new())).collect()),
            progress: Mutex::new(HashMap::new()),
        });
        tb.refresh();
        tb
    }

    pub fn data_dir(&self) -> PathBuf {
        self.mgr.data_dir().to_path_buf()
    }

    fn set(&self, tool: Tool, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.entries.lock().get_mut(&tool) {
            f(e);
        }
    }

    /// Current state of every tool (live numbers for running installs included).
    pub fn snapshot(&self) -> Vec<(Tool, Entry)> {
        let prog = self.progress.lock().clone();
        let entries = self.entries.lock();
        ALL.iter()
            .map(|t| {
                let mut e = entries[t].clone();
                if e.busy == Busy::Installing {
                    if let Some(p) = prog.get(t) {
                        e.done = p.done.load(Relaxed);
                        e.total = p.total.load(Relaxed);
                        e.extracting = p.extracting.load(Relaxed);
                    }
                }
                (*t, e)
            })
            .collect()
    }

    pub fn is_busy(&self) -> bool {
        self.entries.lock().values().any(|e| e.busy != Busy::Idle)
    }

    /// Looks for every tool again (and asks each for its version) in the background.
    pub fn refresh(self: &Arc<Self>) {
        for tool in ALL {
            self.recheck(tool, None);
        }
    }

    fn recheck(self: &Arc<Self>, tool: Tool, message: Option<(bool, String)>) {
        {
            let mut m = self.entries.lock();
            let e = m.get_mut(&tool).unwrap();
            if e.busy == Busy::Checking || e.busy == Busy::Installing || e.busy == Busy::Removing {
                return;
            }
            e.busy = Busy::Checking;
        }
        let me = self.clone();
        thread::spawn(move || {
            let found = locate(me.mgr.data_dir(), tool);
            let ver = found.as_ref().and_then(|i| version(&i.path, tool));
            me.set(tool, |e| {
                e.installed = found;
                e.version = ver;
                e.busy = Busy::Idle;
                if message.is_some() {
                    e.message = message;
                }
            });
        });
    }

    pub fn install(self: &Arc<Self>, tool: Tool) {
        {
            let mut m = self.entries.lock();
            let e = m.get_mut(&tool).unwrap();
            if e.busy == Busy::Installing || e.busy == Busy::Removing {
                return;
            }
            e.busy = Busy::Installing;
            e.message = None;
        }
        let p = Arc::new(Progress::default());
        self.progress.lock().insert(tool, p.clone());
        let me = self.clone();
        thread::spawn(move || {
            let res = install(&me.mgr.client(), me.mgr.data_dir(), tool, &p);
            me.set(tool, |e| e.busy = Busy::Idle);
            let msg = match res {
                Ok(()) => (false, format!("{} installed.", tool.name())),
                Err(e) if e == "cancelled" => (false, "Cancelled.".to_string()),
                Err(e) => (true, e),
            };
            me.recheck(tool, Some(msg));
        });
    }

    pub fn cancel(&self, tool: Tool) {
        if let Some(p) = self.progress.lock().get(&tool) {
            p.cancel.store(true, Relaxed);
        }
    }

    pub fn remove(self: &Arc<Self>, tool: Tool) {
        {
            let mut m = self.entries.lock();
            let e = m.get_mut(&tool).unwrap();
            if e.busy == Busy::Installing || e.busy == Busy::Removing {
                return;
            }
            e.busy = Busy::Removing;
            e.message = None;
        }
        let me = self.clone();
        thread::spawn(move || {
            let res = remove(me.mgr.data_dir(), tool);
            me.set(tool, |e| e.busy = Busy::Idle);
            let msg = match res {
                Ok(()) => (false, format!("{} removed.", tool.name())),
                Err(e) => (true, e),
            };
            me.recheck(tool, Some(msg));
        });
    }
}
