//! Segmented HTTP engine.
//!
//! A *supervisor* thread prepares the file and starts a pool of *workers*. Workers claim segments
//! on demand; when nothing is left to claim, an idle worker splits the biggest segment that is
//! still being downloaded in half (FluxDown's "in-half division rule"), so a slow connection never
//! leaves the others idle at the end. Each connection lives in its own reader thread and talks to
//! the worker over a channel, which gives us stall detection and instant pause without async.
use crate::{
    manager::{Event, Manager},
    task::*,
    util, Status,
};
use parking_lot::Mutex;
use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{
        ACCEPT_ENCODING, CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE,
        RETRY_AFTER,
    },
};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{sync_channel, Receiver, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Smallest piece the initial plan creates.
const MIN_SEG: u64 = 256 * 1024;
/// A segment is only split while both halves would stay at least this big.
const MIN_SPLIT: u64 = 512 * 1024;
/// Upper bound on segments per download, as a multiple of the connection count.
const SEGS_PER_CONN: usize = 8;
const MAX_TRIES: u32 = 6;
/// Waiting for response headers.
const HEAD_WAIT: Duration = Duration::from_secs(45);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
pub struct Probe {
    pub file_name: String,
    pub total: u64,
    pub resumable: bool,
    pub content_type: Option<String>,
    /// `.m3u8` playlist – downloaded segment by segment.
    pub hls: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Only these headers are ever forwarded from the browser (never Range/Host/etc).
pub(crate) fn with_headers(mut req: RequestBuilder, headers: &[(String, String)]) -> RequestBuilder {
    for (k, v) in headers {
        let k = k.to_ascii_lowercase();
        let ok_key = matches!(k.as_str(), "referer" | "user-agent" | "cookie" | "origin" | "authorization");
        let ok_val = !v.is_empty() && v.bytes().all(|b| b >= 32 && b != 127);
        if ok_key && ok_val {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    req
}

fn header_str(resp: &reqwest::blocking::Response, name: reqwest::header::HeaderName) -> Option<String> {
    resp.headers().get(name).map(|v| util::decode_header_bytes(v.as_bytes()))
}

/// Strong validators only: a weak `W/` ETag can change between CDN nodes without the file changing.
fn strong(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|v| !v.starts_with("W/") && !v.is_empty())
}

/// Asks the server for `bytes=0-0` so we learn size, range support and the file name in one request.
pub fn probe(client: &Client, url: &str, headers: &[(String, String)]) -> Result<Probe, String> {
    let resp = with_headers(client.get(url), headers)
        .header(RANGE, "bytes=0-0")
        .header(ACCEPT_ENCODING, "identity")
        .timeout(PROBE_TIMEOUT)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let range_total = header_str(&resp, CONTENT_RANGE)
        .and_then(|v| v.rsplit('/').next().map(|t| t.trim().to_string()))
        .and_then(|v| v.parse::<u64>().ok());
    // `bytes=0-0` on an empty file is unsatisfiable: that is a valid, zero-byte download, not an error.
    let empty_file = status.as_u16() == 416 && range_total == Some(0);
    if !status.is_success() && !empty_file {
        return Err(format!("Server returned HTTP {}", status.as_u16()));
    }
    let ct_lower = header_str(&resp, CONTENT_TYPE).unwrap_or_default().to_ascii_lowercase();
    let hls = reqwest::Url::parse(url).map_or(false, |u| u.path().to_ascii_lowercase().ends_with(".m3u8"))
        || ct_lower.contains("mpegurl");
    let (total, resumable) = if hls {
        (0, true)
    } else if empty_file {
        (0, false)
    } else if status.as_u16() == 206 {
        let total = range_total.unwrap_or(0);
        (total, total > 0)
    } else {
        (resp.content_length().unwrap_or(0), false)
    };
    let mut file_name = resp
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|v| util::file_name_from_disposition_bytes(v.as_bytes()))
        .unwrap_or_else(|| util::file_name_from_url(resp.url().as_str()));
    if hls {
        file_name = util::hls_name(&file_name);
    }
    Ok(Probe {
        file_name,
        total,
        resumable,
        content_type: header_str(&resp, CONTENT_TYPE),
        hls,
        etag: header_str(&resp, ETAG),
        last_modified: header_str(&resp, LAST_MODIFIED),
    })
}

pub(crate) fn fail_with(mgr: &Arc<Manager>, task: &Arc<Task>, epoch: u64, msg: String, transient: bool) {
    if task.epoch.load(Relaxed) != epoch {
        return;
    }
    task.ctl.store(CTL_STOP, Relaxed);
    task.speed.store(0, Relaxed);
    {
        let mut i = task.info.lock();
        i.status = Status::Error;
        i.error = Some(msg);
    }
    task.transient.store(transient, Relaxed);
    let n = task.retries.load(Relaxed);
    task.retry_at.store(util::now_ts() + (5i64 << n.min(5)).min(120), Relaxed);
    mgr.push_event(Event::Failed(task.id));
    mgr.save();
}

/// Permanent failure (bad URL, 404, disk full, DRM…): retrying would not help.
pub(crate) fn fail(mgr: &Arc<Manager>, task: &Arc<Task>, epoch: u64, msg: String) {
    fail_with(mgr, task, epoch, msg, false)
}

// ───────────────────────────── connections ─────────────────────────────

enum Msg {
    Head { status: u16, retry_after: Option<u64> },
    Data(Vec<u8>),
    Eof,
    Fail(String),
}

/// One HTTP request running in its own thread. Dropping it tells the thread to stop and closes
/// the channel, so a worker never has to wait on a blocked socket.
struct Conn {
    rx: Receiver<Msg>,
    abort: Arc<AtomicBool>,
}

impl Conn {
    fn open(req: RequestBuilder) -> Conn {
        let (tx, rx) = sync_channel::<Msg>(8);
        let abort = Arc::new(AtomicBool::new(false));
        let a = abort.clone();
        thread::spawn(move || {
            let mut resp = match req.send() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Msg::Fail(e.to_string()));
                    return;
                }
            };
            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            if tx.send(Msg::Head { status: status.as_u16(), retry_after }).is_err() || !status.is_success() {
                return;
            }
            let mut buf = vec![0u8; 64 * 1024];
            while !a.load(Relaxed) {
                match resp.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(Msg::Eof);
                        return;
                    }
                    Ok(n) => {
                        if tx.send(Msg::Data(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Fail(e.to_string()));
                        return;
                    }
                }
            }
        });
        Conn { rx, abort }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.abort.store(true, Relaxed);
    }
}

