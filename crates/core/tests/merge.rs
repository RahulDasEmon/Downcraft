//! A video-only file and a sound-only file are downloaded as a pair and joined by ffmpeg.
//! Needs `ffmpeg` on PATH (skipped otherwise).
use downcraft_core::{Category, Event, Manager, NewDownload};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn serve(mut s: TcpStream, v: Arc<Vec<u8>>, a: Arc<Vec<u8>>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut range = None;
    loop {
        line.clear();
        if r.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.trim().to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (x, y) = v.split_once('-').unwrap();
            range = Some((x.parse::<usize>().unwrap_or(0), y.parse::<usize>().ok()));
        }
    }
    let data = if path.contains("v.mp4") { v } else { a };
    let len = data.len();
    let (code, lo, hi) = match range {
        Some((x, y)) => ("206 Partial Content", x.min(len), y.map_or(len, |y| (y + 1).min(len))),
        None => ("200 OK", 0, len),
    };
    let mut h = format!("HTTP/1.1 {code}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n", hi - lo);
    if code.starts_with("206") {
        h += &format!("Content-Range: bytes {}-{}/{len}\r\n", lo, hi.saturating_sub(1));
    }
    h += "\r\n";
    let _ = s.write_all(h.as_bytes());
    if method != "HEAD" {
        let _ = s.write_all(&data[lo..hi]);
    }
}

#[test]
fn video_and_audio_are_merged_into_one_file() {
    if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        eprintln!("ffmpeg not found - skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fc-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (vf, af) = (dir.join("src-v.mp4"), dir.join("src-a.m4a"));
    let run = |args: &[&str]| {
        assert!(Command::new("ffmpeg").args(["-y", "-loglevel", "error"]).args(args).status().unwrap().success());
    };
    run(&["-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=10", "-an", "-c:v", "libx264", "-pix_fmt", "yuv420p", vf.to_str().unwrap()]);
    run(&["-f", "lavfi", "-i", "sine=duration=2", "-c:a", "aac", af.to_str().unwrap()]);
    let (v, a) = (Arc::new(std::fs::read(&vf).unwrap()), Arc::new(std::fs::read(&af).unwrap()));

    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    {
        let (v, a) = (v.clone(), a.clone());
        thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (v, a) = (v.clone(), a.clone());
                thread::spawn(move || serve(s, v, a));
            }
        });
    }

    let m = Manager::with_dir(dir.join("data"));
    m.settings.lock().auto_retries = 0;
    let out = dir.join("out");
    let new = |url: String, name: &str| NewDownload {
        url,
        dir: out.clone(),
        file_name: name.into(),
        category: Category::Video,
        description: "orig".into(),
        probe: None,
        queued: false,
        start: false,
        headers: vec![],
    };
    let vt = m.add(new(format!("http://127.0.0.1:{port}/v.mp4"), "Clip [120p].mp4"));
    let at = m.add(new(format!("http://127.0.0.1:{port}/a.m4a"), "Clip [120p].audio.m4a"));
    m.add_merge(&vt, &at);
    m.start(&vt);
    m.start(&at);

    // exactly one completion is reported: the video's, after the merge
    let t0 = Instant::now();
    let mut done = vec![];
    while done.is_empty() {
        done.extend(m.take_events().into_iter().filter_map(|e| if let Event::Completed(id) = e { Some(id) } else { None }));
        assert!(t0.elapsed() < Duration::from_secs(30), "timeout");
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(done, vec![vt.id]);

    let merged = out.join("Clip [120p].mp4");
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0"])
        .arg(&merged)
        .output()
        .unwrap();
    let streams = String::from_utf8_lossy(&probe.stdout).to_string();
    assert!(streams.contains("video") && streams.contains("audio"), "streams: {streams}");
    assert!(!out.join("Clip [120p].audio.m4a").exists(), "audio file should be removed");
    assert!(!out.join("Clip [120p].mp4.fcmerge").exists());
    assert!(m.find(at.id).is_none(), "audio task should be gone");
    assert_eq!(vt.info.lock().description, "orig");
    assert_eq!(vt.total(), std::fs::metadata(&merged).unwrap().len());
    let _ = std::fs::remove_dir_all(&dir);
}
