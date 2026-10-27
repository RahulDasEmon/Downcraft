//! Small OS helpers: start at login, notification sounds, sleep / shut down, and the command that
//! can run after a download. Everything here is best effort and never blocks the UI.
use downcraft_core::{util, AfterQueue};
use std::{path::Path, process::Command};

/// A console window must never flash up for a helper process on Windows.
#[cfg(windows)]
fn quiet(c: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn quiet(c: &mut Command) -> &mut Command {
    c
}

/// Runs a prepared command on its own thread so the zombie is reaped and nothing blocks.
fn run_detached(mut c: Command) {
    std::thread::spawn(move || {
        let _ = quiet(&mut c).status();
    });
}

// ───────────────────────────── start at login ─────────────────────────────

/// Adds or removes DownCraft from the programs that start when the user logs in. The entry passes
/// `--autostart`, which is how the app knows it may start minimized.
pub fn set_autostart(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find the program path: {e}"))?;
    autostart_impl(on, &exe)
}

#[cfg(windows)]
fn autostart_impl(on: bool, exe: &Path) -> Result<(), String> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut c = Command::new("reg");
    if on {
        c.args(["add", key, "/v", "DownCraft", "/t", "REG_SZ", "/d", &format!("\"{}\" --autostart", exe.display()), "/f"]);
    } else {
        c.args(["delete", key, "/v", "DownCraft", "/f"]);
    }
    let out = quiet(&mut c).output().map_err(|e| format!("Could not run reg.exe: {e}"))?;
    // deleting a value that is not there is fine
    if out.status.success() || !on {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(target_os = "macos")]
fn autostart_impl(on: bool, exe: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let file = Path::new(&home).join("Library/LaunchAgents/com.downcraft.app.plist");
    if !on {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.downcraft.app</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--autostart</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n",
        esc(&exe.to_string_lossy())
    );
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, plist).map_err(|e| e.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn autostart_impl(on: bool, exe: &Path) -> Result<(), String> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| Path::new(&h).join(".config")))
        .ok_or_else(|| "HOME is not set".to_string())?;
    let file = base.join("autostart").join("downcraft.desktop");
    if !on {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    // Exec values are quoted with double quotes; these four characters stay special inside them
    let q: String = exe
        .to_string_lossy()
        .chars()
        .flat_map(|c| match c {
            '"' | '`' | '$' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=DownCraft\nComment=Download manager\nExec=\"{q}\" --autostart\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, entry).map_err(|e| e.to_string())
}

// ───────────────────────────── sounds ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    Complete,
    Failed,
    QueueDone,
}

/// Plays `file`, or the system's own sound for that event when `file` is empty. Windows plays
/// `.wav` files only; macOS and Linux play whatever their players understand.
pub fn play_sound(kind: SoundKind, file: &str) {
    let file = file.trim().to_string();
    #[cfg(windows)]
    {
        let script = if file.is_empty() {
            let n = match kind {
                SoundKind::Complete => "Asterisk",
                SoundKind::Failed => "Hand",
                SoundKind::QueueDone => "Exclamation",
            };
            format!("[System.Media.SystemSounds]::{n}.Play(); Start-Sleep -Milliseconds 900")
        } else {
            // the path travels in an environment variable, never inside the script text
            "(New-Object System.Media.SoundPlayer $env:DC_SOUND).PlaySync()".to_string()
        };
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script]).env("DC_SOUND", &file);
        run_detached(c);
    }
    #[cfg(target_os = "macos")]
    {
        let path = if file.is_empty() {
            match kind {
                SoundKind::Complete => "/System/Library/Sounds/Glass.aiff",
                SoundKind::Failed => "/System/Library/Sounds/Basso.aiff",
                SoundKind::QueueDone => "/System/Library/Sounds/Hero.aiff",
            }
            .to_string()
        } else {
            file
        };
        let mut c = Command::new("afplay");
        c.arg(path);
        run_detached(c);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("sh");
        if file.is_empty() {
            let (id, oga) = match kind {
                SoundKind::Complete => ("complete", "complete"),
                SoundKind::Failed => ("dialog-error", "dialog-error"),
                SoundKind::QueueDone => ("message", "message"),
            };
            let script = format!(
                "canberra-gtk-play -i {id} 2>/dev/null || paplay /usr/share/sounds/freedesktop/stereo/{oga}.oga 2>/dev/null"
            );
            c.args(["-c", &script]);
        } else {
            c.args(["-c", "paplay \"$1\" 2>/dev/null || aplay -q \"$1\" 2>/dev/null || ffplay -nodisp -autoexit -loglevel quiet \"$1\"", "sh", &file]);
        }
        run_detached(c);
    }
}

// ───────────────────────────── power ─────────────────────────────

/// Puts the computer to sleep or shuts it down. Other actions do nothing here.
pub fn power(action: AfterQueue) {
    let mut c;
    match action {
        AfterQueue::Shutdown => {
            #[cfg(windows)]
            {
                c = Command::new("shutdown");
                c.args(["/s", "/t", "0"]);
            }
            #[cfg(target_os = "macos")]
            {
                c = Command::new("osascript");
                c.args(["-e", "tell application \"System Events\" to shut down"]);
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                c = Command::new("systemctl");
                c.arg("poweroff");
            }
        }
        AfterQueue::Sleep => {
            #[cfg(windows)]
            {
                c = Command::new("rundll32.exe");
                c.args(["powrprof.dll,SetSuspendState", "0,1,0"]);
            }
            #[cfg(target_os = "macos")]
            {
                c = Command::new("pmset");
                c.arg("sleepnow");
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                c = Command::new("systemctl");
                c.arg("suspend");
            }
        }
        _ => return,
    }
    run_detached(c);
}

// ───────────────────────────── command after a download ─────────────────────────────

/// Runs the user's command for a finished download. `%FILE%` in an argument is replaced by the
/// file's path (the path is appended when the command does not mention it). The command line is
/// split into words by [`util::split_command`] and started directly - no shell is involved, so
/// nothing in a file name can be interpreted as a command.
pub fn run_after_download(cmd: &str, file: &Path) {
    let mut words = util::split_command(cmd);
    if words.is_empty() {
        return;
    }
    let path = file.to_string_lossy().into_owned();
    let uses_token = words.iter().any(|w| w.contains("%FILE%"));
    for w in words.iter_mut() {
        *w = w.replace("%FILE%", &path);
    }
    if !uses_token {
        words.push(path.clone());
    }
    let mut c = Command::new(&words[0]);
    c.args(&words[1..]).env("DOWNCRAFT_FILE", &path);
    if let Some(dir) = file.parent() {
        c.current_dir(dir);
    }
    run_detached(c);
}