enum Wait {
    Dead,
    Stall,
}

fn recv(conn: &Conn, alive: impl Fn() -> bool, limit: Duration) -> Result<Msg, Wait> {
    let t0 = Instant::now();
    loop {
        if !alive() {
            return Err(Wait::Dead);
        }
        match conn.rx.recv_timeout(Duration::from_millis(200)) {
            Ok(m) => return Ok(m),
            Err(RecvTimeoutError::Timeout) => {
                if t0.elapsed() > limit {
                    return Err(Wait::Stall);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(Msg::Fail("Connection closed".into())),
        }
    }
}

// ───────────────────────────── shared run state ─────────────────────────────

struct Ctx {
    mgr: Arc<Manager>,
    task: Arc<Task>,
    epoch: u64,
    client: Client,
    hdrs: Vec<(String, String)>,
    part: PathBuf,
    resumable: bool,
    host: String,
    max_segs: usize,
    fatal: Mutex<Option<(String, bool)>>,
    /// The server answered a ranged resume with something that is not the file we started.
    changed: AtomicBool,
}

impl Ctx {
    fn alive(&self) -> bool {
        self.task.epoch.load(Relaxed) == self.epoch
            && self.task.ctl.load(Relaxed) == CTL_RUN
            && self.fatal.lock().is_none()
    }

    /// No bytes for this long on an open connection = dead connection.
    fn stall(&self) -> Duration {
        Duration::from_secs(self.mgr.stall_secs.load(Relaxed).max(1))
    }

    fn superseded(&self) -> bool {
        self.task.epoch.load(Relaxed) != self.epoch
    }

    fn set_fatal(&self, seg: &Seg, msg: String, transient: bool) {
        seg.state.store(SEG_ERROR, Relaxed);
        let mut f = self.fatal.lock();
        if f.is_none() {
            *f = Some((msg, transient));
        }
    }

    /// Sleeps `d`, waking early when the download is paused or has failed.
    fn sleep(&self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end && self.alive() {
            thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
        }
    }

    /// Blocks long enough to honour both the per-download and the global speed limit.
    fn throttle(&self, n: u64) {
        let task_bps = self.task.limit_kbps.load(Relaxed).saturating_mul(1024);
        self.mgr.account(n);
        let a = self.task.bucket.reserve(n, task_bps);
        let b = self.mgr.global_bucket.reserve(n, self.mgr.global_limit_bps());
        let w = a.max(b);
        if !w.is_zero() {
            self.sleep(w);
        }
    }

    /// Counts a failed attempt; `Err` carries the message once the retry budget is spent.
    fn backoff(&self, tries: &mut u32, msg: String) -> Result<(), String> {
        *tries += 1;
        if *tries > MAX_TRIES {
            return Err(msg);
        }
        self.sleep(Duration::from_secs((*tries).min(5) as u64));
        Ok(())
    }
}

struct LiveGuard<'a> {
    task: &'a Task,
    armed: bool,
}

impl Drop for LiveGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.task.live_workers.fetch_sub(1, Relaxed);
        }
    }
}

