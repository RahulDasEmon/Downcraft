//! Engine behaviour that needs a misbehaving server: slow pieces, connection limits, a file that
//! changes between pause and resume, dead connections, and the global speed limit.
use downcraft_core::{Category, Manager, NewDownload, Status};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

fn pattern(len: usize, salt: u32) -> Vec<u8> {
    (0..len as u32).map(|i| (i.wrapping_add(salt).wrapping_mul(2654435761) >> 13) as u8).collect()
}

#[derive(Default)]
struct Srv {
    data: Mutex<Vec<u8>>,
    etag: Mutex<String>,
    /// requests that start at byte 0 are sent at ~2 MB/s
    slow_first: bool,
    /// more than this many simultaneous requests get a 429
    max_conc: Option<usize>,
    /// the first ranged (non-probe) request gets headers and then silence
    stall_first: AtomicBool,
    active: AtomicUsize,
    peak: AtomicUsize,
    split_hits: AtomicUsize,
}

fn handle(mut s: TcpStream, srv: Arc<Srv>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let (mut range, mut if_range) = (None, None);
    loop {
        line.clear();
        if r.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        let l = line.trim().to_string();
        let lower = l.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("range: bytes=") {
            let (a, b) = v.split_once('-').unwrap();
            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().ok()));
        }
        if lower.starts_with("if-range:") {
            if_range = Some(l[9..].trim().to_string());
        }
    }
    let data = srv.data.lock().unwrap().clone();
    let etag = srv.etag.lock().unwrap().clone();

    let now = srv.active.fetch_add(1, SeqCst) + 1;
    srv.peak.fetch_max(now, SeqCst);
    struct Leave(Arc<Srv>);
    impl Drop for Leave {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, SeqCst);
        }
    }
    let _leave = Leave(srv.clone());

    let is_probe = matches!(range, Some((0, Some(0))));
    if let Some(max) = srv.max_conc {
        if !is_probe && now > max {
            let _ = write!(s, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        }
    }
    // an If-Range that no longer matches means "send the whole, new file"
    let range = if if_range.as_deref().map_or(false, |v| v != etag) { None } else { range };
    match range {
        Some((a, b)) => {
            let b = b.unwrap_or(data.len() - 1).min(data.len() - 1);
            let _ = write!(
                s,
                "HTTP/1.1 206 Partial Content\r\nETag: {etag}\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len(),
                b - a + 1
            );
            if !is_probe && srv.stall_first.swap(false, SeqCst) {
                let _ = s.write_all(&data[a..a + 1000]);
                let _ = s.flush();
                thread::sleep(Duration::from_secs(60));
                return;
            }
            if !is_probe && a > 0 && srv.slow_first {
                srv.split_hits.fetch_add(1, SeqCst);
            }
            let slow = srv.slow_first && a == 0 && !is_probe;
            // with a connection limit, requests must stay open long enough to overlap
            let hold = srv.max_conc.is_some() && !is_probe;
            for chunk in data[a..=b].chunks(16 * 1024) {
                if s.write_all(chunk).is_err() {
                    return;
                }
                if slow {
                    thread::sleep(Duration::from_millis(8));
                } else if hold {
                    thread::sleep(Duration::from_millis(3));
                }
            }
        }
        None => {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
            let _ = s.write_all(&data);
        }
    }
}

fn serve(srv: Srv) -> (String, Arc<Srv>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/big.bin", l.local_addr().unwrap());
    let srv = Arc::new(srv);
    let s2 = srv.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let s2 = s2.clone();
            thread::spawn(move || handle(s, s2));
        }
    });
    (url, srv)
}

