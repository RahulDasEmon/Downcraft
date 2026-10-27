//! DownCraft core: a segmented, resumable HTTP download engine.
//! No GUI code lives here, so it can be tested headless.

mod bridge;
mod category;
mod download;
mod hls;
mod manager;
mod merge;
pub mod ratelimit;
mod schedule;
mod settings;
mod task;
mod transfer;
pub mod tools;
pub mod util;
pub mod ytdlp;

pub use category::Category;
pub use download::{probe, Probe};
pub use bridge::PORT as BRIDGE_PORT;
pub use manager::{AudioPart, Event, EventHook, Incoming, Manager, NewDownload, QuotaStatus};
pub use schedule::Schedule;
pub use transfer::ImportReport;
pub use settings::{
    AfterQueue, ConnRule, DoubleClick, DuplicateMode, IpMode, ProxyMode, Settings, SiteLogin, SoundSlot, Sounds, DEFAULT_CLIPBOARD_EXTS,
    DEFAULT_CLIPBOARD_SKIP, DEFAULT_PROXY_BYPASS,
};
pub use task::{Record, Seg, Status, Task, TaskInfo};
pub use task::{SEG_CONNECTING, SEG_DONE, SEG_ERROR, SEG_IDLE, SEG_RECEIVING, UNKNOWN_END};
