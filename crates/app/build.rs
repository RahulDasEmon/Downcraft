//! Packs `extension/` into `downcraft-extension.zip` (manifest.json at the zip root) so the app can
//! hand the browser extension to the user as a single file.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn add(zip: &mut zip::ZipWriter<fs::File>, root: &Path, dir: &Path) {
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut entries: Vec<PathBuf> = fs::read_dir(dir).expect("read extension dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        println!("cargo:rerun-if-changed={}", p.display());
        if p.file_name().map_or(false, |n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if p.is_dir() {
            add(zip, root, &p);
        } else {
            let name = p.strip_prefix(root).expect("relative").to_string_lossy().replace('\\', "/");
            zip.start_file(name, opts).expect("zip entry");
            zip.write_all(&fs::read(&p).expect("read file")).expect("zip write");
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("extension");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", root.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("downcraft-extension.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(out).expect("create zip"));
    add(&mut zip, &root, &root);
    zip.finish().expect("finish zip");
}
