use std::path::{Path, PathBuf};

/// Downloads a small resource (an avatar, say) into memory through `client`: 15 s timeout, redirects
/// followed, and at most `max` bytes accepted.
pub fn fetch_small(client: &reqwest::blocking::Client, url: &str, max: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let resp = client.get(url).timeout(std::time::Duration::from_secs(15)).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut buf = Vec::new();
    resp.take(max as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() > max {
        return Err("response too large".into());
    }
    Ok(buf)
}

pub fn now_ts() -> i64 {
    chrono::Local::now().timestamp()
}

fn trunc(v: f64, prec: usize) -> f64 {
    let m = 10f64.powi(prec as i32);
    (v * m).floor() / m
}

/// Truncates instead of rounding (like IDM), so a 99.9% file never reads "100.00".
pub fn fmt_size_prec(b: u64, prec: usize) -> String {
    let f = b as f64;
    const K: f64 = 1024.0;
    if b < 1024 {
        format!("{b} Bytes")
    } else if f < K * K {
        format!("{:.*} KB", prec, trunc(f / K, prec))
    } else if f < K * K * K {
        format!("{:.*} MB", prec, trunc(f / (K * K), prec))
    } else {
        format!("{:.*} GB", prec, trunc(f / (K * K * K), prec))
    }
}

/// "57.94 MB" – the format used in the main list.
pub fn fmt_size(b: u64) -> String {
    fmt_size_prec(b, 2)
}

pub fn fmt_rate(bps: u64, prec: usize) -> String {
    if bps == 0 {
        return String::new();
    }
    format!("{}/sec", fmt_size_prec(bps, prec))
}

pub fn fmt_time(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} sec")
    } else if secs < 3600 {
        format!("{} min {} sec", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{} hour(s) {} min", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{} day(s) {} hour(s)", secs / 86_400, (secs % 86_400) / 3600)
    }
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn percent_decode(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(s)).into_owned()
}

pub fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Makes a name safe on every platform: no separators or control characters, no Windows reserved
/// device names, no trailing dots/spaces, at most 180 characters (the extension is kept).
/// Leading dots are kept so `.gitignore`-style names survive.
pub fn sanitize_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let mut name = cleaned.trim().trim_end_matches(|c| c == '.' || c == ' ').to_string();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        return "download".into();
    }
    if name.chars().count() > 180 {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if name.len() - i <= 12 && i > 0 => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        };
        let keep = 180usize.saturating_sub(ext.chars().count());
        name = format!("{}{}", stem.chars().take(keep).collect::<String>(), ext);
    }
    let stem_upper = name.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if RESERVED.contains(&stem_upper.as_str()) {
        name.insert(0, '_');
    }
    name
}

pub fn file_name_from_url(url: &str) -> String {
    let parsed = reqwest::Url::parse(url).ok();
    let name = parsed
        .as_ref()
        .and_then(|u| u.path_segments().and_then(|s| s.last().map(|x| x.to_string())))
        .map(|n| percent_decode(&n))
        .filter(|n| !n.trim().is_empty());
    match name {
        Some(n) => sanitize_file_name(&n),
        // "https://example.com/" -> "example.com"
        None => sanitize_file_name(parsed.as_ref().and_then(|u| u.host_str()).unwrap_or("download")),
    }
}

/// Decodes raw header bytes: UTF-8 when valid (servers do send `filename="中文.zip"` unescaped),
/// otherwise Latin-1, which is what HTTP says non-ASCII header bytes mean.
pub fn decode_header_bytes(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    }
}

/// Splits `attachment; filename="a;b.zip"; filename*=UTF-8''x` into (lower-case key, value) pairs,
/// honouring quoted strings (including `\"` escapes) so a `;` inside a name does not cut it short.
fn disposition_params(h: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = h.chars().peekable();
    // skip the disposition type ("attachment" / "inline")
    while let Some(&c) = chars.peek() {
        chars.next();
        if c == ';' {
            break;
        }
    }
    loop {
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ';' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            if chars.next().is_none() {
                break;
            }
            continue;
        }
        chars.next(); // '='
        while chars.peek().map_or(false, |c| c.is_whitespace()) {
            chars.next();
        }
        let mut val = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            val.push(n);
                        }
                    }
                    '"' => break,
                    _ => val.push(c),
                }
            }
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == ';' {
                    break;
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == ';' {
                    break;
                }
                val.push(c);
            }
        }
        out.push((key.trim().to_ascii_lowercase(), val.trim().to_string()));
    }
    out
}

