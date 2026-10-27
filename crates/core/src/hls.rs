//! HLS (`.m3u8`) downloader: picks the best variant, fetches segments in parallel into a scratch folder
//! (so pause/resume works), decrypts clear-key AES-128, then stitches everything into one file.
//! DRM schemes (SAMPLE-AES, Widevine, FairPlay) and live streams are refused with a clear message.
use crate::{
    download::{fail_with, with_headers},
    manager::{Event, Manager},
    task::*,
    util, Status,
};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use parking_lot::Mutex;
use reqwest::{blocking::Client, Url};
use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

type Hdrs = Vec<(String, String)>;
const MAX_TRIES: u32 = 5;


/// `(offset, length)` of a sub-range of a resource (`#EXT-X-BYTERANGE`).
pub type ByteRange = (u64, u64);

#[derive(Debug, Clone)]
pub struct HSeg {
    pub url: String,
    pub key: Option<usize>,
    pub iv: Option<[u8; 16]>,
    pub seq: u64,
    pub range: Option<ByteRange>,
}

#[derive(Debug, Default)]
pub struct Playlist {
    pub segs: Vec<HSeg>,
    pub keys: Vec<String>,
    pub map: Option<(String, Option<ByteRange>)>,
}

/// `"len@off"` / `"len"`; a missing offset continues right after `prev_end`.
fn parse_range(v: &str, prev_end: Option<u64>) -> Option<ByteRange> {
    let v = v.trim().trim_matches('"');
    let (len, off) = match v.split_once('@') {
        Some((l, o)) => (l.trim().parse::<u64>().ok()?, o.trim().parse::<u64>().ok()?),
        None => (v.parse::<u64>().ok()?, prev_end?),
    };
    Some((off, len))
}

fn attr(line: &str, name: &str) -> Option<String> {
    let mut rest = line.split_once(':')?.1;
    loop {
        let (k, after) = rest.split_once('=')?;
        let k = k.trim_start_matches(',').trim();
        let (v, next) = if let Some(a) = after.strip_prefix('"') {
            let e = a.find('"')?;
            (a[..e].to_string(), a[e + 1..].trim_start_matches(','))
        } else {
            match after.split_once(',') {
                Some((v, n)) => (v.to_string(), n),
                None => (after.to_string(), ""),
            }
        };
        if k.eq_ignore_ascii_case(name) {
            return Some(v);
        }
        if next.is_empty() {
            return None;
        }
        rest = next;
    }
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let h = s.trim_start_matches("0x").trim_start_matches("0X");
    if h.len() > 32 || h.is_empty() {
        return None;
    }
    let v = u128::from_str_radix(h, 16).ok()?;
    Some(v.to_be_bytes())
}

fn fetch_text(client: &Client, url: &str, hdrs: &Hdrs) -> Result<String, String> {
    let r = with_headers(client.get(url), hdrs).timeout(Duration::from_secs(30)).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Server returned HTTP {}", r.status().as_u16()));
    }
    r.text().map_err(|e| e.to_string())
}

