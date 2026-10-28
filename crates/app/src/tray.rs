//! System-tray icon (Windows and macOS). On other platforms `SUPPORTED` is false and nothing is created.
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Set when a browser link had to wake a window that was hidden in the tray, so that it can be
/// hidden again once the pop-ups are gone.
pub static WOKEN_HIDDEN: AtomicBool = AtomicBool::new(false);

pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// The icon is created once and then only shown/hidden: the underlying library accepts its event
/// handlers just once per process, so dropping and re-creating it would leave a dead menu.
pub struct Tray {
    #[cfg(any(windows, target_os = "macos"))]
    _icon: tray_icon::TrayIcon,
}

impl Tray {
    pub fn set_visible(&self, visible: bool) {
        #[cfg(any(windows, target_os = "macos"))]
        let _ = self._icon.set_visible(visible);
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = visible;
    }
}

/// Brings the main window back (also from "hidden in tray" and "minimised").
#[allow(dead_code)] // only the Windows/macOS tray uses it
pub fn reveal(ctx: &egui::Context) {
    rehide(); // a window parked off-screen for a pop-up goes back to normal first
    wake_window(); // works even while the window is hidden and not running its update loop
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
}

#[cfg(any(windows, target_os = "macos"))]
pub fn create(ctx: &egui::Context, quitting: Arc<AtomicBool>, mgr: Arc<downcraft_core::Manager>) -> Option<Tray> {
    use std::sync::atomic::Ordering::Relaxed;
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
        Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    };
    let img = image::load_from_memory(crate::LOGO).ok()?.to_rgba8();
    let img = image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Lanczos3);
    let (w, h) = img.dimensions();
    let icon = Icon::from_rgba(img.into_raw(), w, h).ok()?;

    let menu = Menu::new();
    let show = MenuItem::new("Show DownCraft", true, None);
    let quit = MenuItem::new("Quit", true, None);
    menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit]).ok()?;
    let (show_id, quit_id) = (show.id().clone(), quit.id().clone());

    let c = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == show_id {
            reveal(&c);
        } else if e.id == quit_id {
            quitting.store(true, Relaxed);
            // A hidden window does not run its update loop, so a Close command alone would never be
            // seen: show the window through the OS first, then ask it to close normally.
            wake_window();
            c.send_viewport_cmd(egui::ViewportCommand::Close);
            c.request_repaint();
            // Safety net: if the window still has not closed, stop downloads, save and exit anyway.
            let m = mgr.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                m.pause_all();
                m.save();
                m.save_settings();
                std::process::exit(0);
            });
        }
    }));
    let c = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            reveal(&c);
        }
    }));

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("DownCraft")
        .with_icon(icon)
        .build()
        .ok()?;
    Some(Tray { _icon: icon })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn create(_ctx: &egui::Context, _quitting: Arc<AtomicBool>, _mgr: Arc<downcraft_core::Manager>) -> Option<Tray> {
    None
}

/// Restores and raises the main window through the OS. A hidden or minimised window may not run
/// `update()` at all, so a link from the browser would sit unseen; waking the window from outside
/// guarantees the "Download File Info" dialog opens right away.
#[cfg(windows)]
pub fn wake_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let title: Vec<u16> = "DownCraft\0".encode_utf16().collect();
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if !h.is_null() {
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }
}

#[cfg(not(windows))]
pub fn wake_window() {}

/// Keeps the update loop alive for a pop-up from the browser WITHOUT ever showing the main window.
/// A visible or minimised window is left completely alone. A window hidden in the tray (which would
/// not run its update loop) is "parked": made visible to the OS but moved far off-screen, with no
/// taskbar button and without taking focus. `rehide()` puts it back exactly as it was.
#[cfg(windows)]
pub fn wake_for_dialog() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindowVisible};
    let title: Vec<u16> = "DownCraft\0".encode_utf16().collect();
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if h.is_null() || IsWindowVisible(h) != 0 {
            return;
        }
        park(h);
    }
}

/// The user closed the main window (to the tray) while pop-ups such as "Download complete" or
/// "About" are still open. The pop-ups are drawn by the main window's update loop, and a hidden
/// window stops running it, which would leave them frozen: impossible to move, minimise or close.
/// So the window is parked off-screen instead of hidden; `rehide()` hides it for real once the
/// last pop-up is gone.
#[cfg(windows)]
pub fn park_for_dialogs() {
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let title: Vec<u16> = "DownCraft\0".encode_utf16().collect();
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if !h.is_null() {
            park(h);
        }
    }
}

/// Moves the window far off-screen (no taskbar button, never activated) but keeps it "visible" to
/// the OS so that its update loop keeps running. Remembers position and style for `rehide()`.
#[cfg(windows)]
unsafe fn park(h: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowRect, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        SWP_SHOWWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    let mut saved = PARKED.lock().unwrap_or_else(|e| e.into_inner());
    if saved.is_some() {
        return; // already parked
    }
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetWindowRect(h, &mut r);
    let style = GetWindowLongW(h, GWL_EXSTYLE);
    *saved = Some((r.left, r.top, style));
    WOKEN_HIDDEN.store(true, Ordering::Relaxed);
    SetWindowLongW(h, GWL_EXSTYLE, style | (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) as i32);
    SetWindowPos(h, std::ptr::null_mut(), -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

#[cfg(not(windows))]
pub fn wake_for_dialog() {}

#[cfg(not(windows))]
pub fn park_for_dialogs() {}

/// Undoes `wake_for_dialog`: hides the parked window again and restores its style and position.
#[cfg(windows)]
pub fn rehide() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetWindowLongW, SetWindowPos, ShowWindow, GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    };
    WOKEN_HIDDEN.store(false, Ordering::Relaxed);
    let Some((x, y, style)) = PARKED.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let title: Vec<u16> = "DownCraft\0".encode_utf16().collect();
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if h.is_null() {
            return;
        }
        ShowWindow(h, SW_HIDE);
        SetWindowLongW(h, GWL_EXSTYLE, style);
        SetWindowPos(h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

#[cfg(not(windows))]
pub fn rehide() {
    WOKEN_HIDDEN.store(false, Ordering::Relaxed);
}

#[cfg(windows)]
static PARKED: std::sync::Mutex<Option<(i32, i32, i32)>> = std::sync::Mutex::new(None);
