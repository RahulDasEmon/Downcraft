use downcraft_core::{tools::Tool, ytdlp, Manager, BRIDGE_PORT};
use serde_json::json;
use std::{io::{Read, Write}, net::TcpStream, time::Duration};

fn sample() -> serde_json::Value {
    json!({
        "id": "abc", "title": "Demo", "uploader": "Me", "duration": 61.0,
        "formats": [
            {"format_id":"sb0","ext":"mhtml","protocol":"mhtml","url":"https://i.ytimg.com/sb","vcodec":"none","acodec":"none"},
            {"format_id":"140","ext":"m4a","protocol":"https","url":"https://gv/a140","vcodec":"none","acodec":"mp4a.40.2","abr":129.5,"filesize":1000},
            {"format_id":"251","ext":"webm","protocol":"https","url":"https://gv/a251","vcodec":"none","acodec":"opus","abr":135.0,"filesize_approx":900},
            {"format_id":"18","ext":"mp4","protocol":"https","url":"https://gv/v18","vcodec":"avc1.42001E","acodec":"mp4a.40.2","height":360,"fps":30,"tbr":500.0,"filesize":5000,
             "http_headers":{"User-Agent":"UA/1"}},
            {"format_id":"137","ext":"mp4","protocol":"https","url":"https://gv/v137","vcodec":"avc1.640028","acodec":"none","height":1080,"fps":30,"tbr":4000.0},
            {"format_id":"299","ext":"mp4","protocol":"https","url":"https://gv/v299","vcodec":"avc1.64002a","acodec":"none","height":1080,"fps":30,"tbr":5000.0},
            {"format_id":"248","ext":"webm","protocol":"https","url":"https://gv/v248","vcodec":"vp9","acodec":"none","height":1080,"fps":30,"tbr":2500.0},
            {"format_id":"93","ext":"mp4","protocol":"m3u8_native","url":"https://gv/h93.m3u8","vcodec":"avc1.4d401e","acodec":"mp4a.40.2","height":360,"fps":30,"tbr":600.0},
            {"format_id":"dash","ext":"mp4","protocol":"http_dash_segments","url":"https://gv/x","vcodec":"avc1","acodec":"none","height":720},
            {"format_id":"drm","ext":"mp4","protocol":"https","url":"https://gv/drm","vcodec":"avc1","acodec":"mp4a","height":720,"has_drm":true}
        ]
    })
}

#[test]
fn normalize_groups_dedupes_and_drops_unsupported() {
    let out = ytdlp::normalize(&sample()).unwrap();
    assert_eq!(out["title"], "Demo");
    let f = out["formats"].as_array().unwrap();
    let ids: Vec<&str> = f.iter().map(|x| x["id"].as_str().unwrap()).collect();
    // muxed first (360p HLS vs direct are separate entries), then video-only best-first, then audio
    assert_eq!(ids, ["18", "93", "299", "248", "251", "140"]);
    assert_eq!(f[0]["kind"], "muxed");
    assert_eq!(f[0]["headers"]["User-Agent"], "UA/1");
    assert_eq!(f[1]["hls"], true);
    assert_eq!(f[2]["kind"], "video"); // 299 beat 137 (same height/codec/ext, higher bitrate)
    assert_eq!(f[2]["codec"], "H.264");
    assert_eq!(f[4]["kind"], "audio");
    assert_eq!(f[4]["approx"], true); // 251 only has filesize_approx
}

#[test]
fn normalize_rejects_live_and_empty() {
    assert!(ytdlp::normalize(&json!({"is_live": true, "formats": []})).is_err());
    assert!(ytdlp::normalize(&json!({"formats": []})).is_err());
}

