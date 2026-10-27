use crate::{ratelimit::RateLimiter, util, Category};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed},
        Arc,
    },
};

pub const CTL_RUN: u8 = 0;
pub const CTL_STOP: u8 = 1;

pub const UNKNOWN_END: u64 = u64::MAX;

pub const SEG_IDLE: u8 = 0;
pub const SEG_CONNECTING: u8 = 1;
pub const SEG_RECEIVING: u8 = 2;
pub const SEG_DONE: u8 = 3;
pub const SEG_ERROR: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Status {
    Stopped,
    Connecting,
    Downloading,
    Complete,
    Error,
}

/// One byte-range "connection" of a download.
#[derive(Debug)]
pub struct Seg {
    pub start: u64,
    pub end: std::sync::atomic::AtomicU64,
    pub pos: AtomicU64,
    pub state: AtomicU8,
}

impl Seg {
    pub fn new(start: u64, end: u64, pos: u64) -> Self {
        let s = Seg { start, end: AtomicU64::new(end), pos: AtomicU64::new(pos), state: AtomicU8::new(SEG_IDLE) };
        if end != UNKNOWN_END && pos > end {
            s.state.store(SEG_DONE, Relaxed);
        }
        s
    }
    /// Bytes this segment owns that are on disk. A segment that was split while its worker was
    /// still streaming may briefly have `pos` past its (shrunken) `end`; that overshoot belongs to
    /// the new segment and is not counted twice.
    pub fn downloaded(&self) -> u64 {
        let end = self.end.load(Relaxed);
        let pos = self.pos.load(Relaxed);
        let pos = if end == UNKNOWN_END { pos } else { pos.min(end.saturating_add(1)) };
        pos.saturating_sub(self.start)
    }
    pub fn is_done(&self) -> bool {
        self.state.load(Relaxed) == SEG_DONE
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub file_name: String,
    pub dir: PathBuf,
    pub category: Category,
    pub description: String,
    pub status: Status,
    pub error: Option<String>,
    pub resumable: bool,
    pub probed: bool,
    pub queued: bool,
    pub added: i64,
    pub last_try: i64,
    pub completed_at: i64,
    /// Extra request headers (Referer, User-Agent, Cookie, Origin) captured by the browser extension.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Segmented HLS (`.m3u8`) download instead of a plain file.
    #[serde(default)]
    pub hls: bool,
    /// Strong validators from the first probe; a changed value on resume means the file on the
    /// server is a different one and the partial data must be thrown away.
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

pub struct Task {
    pub id: u64,
    pub url: String,
    pub info: Mutex<TaskInfo>,
    pub total: AtomicU64,
    pub speed: AtomicU64,
    pub ctl: AtomicU8,
    pub epoch: AtomicU64,
    pub limit_kbps: AtomicU64,
    pub segs: RwLock<Vec<Arc<Seg>>>,
    /// HLS progress: bytes of finished segments, finished segment count, total segment count.
    pub bytes_done: AtomicU64,
    pub hls_done: AtomicU64,
    pub hls_count: AtomicU64,
    /// Connections currently owned by this run's workers.
    pub live_workers: AtomicU32,
    /// Dynamic connection ceiling; lowered when the server starts refusing parallel requests.
    pub conn_cap: AtomicU32,
    /// Pacing for this task's own speed limit (the global limit lives in the manager).
    pub bucket: RateLimiter,
    /// Serialises the "split the biggest segment" step between idle workers.
    pub split_lock: Mutex<()>,
    /// Automatic retries used so far, when the next one is due, and whether the last error is worth retrying.
    pub retries: AtomicU32,
    pub retry_at: AtomicI64,
    pub transient: AtomicBool,
}

#[derive(Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub url: String,
    pub info: TaskInfo,
    pub total: u64,
    pub segs: Vec<(u64, u64, u64)>,
}

impl Task {
    pub fn new(id: u64, url: String, info: TaskInfo, total: u64) -> Self {
        Task {
            id,
            url,
            info: Mutex::new(info),
            total: AtomicU64::new(total),
            speed: AtomicU64::new(0),
            ctl: AtomicU8::new(CTL_RUN),
            epoch: AtomicU64::new(0),
            limit_kbps: AtomicU64::new(0),
            segs: RwLock::new(Vec::new()),
            bytes_done: AtomicU64::new(0),
            hls_done: AtomicU64::new(0),
            hls_count: AtomicU64::new(0),
            live_workers: AtomicU32::new(0),
            conn_cap: AtomicU32::new(0),
            bucket: RateLimiter::default(),
            split_lock: Mutex::new(()),
            retries: AtomicU32::new(0),
            retry_at: AtomicI64::new(0),
            transient: AtomicBool::new(false),
        }
    }

    pub fn from_record(r: Record) -> Self {
        let mut info = r.info;
        if matches!(info.status, Status::Connecting | Status::Downloading) {
            info.status = Status::Stopped;
        }
        let resumable = info.resumable;
        let t = Task::new(r.id, r.url, info, r.total);
        if resumable {
            *t.segs.write() = r.segs.into_iter().map(|(s, e, p)| Arc::new(Seg::new(s, e, p))).collect();
        }
        t
    }

    pub fn to_record(&self) -> Record {
        Record {
            id: self.id,
            url: self.url.clone(),
            info: self.info.lock().clone(),
            total: self.total.load(Relaxed),
            segs: self
                .segs
                .read()
                .iter()
                .map(|s| (s.start, s.end.load(Relaxed), s.pos.load(Relaxed)))
                .collect(),
        }
    }

    pub fn total(&self) -> u64 {
        self.total.load(Relaxed)
    }

    pub fn downloaded(&self) -> u64 {
        let (done, hls) = {
            let i = self.info.lock();
            (i.status == Status::Complete, i.hls)
        };
        if done {
            return self.total();
        }
        if hls {
            return self.bytes_done.load(Relaxed);
        }
        self.segs.read().iter().map(|s| s.downloaded()).sum()
    }

    pub fn percent(&self) -> f64 {
        let (done, hls) = {
            let i = self.info.lock();
            (i.status == Status::Complete, i.hls)
        };
        if done {
            return 100.0;
        }
        if hls {
            let c = self.hls_count.load(Relaxed);
            return if c == 0 { 0.0 } else { self.hls_done.load(Relaxed) as f64 / c as f64 * 100.0 };
        }
        let t = self.total();
        if t == 0 {
            return 0.0;
        }
        (self.downloaded() as f64 / t as f64 * 100.0).min(100.0)
    }

    pub fn speed(&self) -> u64 {
        self.speed.load(Relaxed)
    }

    pub fn eta(&self) -> Option<u64> {
        let sp = self.speed();
        let t = self.total();
        if sp == 0 || t == 0 {
            return None;
        }
        Some(t.saturating_sub(self.downloaded()) / sp)
    }

    pub fn final_path(&self) -> PathBuf {
        let i = self.info.lock();
        i.dir.join(&i.file_name)
    }

    pub fn part_path(&self) -> PathBuf {
        let i = self.info.lock();
        i.dir.join(format!("{}.fcpart", i.file_name))
    }

    /// Folder holding the finished segments of an HLS download until they are stitched together.
    pub fn hls_dir(&self) -> PathBuf {
        let i = self.info.lock();
        i.dir.join(format!("{}.fcparts", i.file_name))
    }

    pub fn is_active(&self) -> bool {
        matches!(self.info.lock().status, Status::Connecting | Status::Downloading)
    }

    pub fn touch_last_try(&self) {
        self.info.lock().last_try = util::now_ts();
    }
}