enum Claim {
    Seg(Arc<Seg>),
    Wait,
    AllDone,
}

/// Hands out the next idle segment, or splits the biggest active one in half.
fn claim(ctx: &Ctx) -> Claim {
    let mut all_done = true;
    let snapshot: Vec<Arc<Seg>> = ctx.task.segs.read().clone();
    for s in &snapshot {
        match s.state.load(Relaxed) {
            SEG_DONE => {}
            SEG_IDLE => {
                if s.state.compare_exchange(SEG_IDLE, SEG_CONNECTING, Relaxed, Relaxed).is_ok() {
                    return Claim::Seg(s.clone());
                }
                all_done = false;
            }
            _ => all_done = false,
        }
    }
    if all_done {
        return Claim::AllDone;
    }
    if !ctx.resumable {
        return Claim::Wait;
    }
    match split(ctx) {
        Some(s) => Claim::Seg(s),
        None => Claim::Wait,
    }
}

/// In-half division: shrink the segment with the most bytes left and give its tail to the caller.
/// The victim notices its smaller `end` on its next chunk; bytes it may still write past the new
/// end are identical to what the new segment writes, so overlap is harmless.
fn split(ctx: &Ctx) -> Option<Arc<Seg>> {
    let _g = ctx.task.split_lock.lock();
    let segs: Vec<Arc<Seg>> = ctx.task.segs.read().clone();
    if segs.len() >= ctx.max_segs {
        return None;
    }
    let mut best: Option<(usize, u64)> = None;
    for (i, s) in segs.iter().enumerate() {
        if !matches!(s.state.load(Relaxed), SEG_CONNECTING | SEG_RECEIVING) {
            continue;
        }
        let end = s.end.load(Relaxed);
        let pos = s.pos.load(Relaxed);
        if end == UNKNOWN_END || pos > end {
            continue;
        }
        let rem = end + 1 - pos;
        if best.map_or(true, |(_, r)| rem > r) {
            best = Some((i, rem));
        }
    }
    let (idx, rem) = best?;
    if rem < 2 * MIN_SPLIT {
        return None;
    }
    let victim = &segs[idx];
    let pos = victim.pos.load(Relaxed);
    let end = victim.end.load(Relaxed);
    if pos > end || end + 1 - pos < 2 * MIN_SPLIT {
        return None;
    }
    let mid = pos + (end + 1 - pos) / 2;
    victim.end.store(mid - 1, Relaxed);
    let fresh = Arc::new(Seg::new(mid, end, mid));
    fresh.state.store(SEG_CONNECTING, Relaxed);
    let mut w = ctx.task.segs.write();
    let at = w.iter().position(|s| Arc::ptr_eq(s, victim)).map_or(w.len(), |p| p + 1);
    w.insert(at, fresh.clone());
    Some(fresh)
}

