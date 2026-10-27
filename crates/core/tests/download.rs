//! Spins up a tiny Range-capable HTTP server and downloads from it with the real engine.
use downcraft_core::{Category, Manager, NewDownload, Status};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn body() -> Arc<Vec<u8>> {
    Arc::new((0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect())
}

fn handle(mut s: TcpStream, data: Arc<Vec<u8>>, ranges: bool) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut range = None;
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    loop {
        line.clear();
        r.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (a, b) = v.trim().split_once('-').unwrap();
            let a: usize = a.parse().unwrap();
            let b: usize = if b.is_empty() { data.len() - 1 } else { b.parse().unwrap() };
            range = Some((a, b.min(data.len() - 1)));
        }
    }
    match (range, ranges) {
        (Some((a, b)), true) => {
            let _ = write!(
                s,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len(),
                b - a + 1
            );
            let _ = s.write_all(&data[a..=b]);
        }
        _ => {
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=\"served.bin\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = s.write_all(&data);
        }
    }
}

fn serve(ranges: bool) -> (String, Arc<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.bin", l.local_addr().unwrap());
    let data = body();
    let d = data.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let d = d.clone();
            thread::spawn(move || handle(s, d, ranges));
        }
    });
    (url, data)
}

fn wait_done(m: &Arc<Manager>, id: u64) {
    let t0 = Instant::now();
    loop {
        let t = m.find(id).unwrap();
        let st = t.info.lock().status;
        match st {
            Status::Complete => return,
            Status::Error => panic!("failed: {:?}", t.info.lock().error),
            _ => {}
        }
        assert!(t0.elapsed() < Duration::from_secs(30), "timeout");
        thread::sleep(Duration::from_millis(50));
    }
}

fn mgr(tag: &str) -> (Arc<Manager>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fc-test-{tag}-{}", std::process::id()));
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

#[test]
fn segmented_download_is_byte_exact() {
    let (url, data) = serve(true);
    let (m, dir) = mgr("seg");
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id);
    assert!(t.segs.read().len() > 1, "expected multiple connections");
    assert_eq!(std::fs::read(t.final_path()).unwrap(), *data);
    assert_eq!(t.info.lock().file_name, "file.bin");
}

#[test]
fn falls_back_to_single_connection_without_range_support() {
    let (url, data) = serve(false);
    let (m, dir) = mgr("norange");
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id);
    assert_eq!(t.segs.read().len(), 1);
    assert!(!t.info.lock().resumable);
    assert_eq!(t.info.lock().file_name, "served.bin");
    assert_eq!(std::fs::read(t.final_path()).unwrap(), *data);
}

#[test]
fn pause_and_resume_keeps_data_intact() {
    let (url, data) = serve(true);
    let (m, dir) = mgr("pause");
    let mut r = req(&url, &dir);
    r.start = false;
    let t = m.add(r);
    t.limit_kbps.store(2000, std::sync::atomic::Ordering::Relaxed);
    m.start(&t);
    thread::sleep(Duration::from_millis(500));
    m.pause(&t);
    assert_eq!(t.info.lock().status, Status::Stopped);
    let partial = t.downloaded();
    assert!(partial > 0 && partial < data.len() as u64, "partial = {partial}");
    t.limit_kbps.store(0, std::sync::atomic::Ordering::Relaxed);
    m.start(&t);
    wait_done(&m, t.id);
    assert_eq!(std::fs::read(t.final_path()).unwrap(), *data);
}

#[test]
fn state_survives_restart() {
    let (url, _data) = serve(true);
    let (m, dir) = mgr("persist");
    let t = m.add(req(&url, &dir));
    wait_done(&m, t.id);
    m.save();
    let m2 = Manager::with_dir(dir.join("data"));
    assert_eq!(m2.tasks.lock().len(), 1);
    assert_eq!(m2.tasks.lock()[0].info.lock().status, Status::Complete);
    let _ = Read::bytes(std::io::empty()).count();
}
