//! HLS end-to-end: master -> variant, plain and AES-128 encrypted, pause/resume-safe stitching.
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use downcraft_core::{Category, Manager, NewDownload, Status};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn plain_segments() -> Vec<Vec<u8>> {
    (0..9u32).map(|k| (0..90_000u32).map(|i| (i.wrapping_mul(2654435761).wrapping_add(k * 7919) >> 11) as u8).collect()).collect()
}

fn serve(routes: HashMap<String, (String, Vec<u8>)>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let routes = Arc::new(routes);
    thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            let routes = routes.clone();
            thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    line.clear();
                    r.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                }
                match routes.get(&path) {
                    Some((ct, body)) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    base
}

fn download(base: &str, path: &str, tag: &str) -> (Vec<u8>, String) {
    let dir = std::env::temp_dir().join(format!("fc-hls-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let m = Manager::with_dir(dir.join("data"));
    let t = m.add(NewDownload {
        url: format!("{base}{path}"),
        dir: dir.join("out"),
        file_name: String::new(),
        category: Category::Video,
        description: String::new(),
        probe: None,
        queued: false,
        start: true,
        headers: vec![],
    });
    let t0 = Instant::now();
    loop {
        let st = t.info.lock().status;
        if st == Status::Complete {
            break;
        }
        assert!(st != Status::Error, "failed: {:?}", t.info.lock().error);
        assert!(t0.elapsed() < Duration::from_secs(30), "timeout");
        thread::sleep(Duration::from_millis(50));
    }
    let name = t.info.lock().file_name.clone();
    assert!(!t.hls_dir().exists(), "scratch folder should be cleaned up");
    (std::fs::read(t.final_path()).unwrap(), name)
}

fn playlist(n: usize, key_line: &str) -> String {
    let mut s = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n");
    s += key_line;
    for i in 0..n {
        s += &format!("#EXTINF:4.0,\nseg{i}.ts\n");
    }
    s + "#EXT-X-ENDLIST\n"
}

#[test]
fn hls_master_plain() {
    let segs = plain_segments();
    let mut r: HashMap<String, (String, Vec<u8>)> = HashMap::new();
    r.insert("/master.m3u8".into(), ("application/vnd.apple.mpegurl".into(),
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=3000000\nhigh/index.m3u8\n".to_vec()));
    r.insert("/low/index.m3u8".into(), ("application/x-mpegurl".into(), playlist(1, "").into_bytes()));
    r.insert("/low/seg0.ts".into(), ("video/mp2t".into(), b"WRONG-VARIANT".to_vec()));
    r.insert("/high/index.m3u8".into(), ("application/x-mpegurl".into(), playlist(segs.len(), "").into_bytes()));
    for (i, s) in segs.iter().enumerate() {
        r.insert(format!("/high/seg{i}.ts"), ("video/mp2t".into(), s.clone()));
    }
    let base = serve(r);
    let (data, name) = download(&base, "/master.m3u8", "plain");
    assert_eq!(data, segs.concat(), "must pick the highest bandwidth variant and keep order");
    assert_eq!(name, "master.ts");
}

#[test]
fn hls_aes128_decrypts() {
    type Enc = cbc::Encryptor<aes::Aes128>;
    let segs = plain_segments();
    let key = *b"0123456789abcdef";
    let mut r: HashMap<String, (String, Vec<u8>)> = HashMap::new();
    r.insert("/enc/key.bin".into(), ("application/octet-stream".into(), key.to_vec()));
    // no explicit IV -> IV is the media sequence number
    r.insert("/enc/index.m3u8".into(), ("application/x-mpegurl".into(), playlist(segs.len(), "#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n").into_bytes()));
    for (i, s) in segs.iter().enumerate() {
        let iv = (i as u128).to_be_bytes();
        let ct = Enc::new_from_slices(&key, &iv).unwrap().encrypt_padded_vec_mut::<Pkcs7>(s);
        r.insert(format!("/enc/seg{i}.ts"), ("video/mp2t".into(), ct));
    }
    let base = serve(r);
    let (data, _) = download(&base, "/enc/index.m3u8", "aes");
    assert_eq!(data, segs.concat());
}

#[test]
fn live_and_drm_are_refused_politely() {
    let mut r: HashMap<String, (String, Vec<u8>)> = HashMap::new();
    r.insert("/live.m3u8".into(), ("application/x-mpegurl".into(), b"#EXTM3U\n#EXTINF:4,\ns.ts\n".to_vec()));
    let base = serve(r);
    let dir = std::env::temp_dir().join(format!("fc-hls-live-{}", std::process::id()));
    let m = Manager::with_dir(dir.join("data"));
    let t = m.add(NewDownload {
        url: format!("{base}/live.m3u8"), dir: dir.join("out"), file_name: String::new(), category: Category::Video,
        description: String::new(), probe: None, queued: false, start: true, headers: vec![],
    });
    let t0 = Instant::now();
    while t.info.lock().status != Status::Error {
        assert!(t0.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(50));
    }
    assert!(t.info.lock().error.clone().unwrap().contains("live"));
}