enum Outcome {
    Done,
    /// Give the segment back (state = idle) so another worker can take it.
    Released,
    Abort,
}

fn worker(ctx: Arc<Ctx>) {
    let task = &ctx.task;
    let mut guard = LiveGuard { task, armed: true };
    loop {
        if !ctx.alive() {
            return;
        }
        // The server refused parallel connections: surplus workers retire one by one.
        let cap = task.conn_cap.load(Relaxed);
        if cap > 0 {
            let live = task.live_workers.load(Relaxed);
            if live > cap && task.live_workers.compare_exchange(live, live - 1, Relaxed, Relaxed).is_ok() {
                guard.armed = false;
                return;
            }
        }
        match claim(&ctx) {
            Claim::AllDone => return,
            Claim::Wait => ctx.sleep(Duration::from_millis(100)),
            Claim::Seg(s) => match run_seg(&ctx, &s) {
                Outcome::Done | Outcome::Abort => {}
                Outcome::Released => s.state.store(SEG_IDLE, Relaxed),
            },
        }
    }
}

fn is_disk_full(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(28) | Some(112) | Some(39) | Some(122))
}

fn run_seg(ctx: &Ctx, seg: &Arc<Seg>) -> Outcome {
    let task = &ctx.task;
    let mut tries = 0u32;
    let validator = {
        let i = task.info.lock();
        let ignore_date = ctx.mgr.settings.lock().ignore_modified_on_resume;
        strong(&i.etag)
            .map(String::from)
            .or_else(|| if ignore_date { None } else { i.last_modified.clone() })
    };

    'outer: loop {
        if !ctx.alive() {
            return Outcome::Abort;
        }
        if !ctx.resumable {
            seg.pos.store(seg.start, Relaxed);
        }
        let mut pos = seg.pos.load(Relaxed);
        let end = seg.end.load(Relaxed);
        if end != UNKNOWN_END && pos > end {
            seg.state.store(SEG_DONE, Relaxed);
            return Outcome::Done;
        }
        seg.state.store(SEG_CONNECTING, Relaxed);

        let mut req = with_headers(ctx.client.get(&task.url), &ctx.hdrs).header(ACCEPT_ENCODING, "identity");
        if ctx.resumable {
            let r = if end == UNKNOWN_END { format!("bytes={pos}-") } else { format!("bytes={pos}-{end}") };
            req = req.header(RANGE, r);
            if let Some(v) = &validator {
                req = req.header(IF_RANGE, v.as_str());
            }
        }
        let conn = Conn::open(req);

        let (status, retry_after) = match recv(&conn, || ctx.alive(), HEAD_WAIT) {
            Err(Wait::Dead) => return Outcome::Abort,
            Err(Wait::Stall) => {
                if let Err(m) = ctx.backoff(&mut tries, "Server did not respond".into()) {
                    ctx.set_fatal(seg, m, true);
                    return Outcome::Abort;
                }
                continue 'outer;
            }
            Ok(Msg::Head { status, retry_after }) => (status, retry_after),
            Ok(Msg::Fail(e)) => {
                if let Err(m) = ctx.backoff(&mut tries, e) {
                    ctx.set_fatal(seg, m, true);
                    return Outcome::Abort;
                }
                continue 'outer;
            }
            Ok(_) => continue 'outer,
        };

        // Servers that throttle by connection count answer extra connections with 429/403/503.
        // Back off to fewer connections for this task *and* remember it for the host.
        if matches!(status, 429 | 403 | 503) && ctx.resumable {
            let live = task.live_workers.load(Relaxed);
            if live > 1 {
                let cap = live - 1;
                task.conn_cap.store(cap, Relaxed);
                ctx.mgr.note_host_cap(&ctx.host, cap);
                ctx.sleep(Duration::from_secs(retry_after.unwrap_or(2).min(15)));
                return Outcome::Released;
            }
        }
        if status >= 500 || status == 429 {
            if let Err(m) = ctx.backoff(&mut tries, format!("Server returned HTTP {status}")) {
                ctx.set_fatal(seg, m, true);
                return Outcome::Abort;
            }
            continue 'outer;
        }
        if status == 416 && ctx.resumable {
            ctx.changed.store(true, Relaxed);
            ctx.set_fatal(seg, "The file on the server changed".into(), false);
            return Outcome::Abort;
        }
        if !(200..300).contains(&status) {
            ctx.set_fatal(seg, format!("Server returned HTTP {status}"), false);
            return Outcome::Abort;
        }
        if ctx.resumable && status != 206 {
            // With If-Range a plain 200 means "the validator no longer matches": a different file.
            if validator.is_some() && status == 200 {
                ctx.changed.store(true, Relaxed);
                ctx.set_fatal(seg, "The file on the server changed".into(), false);
            } else {
                ctx.set_fatal(seg, "Server stopped honouring range requests".into(), false);
            }
            return Outcome::Abort;
        }

        let mut file = match OpenOptions::new().write(true).open(&ctx.part) {
            Ok(f) => f,
            Err(e) => {
                ctx.set_fatal(seg, format!("Cannot open file: {e}"), false);
                return Outcome::Abort;
            }
        };
        if let Err(e) = file.seek(SeekFrom::Start(pos)) {
            ctx.set_fatal(seg, format!("Cannot seek: {e}"), false);
            return Outcome::Abort;
        }
        seg.state.store(SEG_RECEIVING, Relaxed);

        loop {
            let msg = match recv(&conn, || ctx.alive(), ctx.stall()) {
                Err(Wait::Dead) => return Outcome::Abort,
                Err(Wait::Stall) => {
                    if let Err(m) = ctx.backoff(&mut tries, "Connection stalled".into()) {
                        ctx.set_fatal(seg, m, true);
                        return Outcome::Abort;
                    }
                    continue 'outer;
                }
                Ok(m) => m,
            };
            match msg {
                Msg::Head { .. } => {}
                Msg::Data(d) => {
                    if ctx.superseded() || !ctx.alive() {
                        return Outcome::Abort; // never write after being superseded
                    }
                    let end = seg.end.load(Relaxed); // may have shrunk: another worker took our tail
                    let mut n = d.len() as u64;
                    if end != UNKNOWN_END {
                        if pos > end {
                            seg.state.store(SEG_DONE, Relaxed);
                            return Outcome::Done;
                        }
                        n = n.min(end + 1 - pos);
                    }
                    if let Err(e) = file.write_all(&d[..n as usize]) {
                        let m = if is_disk_full(&e) { "The disk is full".to_string() } else { format!("Write failed: {e}") };
                        ctx.set_fatal(seg, m, false);
                        return Outcome::Abort;
                    }
                    pos += n;
                    if !ctx.superseded() {
                        seg.pos.store(pos, Relaxed);
                    }
                    tries = 0;
                    if end != UNKNOWN_END && pos > end {
                        seg.state.store(SEG_DONE, Relaxed);
                        return Outcome::Done;
                    }
                    ctx.throttle(n);
                }
                Msg::Eof => {
                    let end = seg.end.load(Relaxed);
                    if end == UNKNOWN_END || pos > end {
                        seg.state.store(SEG_DONE, Relaxed);
                        return Outcome::Done;
                    }
                    if let Err(m) = ctx.backoff(&mut tries, "Connection closed early".into()) {
                        ctx.set_fatal(seg, m, true);
                        return Outcome::Abort;
                    }
                    continue 'outer; // reconnect from `pos`
                }
                Msg::Fail(e) => {
                    if let Err(m) = ctx.backoff(&mut tries, format!("Connection lost: {e}")) {
                        ctx.set_fatal(seg, m, true);
                        return Outcome::Abort;
                    }
                    continue 'outer;
                }
            }
        }
    }
}

