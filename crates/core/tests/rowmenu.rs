//! Engine behind the download list's right-click menu: add to / delete from queue,
//! Move/Rename and Redownload.
use downcraft_core::{Category, Manager, NewDownload, Status};
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("downcraft-rowmenu-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A finished download whose file really exists in `dir`.
fn finished(mgr: &std::sync::Arc<Manager>, dir: &Path, name: &str) -> std::sync::Arc<downcraft_core::Task> {
    let t = mgr.add(NewDownload {
        url: "http://127.0.0.1:9/x.bin".into(),
        dir: dir.to_path_buf(),
        file_name: name.into(),
        category: Category::Compressed,
        description: "d".into(),
        probe: None,
        queued: false,
        start: false,
        headers: vec![],
    });
    // `add` picks a free name, so the file is written afterwards
    assert_eq!(t.info.lock().file_name, name);
    std::fs::write(dir.join(name), b"hello").unwrap();
    t.info.lock().status = Status::Complete;
    t
}

#[test]
fn queue_flag_can_be_set_and_cleared() {
    let data = temp_dir("q-data");
    let dir = temp_dir("q-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    assert!(!t.info.lock().queued);
    mgr.set_queued(&t, true);
    assert!(t.info.lock().queued);
    mgr.set_queued(&t, false);
    assert!(!t.info.lock().queued);
}

#[test]
fn rename_moves_the_finished_file_and_the_entry() {
    let data = temp_dir("r-data");
    let dir = temp_dir("r-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    mgr.move_task(&t, &dir.join("b.zip")).unwrap();
    assert!(!dir.join("a.zip").exists());
    assert_eq!(std::fs::read(dir.join("b.zip")).unwrap(), b"hello");
    assert_eq!(t.info.lock().file_name, "b.zip");
    assert_eq!(t.final_path(), dir.join("b.zip"));
}

#[test]
fn move_goes_to_a_new_folder_and_creates_it() {
    let data = temp_dir("m-data");
    let dir = temp_dir("m-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    let target = dir.join("sub").join("deeper").join("a.zip");
    mgr.move_task(&t, &target).unwrap();
    assert!(target.exists());
    assert!(!dir.join("a.zip").exists());
    assert_eq!(t.info.lock().dir, dir.join("sub").join("deeper"));
}

#[test]
fn rename_refuses_to_overwrite_and_leaves_everything_as_it_was() {
    let data = temp_dir("o-data");
    let dir = temp_dir("o-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    std::fs::write(dir.join("b.zip"), b"other").unwrap();
    let e = mgr.move_task(&t, &dir.join("b.zip")).unwrap_err();
    assert!(e.contains("already exists"), "{e}");
    assert_eq!(std::fs::read(dir.join("b.zip")).unwrap(), b"other");
    assert!(dir.join("a.zip").exists());
    assert_eq!(t.info.lock().file_name, "a.zip");
}

#[test]
fn rename_rejects_bad_names() {
    let data = temp_dir("n-data");
    let dir = temp_dir("n-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    assert!(mgr.move_task(&t, &dir.join("bad?name.zip")).is_err());
    assert!(mgr.move_task(&t, &dir).is_err() || t.info.lock().file_name == "a.zip");
    assert_eq!(t.info.lock().file_name, "a.zip");
}

#[test]
fn partial_data_follows_an_unfinished_download() {
    let data = temp_dir("p-data");
    let dir = temp_dir("p-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    std::fs::remove_file(dir.join("a.zip")).unwrap();
    t.info.lock().status = Status::Stopped;
    std::fs::write(dir.join("a.zip.fcpart"), b"par").unwrap();
    mgr.move_task(&t, &dir.join("c.zip")).unwrap();
    assert!(dir.join("c.zip.fcpart").exists());
    assert!(!dir.join("a.zip.fcpart").exists());
}

#[test]
fn redownload_removes_the_file_and_keeps_the_entry_details() {
    let data = temp_dir("d-data");
    let dir = temp_dir("d-files");
    let mgr = Manager::with_dir(data);
    let t = finished(&mgr, &dir, "a.zip");
    mgr.set_queued(&t, true);
    let old_id = t.id;
    let fresh = mgr.redownload(&t);
    assert_ne!(fresh.id, old_id);
    assert!(mgr.find(old_id).is_none());
    assert!(mgr.find(fresh.id).is_some());
    assert!(!dir.join("a.zip").exists(), "the old file must be gone");
    let i = fresh.info.lock();
    assert_eq!(i.file_name, "a.zip");
    assert_eq!(i.description, "d");
    assert!(i.queued);
    assert_ne!(i.status, Status::Complete);
    assert_eq!(fresh.url, "http://127.0.0.1:9/x.bin");
}
