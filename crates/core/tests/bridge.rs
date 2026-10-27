use downcraft_core::{Manager, BRIDGE_PORT};
use std::{io::{Read, Write}, net::TcpStream, time::Duration};

fn http(req: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", BRIDGE_PORT)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn post(origin: Option<&str>, host: &str, body: &str) -> String {
    let o = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
    http(&format!(
        "POST /add HTTP/1.1\r\nHost: {host}\r\n{o}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ))
}

#[test]
fn bridge_accepts_extension_and_rejects_web_pages() {
    let dir = std::env::temp_dir().join(format!("fc-bridge-{}", std::process::id()));
    let m = Manager::with_dir(dir);
    if !m.start_bridge() {
        eprintln!("port busy, skipping");
        return;
    }
    let host = format!("127.0.0.1:{BRIDGE_PORT}");
    let body = r#"{"url":"https://example.com/a.mp4","file_name":"A.mp4","headers":{"Referer":"https://site/"}}"#;

    assert!(http(&format!("GET /ping HTTP/1.1\r\nHost: {host}\r\n\r\n")).contains("DownCraft"));

    // a web page is not the extension: it must not mark the extension as installed
    assert!(!m.extension_seen());
    let _ = post(Some("https://evil.example"), &host, body);
    assert!(!m.extension_seen());

    let ok = post(Some("chrome-extension://abcdef"), &host, body);
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    // hearing from the extension is remembered, and saved for the next start
    assert!(m.extension_seen());
    let saved = std::fs::read_to_string(m.data_dir().join("settings.json")).unwrap();
    assert!(saved.contains("\"extension_seen\": true") || saved.contains("\"extension_seen\":true"), "{saved}");
    assert!(ok.contains("Access-Control-Allow-Origin: chrome-extension://abcdef"));
    let inc = m.take_incoming();
    assert_eq!(inc.len(), 1);
    assert_eq!(inc[0].url, "https://example.com/a.mp4");
    assert_eq!(inc[0].file_name.as_deref(), Some("A.mp4"));
    assert_eq!(inc[0].headers[0].0, "Referer");

    assert!(post(Some("https://evil.example"), &host, body).starts_with("HTTP/1.1 403"));
    assert!(post(None, &host, body).starts_with("HTTP/1.1 403"));
    assert!(post(Some("chrome-extension://abc"), "evil.example:19870", body).starts_with("HTTP/1.1 403"));
    assert!(post(Some("chrome-extension://abc"), &host, r#"{"url":"file:///etc/passwd"}"#).starts_with("HTTP/1.1 400"));
    assert!(m.take_incoming().is_empty());
}
