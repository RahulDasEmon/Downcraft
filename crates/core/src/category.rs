use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Category {
    Compressed,
    Documents,
    Music,
    Programs,
    Video,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Compressed,
        Category::Documents,
        Category::Music,
        Category::Programs,
        Category::Video,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Compressed => "Compressed",
            Category::Documents => "Documents",
            Category::Music => "Music",
            Category::Programs => "Programs",
            Category::Video => "Video",
        }
    }

    pub fn from_file_name(name: &str) -> Category {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "zip" | "rar" | "7z" | "gz" | "tar" | "bz2" | "xz" | "tgz" | "iso" | "z" | "cab" | "arj" | "lzh" => {
                Category::Compressed
            }
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "mid" => Category::Music,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts" => {
                Category::Video
            }
            "exe" | "msi" | "dmg" | "apk" | "deb" | "rpm" | "appimage" | "bat" | "sh" | "jar" | "pkg" => {
                Category::Programs
            }
            _ => Category::Documents,
        }
    }
}