#[cfg(unix)]
#[test]
fn bridge_ytdlp_endpoint_uses_the_binary() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("fc-yt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("fake-yt-dlp");
    // prints the sample JSON no matter the arguments, but fails for the "bad" URL
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {}/args.log\nfor a; do last=\"$a\"; done\nif [ \"$last\" = \"https://bad.example/\" ]; then echo 'ERROR: Video unavailable' >&2; exit 1; fi\ncat <<'EOF'\n{}\nEOF\n",
        dir.display(),
        sample()
    );
    std::fs::write(&fake, script).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var(ytdlp::ENV_PATH, &fake);
    // the bridge answers only when ffmpeg (and Deno, for YouTube) are installed too
    let tools_dir = downcraft_core::tools::tools_dir(&dir.join("data"));
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(tools_dir.join("deno"), "").unwrap();
    std::fs::write(tools_dir.join("ffmpeg"), "").unwrap();

    let m = Manager::with_dir(dir.join("data"));
    if !m.start_bridge() {
        eprintln!("port busy, skipping");
        return;
    }
    let host = format!("127.0.0.1:{BRIDGE_PORT}");
    let call_with = |origin: Option<&str>, url: &str, cookies: serde_json::Value| {
        let body = json!({ "url": url, "cookies": cookies }).to_string();
        let o = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
        let req = format!("POST /ytdlp HTTP/1.1\r\nHost: {host}\r\n{o}Content-Length: {}\r\n\r\n{body}", body.len());
        let mut s = TcpStream::connect(("127.0.0.1", BRIDGE_PORT)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    };
    let call = |origin: Option<&str>, url: &str| call_with(origin, url, json!([]));
    let ok = call(Some("chrome-extension://abc"), "https://www.youtube.com/watch?v=abc");
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    let body: serde_json::Value = serde_json::from_str(ok.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["formats"].as_array().unwrap().len(), 6);

    let bad = call(Some("chrome-extension://abc"), "https://bad.example/");
    assert!(bad.contains("Video unavailable"), "{bad}");

    // cookies from the extension reach yt-dlp as a cookie file, which is gone again afterwards
    let log = dir.join("args.log");
    let _ = std::fs::remove_file(&log);
    let c = json!([{"domain": ".instagram.com", "path": "/", "secure": true, "name": "sessionid", "value": "abc"}]);
    let with = call_with(Some("chrome-extension://abc"), "https://www.instagram.com/reel/xyz/", c);
    assert!(with.starts_with("HTTP/1.1 200"), "{with}");
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains("--cookies ") && args.contains("--no-playlist"), "{args}");
    assert!(!args.contains("-- --cookies"), "{args}");
    let leftovers = std::fs::read_dir(dir.join("data").join("tmp")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(leftovers, 0, "temporary cookie file was not removed");

    // a missing helper program is reported at once, naming what to install
    let answer = |r: &str| -> serde_json::Value { serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap() };
    let system_has = |t| downcraft_core::tools::locate(&dir.join("data"), t).is_some();
    std::fs::remove_file(tools_dir.join("deno")).unwrap();
    if !system_has(Tool::Deno) {
        let yt = answer(&call(Some("chrome-extension://abc"), "https://www.youtube.com/watch?v=abc"));
        assert_eq!(yt["ok"], false);
        assert_eq!(yt["missing"], json!(["Deno"]));
        assert!(yt["error"].as_str().unwrap().contains("Deno isn't installed"), "{yt}");
        // ... and the app is asked to open its Tools window (once; a second ask within a minute is ignored)
        assert!(m.take_tools_request());
        assert!(!m.take_tools_request());
        m.request_tools_window();
        assert!(!m.take_tools_request());
        // Deno is only needed for YouTube
        let ig = answer(&call(Some("chrome-extension://abc"), "https://www.instagram.com/reel/xyz/"));
        assert_eq!(ig["ok"], true, "{ig}");
    }
    std::fs::remove_file(tools_dir.join("ffmpeg")).unwrap();
    if !system_has(Tool::Ffmpeg) {
        let ig = answer(&call(Some("chrome-extension://abc"), "https://www.instagram.com/reel/xyz/"));
        assert_eq!(ig["ok"], false);
        assert_eq!(ig["missing"], json!(["ffmpeg"]));
    }

    assert!(call(None, "https://www.youtube.com/watch?v=abc").starts_with("HTTP/1.1 403"));
    assert!(call(Some("https://evil.example"), "https://www.youtube.com/watch?v=abc").starts_with("HTTP/1.1 403"));
}

