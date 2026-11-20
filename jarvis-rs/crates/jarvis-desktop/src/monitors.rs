//! Monitors and the cursor, in physical pixels on the virtual desktop.

use serde::Serialize;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
use windows::core::BOOL;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Monitor {
    /// 1-based: main screen first, then left-to-right.
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

impl Monitor {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Run `f` with this thread temporarily per-monitor-DPI aware (physical coordinates everywhere).
pub fn per_monitor<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let out = f();
        if !prev.is_invalid() {
            SetThreadDpiAwarenessContext(prev);
        }
        out
    }
}

pub fn list() -> Vec<Monitor> {
    unsafe extern "system" fn collect(hmon: HMONITOR, _hdc: HDC, _rc: *mut RECT, data: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(data.0 as *mut Vec<(RECT, bool)>);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(hmon, &mut info).as_bool() {
                out.push((info.rcMonitor, info.dwFlags & 1 == 1));
            }
            BOOL(1)
        }
    }
    let mut found: Vec<(RECT, bool)> = Vec::new();
    per_monitor(|| unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut found as *mut _ as isize));
    });
    found.sort_by_key(|(r, primary)| (!primary, r.left, r.top));
    found
        .into_iter()
        .enumerate()
        .map(|(i, (r, primary))| Monitor { index: i + 1, x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top, primary })
        .collect()
}

pub fn set_cursor(x: i32, y: i32) {
    per_monitor(|| unsafe {
        let _ = SetCursorPos(x, y);
    });
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    per_monitor(|| unsafe {
        let _ = GetCursorPos(&mut p);
    });
    (p.x, p.y)
}