pub fn parse_media(text: &str, base: &Url) -> Result<Playlist, String> {
    let resolve = |u: &str| base.join(u).map(|u| u.to_string()).map_err(|e| e.to_string());
    let mut pl = Playlist::default();
    let (mut seq, mut pending, mut endlist, mut vod) = (0u64, false, false, false);
    let mut cur: Option<(usize, Option<[u8; 16]>)> = None;
    let mut pending_range: Option<ByteRange> = None;
    let mut last_range: Option<(String, u64)> = None; // (url, end offset) for open-ended byte ranges
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(r) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            seq = r.trim().parse().unwrap_or(0);
        } else if line.starts_with("#EXT-X-PLAYLIST-TYPE:VOD") {
            vod = true;
        } else if line.starts_with("#EXT-X-KEY:") {
            let method = attr(line, "METHOD").unwrap_or_default().to_ascii_uppercase();
            match method.as_str() {
                "NONE" => cur = None,
                "AES-128" => {
                    let uri = attr(line, "URI").ok_or("AES-128 key has no URI")?;
                    pl.keys.push(resolve(&uri)?);
                    cur = Some((pl.keys.len() - 1, attr(line, "IV").and_then(|v| parse_iv(&v))));
                }
                other => return Err(format!("This stream uses {other} protection (DRM) and can't be downloaded.")),
            }
        } else if line.starts_with("#EXT-X-MAP:") {
            let uri = resolve(&attr(line, "URI").ok_or("EXT-X-MAP without URI")?)?;
            let range = attr(line, "BYTERANGE").and_then(|r| parse_range(&r, None));
            pl.map = Some((uri, range));
        } else if let Some(r) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let prev = last_range.as_ref().map(|l| l.1);
            pending_range = Some(parse_range(r, prev).ok_or("Malformed EXT-X-BYTERANGE")?);
        } else if line.starts_with("#EXT-X-ENDLIST") {
            endlist = true;
        } else if line.starts_with("#EXTINF") {
            pending = true;
        } else if line.starts_with('#') {
        } else if pending {
            pending = false;
            let url = resolve(line)?;
            let range = pending_range.take();
            if let Some((off, len)) = range {
                last_range = Some((url.clone(), off + len));
            } else {
                last_range = None;
            }
            pl.segs.push(HSeg { url, key: cur.map(|c| c.0), iv: cur.and_then(|c| c.1), seq, range });
            seq += 1;
        }
    }
    if !endlist && !vod {
        return Err("This is a live stream; live streams can't be downloaded.".into());
    }
    if pl.segs.is_empty() {
        return Err("The playlist contains no segments.".into());
    }
    Ok(pl)
}

/// Follows master playlists (best BANDWIDTH wins) down to a media playlist.
pub fn load(client: &Client, start: &str, hdrs: &Hdrs) -> Result<Playlist, String> {
    let mut url = Url::parse(start).map_err(|e| e.to_string())?;
    for _ in 0..4 {
        let text = fetch_text(client, url.as_str(), hdrs)?;
        if !text.trim_start().starts_with("#EXTM3U") {
            return Err("The address did not return a valid HLS playlist.".into());
        }
        if text.contains("#EXT-X-STREAM-INF") {
            let mut best: Option<(u64, String)> = None;
            let mut lines = text.lines().map(str::trim);
            while let Some(l) = lines.next() {
                if l.starts_with("#EXT-X-STREAM-INF") {
                    let bw = attr(l, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
                    if let Some(uri) = lines.by_ref().find(|x| !x.is_empty() && !x.starts_with('#')) {
                        if best.as_ref().map_or(true, |b| bw > b.0) {
                            best = Some((bw, uri.to_string()));
                        }
                    }
                }
            }
            let (_, uri) = best.ok_or("Master playlist lists no variants.")?;
            url = url.join(&uri).map_err(|e| e.to_string())?;
            continue;
        }
        return parse_media(&text, &url);
    }
    Err("Too many nested playlists.".into())
}

fn get_bytes(client: &Client, url: &str, hdrs: &Hdrs, range: Option<ByteRange>) -> Result<Vec<u8>, String> {
    let mut req = with_headers(client.get(url), hdrs).timeout(Duration::from_secs(60));
    if let Some((off, len)) = range {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-{}", off, off + len.max(1) - 1));
    }
    let r = req.send().map_err(|e| e.to_string())?;
    let st = r.status();
    if !st.is_success() {
        return Err(format!("HTTP {}", st.as_u16()));
    }
    let bytes = r.bytes().map_err(|e| e.to_string())?.to_vec();
    match range {
        // a server that ignores Range sends the whole resource: cut our piece out of it
        Some((off, len)) if st.as_u16() == 200 => {
            let (a, b) = (off as usize, (off + len) as usize);
            bytes.get(a..b).map(|s| s.to_vec()).ok_or_else(|| "Byte range outside the resource".to_string())
        }
        _ => Ok(bytes),
    }
}

fn decrypt(data: &[u8], key: &[u8], iv: [u8; 16]) -> Result<Vec<u8>, String> {
    type Dec = cbc::Decryptor<aes::Aes128>;
    Dec::new_from_slices(key, &iv)
        .map_err(|_| "Bad AES key length".to_string())?
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| "Segment decryption failed (wrong key?)".to_string())
}

