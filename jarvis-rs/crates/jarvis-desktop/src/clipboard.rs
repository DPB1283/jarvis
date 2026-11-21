//! Native clipboard + paste. Replaces the encoded PowerShell paste script, which Windows Defender flagged as
//! a trojan (`!#SLF:CMD_HSTR:General.ML.B`, a command-line heuristic on `Add-Type`/`DllImport`/`SendKeys`),
//! killing the process and surfacing as "Access is denied. (os error 5)".

use anyhow::{Context, Result, bail, ensure};
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};

const TEXT: u32 = CF_UNICODETEXT.0 as u32;
const FILES: u32 = CF_HDROP.0 as u32;

/// Holds the clipboard open; another process (clipboard managers, recorders) may briefly own it, so retry.
struct Open;
impl Open {
    fn new() -> Result<Self> {
        let mut last = None;
        for _ in 0..40 {
            match unsafe { OpenClipboard(None::<HWND>) } {
                Ok(()) => return Ok(Self),
                Err(error) => last = Some(error),
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        bail!("Could not open the clipboard: {}", last.map(|e| e.to_string()).unwrap_or_default())
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn set(format: u32, bytes: &[u8]) -> Result<()> {
    unsafe {
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).context("Clipboard allocation failed")?;
        let target = GlobalLock(memory);
        ensure!(!target.is_null(), "Clipboard allocation could not be locked");
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        // The clipboard owns the memory once SetClipboardData succeeds.
        SetClipboardData(format, Some(HANDLE(memory.0))).context("Could not place data on the clipboard")?;
    }
    Ok(())
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// Current clipboard text, if it holds text.
pub fn get_text() -> Option<String> {
    let _open = Open::new().ok()?;
    unsafe {
        IsClipboardFormatAvailable(TEXT).ok()?;
        let handle = GetClipboardData(TEXT).ok()?;
        let memory = HGLOBAL(handle.0);
        let pointer = GlobalLock(memory).cast::<u16>();
        if pointer.is_null() {
            return None;
        }
        let units = GlobalSize(memory) / 2;
        let slice = std::slice::from_raw_parts(pointer, units);
        let end = slice.iter().position(|unit| *unit == 0).unwrap_or(units);
        let text = String::from_utf16_lossy(&slice[..end]);
        let _ = GlobalUnlock(memory);
        Some(text)
    }
}

pub fn set_text(text: &str) -> Result<()> {
    let _open = Open::new()?;
    unsafe { EmptyClipboard() }?;
    if text.is_empty() {
        return Ok(());
    }
    set(TEXT, &utf16z(text))
}

/// Put a file on the clipboard exactly as Explorer's Copy does; pasting it uploads it in chat apps.
pub fn set_file(path: &str) -> Result<()> {
    let absolute = std::fs::canonicalize(path).with_context(|| format!("Cannot find {path}"))?;
    let wide = absolute.to_string_lossy().replace(r"\\?\", "");
    // DROPFILES: pFiles = 20, pt = (0,0), fNC = 0, fWide = 1; then "path\0\0" in UTF-16.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&20u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend(utf16z(&wide));
    bytes.extend_from_slice(&[0, 0]);
    let _open = Open::new()?;
    unsafe { EmptyClipboard() }?;
    set(FILES, &bytes)
}

/// Paste `text` or a file into window `hwnd` with Ctrl+V, then restore the previous clipboard text.
/// Fails without touching the keyboard if that window is no longer in front.
pub fn paste(hwnd: usize, text: &str, file: Option<&str>) -> Result<()> {
    let in_front = || crate::window::foreground().hwnd == hwnd;
    ensure!(in_front(), "The target window changed");
    let previous = get_text();
    match file {
        Some(path) => {
            ensure!(std::path::Path::new(path).is_file(), "Image file is missing");
            set_file(path)?;
        }
        None => set_text(text)?,
    }
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let sent = if in_front() {
        crate::input::chord("ctrl+v")
    } else {
        Err(anyhow::anyhow!("The target window changed"))
    };
    // Give the app time to read the clipboard before it is restored (files take longer to ingest).
    std::thread::sleep(Duration::from_millis(if file.is_some() { 1500 } else { 800 }));
    if unsafe { GetClipboardSequenceNumber() } == sequence {
        // Nothing else touched it meanwhile: put back what the user had copied (text only).
        match previous {
            Some(old) => {
                let _ = set_text(&old);
            }
            None => {
                let _ = set_text("");
            }
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "uses the real clipboard (restores it afterwards)"]
    fn text_round_trips_and_restores() {
        let before = get_text();
        let sample = "Jarvis clipboard probe \u{1F600} line1\nline2";
        set_text(sample).unwrap();
        assert_eq!(get_text().as_deref(), Some(sample));
        set_file(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
        assert!(get_text().is_none(), "a file drop is not text");
        set_text(before.as_deref().unwrap_or("")).unwrap();
        assert_eq!(get_text(), before.filter(|t| !t.is_empty()));
    }
}

/// Changes every time the clipboard changes (0 = unknown). Lets a watcher skip work when nothing was copied.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// True when whoever copied asked clipboard history tools not to record it (password managers set this).
pub fn is_sensitive() -> bool {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::core::w;
    unsafe {
        let format = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        format != 0 && IsClipboardFormatAvailable(format).is_ok()
    }
}