#[test]
fn video_sites_are_recognised() {
    for u in [
        "https://www.facebook.com/watch/?v=1",
        "https://fb.watch/abc/",
        "https://www.instagram.com/reel/Cxyz/",
        "https://x.com/user/status/123",
        "https://mobile.twitter.com/user/status/123",
        "https://vm.tiktok.com/abc",
        "https://www.youtube.com/watch?v=abc",
    ] {
        assert!(ytdlp::is_video_site(u), "{u}");
    }
    for u in ["https://example.com/a.mp4", "https://notx.com/status/1", "https://evil.com/?u=https://x.com/", "ftp://x.com/a", "x.com/a"] {
        assert!(!ytdlp::is_video_site(u), "{u}");
    }
    assert_eq!(ytdlp::page_origin("https://www.instagram.com/p/abc/?utm=1"), "https://www.instagram.com");
}

// What yt-dlp returns for a tweet: plain MP4 renditions plus HLS, no separate sound tracks.
fn tweet() -> serde_json::Value {
    json!({
        "id": "1", "title": "Some tweet text", "uploader": "someone",
        "formats": [
            {"format_id":"hls-audio-32000-Audio","ext":"mp4","protocol":"m3u8_native","url":"https://video.twimg.com/a.m3u8","vcodec":"none","acodec":"mp4a.40.2"},
            {"format_id":"http-632","ext":"mp4","protocol":"https","url":"https://video.twimg.com/632.mp4","vcodec":"avc1","acodec":"mp4a.40.2","height":360,"tbr":632.0},
            {"format_id":"http-950","ext":"mp4","protocol":"https","url":"https://video.twimg.com/950.mp4","vcodec":"avc1","acodec":"mp4a.40.2","height":720,"tbr":950.0,
             "http_headers":{"User-Agent":"UA/2","Referer":"https://elsewhere/"}},
            {"format_id":"hls-950","ext":"mp4","protocol":"m3u8_native","url":"https://video.twimg.com/950.m3u8","vcodec":"avc1","acodec":"mp4a.40.2","height":720,"tbr":960.0}
        ]
    })
}

// Facebook / Instagram style: the good pictures are video-only and the sound is a separate file.
fn dash_post() -> serde_json::Value {
    json!({
        "id": "2", "title": "", "uploader": "A Page",
        "formats": [
            {"format_id":"a","ext":"m4a","protocol":"https","url":"https://scontent/a.m4a","vcodec":"none","acodec":"mp4a.40.2","abr":64.0,"filesize":300},
            {"format_id":"v1","ext":"mp4","protocol":"https","url":"https://scontent/v1.mp4","vcodec":"avc1","acodec":"none","height":1080,"fps":30,"tbr":3000.0,"filesize":4000},
            {"format_id":"v2","ext":"mp4","protocol":"https","url":"https://scontent/v2.mp4","vcodec":"avc1","acodec":"none","height":540,"fps":30,"tbr":900.0},
            {"format_id":"sd","ext":"mp4","protocol":"https","url":"https://scontent/sd.mp4","vcodec":"avc1","acodec":"mp4a.40.2","height":360,"tbr":400.0}
        ]
    })
}

#[test]
fn choices_for_a_tweet_prefer_direct_files_and_set_referer() {
    let n = ytdlp::normalize(&tweet()).unwrap();
    let c = ytdlp::choices(&n, "https://x.com/someone/status/1");
    let ids: Vec<(&str, u64)> = c.iter().map(|x| (x.ext.as_str(), x.height)).collect();
    assert_eq!(ids, [("mp4", 720), ("mp4", 360)]); // HLS twin of 720p and the HLS-only audio are not offered
    assert_eq!(c[0].url, "https://video.twimg.com/950.mp4");
    assert!(c[0].audio.is_none());
    let h: std::collections::HashMap<_, _> = c[0].headers.iter().cloned().collect();
    assert_eq!(h["User-Agent"], "UA/2");
    assert_eq!(h["Referer"], "https://x.com/");
    assert_eq!(h["Origin"], "https://x.com");
    assert_eq!(ytdlp::file_name("  Some\ntweet: \"text\"  ", "someone", &c[0]), "Some tweet text [720p].mp4");
}

