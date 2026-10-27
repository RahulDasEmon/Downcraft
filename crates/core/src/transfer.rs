//! Export / import of the download list.
//!
//! * Export writes a small JSON file (`.json`) with every download's address, name, folder,
//!   category and description. Only the `Referer` header is kept - cookies are session secrets and
//!   never leave the app.
//! * Import reads that file, or a plain text file with one link per line. Links that are already in
//!   the list are skipped; everything else is added *stopped*, ready to be resumed.
use crate::{util, Category, Manager, NewDownload};
use serde::{Deserialize, Serialize};
use std::{io, path::Path, sync::Arc};

const FORMAT: &str = "downcraft-export";

#[derive(Serialize, Deserialize)]
struct Entry {
    url: String,
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    dir: std::path::PathBuf,
    category: Option<Category>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    referer: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    exported: i64,
    downloads: Vec<Entry>,
}

/// What an import did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub added: usize,
    /// Already in the list, or not a usable http(s) link.
    pub skipped: usize,
}

impl Manager {
    /// Writes the whole list to `path`; returns how many downloads were exported.
    pub fn export_to(&self, path: &Path) -> io::Result<usize> {
        let downloads: Vec<Entry> = self
            .tasks
            .lock()
            .iter()
            .map(|t| {
                let i = t.info.lock();
                let referer = i.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("referer")).map(|(_, v)| v.clone());
                Entry {
                    url: t.url.clone(),
                    file_name: i.file_name.clone(),
                    dir: i.dir.clone(),
                    category: Some(i.category),
                    description: i.description.clone(),
                    referer,
                }
            })
            .collect();
        let n = downloads.len();
        let file = ExportFile { format: FORMAT.into(), version: 1, exported: util::now_ts(), downloads };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        std::fs::write(path, bytes)?;
        Ok(n)
    }

    /// Adds the downloads found in `path` (an export file or a text file of links).
    pub fn import_from(self: &Arc<Self>, path: &Path) -> io::Result<ImportReport> {
        let raw = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&raw);
        let text = text.trim_start_matches('\u{feff}');
        let entries: Vec<Entry> = if text.trim_start().starts_with('{') {
            let f: ExportFile = serde_json::from_str(text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("not a DownCraft export file ({e})")))?;
            if f.format != FORMAT {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "not a DownCraft export file"));
            }
            f.downloads
        } else {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| Entry {
                    url: l.to_string(),
                    file_name: String::new(),
                    dir: Default::default(),
                    category: None,
                    description: String::new(),
                    referer: None,
                })
                .collect()
        };

        let mut report = ImportReport::default();
        for e in entries {
            let url = e.url.trim().to_string();
            let lower = url.to_ascii_lowercase();
            let usable = lower.starts_with("http://") || lower.starts_with("https://");
            if !usable || self.tasks.lock().iter().any(|t| t.url == url) {
                report.skipped += 1;
                continue;
            }
            let name_for_cat = if e.file_name.is_empty() { util::file_name_from_url(&url) } else { e.file_name.clone() };
            let category = e.category.unwrap_or_else(|| Category::from_file_name(&name_for_cat));
            // keep the saved folder when it exists on this machine, else the default one for the category
            let dir = if !e.dir.as_os_str().is_empty() && e.dir.is_dir() { e.dir } else { self.settings.lock().dir_for(category) };
            let headers = e.referer.filter(|r| !r.is_empty()).map(|r| vec![("Referer".to_string(), r)]).unwrap_or_default();
            self.add(NewDownload {
                url,
                dir,
                file_name: e.file_name,
                category,
                description: e.description,
                probe: None,
                queued: false,
                start: false,
                headers,
            });
            report.added += 1;
        }
        Ok(report)
    }
}