/// RFC 5987 `charset'lang'percent-encoded` value.
fn decode_ext_value(v: &str) -> String {
    let mut it = v.splitn(3, '\'');
    let (charset, _lang, enc) = (it.next().unwrap_or(""), it.next(), it.next());
    let Some(enc) = enc else { return percent_decode(v) };
    let bytes = percent_decode_bytes(enc);
    if charset.eq_ignore_ascii_case("iso-8859-1") || charset.eq_ignore_ascii_case("latin1") {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// File name from a `Content-Disposition` value. `filename*` (RFC 5987/6266) beats `filename`;
/// RFC 2231 continuations (`filename*0*`, `filename*1*`, …) are joined.
pub fn file_name_from_disposition(h: &str) -> Option<String> {
    let params = disposition_params(h);
    let get = |k: &str| params.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.as_str());
    let mut best = get("filename*").map(decode_ext_value).filter(|s| !s.trim().is_empty());
    if best.is_none() {
        let mut parts: Vec<(u32, String)> = Vec::new();
        for (k, v) in &params {
            if let Some(rest) = k.strip_prefix("filename*") {
                if let Ok(n) = rest.trim_end_matches('*').parse::<u32>() {
                    parts.push((n, v.clone()));
                }
            }
        }
        if !parts.is_empty() {
            parts.sort_by_key(|p| p.0);
            let joined: String = parts.into_iter().map(|p| p.1).collect();
            best = Some(decode_ext_value(&joined)).filter(|s| !s.trim().is_empty());
        }
    }
    let name = best.or_else(|| get("filename").filter(|s| !s.trim().is_empty()).map(String::from))?;
    // a path in the header must never escape the target folder
    let leaf = name.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(&name);
    Some(sanitize_file_name(leaf))
}

pub fn file_name_from_disposition_bytes(raw: &[u8]) -> Option<String> {
    file_name_from_disposition(&decode_header_bytes(raw))
}

/// Free space (bytes) on the volume that holds `dir`, looking at the nearest existing ancestor.
pub fn free_space(dir: &Path) -> Option<u64> {
    let mut p = dir;
    loop {
        if p.exists() {
            return fs4::available_space(p).ok();
        }
        p = p.parent()?;
    }
}

/// `master.m3u8` -> `master.ts` (HLS is saved as one MPEG-TS / fMP4 file).
pub fn hls_name(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".ts") || l.ends_with(".mp4") {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(i) if l.ends_with(".m3u8") || l.ends_with(".m3u") => format!("{}.ts", &name[..i]),
        _ => format!("{name}.ts"),
    }
}

/// Like `hls_name` but leaves every other file name untouched.
pub fn hls_name_if_playlist(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".m3u8") || l.ends_with(".m3u") {
        hls_name(name)
    } else {
        name.to_string()
    }
}

/// Returns a file name that collides with neither a finished file nor a `.fcpart`.
pub fn unique_name(dir: &Path, name: &str) -> String {
    let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}.fcpart")).exists() || dir.join(format!("{n}.fcparts")).exists();
    if !taken(name) {
        return name.to_string();
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|c| !taken(c))
        .unwrap()
}

pub fn split_path(full: &str) -> (PathBuf, String) {
    let p = Path::new(full);
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    (dir, name)
}

// ───────────── matching helpers used by the options (site logins, per-server limits, clipboard) ─────────────

/// Case-insensitive wildcard match: `*` is any run of characters, `?` exactly one.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// `example.com` matches that host and every sub-domain of it; `*.example.com` and other
/// wildcard patterns are matched literally with [`glob_match`].
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().trim_start_matches('.').to_ascii_lowercase();
    let h = host.trim().to_ascii_lowercase();
    if p.is_empty() || h.is_empty() {
        return false;
    }
    if let Some(bare) = p.strip_prefix("*.") {
        return h == bare || h.ends_with(&format!(".{bare}"));
    }
    if p.contains('*') || p.contains('?') {
        return glob_match(&p, &h);
    }
    h == p || h.ends_with(&format!(".{p}"))
}

