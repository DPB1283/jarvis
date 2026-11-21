//! Win32 for Jarvis (Windows only). Port of `jarvis/src/main/win32.ts`, the screen/input parts of
//! `tools/computer.ts`, and the clipboard/focus parts of `tools/desktop.ts`.
//!
//! Everything works in physical pixels with per-monitor DPI awareness, so mixed-scaling multi-monitor setups behave.

#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod monitors;
#[cfg(windows)]
pub mod window;

// Linux (X11) has the same API, built on xdotool / xclip / xrandr / xprop.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{clipboard, input, monitors, window};
