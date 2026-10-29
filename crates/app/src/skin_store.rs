//! Skin gallery state: the list published on GitHub, the previews, and the skins already installed.
//!
//! All network work runs on background threads; the Appearance page only reads a snapshot each
//! frame. An installed skin is kept as `<data dir>/skins/<id>.zip`, so switching back to it later
//! needs no download. The skin in use is still the one saved by [`crate::skin`].
use crate::{icons, skin};
use downcraft_core::{skinstore as net, Manager};
use eframe::egui::{ColorImage, Context};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    thread,
};

#[derive(Clone, Default)]
pub enum Catalog {
    #[default]
    Idle,
    Loading,
    Ready(Vec<net::SkinEntry>),
    Failed(String),
}

#[derive(Clone)]
pub enum Job {
    Busy,
    Failed(String),
}

enum Preview {
    Ready(ColorImage),
    Failed,
}

#[derive(Default)]
struct Inner {
    catalog: Catalog,
    previews: HashMap<String, Preview>,
    jobs: HashMap<String, Job>,
}

pub struct SkinStore {
    mgr: Arc<Manager>,
    inner: Mutex<Inner>,
}

impl SkinStore {
    pub fn new(mgr: Arc<Manager>) -> Arc<Self> {
        Arc::new(SkinStore { mgr, inner: Mutex::new(Inner::default()) })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn library(&self) -> PathBuf {
        self.mgr.data_dir().join("skins")
    }

    fn zip_path(&self, id: &str) -> PathBuf {
        self.library().join(format!("{id}.zip"))
    }

    // ───────────── reading ─────────────

    pub fn catalog(&self) -> Catalog {
        self.lock().catalog.clone()
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.lock().jobs.get(id).cloned()
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.zip_path(id).is_file()
    }

    /// Ids of every skin pack saved on this PC, so they can be used while offline too.
    pub fn installed_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.library())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.strip_suffix(".zip").map(str::to_string)
            })
            .collect();
        v.sort();
        v
    }

    /// The decoded preview, handed over once (the page keeps it as a texture).
    pub fn take_preview(&self, id: &str) -> Option<ColorImage> {
        let mut g = self.lock();
        match g.previews.get(id) {
            Some(Preview::Ready(_)) => match g.previews.remove(id) {
                Some(Preview::Ready(img)) => Some(img),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn preview_failed(&self, id: &str) -> bool {
        matches!(self.lock().previews.get(id), Some(Preview::Failed))
    }

    // ───────────── actions ─────────────

    /// Loads the list again (and the previews missing so far) in the background.
    pub fn refresh(self: &Arc<Self>, ctx: &Context) {
        {
            let mut g = self.lock();
            if matches!(g.catalog, Catalog::Loading) {
                return;
            }
            g.catalog = Catalog::Loading;
        }
        let (me, ctx) = (self.clone(), ctx.clone());
        thread::spawn(move || {
            let client = me.mgr.client();
            let list = net::fetch_index(&client);
            let entries = list.as_ref().map(|l| l.clone()).unwrap_or_default();
            me.lock().catalog = match list {
                Ok(l) => Catalog::Ready(l),
                Err(e) => Catalog::Failed(e),
            };
            ctx.request_repaint();
            for e in entries {
                if e.previews.is_empty() || matches!(me.lock().previews.get(&e.id), Some(Preview::Ready(_))) {
                    continue;
                }
                // the first candidate (.png, .jpg, .jpeg) that downloads and decodes wins
                let mut img = Err("no preview".to_string());
                for url in &e.previews {
                    img = net::fetch(&client, url, net::MAX_PREVIEW).and_then(|b| decode_preview(&b));
                    if img.is_ok() {
                        break;
                    }
                }
                me.lock().previews.insert(e.id.clone(), img.map_or(Preview::Failed, Preview::Ready));
                ctx.request_repaint();
            }
        });
    }

    /// Opening the page: loads the list unless it is already there (or on its way).
    pub fn refresh_if_needed(self: &Arc<Self>, ctx: &Context) {
        if matches!(self.catalog(), Catalog::Idle | Catalog::Failed(_)) {
            self.refresh(ctx);
        }
    }

    /// Downloads a skin, checks it, remembers it and switches to it, all in one go.
    pub fn install(self: &Arc<Self>, entry: net::SkinEntry, ctx: &Context) {
        {
            let mut g = self.lock();
            if matches!(g.jobs.get(&entry.id), Some(Job::Busy)) {
                return;
            }
            g.jobs.insert(entry.id.clone(), Job::Busy);
        }
        let (me, ctx) = (self.clone(), ctx.clone());
        thread::spawn(move || {
            let res = me.download_and_apply(&entry);
            {
                let mut g = me.lock();
                match res {
                    Ok(()) => g.jobs.remove(&entry.id),
                    Err(m) => g.jobs.insert(entry.id.clone(), Job::Failed(m)),
                };
            }
            ctx.request_repaint();
        });
    }

    fn download_and_apply(&self, e: &net::SkinEntry) -> Result<(), String> {
        let bytes = net::fetch(&self.mgr.client(), &e.zip, net::MAX_ZIP)?;
        fs::create_dir_all(self.library()).map_err(|x| format!("cannot save the skin ({x})"))?;
        let path = self.zip_path(&e.id);
        fs::write(&path, &bytes).map_err(|x| format!("cannot save the skin ({x})"))?;
        match skin::import_zip(&path, self.mgr.data_dir()) {
            Ok(s) => {
                icons::set_skin(Some(s));
                Ok(())
            }
            Err(m) => {
                let _ = fs::remove_file(&path); // never keep a pack that does not load
                Err(m)
            }
        }
    }

    /// Switches to a skin that is already on this PC.
    pub fn use_installed(&self, id: &str) -> Result<(), String> {
        let s = skin::import_zip(&self.zip_path(id), self.mgr.data_dir())?;
        icons::set_skin(Some(s));
        Ok(())
    }

    /// Deletes the saved pack (a skin in use keeps working until another one is chosen).
    pub fn remove(&self, id: &str) {
        let _ = fs::remove_file(self.zip_path(id));
        self.lock().jobs.remove(id);
    }

    pub fn clear_job(&self, id: &str) {
        self.lock().jobs.remove(id);
    }
}

fn decode_preview(bytes: &[u8]) -> Result<ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|_| "unreadable picture".to_string())?;
    // previews are wide, thin strips; nothing needs more than the width of the page
    let img = img.thumbnail(760, 160).to_rgba8();
    let (w, h) = img.dimensions();
    Ok(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
}