// ───────────────────────────── supervisor ─────────────────────────────

/// Waits (briefly) for workers of an earlier, superseded run to leave, so two runs never share a file.
fn wait_for_old_workers(task: &Task) {
    let t0 = Instant::now();
    while task.live_workers.load(Relaxed) > 0 && t0.elapsed() < Duration::from_secs(8) {
        thread::sleep(Duration::from_millis(30));
    }
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())).unwrap_or_default()
}

enum Flow {
    Finished,
    /// The server file changed: partial data was dropped, go around once more.
    Restart,
}

fn discard_partial(task: &Task) {
    let _ = std::fs::remove_file(task.part_path());
    task.segs.write().clear();
}

/// Supervisor thread for one task: prepares the file, runs the worker pool and watches it.
pub(crate) fn run(mgr: Arc<Manager>, task: Arc<Task>, epoch: u64) {
    wait_for_old_workers(&task);
    if task.epoch.load(Relaxed) != epoch {
        return;
    }
    for round in 0..2 {
        match run_once(&mgr, &task, epoch, round == 0) {
            Flow::Finished => return,
            Flow::Restart => {}
        }
    }
    fail(&mgr, &task, epoch, "The file on the server keeps changing".into());
}

fn run_once(mgr: &Arc<Manager>, task: &Arc<Task>, epoch: u64, may_restart: bool) -> Flow {
    let client = mgr.client();
    let saved_hdrs = task.info.lock().headers.clone();
    let hdrs = mgr.with_auth(&task.url, &saved_hdrs);
    let (probed, hls) = {
        let i = task.info.lock();
        (i.probed, i.hls)
    };
    let had_progress = task.downloaded() > 0;

    // First run: learn size / range support / name. Later runs with partial data: ask again and make
    // sure it is still the same file before appending to it.
    if !probed || (had_progress && !hls) {
        match probe(&client, &task.url, &hdrs) {
            Ok(p) => {
                let changed = probed && {
                    let i = task.info.lock();
                    (p.total != task.total() && !p.hls && p.total > 0)
                        || matches!((strong(&i.etag), strong(&p.etag)), (Some(a), Some(b)) if a != b)
                };
                if changed {
                    discard_partial(task);
                }
                let mut i = task.info.lock();
                i.resumable = p.resumable;
                i.probed = true;
                i.hls = p.hls;
                i.etag = p.etag.clone();
                i.last_modified = p.last_modified.clone();
                if i.file_name.is_empty() {
                    i.file_name = util::unique_name(&i.dir, &p.file_name);
                }
                drop(i);
                task.total.store(p.total, Relaxed);
            }
            Err(e) => {
                let transient = !e.starts_with("Server returned HTTP 4");
                fail_with(mgr, task, epoch, e, transient);
                return Flow::Finished;
            }
        }
    }
    if task.epoch.load(Relaxed) != epoch {
        return Flow::Finished;
    }
    if task.info.lock().hls {
        crate::hls::run(mgr.clone(), task.clone(), epoch, client, hdrs);
        return Flow::Finished;
    }

    let resumable = task.info.lock().resumable;
    let dir = task.info.lock().dir.clone();
    let part = task.part_path();
    let host = host_of(&task.url);
    let connections = (mgr.settings.lock().connections_for(&host) as usize)
        .min(mgr.host_cap(&host).unwrap_or(32) as usize)
        .max(1);

    if task.segs.read().is_empty() || !part.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            fail(mgr, task, epoch, format!("Cannot create folder: {e}"));
            return Flow::Finished;
        }
        let total = task.total();
        if total > 0 {
            if let Some(free) = util::free_space(&dir) {
                if free < total.saturating_add(1024 * 1024) {
                    fail(
                        mgr,
                        task,
                        epoch,
                        format!("Not enough disk space: {} needed, {} free", util::fmt_size(total), util::fmt_size(free)),
                    );
                    return Flow::Finished;
                }
            }
        }
        match OpenOptions::new().create(true).write(true).truncate(true).open(&part) {
            Ok(f) => {
                if total > 0 {
                    if let Err(e) = f.set_len(total) {
                        let m = if is_disk_full(&e) { "The disk is full".to_string() } else { format!("Cannot allocate file: {e}") };
                        fail(mgr, task, epoch, m);
                        return Flow::Finished;
                    }
                }
            }
            Err(e) => {
                fail(mgr, task, epoch, format!("Cannot create file: {e}"));
                return Flow::Finished;
            }
        }
        let mut segs = Vec::new();
        if total > 0 && resumable {
            let n = connections.min(((total + MIN_SEG - 1) / MIN_SEG).max(1) as usize);
            let base = total / n as u64;
            for k in 0..n as u64 {
                let s = k * base;
                let e = if k == n as u64 - 1 { total - 1 } else { s + base - 1 };
                segs.push(Arc::new(Seg::new(s, e, s)));
            }
        } else if total > 0 {
            segs.push(Arc::new(Seg::new(0, total - 1, 0)));
        } else {
            segs.push(Arc::new(Seg::new(0, UNKNOWN_END, 0)));
        }
        *task.segs.write() = segs;
    }

    // Anything not finished starts from "idle" again (a pause leaves segments mid-flight).
    for s in task.segs.read().iter() {
        if !s.is_done() {
            s.state.store(SEG_IDLE, Relaxed);
        }
    }

    let total = task.total();
    let workers = if resumable && total >= 2 * MIN_SPLIT { connections } else { 1 };
    task.conn_cap.store(workers as u32, Relaxed);
    let ctx = Arc::new(Ctx {
        mgr: mgr.clone(),
        task: task.clone(),
        epoch,
        client,
        hdrs,
        part: part.clone(),
        resumable,
        host,
        max_segs: connections * SEGS_PER_CONN,
        fatal: Mutex::new(None),
        changed: AtomicBool::new(false),
    });
    let mut handles = Vec::new();
    for _ in 0..workers {
        task.live_workers.fetch_add(1, Relaxed);
        let c = ctx.clone();
        handles.push(thread::spawn(move || worker(c)));
    }
    {
        let mut i = task.info.lock();
        if task.epoch.load(Relaxed) == epoch {
            i.status = Status::Downloading;
            i.error = None;
        }
    }

    let mut last_bytes = task.downloaded();
    let mut last_t = Instant::now();
    let mut ema = 0f64;
    let mut last_save = Instant::now();
    let outcome = loop {
        thread::sleep(Duration::from_millis(150));
        if task.epoch.load(Relaxed) != epoch {
            return Flow::Finished; // paused / superseded; pause() already updated the state
        }
        let dt = last_t.elapsed();
        if dt >= Duration::from_millis(500) {
            let cur = task.downloaded();
            let inst = cur.saturating_sub(last_bytes) as f64 / dt.as_secs_f64();
            ema = if ema == 0.0 { inst } else { 0.6 * ema + 0.4 * inst };
            task.speed.store(ema as u64, Relaxed);
            last_bytes = cur;
            last_t = Instant::now();
        }
        if last_save.elapsed() > Duration::from_secs(3) {
            mgr.save();
            last_save = Instant::now();
        }
        if let Some(f) = ctx.fatal.lock().clone() {
            break Err(f);
        }
        if task.segs.read().iter().all(|s| s.is_done()) {
            break Ok(());
        }
        if handles.iter().all(|h| h.is_finished()) {
            // every worker left without finishing (e.g. all retired): nothing more will happen
            break Err(("Download stopped unexpectedly".to_string(), true));
        }
    };
    for h in handles {
        let _ = h.join();
    }
    if task.epoch.load(Relaxed) != epoch {
        return Flow::Finished;
    }
    if let Err((msg, transient)) = outcome {
        if ctx.changed.load(Relaxed) && may_restart {
            discard_partial(task);
            task.info.lock().probed = false;
            return Flow::Restart;
        }
        fail_with(mgr, task, epoch, msg, transient);
        return Flow::Finished;
    }

    if task.total() == 0 {
        let sum: u64 = task.segs.read().iter().map(|s| s.downloaded()).sum();
        task.total.store(sum, Relaxed);
    }
    if let Err(msg) = verify_coverage(task) {
        fail_with(mgr, task, epoch, msg, true);
        // the plan is broken: next attempt starts clean instead of trusting these segments
        discard_partial(task);
        return Flow::Finished;
    }
    let final_path = task.final_path();
    if let Err(e) = std::fs::rename(&part, &final_path) {
        fail(mgr, task, epoch, format!("Cannot finalize file: {e}"));
        return Flow::Finished;
    }
    if mgr.settings.lock().set_file_date {
        let lm = task.info.lock().last_modified.clone();
        apply_server_date(&final_path, lm);
    }
    task.speed.store(0, Relaxed);
    {
        let mut i = task.info.lock();
        i.status = Status::Complete;
        i.queued = false;
        i.error = None;
        i.completed_at = util::now_ts();
    }
    task.retries.store(0, Relaxed);
    mgr.push_event(Event::Completed(task.id));
    mgr.save();
    Flow::Finished
}

