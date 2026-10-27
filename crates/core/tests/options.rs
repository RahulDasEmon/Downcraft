//! Engine behaviour behind the Options dialog: site logins, per-server connection limits,
//! download quota, file date, queue-finished event.
use downcraft_core::{Category, ConnRule, Event, Manager, NewDownload, SiteLogin, Status};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const SIZE: usize = 3_000_000;

fn body() -> Arc<Vec<u8>> {
    Arc::new((0..SIZE as u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect())
}

/// Range-capable server. With `secret` set it answers 401 unless `Authorization: Basic <secret>` arrives.
fn handle(mut s: TcpStream, data: Arc<Vec<u8>>, secret: Option<String>, conns: Arc<AtomicUsize>) {
    conns.fetch_add(1, Relaxed);
    let mut r = BufReader::new(s.try_clone().unwrap());
    let (mut range, mut auth) = (None, None);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    loop {
        line.clear();
        r.read_line(&mut line).unwrap();
        let l = line.trim().to_string();
        if l.is_empty() {
            break;
        }
        let low = l.to_ascii_lowercase();
        if let Some(v) = low.strip_prefix("range: bytes=") {
            let (a, b) = v.trim().split_once('-').unwrap();
            let a: usize = a.parse().unwrap();
            let b: usize = if b.is_empty() { data.len() - 1 } else { b.parse().unwrap() };
            range = Some((a, b.min(data.len() - 1)));
        }
        if low.starts_with("authorization: basic ") {
            auth = Some(l["authorization: basic ".len()..].to_string());
        }
    }
    if let Some(sec) = &secret {
        if auth.as_deref() != Some(sec.as_str()) {
            let _ = write!(s, "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        }
    }
    let lm = "Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\n";
    match range {
        Some((a, b)) => {
            let _ = write!(
                s,
                "HTTP/1.1 206 Partial Content\r\n{lm}Content-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len(),
                b - a + 1
            );
            let _ = s.write_all(&data[a..=b]);
        }
        None => {
            let _ = write!(s, "HTTP/1.1 200 OK\r\n{lm}Content-Length: {}\r\nConnection: close\r\n\r\n", data.len());
            let _ = s.write_all(&data);
        }
    }
}

fn serve(secret: Option<&str>) -> (String, Arc<Vec<u8>>, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/private/file.bin", l.local_addr().unwrap());
    let data = body();
    let conns = Arc::new(AtomicUsize::new(0));
    let (d, c, sec) = (data.clone(), conns.clone(), secret.map(String::from));
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (d, c, sec) = (d.clone(), c.clone(), sec.clone());
            thread::spawn(move || handle(s, d, sec, c));
        }
    });
    (url, data, conns)
}

fn mgr(tag: &str) -> (Arc<Manager>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fc-opt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (Manager::with_dir(dir.join("data")), dir)
}

fn req(url: &str, dir: &std::path::Path) -> NewDownload {
    NewDownload {
        url: url.into(),
        dir: dir.join("out"),
        file_name: String::new(),
        category: Category::Programs,
        description: String::new(),
        probe: None,
        queued: false,
        start: true,
        headers: vec![],
    }
}