/// Standard base64 (with padding), for HTTP Basic credentials.
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if c.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Percent-encodes everything except unreserved characters (for proxy user names and passwords in a URL).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Splits a command line into program and arguments the way a shell would for simple cases:
/// spaces separate words, double quotes group them. No variables, no pipes, no substitution -
/// the command is never given to a shell, so a file name can never inject anything.
pub fn split_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for ch in cmd.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_truncate() {
        assert_eq!(fmt_size(7_749_486), "7.39 MB");
        assert_eq!(fmt_size(2_096_000), "1.99 MB");
        assert_eq!(fmt_size(8), "8 Bytes");
        assert_eq!(fmt_size_prec(13_238_272, 3), "12.625 MB");
    }

    #[test]
    fn names() {
        assert_eq!(file_name_from_url("https://x.com/a/b%20c.mp4?x=1"), "b c.mp4");
        assert_eq!(
            file_name_from_disposition("attachment; filename=\"a.zip\"").as_deref(),
            Some("a.zip")
        );
        assert_eq!(
            file_name_from_disposition("attachment; filename*=UTF-8''na%C3%AFve.txt").as_deref(),
            Some("naïve.txt")
        );
    }

    #[test]
    fn disposition_edge_cases() {
        // semicolon inside quotes, escaped quote, filename* wins, paths are stripped
        assert_eq!(file_name_from_disposition("attachment; filename=\"a;b.zip\"").as_deref(), Some("a;b.zip"));
        assert_eq!(
            file_name_from_disposition("attachment; filename=\"x.zip\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.zip").as_deref(),
            Some("中文.zip")
        );
        assert_eq!(file_name_from_disposition("attachment; filename=\"../../etc/passwd\"").as_deref(), Some("passwd"));
        assert_eq!(file_name_from_disposition("attachment; filename*0*=UTF-8''ab; filename*1*=cd.txt").as_deref(), Some("abcd.txt"));
        assert_eq!(file_name_from_disposition("inline").as_deref(), None);
        assert_eq!(file_name_from_disposition_bytes("attachment; filename=\"中文.zip\"".as_bytes()).as_deref(), Some("中文.zip"));
        assert_eq!(file_name_from_disposition_bytes(b"attachment; filename=\"caf\xe9.txt\"").as_deref(), Some("café.txt"));
    }

    #[test]
    fn sanitize_rules() {
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("nul"), "_nul");
        assert_eq!(sanitize_file_name(".gitignore"), ".gitignore");
        assert_eq!(sanitize_file_name("a/b:c?.mp4"), "a_b_c_.mp4");
        assert_eq!(sanitize_file_name("..."), "download");
        assert_eq!(sanitize_file_name("name. "), "name");
        let long = format!("{}.mkv", "x".repeat(300));
        let s = sanitize_file_name(&long);
        assert!(s.chars().count() <= 180 && s.ends_with(".mkv"));
        assert_eq!(file_name_from_url("https://example.com/"), "example.com");
    }

    #[test]
    fn time() {
        assert_eq!(fmt_time(219), "3 min 39 sec");
        assert_eq!(fmt_time(0), "0 sec");
    }

    #[test]
    fn glob_and_host_matching() {
        assert!(glob_match("R0*", "r01"));
        assert!(glob_match("*.zip", "A.ZIP"));
        assert!(!glob_match("zip", "zipx"));
        assert!(glob_match("m?3", "mp3"));
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("example.com", "cdn.example.com"));
        assert!(!host_matches("example.com", "badexample.com"));
        assert!(host_matches("*.example.com", "example.com"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(host_matches("cdn*.example.*", "cdn3.example.org"));
        assert!(!host_matches("", "example.com"));
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"user:pa55"), "dXNlcjpwYTU1");
    }

    #[test]
    fn command_lines_split_without_a_shell() {
        assert_eq!(split_command("clamscan %FILE%"), vec!["clamscan", "%FILE%"]);
        assert_eq!(
            split_command("\"C:\\Program Files\\x.exe\" -Scan -File \"%FILE%\""),
            vec!["C:\\Program Files\\x.exe", "-Scan", "-File", "%FILE%"]
        );
        assert!(split_command("   ").is_empty());
        assert_eq!(percent_encode("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
    }
}