fn write_atomic(path: &PathBuf, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)
}

pub(crate) fn run(mgr: Arc<Manager>, task: Arc<Task>, epoch: u64, client: Client, hdrs: Hdrs) {
    let pl = match load(&client, &task.url, &hdrs) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            let transient = !(e.contains("HTTP 4") || e.contains("DRM") || e.contains("live stream") || e.contains("valid HLS"));
            return fail_with(&mgr, &task, epoch, e, transient);
        }
    };
    if task.epoch.load(Relaxed) != epoch {
        return;
    }
    let n = pl.segs.len();
    let dir = {
        let i = task.info.lock();
        i.dir.clone()
    };
    if let Err(e) = fs::create_dir_all(&dir).and_then(|_| fs::create_dir_all(task.hls_dir())) {
        return fail_with(&mgr, &task, epoch, format!("Cannot create folder: {e}"), false);
    }
    let seg_dir = task.hls_dir();
    task.hls_count.store(n as u64, Relaxed);
    task.hls_done.store(0, Relaxed);
    task.bytes_done.store(0, Relaxed);
    task.info.lock().resumable = true;

    // fragmented-MP4 init segment
    if let Some((m, range)) = &pl.map {
        let p = seg_dir.join("init.seg");
        if !p.exists() {
            match get_bytes(&client, m, &hdrs, *range).and_then(|b| write_atomic(&p, &b).map_err(|e| e.to_string())) {
                Ok(()) => {}
                Err(e) => return fail_with(&mgr, &task, epoch, format!("Init segment failed: {e}"), !e.contains("HTTP 4")),
            }
        }
    }

    let next = Arc::new(AtomicUsize::new(0));
    let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let keys: Arc<Mutex<HashMap<usize, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let workers = mgr.settings.lock().connections.clamp(1, 16) as usize;
    let mut handles = Vec::new();
    for _ in 0..workers.min(n) {
        task.live_workers.fetch_add(1, Relaxed);
        let (m, c, t, pl, h, nx, er, ks, sd) = (
            mgr.clone(),
            client.clone(),
            task.clone(),
            pl.clone(),
            hdrs.clone(),
            next.clone(),
            err.clone(),
            keys.clone(),
            seg_dir.clone(),
        );
        handles.push(thread::spawn(move || {
            worker(&m, c, &t, pl, h, nx, er, ks, sd, epoch);
            t.live_workers.fetch_sub(1, Relaxed);
        }));
    }
    {
        let mut i = task.info.lock();
        if task.epoch.load(Relaxed) == epoch {
            i.status = Status::Downloading;
            i.error = None;
        }
    }

    let (mut last_b, mut last_t, mut ema) = (0u64, Instant::now(), 0f64);
    let mut last_save = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(150));
        if task.epoch.load(Relaxed) != epoch {
            return;
        }
        let done = task.hls_done.load(Relaxed);
        let bytes = task.bytes_done.load(Relaxed);
        if done > 0 {
            task.total.store(bytes * n as u64 / done, Relaxed); // running estimate
        }
        let dt = last_t.elapsed();
        if dt >= Duration::from_millis(500) {
            let inst = bytes.saturating_sub(last_b) as f64 / dt.as_secs_f64();
            ema = if ema == 0.0 { inst } else { 0.6 * ema + 0.4 * inst };
            task.speed.store(ema as u64, Relaxed);
            last_b = bytes;
            last_t = Instant::now();
        }
        if last_save.elapsed() > Duration::from_secs(3) {
            mgr.save();
            last_save = Instant::now();
        }
        if let Some(e) = err.lock().clone() {
            let transient = !(e.contains("HTTP 4") || e.contains("decryption") || e.contains("Key download") || e.contains("Write failed"));
            return fail_with(&mgr, &task, epoch, e, transient);
        }
        if done as usize >= n {
            break;
        }
    }
    for h in handles {
        let _ = h.join();
    }
    if task.epoch.load(Relaxed) != epoch {
        return;
    }

    // stitch segments into one file
    let fmp4 = pl.map.is_some();
    if fmp4 {
        let mut i = task.info.lock();
        if i.file_name.to_ascii_lowercase().ends_with(".ts") {
            let stem = i.file_name[..i.file_name.len() - 3].to_string();
            i.file_name = util::unique_name(&i.dir, &format!("{stem}.mp4"));
        }
    }
    let part = task.part_path();
    let result = (|| -> std::io::Result<()> {
        let mut out = BufWriter::new(fs::File::create(&part)?);
        if fmp4 {
            std::io::copy(&mut fs::File::open(seg_dir.join("init.seg"))?, &mut out)?;
        }
        for k in 0..n {
            std::io::copy(&mut fs::File::open(seg_dir.join(format!("{k:06}.seg")))?, &mut out)?;
        }
        out.flush()
    })();
    if let Err(e) = result {
        return fail_with(&mgr, &task, epoch, format!("Joining segments failed: {e}"), false);
    }
    if let Err(e) = fs::rename(&part, task.final_path()) {
        return fail_with(&mgr, &task, epoch, format!("Cannot finalize file: {e}"), false);
    }
    let _ = fs::remove_dir_all(&seg_dir);
    task.total.store(task.bytes_done.load(Relaxed), Relaxed);
    task.speed.store(0, Relaxed);
    {
        let mut i = task.info.lock();
        i.status = Status::Complete;
        i.queued = false;
        i.error = None;
        i.completed_at = util::now_ts();
    }
    task.retries.store(0, Relaxed);
    mgr.push_event(Event::Completed(task.id));
    mgr.save();
}

