//! Clipboard monitor: notices when a link to a download is copied.
use arboard::Clipboard;
use downcraft_core::Settings;
use std::time::{Duration, Instant};

pub struct Monitor {
    cb: Option<Clipboard>,
    /// Text seen on the previous look; `None` until the first look after the monitor was switched on,
    /// so whatever was already on the clipboard is never offered.
    last: Option<String>,
    next: Instant,
}

impl Monitor {
    pub fn new() -> Self {
        Monitor { cb: None, last: None, next: Instant::now() }
    }

    /// Call every frame. Returns the links that were just copied and match the settings.
    pub fn poll(&mut self, s: &Settings) -> Vec<String> {
        if !s.clipboard_monitor {
            self.last = None;
            self.cb = None; // release the clipboard handle while the feature is off
            return vec![];
        }
        if Instant::now() < self.next {
            return vec![];
        }
        self.next = Instant::now() + Duration::from_millis(700);
        if self.cb.is_none() {
            match Clipboard::new() {
                Ok(c) => self.cb = Some(c),
                Err(_) => {
                    self.next = Instant::now() + Duration::from_secs(5);
                    return vec![];
                }
            }
        }
        let Some(cb) = self.cb.as_mut() else { return vec![] };
        let text = cb.get_text().unwrap_or_default();
        if self.last.as_deref() == Some(text.as_str()) {
            return vec![];
        }
        let first_look = self.last.is_none();
        self.last = Some(text.clone());
        if first_look {
            return vec![];
        }
        parse_links(&text, s)
    }
}

/// Every non-empty line must be a link to a wanted file type (a paragraph that merely contains a
/// link is not a download). At most 30 links are taken at once.
pub fn parse_links(text: &str, s: &Settings) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() || lines.len() > 30 || !lines.iter().all(|l| s.wants_clipboard_url(l)) {
        return vec![];
    }
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pure_link_lists_are_taken() {
        let s = Settings::default();
        assert_eq!(parse_links("https://a.com/x.zip", &s), vec!["https://a.com/x.zip"]);
        assert_eq!(
            parse_links("  https://a.com/x.zip \n\nhttps://a.com/y.iso\nhttps://a.com/x.zip", &s),
            vec!["https://a.com/x.zip", "https://a.com/y.iso"]
        );
        assert!(parse_links("see https://a.com/x.zip for details", &s).is_empty());
        assert!(parse_links("https://a.com/x.zip\nhttps://a.com/page.html", &s).is_empty());
        assert!(parse_links("", &s).is_empty());
    }
}