#[test]
fn choices_join_video_only_with_audio() {
    let n = ytdlp::normalize(&dash_post()).unwrap();
    let c = ytdlp::choices(&n, "https://www.facebook.com/watch/?v=2");
    let q: Vec<(u64, bool)> = c.iter().map(|x| (x.height, x.audio.is_some())).collect();
    // 1080p and 540p need the sound track merged in, 360p is complete, then the audio-only row
    assert_eq!(q, [(1080, true), (540, true), (360, false), (0, false)]);
    assert_eq!(c[0].audio.as_ref().unwrap().url, "https://scontent/a.m4a");
    assert_eq!(c[0].ext, "mp4");
    assert_eq!(c[0].size, 4300);
    assert!(c[3].audio_only);
    assert_eq!(ytdlp::file_name("", "A Page", &c[0]), "A Page [1080p].mp4");
}

#[test]
fn a_post_with_several_videos_lists_every_entry() {
    let multi = json!({ "title": "thread", "entries": [tweet(), dash_post(), {"formats": []}] });
    let n = ytdlp::normalize(&multi).unwrap();
    assert_eq!(n["entries"].as_array().unwrap().len(), 2); // the empty one is dropped
    assert_eq!(n["title"], "Some tweet text"); // the head is the first entry
    assert_eq!(n["entries"][1]["uploader"], "A Page");
    // an entry without a title of its own takes the post's
    assert_eq!(n["entries"][1]["title"], "thread");
    // a single-entry playlist collapses to that entry
    let one = ytdlp::normalize(&json!({ "title": "t", "entries": [tweet()] })).unwrap();
    assert!(one.get("entries").is_none());
    assert!(ytdlp::normalize(&json!({ "entries": [{"formats": []}] })).is_err());
}

#[test]
fn cookie_file_is_netscape_and_removed_on_drop() {
    let dir = std::env::temp_dir().join(format!("fc-ck-{}", std::process::id()));
    let cookies = json!([
        {"domain": ".instagram.com", "path": "/", "secure": true, "expires": 1900000000.5, "name": "sessionid", "value": "abc"},
        {"domain": "x.com", "name": "ct0", "value": "tok"},
        {"domain": ".evil.com", "name": "bad\tname", "value": "v"},
        {"domain": ".evil.com", "name": "bad", "value": "line\nbreak"}
    ]);
    let f = ytdlp::write_cookie_file(&dir, &cookies).unwrap();
    let text = std::fs::read_to_string(&f.0).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "# Netscape HTTP Cookie File");
    assert_eq!(lines[1], ".instagram.com\tTRUE\t/\tTRUE\t1900000000\tsessionid\tabc");
    assert!(lines[2].starts_with("x.com\tFALSE\t/\tFALSE\t"));
    assert_eq!(lines.len(), 3); // the two malformed cookies were skipped
    let path = f.0.clone();
    drop(f);
    assert!(!path.exists());
    assert!(ytdlp::write_cookie_file(&dir, &json!([])).is_none());
    assert!(ytdlp::write_cookie_file(&dir, &json!(null)).is_none());
}

#[test]
fn videos_drops_entries_without_rows() {
    let n = ytdlp::normalize(&json!({ "title": "thread", "entries": [tweet(), dash_post()] })).unwrap();
    let v = ytdlp::videos(&n, "https://x.com/a/status/1");
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].choices[0].url, "https://video.twimg.com/950.mp4");
    assert_eq!(v[1].uploader, "A Page");
    assert_eq!(v[0].file_name(&v[0].choices[0]), "Some tweet text [720p].mp4");
    let single = ytdlp::videos(&ytdlp::normalize(&tweet()).unwrap(), "https://x.com/a/status/1");
    assert_eq!(single.len(), 1);
}

#[test]
fn missing_message_names_what_to_install() {
    assert_eq!(ytdlp::missing_message(&[]), "");
    assert_eq!(
        ytdlp::missing_message(&[Tool::YtDlp]),
        "yt-dlp isn't installed. In DownCraft open Tasks → Tools and press Get next to yt-dlp."
    );
    let two = ytdlp::missing_message(&[Tool::YtDlp, Tool::Deno]);
    assert!(two.starts_with("yt-dlp and Deno aren't installed."), "{two}");
    assert!(two.ends_with("press Get next to yt-dlp and Deno."), "{two}");
    let three = ytdlp::missing_message(&[Tool::YtDlp, Tool::Deno, Tool::Ffmpeg]);
    assert!(three.starts_with("yt-dlp, Deno and ffmpeg aren't installed."), "{three}");
}