fn mgr(tag: &str, connections: u32) -> (Arc<Manager>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fc-eng-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let m = Manager::with_dir(dir.join("data"));
    {
        let mut s = m.settings.lock();
        s.connections = connections;
        s.auto_retries = 0; // keep failures visible to the tests
    }
    (m, dir)
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

fn wait_done(m: &Arc<Manager>, id: u64, secs: u64) {
    let t0 = Instant::now();
    loop {
        let t = m.find(id).unwrap();
        let st = t.info.lock().status;
        match st {
            Status::Complete => return,
            Status::Error => panic!("failed: {:?}", t.info.lock().error),
            _ => {}
        }
        assert!(t0.elapsed() < Duration::from_secs(secs), "timeout after {secs}s");
        thread::sleep(Duration::from_millis(50));
    }
}

fn server(len: usize, etag: &str, mk: impl FnOnce(&mut Srv)) -> (String, Arc<Srv>, Vec<u8>) {
    let data = pattern(len, 0);
    let mut s = Srv { data: Mutex::new(data.clone()), etag: Mutex::new(etag.into()), ..Default::default() };
    mk(&mut s);
    let (url, srv) = serve(s);
    (url, srv, data)
}

#[test]
fn slow_segment_is_split_so_idle_workers_help() {
    // 16 MB over 4 connections = four 4 MB pieces; the first crawls at ~2 MB/s (2 s) while the
    // others finish at once. The finished workers must split the first piece instead of idling.
    let (url, srv, data) = server(16_000_000, "\"v1\"", |s| s.slow_first = true);
    let (m, dir) = mgr("split", 4);
    let t0 = Instant::now();
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id, 40);
    assert!(t.segs.read().len() > 4, "no dynamic split happened: {} segments", t.segs.read().len());
    assert!(srv.split_hits.load(SeqCst) > 0);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), data, "output must stay byte-exact after splitting");
    assert!(t0.elapsed() < Duration::from_millis(1900), "splitting should beat the slow connection ({:?})", t0.elapsed());
    // segments tile the file exactly
    let mut v: Vec<(u64, u64)> = t.segs.read().iter().map(|s| (s.start, s.end.load(SeqCst))).collect();
    v.sort();
    assert_eq!(v[0].0, 0);
    for w in v.windows(2) {
        assert_eq!(w[0].1 + 1, w[1].0);
    }
    assert_eq!(v.last().unwrap().1, 15_999_999);
}

#[test]
fn server_that_limits_connections_gets_fewer_of_them() {
    let (url, srv, data) = server(6_000_000, "\"v1\"", |s| {
        s.slow_first = false;
        s.max_conc = Some(2);
    });
    let (m, dir) = mgr("cap", 6);
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id, 60);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), data);
    let cap = m.host_cap("127.0.0.1").expect("host cap must be remembered");
    assert!(cap < 6, "cap = {cap}");
    assert!(srv.peak.load(SeqCst) >= 2);
}

#[test]
fn changed_file_on_resume_restarts_instead_of_corrupting() {
    let (url, srv, _old) = server(4_000_000, "\"v1\"", |_| {});
    let (m, dir) = mgr("changed", 4);
    let mut r = req(&url, &dir);
    r.start = false;
    let t = m.add(r);
    t.limit_kbps.store(1500, SeqCst);
    m.start(&t);
    thread::sleep(Duration::from_millis(700));
    m.pause(&t);
    assert!(t.downloaded() > 0 && t.downloaded() < 4_000_000);
    // the file on the server is replaced by different content of the same size
    let newer = pattern(4_000_000, 777);
    *srv.data.lock().unwrap() = newer.clone();
    *srv.etag.lock().unwrap() = "\"v2\"".into();
    t.limit_kbps.store(0, SeqCst);
    m.start(&t);
    wait_done(&m, t.id, 30);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), newer);
}

#[test]
fn stalled_connection_is_reopened() {
    let (url, _srv, data) = server(2_000_000, "\"v1\"", |s| s.stall_first = AtomicBool::new(true));
    let (m, dir) = mgr("stall", 2);
    m.stall_secs.store(2, SeqCst);
    let t0 = Instant::now();
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id, 30);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), data);
    assert!(t0.elapsed() >= Duration::from_secs(2), "the stall was not even noticed");
}

#[test]
fn global_speed_limit_applies_to_all_downloads_together() {
    let (url, _srv, data) = server(3_000_000, "\"v1\"", |_| {});
    let (m, dir) = mgr("global", 4);
    {
        m.settings.lock().global_speed_limit_kbps = 1500;
    }
    m.apply_settings();
    let t0 = Instant::now();
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id, 30);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), data);
    // 3 MB at 1.5 MB/s ≈ 2 s, minus the 250 ms burst credit
    assert!(t0.elapsed() >= Duration::from_millis(1400), "limit ignored: {:?}", t0.elapsed());
}

#[test]
fn failed_network_download_is_retried_automatically() {
    // nothing listens on the port: the first attempts fail, the download is marked transient
    let (m, dir) = mgr("retry", 2);
    m.settings.lock().auto_retries = 2;
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x.bin", l.local_addr().unwrap());
    drop(l);
    let t = m.add(req(&url, &dir));
    let t0 = Instant::now();
    while t.info.lock().status != Status::Error {
        assert!(t0.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(50));
    }
    assert!(t.transient.load(SeqCst), "connection refused is worth retrying");
    // the 500 ms heartbeat restarts it once its back-off (5 s) is over
    let t1 = Instant::now();
    while t.retries.load(SeqCst) == 0 {
        assert!(t1.elapsed() < Duration::from_secs(12), "no automatic retry happened");
        thread::sleep(Duration::from_millis(100));
    }
}
