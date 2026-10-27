//! Tiny loopback HTTP endpoint for the browser extension.
//!
//! * `GET  /ping` – lets the extension check that DownCraft is running.
//! * `POST /add`  – `{ "url": "...", "file_name": "...", "headers": { "Referer": "...", "Cookie": "..." } }`
//! * `POST /ytdlp` – `{ "url": "<page url>", "cookies": [ … ] }` → the formats yt-dlp finds for that page
//!   (see `ytdlp.rs`). `cookies` is optional: the browser's cookies for the page, for posts that need a sign-in.
//!   When yt-dlp, Deno (YouTube) or ffmpeg isn't installed the answer is `{ "ok": false, "missing": ["Deno", …], "error": "…" }`.
//!
//! Safety: bound to 127.0.0.1 only; the `Host` header must be loopback (blocks DNS rebinding) and
//! `POST`/`OPTIONS` require an `Origin` of `chrome-extension://` or `moz-extension://`, which a normal
//! web page cannot forge. Nothing is downloaded until the user confirms in *Download File Info*.
use crate::manager::{AudioPart, Incoming, Manager};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

pub const PORT: u16 = 19870;

pub fn spawn(mgr: Arc<Manager>) -> bool {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) else {
        return false;
    };
    thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let m = mgr.clone();
            thread::spawn(move || {
                let _ = handle(s, m);
            });
        }
    });
    true
}

fn reply(s: &mut TcpStream, code: &str, origin: Option<&str>, body: &str) -> std::io::Result<()> {
    let mut h = format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    if let Some(o) = origin {
        h += &format!(
            "Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Max-Age: 600\r\n"
        );
    }
    h += "\r\n";
    s.write_all(h.as_bytes())?;
    s.write_all(body.as_bytes())
}

fn handle(mut s: TcpStream, mgr: Arc<Manager>) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.by_ref().take(8192).read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").split('?').next().unwrap_or("").to_string();

    let mut hdrs: HashMap<String, String> = HashMap::new();
    loop {
        line.clear();
        r.by_ref().take(8192).read_line(&mut line)?;
        if line.trim().is_empty() || hdrs.len() > 64 {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            hdrs.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let host_ok = hdrs
        .get("host")
        .map_or(false, |h| h == &format!("127.0.0.1:{PORT}") || h == &format!("localhost:{PORT}"));
    if !host_ok {
        return reply(&mut s, "403 Forbidden", None, r#"{"ok":false,"error":"bad host"}"#);
    }
    let origin = hdrs.get("origin").cloned();
    let origin_ok = origin
        .as_deref()
        .map_or(false, |o| o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"));
    let cors = if origin_ok { origin.as_deref() } else { None };
    if origin_ok {
        mgr.note_extension();
    }

    match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) if origin_ok => reply(&mut s, "204 No Content", cors, ""),
        ("GET", "/ping") => reply(&mut s, "200 OK", cors, &format!(r#"{{"ok":true,"app":"DownCraft","version":"{}"}}"#, env!("CARGO_PKG_VERSION"))),
        ("POST", "/add") if origin_ok => {
            let len: usize = hdrs.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            if len == 0 || len > 256 * 1024 {
                return reply(&mut s, "400 Bad Request", cors, r#"{"ok":false,"error":"bad length"}"#);
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body)?;
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let url = v["url"].as_str().unwrap_or("").trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return reply(&mut s, "400 Bad Request", cors, r#"{"ok":false,"error":"bad url"}"#);
            }
            let hdrs_of = |o: &serde_json::Value| -> Vec<(String, String)> {
                o.as_object()
                    .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                    .unwrap_or_default()
            };
            let headers = hdrs_of(&v["headers"]);
            let file_name = v["file_name"].as_str().map(|s| s.to_string()).filter(|s| !s.is_empty());
            let size = v["size"].as_u64().unwrap_or(0);
            let source = v["source"].as_str().unwrap_or("media").chars().take(16).collect();
            // optional sound track of a video-only link
            let audio = v["audio"]["url"]
                .as_str()
                .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                .map(|u| AudioPart {
                    url: u.to_string(),
                    headers: hdrs_of(&v["audio"]["headers"]),
                    size: v["audio"]["size"].as_u64().unwrap_or(0),
                    ext: v["audio"]["ext"].as_str().unwrap_or("m4a").chars().filter(|c| c.is_ascii_alphanumeric()).take(5).collect(),
                });
            mgr.push_incoming(Incoming { url, headers, file_name, size, source, audio });
            reply(&mut s, "200 OK", cors, r#"{"ok":true}"#)
        }
        ("POST", "/ytdlp") if origin_ok => {
            let len: usize = hdrs.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            if len == 0 || len > 256 * 1024 {
                return reply(&mut s, "400 Bad Request", cors, r#"{"ok":false,"error":"bad length"}"#);
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body)?;
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let url = v["url"].as_str().unwrap_or("").trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return reply(&mut s, "400 Bad Request", cors, r#"{"ok":false,"error":"bad url"}"#);
            }
            // A helper program is missing (yt-dlp, Deno on YouTube, ffmpeg): answer at once so the
            // extension can show its "Download this video" popup telling the user what to install.
            let missing = crate::ytdlp::missing_tools(mgr.data_dir(), &url);
            if !missing.is_empty() {
                mgr.request_tools_window(); // the app opens its Tools window so the user can press Get
                let names: Vec<&str> = missing.iter().map(|t| t.name()).collect();
                let out = serde_json::json!({ "ok": false, "missing": names, "error": crate::ytdlp::missing_message(&missing) });
                return reply(&mut s, "200 OK", cors, &out.to_string());
            }
            // Can take a few seconds (yt-dlp talks to the site); this runs on the connection's own thread.
            let mut opts = mgr.ytdlp_opts();
            let cookies = crate::ytdlp::write_cookie_file(mgr.data_dir(), &v["cookies"]); // deleted when dropped
            if let Some(c) = &cookies {
                opts.cookies_file = Some(c.0.clone());
            }
            let out = match crate::ytdlp::formats(mgr.data_dir(), &url, &opts) {
                Ok(v) => v,
                Err(e) => serde_json::json!({ "ok": false, "error": e }),
            };
            reply(&mut s, "200 OK", cors, &out.to_string())
        }
        ("POST", _) | ("OPTIONS", _) => reply(&mut s, "403 Forbidden", None, r#"{"ok":false,"error":"origin not allowed"}"#),
        _ => reply(&mut s, "404 Not Found", cors, r#"{"ok":false}"#),
    }
}
