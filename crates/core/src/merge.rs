//! "Video + audio": YouTube serves its best pictures without sound. The browser extension sends the
//! video link together with the best matching audio link; both are downloaded as ordinary tasks and,
//! once both are complete, ffmpeg joins them (stream copy, no re-encoding) into the video's file.
use crate::{
    manager::Event,
    task::Task,
    tools::{self, Tool},
    Manager,
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::Ordering::Relaxed, Arc},
};

pub(crate) struct MergeJob {
    pub video: Arc<Task>,
    pub audio: Arc<Task>,
    pub video_done: bool,
    pub audio_done: bool,
}

impl Manager {
    /// Pairs two tasks: when both are complete the audio is merged into the video's file and the
    /// audio task disappears. Register the pair before starting either task.
    pub fn add_merge(&self, video: &Arc<Task>, audio: &Arc<Task>) {
        self.merges.lock().push(MergeJob { video: video.clone(), audio: audio.clone(), video_done: false, audio_done: false });
    }

    /// Called for every completed task. Returns true when the task belongs to a pair: its completion
    /// is then reported only after the merge.
    pub(crate) fn merge_gate(self: &Arc<Self>, id: u64) -> bool {
        let ready = {
            let mut jobs = self.merges.lock();
            let Some(pos) = jobs.iter().position(|j| j.video.id == id || j.audio.id == id) else {
                return false;
            };
            if jobs[pos].video.id == id {
                jobs[pos].video_done = true;
            } else {
                jobs[pos].audio_done = true;
            }
            if jobs[pos].video_done && jobs[pos].audio_done {
                Some(jobs.remove(pos))
            } else {
                None
            }
        };
        if let Some(job) = ready {
            let m = self.clone();
            std::thread::spawn(move || run_merge(m, job));
        }
        true
    }
}

fn run_merge(mgr: Arc<Manager>, job: MergeJob) {
    let (video, audio) = (job.video, job.audio);
    let old_desc = {
        let mut i = video.info.lock();
        let d = i.description.clone();
        i.description = "Merging video and audio…".into();
        d
    };
    let result = merge_files(mgr.data_dir(), &video.final_path(), &audio.final_path());
    match result {
        Ok(size) => {
            video.total.store(size, Relaxed);
            video.info.lock().description = old_desc;
            mgr.delete(&audio, true);
        }
        Err(e) => {
            // keep both files: nothing is lost, the user can merge them by hand
            video.info.lock().description = format!("Not merged: {e}. The sound is saved as a separate file.");
        }
    }
    mgr.save();
    mgr.push_event_raw(Event::Completed(video.id));
}

fn merge_files(data_dir: &Path, video: &Path, audio: &Path) -> Result<u64, String> {
    let ffmpeg = tools::locate(data_dir, Tool::Ffmpeg)
        .map(|i| i.path)
        .ok_or_else(|| "ffmpeg is not installed (Tools → ffmpeg → Install)".to_string())?;
    let tmp = PathBuf::from(format!("{}.fcmerge", video.display()));
    let fmt = match video.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp4") => "mp4",
        Some("webm") => "webm",
        _ => "matroska",
    };
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]);
    if fmt == "mp4" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.args(["-f", fmt]).arg(&tmp).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("could not start ffmpeg: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed");
        return Err(last.chars().take(160).collect());
    }
    // rename replaces the video-only file in one step; if the platform refuses, remove it first
    if std::fs::rename(&tmp, video).is_err() {
        std::fs::remove_file(video).map_err(|e| format!("could not replace the video file: {e}"))?;
        std::fs::rename(&tmp, video).map_err(|e| format!("could not replace the video file: {e}"))?;
    }
    std::fs::metadata(video).map(|m| m.len()).map_err(|e| e.to_string())
}
