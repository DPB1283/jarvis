//! Foreground window, its process, fullscreen detection (game mode), idle time.

use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId};
use windows::core::PWSTR;

use crate::monitors::per_monitor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Debug, Clone)]
pub struct Foreground {
    pub hwnd: usize,
    pub bounds: Option<Rect>,
    pub pid: u32,
    /// Executable file name, e.g. "cs2.exe" ("" if unreadable).
    pub exe: String,
    pub title: String,
    /// The foreground window covers its whole monitor (borderless) or runs exclusive D3D fullscreen.
    pub fullscreen: bool,
    /// The monitor the window is on (physical pixels).
    pub monitor: Option<Rect>,
}

pub fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or_default().to_string()
    }
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

const DESKTOP_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Windows.UI.Core.CoreWindow"];

pub fn foreground() -> Foreground {
    per_monitor(|| unsafe {
        let h = GetForegroundWindow();
        let mut pid = 0u32;
        if !h.is_invalid() {
            GetWindowThreadProcessId(h, Some(&mut pid));
        }
        let exe = process_name(pid);
        let mut title_buf = [0u16; 512];
        let title = if h.is_invalid() {
            String::new()
        } else {
            let n = GetWindowTextW(h, &mut title_buf).max(0) as usize;
            String::from_utf16_lossy(&title_buf[..n])
        };
        let mut bounds = RECT::default();
        let bounds = GetWindowRect(h,&mut bounds).ok().map(|_|bounds.into());
        let mut fg = Foreground { hwnd:h.0 as usize, bounds, pid, exe, title, fullscreen: false, monitor: None };
        if h.is_invalid() {
            return fg;
        }
        let hmon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            fg.monitor = Some(info.rcMonitor.into());
        }
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN {
                fg.fullscreen = true;
                return fg;
            }
        }
        if DESKTOP_CLASSES.contains(&class_name(h).as_str()) {
            return fg;
        }
        let mut r = RECT::default();
        if let (Some(m), true) = (fg.monitor, GetWindowRect(h, &mut r).is_ok()) {
            fg.fullscreen = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        }
        fg
    })
}

/// Seconds since the last keyboard/mouse input.
pub fn idle_seconds() -> f64 {
    unsafe {
        let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !GetLastInputInfo(&mut info).as_bool() {
            return 0.0;
        }
        GetTickCount().wrapping_sub(info.dwTime) as f64 / 1000.0
    }
}


/// A visible top-level window found by [`find`].
#[derive(Debug, Clone)]
pub struct Found {
    pub bounds: Rect,
    pub title: String,
    pub exe: String,
}

/// The frontmost visible, non-minimised top-level window whose title or program name contains `query`
/// (case-insensitive), skipping the desktop and taskbar. Bounds are physical pixels.
pub fn find(query: &str) -> Option<Found> {
    use windows::Win32::Foundation::LPARAM;
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, IsIconic, IsWindowVisible};

    struct Search {
        needle: String,
        found: Option<Found>,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        unsafe {
            if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
                return true.into();
            }
            if DESKTOP_CLASSES.contains(&class_name(window).as_str()) {
                return true.into();
            }
            let mut buffer = [0u16; 512];
            let length = GetWindowTextW(window, &mut buffer).max(0) as usize;
            let title = String::from_utf16_lossy(&buffer[..length]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let exe = process_name(pid);
            let matches = title.to_lowercase().contains(&search.needle)
                || exe.to_lowercase().trim_end_matches(".exe").contains(search.needle.trim_end_matches(".exe"));
            if !matches || title.is_empty() {
                return true.into();
            }
            let mut rect = RECT::default();
            if GetWindowRect(window, &mut rect).is_err() || rect.right <= rect.left || rect.bottom <= rect.top {
                return true.into();
            }
            search.found = Some(Found { bounds: rect.into(), title, exe });
        }
        false.into() // EnumWindows walks front to back, so the first match is the frontmost
    }

    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    per_monitor(|| {
        let mut search = Search { needle, found: None };
        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
        }
        search.found
    })
}

#[cfg(test)]
mod find_tests {
    #[test]
    #[ignore = "needs real windows on screen"]
    fn finds_live_windows() {
        for query in ["Jarvis HUD", "Discord", "code", "definitely-not-a-window-9f3"] {
            println!("{query:?} -> {:?}", super::find(query));
        }
    }
}
