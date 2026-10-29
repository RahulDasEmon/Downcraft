//! The online toolbar-skin gallery.
//!
//! Skins are published on the `Toolbar-Skin` branch of the DownCraft repository:
//!
//! ```text
//! skin.json            the index (see below)
//! skin/<id>.zip        one skin pack per entry
//! preview/<id>.png     a picture of the toolbar with that skin (.png, .jpg or .jpeg)
//! ```
//!
//! `skin.json` is `{ "skins": [ { "id": "neon", "name": "Neon", "author": "…" } ] }` (a bare array
//! works too). `zip` and `preview` are optional: they default to `skin/<id>.zip` and
//! `preview/<id>.png` (then `.jpg`, `.jpeg`: the first one that exists is used). Adding a skin to the gallery is therefore: drop the two files in, add one line
//! to `skin.json`, push. No new release of the app is needed.
use serde::Deserialize;
use std::io::Read;

/// Where the gallery lives (raw file access on the branch).
pub const BASE: &str = "https://raw.githubusercontent.com/RahulDasEmon/Downcraft/Toolbar-Skin/";
/// The page people can browse in a web browser.
pub const PAGE: &str = "https://github.com/RahulDasEmon/Downcraft/tree/Toolbar-Skin";

/// Largest skin pack / preview / index accepted.
pub const MAX_ZIP: u64 = 16 * 1024 * 1024;
pub const MAX_PREVIEW: u64 = 4 * 1024 * 1024;
pub const MAX_INDEX: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct SkinEntry {
    /// Lower-case letters, digits, `-` and `_`: safe to use as a file name.
    pub id: String,
    pub name: String,
    pub author: String,
    /// Full URL of the skin pack.
    pub zip: String,
    /// Full URLs to try for the preview picture, in order (one when `skin.json` names it).
    pub previews: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    id: Option<String>,
    name: Option<String>,
    author: Option<String>,
    zip: Option<String>,
    preview: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Index {
    List(Vec<Raw>),
    Wrapped { skins: Vec<Raw> },
}

fn clean_id(s: &str) -> String {
    s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect::<String>().trim_matches('-').to_string()
}

/// Turns a path from the index into a full URL on the branch. Absolute links are only accepted
/// when they point into the same branch.
fn resolve(path: &str) -> Option<String> {
    let p = path.trim();
    if p.is_empty() {
        return None;
    }
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.starts_with(BASE).then(|| p.to_string());
    }
    let p = p.trim_start_matches('/');
    if p.split(['/', '\\']).any(|seg| seg == ".." || seg.contains(':')) {
        return None;
    }
    Some(format!("{BASE}{}", p.replace(' ', "%20")))
}

/// Reads `skin.json`. Entries that cannot be used are skipped rather than failing the whole list.
pub fn parse_index(text: &str) -> Result<Vec<SkinEntry>, String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err("the skin list (skin.json) is empty".into());
    }
    let raws = match serde_json::from_str::<Index>(text).map_err(|e| format!("the skin list is not valid ({e})"))? {
        Index::List(l) => l,
        Index::Wrapped { skins } => skins,
    };
    let mut out: Vec<SkinEntry> = Vec::new();
    for r in raws {
        let from_zip = r
            .zip
            .as_deref()
            .and_then(|z| z.rsplit(['/', '\\']).next())
            .map(|f| f.strip_suffix(".zip").unwrap_or(f).to_string())
            .unwrap_or_default();
        let id = clean_id(&r.id.clone().or_else(|| r.name.clone()).filter(|s| !s.trim().is_empty()).unwrap_or(from_zip));
        if id.is_empty() || out.iter().any(|e| e.id == id) {
            continue;
        }
        let Some(zip) = resolve(&r.zip.unwrap_or_else(|| format!("skin/{id}.zip"))) else { continue };
        let previews: Vec<String> = match r.preview.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => resolve(p).into_iter().collect(),
            None => ["png", "jpg", "jpeg"].iter().filter_map(|ext| resolve(&format!("preview/{id}.{ext}"))).collect(),
        };
        let name = r.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
        out.push(SkinEntry { id, name, author: r.author.unwrap_or_default().trim().to_string(), zip, previews });
    }
    Ok(out)
}

/// Downloads `url` into memory, refusing anything bigger than `max` bytes.
pub fn fetch(client: &reqwest::blocking::Client, url: &str, max: u64) -> Result<Vec<u8>, String> {
    // the shared client has no overall timeout (downloads can take hours); a skin must not hang
    let resp = client.get(url).timeout(std::time::Duration::from_secs(60)).send().map_err(|e| format!("cannot reach GitHub ({})", short(&e.to_string())))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(if status.as_u16() == 404 { "not found on GitHub".to_string() } else { format!("GitHub answered {status}") });
    }
    if resp.content_length().map_or(false, |n| n > max) {
        return Err("the file is too large".into());
    }
    let mut buf = Vec::new();
    resp.take(max + 1).read_to_end(&mut buf).map_err(|e| format!("download interrupted ({})", short(&e.to_string())))?;
    if buf.len() as u64 > max {
        return Err("the file is too large".into());
    }
    Ok(buf)
}

fn short(s: &str) -> String {
    if s.chars().count() > 90 { format!("{}…", s.chars().take(89).collect::<String>()) } else { s.to_string() }
}

/// The list of skins currently published.
pub fn fetch_index(client: &reqwest::blocking::Client) -> Result<Vec<SkinEntry>, String> {
    let bytes = fetch(client, &format!("{BASE}skin.json"), MAX_INDEX)?;
    parse_index(&String::from_utf8_lossy(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wrapped_and_bare_lists() {
        let a = parse_index(r#"{"skins":[{"id":"neon","name":"Neon","author":"Emon"}]}"#).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].zip, format!("{BASE}skin/neon.zip"));
        assert_eq!(a[0].previews.len(), 3);
        assert_eq!(a[0].previews[0], format!("{BASE}preview/neon.png"));
        assert!(a[0].previews[1].ends_with("preview/neon.jpg"));
        let b = parse_index(r#"[{"name":"Dark Glass","zip":"skin/dark-glass.zip","preview":"preview/dg.png"}]"#).unwrap();
        assert_eq!(b[0].id, "dark-glass");
        assert_eq!(b[0].name, "Dark Glass");
        assert_eq!(b[0].previews.len(), 1);
        assert!(b[0].previews[0].ends_with("preview/dg.png"));
    }

    #[test]
    fn empty_or_broken_index_is_an_error() {
        assert!(parse_index("").is_err());
        assert!(parse_index("  \n").unwrap_err().contains("empty"));
        assert!(parse_index("{oops").is_err());
    }

    #[test]
    fn unsafe_entries_are_skipped() {
        let t = r#"{"skins":[
            {"id":"a","zip":"../../evil.zip"},
            {"id":"b","zip":"https://evil.example/x.zip"},
            {"id":"ok"},
            {"id":"ok"}
        ]}"#;
        let l = parse_index(t).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, "ok");
    }
}