#[allow(clippy::too_many_arguments)]
fn worker(
    mgr: &Arc<Manager>,
    client: Client,
    task: &Arc<Task>,
    pl: Arc<Playlist>,
    hdrs: Hdrs,
    next: Arc<AtomicUsize>,
    err: Arc<Mutex<Option<String>>>,
    keys: Arc<Mutex<HashMap<usize, Vec<u8>>>>,
    seg_dir: PathBuf,
    epoch: u64,
) {
    let alive = || task.epoch.load(Relaxed) == epoch && task.ctl.load(Relaxed) == CTL_RUN && err.lock().is_none();
    let nap = |d: Duration| {
        let end = Instant::now() + d;
        while Instant::now() < end && alive() {
            thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
        }
    };
    loop {
        if !alive() {
            return;
        }
        let idx = next.fetch_add(1, Relaxed);
        if idx >= pl.segs.len() {
            return;
        }
        let path = seg_dir.join(format!("{idx:06}.seg"));
        if let Ok(m) = fs::metadata(&path) {
            task.bytes_done.fetch_add(m.len(), Relaxed);
            task.hls_done.fetch_add(1, Relaxed);
            continue; // already downloaded before a pause / restart
        }
        let seg = &pl.segs[idx];
        let mut tries = 0u32;
        let data = loop {
            if !alive() {
                return;
            }
            let attempt = (|| -> Result<Vec<u8>, String> {
                let raw = get_bytes(&client, &seg.url, &hdrs, seg.range)?;
                match seg.key {
                    None => Ok(raw),
                    Some(ki) => {
                        let key = {
                            let cached = keys.lock().get(&ki).cloned();
                            match cached {
                                Some(k) => k,
                                None => {
                                    let k = get_bytes(&client, &pl.keys[ki], &hdrs, None).map_err(|e| format!("Key download failed: {e}"))?;
                                    keys.lock().insert(ki, k.clone());
                                    k
                                }
                            }
                        };
                        decrypt(&raw, &key, seg.iv.unwrap_or_else(|| (seg.seq as u128).to_be_bytes()))
                    }
                }
            })();
            match attempt {
                Ok(d) => break d,
                Err(e) => {
                    tries += 1;
                    let fatal = e.starts_with("HTTP 4") || e.contains("decryption") || e.contains("Key download");
                    if tries > MAX_TRIES || fatal {
                        let mut g = err.lock();
                        if g.is_none() {
                            *g = Some(format!("Segment {} failed: {e}", idx + 1));
                        }
                        return;
                    }
                    nap(Duration::from_millis(400 * tries as u64));
                }
            }
        };
        if !alive() {
            return;
        }
        if let Err(e) = write_atomic(&path, &data) {
            let mut g = err.lock();
            if g.is_none() {
                *g = Some(format!("Write failed: {e}"));
            }
            return;
        }
        task.bytes_done.fetch_add(data.len() as u64, Relaxed);
        task.hls_done.fetch_add(1, Relaxed);

        // honour the per-download and the global limit (both book time on shared timelines)
        mgr.account(data.len() as u64);
        let a = task.bucket.reserve(data.len() as u64, task.limit_kbps.load(Relaxed).saturating_mul(1024));
        let b = mgr.global_bucket.reserve(data.len() as u64, mgr.global_limit_bps());
        nap(a.max(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_attributes_and_ivs() {
        let l = r#"#EXT-X-KEY:METHOD=AES-128,URI="k,1.bin",IV=0x000102030405060708090a0b0c0d0e0f"#;
        assert_eq!(attr(l, "URI").as_deref(), Some("k,1.bin"));
        assert_eq!(attr(l, "METHOD").as_deref(), Some("AES-128"));
        assert_eq!(parse_iv("0x01").unwrap()[15], 1);
    }

    #[test]
    fn byte_range_segments_and_map() {
        let base = Url::parse("https://x.test/a/index.m3u8").unwrap();
        let t = "#EXTM3U\n#EXT-X-MAP:URI=\"f.mp4\",BYTERANGE=\"720@0\"\n#EXTINF:4,\n#EXT-X-BYTERANGE:1000@720\nf.mp4\n\
                 #EXTINF:4,\n#EXT-X-BYTERANGE:500\nf.mp4\n#EXT-X-ENDLIST\n";
        let p = parse_media(t, &base).unwrap();
        assert_eq!(p.map, Some(("https://x.test/a/f.mp4".to_string(), Some((0, 720)))));
        assert_eq!(p.segs[0].range, Some((720, 1000)));
        assert_eq!(p.segs[1].range, Some((1720, 500)), "open-ended range continues after the previous one");
    }

    #[test]
    fn media_playlist_rules() {
        let base = Url::parse("https://x.test/a/index.m3u8").unwrap();
        let ok = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:4,\ns1.ts\n#EXTINF:4,\n/s2.ts\n#EXT-X-ENDLIST\n";
        let p = parse_media(ok, &base).unwrap();
        assert_eq!(p.segs[0].url, "https://x.test/a/s1.ts");
        assert_eq!(p.segs[1].url, "https://x.test/s2.ts");
        assert_eq!(p.segs[1].seq, 8);
        assert!(parse_media("#EXTM3U\n#EXTINF:4,\ns.ts\n", &base).unwrap_err().contains("live"));
        assert!(parse_media("#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:4,\ns.ts\n#EXT-X-ENDLIST\n", &base)
            .unwrap_err()
            .contains("DRM"));
    }
}