fn wait_status(m: &Arc<Manager>, id: u64, want: Status, secs: u64) -> Status {
    let t0 = Instant::now();
    loop {
        let st = m.find(id).unwrap().info.lock().status;
        if st == want || t0.elapsed() > Duration::from_secs(secs) {
            return st;
        }
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn site_login_is_sent_and_only_when_it_matches() {
    // "me:s3cret" in base64
    let (url, data, _) = serve(Some("bWU6czNjcmV0"));
    let (m, dir) = mgr("auth");

    // no login saved -> the server refuses and the download fails permanently
    let t = m.add(req(&url, &dir));
    assert_eq!(wait_status(&m, t.id, Status::Complete, 10), Status::Error);
    assert!(t.info.lock().error.clone().unwrap().contains("401"));
    m.delete(&t, true);

    // a login for another site does not leak to this one
    m.settings.lock().logins.push(SiteLogin { site: "example.org".into(), user: "me".into(), pass: "s3cret".into() });
    let t = m.add(req(&url, &dir));
    assert_eq!(wait_status(&m, t.id, Status::Complete, 10), Status::Error);
    m.delete(&t, true);

    // the matching login (host + path prefix) makes it work
    m.settings.lock().logins.push(SiteLogin { site: "127.0.0.1/private".into(), user: "me".into(), pass: "s3cret".into() });
    let t = m.add(req(&url, &dir));
    assert_eq!(wait_status(&m, t.id, Status::Complete, 15), Status::Complete);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), *data);
    // and the secret never lands in the saved download list
    assert!(t.info.lock().headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
}

#[test]
fn per_server_rule_limits_the_connections() {
    let (url, data, _) = serve(None);
    let (m, dir) = mgr("rule");
    {
        let mut s = m.settings.lock();
        s.connections = 8;
        s.conn_rules.push(ConnRule { host: "127.0.0.1".into(), connections: 2 });
    }
    let t = m.add(req(&url, &dir));
    assert_eq!(wait_status(&m, t.id, Status::Complete, 15), Status::Complete);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), *data);
    assert!(t.conn_cap.load(Relaxed) <= 2, "conn_cap = {}", t.conn_cap.load(Relaxed));
}

#[test]
fn quota_stops_downloads_and_blocks_new_ones() {
    let (url, _, _) = serve(None);
    let (m, dir) = mgr("quota");
    {
        let mut s = m.settings.lock();
        s.quota_on = true;
        s.quota_mb = 1;
        s.quota_hours = 1;
        s.global_speed_limit_kbps = 1500; // slow enough that the quota is hit before the file ends
    }
    m.apply_settings();
    let t = m.add(req(&url, &dir));
    let t0 = Instant::now();
    while !m.quota_blocked() && t0.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(m.quota_blocked(), "quota never tripped");
    assert!(m.take_quota_notice());
    assert!(!m.take_quota_notice(), "the notice fires once");
    assert_eq!(t.info.lock().status, Status::Stopped);
    // a manual start is refused with an explanation
    m.start(&t);
    assert_eq!(t.info.lock().status, Status::Stopped);
    assert!(t.info.lock().error.clone().unwrap().contains("quota"));
    // the reset button clears it
    m.reset_quota();
    assert!(!m.quota_blocked());
}

#[test]
fn finished_file_gets_the_servers_date_when_asked() {
    let (url, _, _) = serve(None);
    let (m, dir) = mgr("date");
    m.settings.lock().set_file_date = true;
    let t = m.add(req(&url, &dir));
    assert_eq!(wait_status(&m, t.id, Status::Complete, 15), Status::Complete);
    let mtime = std::fs::metadata(t.final_path()).unwrap().modified().unwrap();
    let secs = mtime.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(secs, 1_445_412_480); // Wed, 21 Oct 2015 07:28:00 GMT
}

#[test]
fn queue_reports_when_it_runs_dry() {
    let (url, _, _) = serve(None);
    let (m, dir) = mgr("queue");
    let seen = Arc::new(AtomicUsize::new(0));
    let s2 = seen.clone();
    m.set_event_hook(move |e| {
        if e == Event::QueueFinished {
            s2.fetch_add(1, Relaxed);
        }
    });
    let mut r = req(&url, &dir);
    r.queued = true;
    r.start = false;
    m.add(r);
    m.start_queue();
    let t0 = Instant::now();
    while seen.load(Relaxed) == 0 && t0.elapsed() < Duration::from_secs(20) {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(seen.load(Relaxed), 1);
    // starting an empty queue is not a "finished" queue
    m.start_queue();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(seen.load(Relaxed), 1);
}
