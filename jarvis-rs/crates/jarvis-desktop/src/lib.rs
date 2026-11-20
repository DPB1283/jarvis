//! Win32 for Jarvis (Windows only). Port of `jarvis/src/main/win32.ts`, the screen/input parts of
//! `tools/computer.ts`, and the clipboard/focus parts of `tools/desktop.ts`.
//!
//! Everything works in physical pixels with per-monitor DPI awareness, so mixed-scaling multi-monitor setups behave.

pub mod clipboard;
pub mod monitors;
pub mod window;
pub mod input;