/// Gives the finished file the `Last-Modified` date the server reported (best effort).
fn apply_server_date(path: &std::path::Path, last_modified: Option<String>) {
    let Some(lm) = last_modified else { return };
    let Ok(dt) = chrono::DateTime::parse_from_rfc2822(lm.trim()) else { return };
    if dt.timestamp() <= 0 {
        return;
    }
    let t = std::time::UNIX_EPOCH + Duration::from_secs(dt.timestamp() as u64);
    if let Ok(f) = OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(t);
    }
}

/// Invariant behind byte-exact output: the segments tile `[0, total)` with no gap and no overlap
/// and every one of them reached its end.
fn verify_coverage(task: &Task) -> Result<(), String> {
    let total = task.total();
    let mut segs: Vec<(u64, u64, u64)> =
        task.segs.read().iter().map(|s| (s.start, s.end.load(Relaxed), s.pos.load(Relaxed))).collect();
    segs.sort_by_key(|s| s.0);
    let mut next = 0u64;
    for (start, end, pos) in segs {
        if end == UNKNOWN_END {
            return Ok(()); // unknown length: a single stream, nothing to tile
        }
        if start != next || pos <= end {
            return Err("Integrity check failed: the downloaded pieces do not cover the file".into());
        }
        next = end + 1;
    }
    if total > 0 && next != total {
        return Err("Integrity check failed: the file is incomplete".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_etags_are_not_validators() {
        assert_eq!(strong(&Some("\"abc\"".into())), Some("\"abc\""));
        assert_eq!(strong(&Some("W/\"abc\"".into())), None);
        assert_eq!(strong(&None), None);
    }
}
