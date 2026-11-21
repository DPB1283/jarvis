//! Linux (X11) backend with the same API as the Windows one. It drives the standard X11 command-line tools
//! (`xdotool`, `xclip`, `xrandr`, `xprop`, optionally `xprintidle`) instead of linking X11 libraries, so a build
//! needs nothing extra; at run time they have to be installed (see the README). Wayland is not supported.

use anyhow::{Context, Result, bail, ensure};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Run a program and return its stdout, or an error that names the tool (usually "not installed").
fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("`{program}` is not installed (needed for desktop control on Linux)"))?;
    ensure!(
        output.status.success(),
        "`{program}` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn xdotool(args: &[&str]) -> Result<String> {
    run("xdotool", args)
}

pub mod monitors {
    use super::*;
    use serde::Serialize;

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

    /// X11 coordinates are already physical pixels; nothing to switch.
    pub fn per_monitor<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    /// Parse `xrandr --query` output: "DP-1 connected primary 2560x1440+0+0 ...".
    pub(crate) fn parse_xrandr(text: &str) -> Vec<Monitor> {
        let mut found: Vec<Monitor> = Vec::new();
        for line in text.lines() {
            if !line.contains(" connected") {
                continue;
            }
            let primary = line.contains(" connected primary");
            let Some(geometry) = line.split_whitespace().find(|w| w.contains('x') && w.contains('+') && w.chars().next().is_some_and(|c| c.is_ascii_digit())) else {
                continue; // connected but switched off
            };
            let (size, offset) = geometry.split_once('+').unwrap_or((geometry, "0+0"));
            let (width, height) = size.split_once('x').unwrap_or(("0", "0"));
            let (x, y) = offset.split_once('+').unwrap_or(("0", "0"));
            let (width, height) = (width.parse().unwrap_or(0), height.parse().unwrap_or(0));
            if width == 0 || height == 0 {
                continue;
            }
            found.push(Monitor { index: 0, x: x.parse().unwrap_or(0), y: y.parse().unwrap_or(0), width, height, primary });
        }
        found.sort_by_key(|m| (!m.primary, m.x, m.y));
        for (i, m) in found.iter_mut().enumerate() {
            m.index = i + 1;
        }
        found
    }

    pub fn list() -> Vec<Monitor> {
        let parsed = run("xrandr", &["--query"]).map(|t| parse_xrandr(&t)).unwrap_or_default();
        if !parsed.is_empty() {
            return parsed;
        }
        // No xrandr: ask the root window for the whole screen.
        let size = run("xdotool", &["getdisplaygeometry"]).unwrap_or_default();
        let mut parts = size.split_whitespace().filter_map(|v| v.parse::<i32>().ok());
        match (parts.next(), parts.next()) {
            (Some(width), Some(height)) => vec![Monitor { index: 1, x: 0, y: 0, width, height, primary: true }],
            _ => Vec::new(),
        }
    }

    pub fn set_cursor(x: i32, y: i32) {
        let _ = xdotool(&["mousemove", &x.to_string(), &y.to_string()]);
    }

    pub fn cursor() -> (i32, i32) {
        let text = xdotool(&["getmouselocation", "--shell"]).unwrap_or_default();
        let value = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0)
        };
        (value("X"), value("Y"))
    }
}

pub mod window {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    impl Rect {
        pub fn contains(&self, x: i32, y: i32) -> bool {
            x >= self.left && x < self.right && y >= self.top && y < self.bottom
        }
    }

    #[derive(Debug, Clone)]
    pub struct Foreground {
        /// The X11 window id.
        pub hwnd: usize,
        pub bounds: Option<Rect>,
        pub pid: u32,
        /// Program name ("" if unreadable).
        pub exe: String,
        pub title: String,
        /// Covers its whole monitor or asks for fullscreen.
        pub fullscreen: bool,
        pub monitor: Option<Rect>,
    }

    /// Program name for a process id, from /proc (no extension on Linux).
    pub fn process_name(pid: u32) -> String {
        if pid == 0 {
            return String::new();
        }
        std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_owned()).unwrap_or_default()
    }

    struct Info {
        bounds: Rect,
        pid: u32,
        title: String,
    }

    /// `xdotool getwindowgeometry --shell` gives X, Y, WIDTH, HEIGHT.
    pub(crate) fn parse_geometry(text: &str) -> Option<Rect> {
        let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).and_then(|v| v.trim().parse::<i32>().ok());
        let (x, y, w, h) = (value("X")?, value("Y")?, value("WIDTH")?, value("HEIGHT")?);
        Some(Rect { left: x, top: y, right: x + w, bottom: y + h })
    }

    fn info(id: usize) -> Option<Info> {
        let ids = id.to_string();
        let bounds = parse_geometry(&xdotool(&["getwindowgeometry", "--shell", &ids]).ok()?)?;
        let pid = xdotool(&["getwindowpid", &ids]).ok().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
        let title = xdotool(&["getwindowname", &ids]).unwrap_or_default().trim().to_owned();
        Some(Info { bounds, pid, title })
    }

    fn monitor_of(r: Rect) -> Option<Rect> {
        let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        super::monitors::list()
            .into_iter()
            .find(|m| m.contains(cx, cy))
            .map(|m| Rect { left: m.x, top: m.y, right: m.x + m.width, bottom: m.y + m.height })
    }

    fn state_has(id: usize, flag: &str) -> bool {
        run("xprop", &["-id", &id.to_string(), "_NET_WM_STATE"]).is_ok_and(|s| s.contains(flag))
    }

    pub fn foreground() -> Foreground {
        let Some(id) = xdotool(&["getactivewindow"]).ok().and_then(|s| s.trim().parse::<usize>().ok()) else {
            return Foreground { hwnd: 0, bounds: None, pid: 0, exe: String::new(), title: String::new(), fullscreen: false, monitor: None };
        };
        let Some(i) = info(id) else {
            return Foreground { hwnd: id, bounds: None, pid: 0, exe: String::new(), title: String::new(), fullscreen: false, monitor: None };
        };
        let monitor = monitor_of(i.bounds);
        let covers = monitor.is_some_and(|m| i.bounds.left <= m.left && i.bounds.top <= m.top && i.bounds.right >= m.right && i.bounds.bottom >= m.bottom);
        Foreground {
            hwnd: id,
            bounds: Some(i.bounds),
            pid: i.pid,
            exe: process_name(i.pid),
            title: i.title,
            fullscreen: covers || state_has(id, "_NET_WM_STATE_FULLSCREEN"),
            monitor,
        }
    }

    /// Seconds since the last keyboard or mouse input (needs `xprintidle`; 0 if it is not installed).
    pub fn idle_seconds() -> f64 {
        run("xprintidle", &[]).ok().and_then(|ms| ms.trim().parse::<f64>().ok()).map(|ms| ms / 1000.0).unwrap_or(0.0)
    }

    #[derive(Debug, Clone)]
    pub struct Found {
        pub hwnd: usize,
        pub bounds: Rect,
        pub title: String,
        pub exe: String,
    }

    /// Window ids from `_NET_CLIENT_LIST_STACKING` (bottom to top), parsed from xprop output.
    pub(crate) fn parse_stacking(text: &str) -> Vec<usize> {
        text.split(['#', ','])
            .filter_map(|part| part.trim().strip_prefix("0x").and_then(|h| usize::from_str_radix(h, 16).ok()))
            .collect()
    }

    /// Every visible, non-minimised window with a title, frontmost first.
    pub fn list() -> Vec<Found> {
        let Ok(text) = run("xprop", &["-root", "_NET_CLIENT_LIST_STACKING"]) else { return Vec::new() };
        parse_stacking(&text)
            .into_iter()
            .rev()
            .filter(|id| !state_has(*id, "_NET_WM_STATE_HIDDEN"))
            .filter_map(|id| {
                let i = info(id)?;
                (!i.title.is_empty()).then(|| Found { hwnd: id, bounds: i.bounds, exe: process_name(i.pid), title: i.title })
            })
            .collect()
    }

    /// Bring a window to the front.
    pub fn activate(hwnd: usize) -> Result<()> {
        xdotool(&["windowactivate", "--sync", &hwnd.to_string()]).map(|_| ())
    }

    /// Ask a window to close like its close button (or kill its program when `force`).
    pub fn close(hwnd: usize, force: bool) -> Result<()> {
        xdotool(&[if force { "windowkill" } else { "windowquit" }, &hwnd.to_string()]).map(|_| ())
    }

    /// The frontmost visible, non-minimised window whose title or program name contains `query`.
    pub fn find(query: &str) -> Option<Found> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        let stacking = parse_stacking(&run("xprop", &["-root", "_NET_CLIENT_LIST_STACKING"]).ok()?);
        for id in stacking.into_iter().rev() {
            if state_has(id, "_NET_WM_STATE_HIDDEN") {
                continue;
            }
            let Some(i) = info(id) else { continue };
            let exe = process_name(i.pid);
            if i.title.is_empty() || !(i.title.to_lowercase().contains(&needle) || exe.to_lowercase().contains(&needle)) {
                continue;
            }
            if i.bounds.right <= i.bounds.left || i.bounds.bottom <= i.bounds.top {
                continue;
            }
            return Some(Found { hwnd: id, bounds: i.bounds, title: i.title, exe });
        }
        None
    }
}

pub mod input {
    use super::*;

    /// Our key names ("ctrl", "enter", "num/", "f5"...) as xdotool / X keysyms.
    pub(crate) fn keysym(token: &str) -> Result<String> {
        let raw = token.trim();
        let lower = raw.to_ascii_lowercase();
        let compact = lower.replace([' ', '_', '-'], "");
        // Numpad keys.
        if let Some(rest) = compact.strip_prefix("numpad").or_else(|| compact.strip_prefix("num")).or_else(|| compact.strip_prefix("kp")) {
            let minus = raw.ends_with('-');
            let sym = match rest {
                "/" | "divide" | "slash" => Some("KP_Divide"),
                "*" | "multiply" | "asterisk" | "star" => Some("KP_Multiply"),
                "+" | "plus" | "add" => Some("KP_Add"),
                "" if minus => Some("KP_Subtract"),
                "minus" | "subtract" | "dash" => Some("KP_Subtract"),
                "." | "decimal" | "period" | "dot" => Some("KP_Decimal"),
                "enter" | "return" => Some("KP_Enter"),
                "lock" => Some("Num_Lock"),
                d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => return Ok(format!("KP_{d}")),
                _ => None,
            };
            if let Some(sym) = sym {
                return Ok(sym.to_owned());
            }
        }
        Ok(match lower.as_str() {
            "ctrl" | "control" => "ctrl",
            "shift" => "shift",
            "alt" => "alt",
            "win" | "super" | "windows" | "meta" | "cmd" => "super",
            "enter" | "return" => "Return",
            "esc" | "escape" => "Escape",
            "tab" => "Tab",
            "space" => "space",
            "backspace" => "BackSpace",
            "end" => "End",
            "home" => "Home",
            "left" => "Left",
            "up" => "Up",
            "right" => "Right",
            "down" => "Down",
            "del" | "delete" => "Delete",
            "insert" => "Insert",
            "pageup" | "page_up" => "Prior",
            "pagedown" | "page_down" => "Next",
            "minus" | "-" => "minus",
            "equal" | "=" => "equal",
            "period" | "." => "period",
            "comma" | "," => "comma",
            "slash" | "/" => "slash",
            "backslash" | "\\" => "backslash",
            "grave" | "`" => "grave",
            "semicolon" | ";" => "semicolon",
            "apostrophe" | "'" => "apostrophe",
            "bracketleft" | "[" => "bracketleft",
            "bracketright" | "]" => "bracketright",
            t if t.len() == 1 && t.as_bytes()[0].is_ascii_alphanumeric() => return Ok(t.to_owned()),
            t if t.starts_with('f') && t[1..].parse::<u16>().is_ok_and(|n| (1..=24).contains(&n)) => return Ok(t.to_ascii_uppercase()),
            _ => bail!("Unsupported shortcut key {token}"),
        }
        .to_owned())
    }

    /// Split "ctrl+shift+num+" without breaking the trailing numpad plus.
    pub(crate) fn split_combo(combo: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut start = 0;
        for (i, b) in combo.bytes().enumerate() {
            if b == b'+' {
                let before = combo[start..i].trim().to_ascii_lowercase();
                if i + 1 == combo.len() && (before == "num" || before == "numpad" || before == "kp") {
                    continue;
                }
                parts.push(&combo[start..i]);
                start = i + 1;
            }
        }
        parts.push(&combo[start..]);
        parts
    }

    fn symbols(combo: &str) -> Result<Vec<String>> {
        split_combo(combo).into_iter().filter(|s| !s.trim().is_empty()).map(keysym).collect()
    }

    pub fn chord(combo: &str) -> Result<()> {
        let keys = symbols(combo)?;
        xdotool(&["key", "--clearmodifiers", &keys.join("+")]).map(|_| ())
    }

    pub fn type_text(text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        xdotool(&["type", "--clearmodifiers", "--delay", "2", "--", text]).map(|_| ())
    }

    /// Keys held down until dropped.
    pub struct Held(Vec<String>);
    impl Drop for Held {
        fn drop(&mut self) {
            for key in self.0.iter().rev() {
                let _ = xdotool(&["keyup", key]);
            }
        }
    }

    pub fn held(combo: &str) -> Result<Held> {
        let mut held = Held(Vec::new());
        for key in symbols(combo)? {
            xdotool(&["keydown", &key])?;
            held.0.push(key);
        }
        Ok(held)
    }

    fn button(name: &str) -> Result<&'static str> {
        Ok(match name {
            "left" => "1",
            "middle" => "2",
            "right" => "3",
            _ => bail!("Unknown mouse button"),
        })
    }

    pub fn mouse(name: &str, down: bool) -> Result<()> {
        xdotool(&[if down { "mousedown" } else { "mouseup" }, button(name)?]).map(|_| ())
    }

    pub struct LeftHeld;
    impl Drop for LeftHeld {
        fn drop(&mut self) {
            let _ = mouse("left", false);
        }
    }
    pub fn left_held() -> Result<LeftHeld> {
        mouse("left", true)?;
        Ok(LeftHeld)
    }

    pub fn scroll(dir: &str, steps: i32) -> Result<()> {
        let b = match dir {
            "up" => "4",
            "down" => "5",
            "left" => "6",
            "right" => "7",
            _ => bail!("Unknown scroll direction"),
        };
        xdotool(&["click", "--repeat", &steps.max(1).to_string(), b]).map(|_| ())
    }

    static RESTORE: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

    fn borrow_cursor(x: i32, y: i32) {
        let mut saved = RESTORE.lock().unwrap();
        if saved.is_none() {
            *saved = Some(super::monitors::cursor());
        }
        super::monitors::set_cursor(x, y);
    }

    fn return_cursor() {
        if let Some((x, y)) = RESTORE.lock().unwrap().take() {
            super::monitors::set_cursor(x, y);
        }
    }

    /// Click, drag or scroll at a screen point; the real cursor is put back afterwards (same behaviour as on
    /// Windows). The visible Jarvis pointer is drawn by the app.
    pub fn virtual_mouse(x: i32, y: i32, message: &str, modifiers: &str) -> Result<()> {
        let mods = modifiers
            .to_ascii_lowercase()
            .split(['+', ' ', ','])
            .filter(|k| matches!(*k, "ctrl" | "control" | "shift" | "alt"))
            .collect::<Vec<_>>()
            .join("+");
        let _held = if mods.is_empty() { None } else { held(&mods).ok() };
        let pause = || std::thread::sleep(Duration::from_millis(12));
        match message {
            "move" => {}
            "drag_move" => super::monitors::set_cursor(x, y),
            "left_down" | "right_down" | "middle_down" => {
                borrow_cursor(x, y);
                pause();
                mouse(message.trim_end_matches("_down"), true)?;
            }
            "left_up" | "right_up" | "middle_up" => {
                super::monitors::set_cursor(x, y);
                let result = mouse(message.trim_end_matches("_up"), false);
                pause();
                return_cursor();
                result?;
            }
            "up" | "down" | "left" | "right" => {
                borrow_cursor(x, y);
                pause();
                let result = scroll(message, 1);
                return_cursor();
                result?;
            }
            _ => bail!("Unsupported virtual mouse action"),
        }
        Ok(())
    }
}

pub mod clipboard {
    use super::*;

    /// No change counter on X11: 0 means "unknown", so watchers just compare the text.
    pub fn sequence() -> u32 {
        0
    }

    /// X11 has no standard "do not record" flag.
    pub fn is_sensitive() -> bool {
        false
    }

    /// Current clipboard text, if it holds text.
    pub fn get_text() -> Option<String> {
        run("xclip", &["-selection", "clipboard", "-o"]).ok()
    }

    /// Hand data to xclip, which keeps serving it from a background process.
    fn serve(args: &[&str], data: &[u8]) -> Result<()> {
        let mut child = Command::new("xclip")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("`xclip` is not installed (needed for the clipboard on Linux)")?;
        child.stdin.take().context("xclip has no input")?.write_all(data)?;
        let status = child.wait()?;
        ensure!(status.success(), "xclip failed");
        Ok(())
    }

    pub fn set_text(text: &str) -> Result<()> {
        serve(&["-selection", "clipboard", "-i"], text.as_bytes())
    }

    /// Put a file on the clipboard as a file URI, the way a file manager's Copy does.
    pub fn set_file(path: &str) -> Result<()> {
        let absolute = std::fs::canonicalize(path).with_context(|| format!("Cannot find {path}"))?;
        let uri = format!("file://{}\n", absolute.display());
        serve(&["-selection", "clipboard", "-t", "text/uri-list", "-i"], uri.as_bytes())
    }

    /// Paste `text` or a file into window `hwnd` with Ctrl+V (Ctrl+Shift+V in terminals), then restore the
    /// previous clipboard text. Fails without touching the keyboard if that window is no longer in front.
    pub fn paste(hwnd: usize, text: &str, file: Option<&str>) -> Result<()> {
        let in_front = || super::window::foreground().hwnd == hwnd;
        ensure!(in_front(), "The target window changed");
        let previous = get_text();
        match file {
            Some(path) => {
                ensure!(std::path::Path::new(path).is_file(), "Image file is missing");
                set_file(path)?;
            }
            None => set_text(text)?,
        }
        let terminal = {
            let fg = super::window::foreground();
            let name = fg.exe.to_lowercase();
            ["gnome-terminal", "konsole", "xterm", "alacritty", "kitty", "tilix", "xfce4-terminal", "terminator", "wezterm"].iter().any(|t| name.contains(t))
        };
        let sent = if in_front() {
            super::input::chord(if terminal { "ctrl+shift+v" } else { "ctrl+v" })
        } else {
            Err(anyhow::anyhow!("The target window changed"))
        };
        std::thread::sleep(Duration::from_millis(if file.is_some() { 1500 } else { 800 }));
        if let Some(old) = previous {
            let _ = set_text(&old);
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrandr_output_is_parsed_main_screen_first() {
        let text = "Screen 0: minimum 320 x 200, current 4480 x 1440, maximum 16384 x 16384\n\
            HDMI-1 connected 1920x1080+2560+0 (normal left inverted right x axis y axis) 527mm x 296mm\n\
            DP-1 connected primary 2560x1440+0+0 (normal left inverted right x axis y axis) 597mm x 336mm\n\
            DP-2 disconnected (normal left inverted right x axis y axis)\n\
            HDMI-2 connected (normal left inverted right x axis y axis)\n";
        let m = monitors::parse_xrandr(text);
        assert_eq!(m.len(), 2, "disconnected and switched-off outputs are skipped");
        assert_eq!((m[0].index, m[0].x, m[0].width, m[0].height, m[0].primary), (1, 0, 2560, 1440, true));
        assert_eq!((m[1].index, m[1].x, m[1].width), (2, 2560, 1920));
    }

    #[test]
    fn xdotool_geometry_and_xprop_lists_are_parsed() {
        let r = window::parse_geometry("WINDOW=88080391\nX=100\nY=50\nWIDTH=800\nHEIGHT=600\nSCREEN=0\n").unwrap();
        assert_eq!((r.left, r.top, r.right, r.bottom), (100, 50, 900, 650));
        assert!(window::parse_geometry("garbage").is_none());
        assert_eq!(
            window::parse_stacking("_NET_CLIENT_LIST_STACKING(WINDOW): window id # 0x1a00003, 0x2400004, 0x3c00007"),
            vec![0x1a00003, 0x2400004, 0x3c00007]
        );
    }

    #[test]
    fn keys_map_to_x_keysyms() {
        assert_eq!(input::keysym("ctrl").unwrap(), "ctrl");
        assert_eq!(input::keysym("Enter").unwrap(), "Return");
        assert_eq!(input::keysym("num/").unwrap(), "KP_Divide");
        assert_eq!(input::keysym("numpad 7").unwrap(), "KP_7");
        assert_eq!(input::keysym("numenter").unwrap(), "KP_Enter");
        assert_eq!(input::keysym("f5").unwrap(), "F5");
        assert_eq!(input::keysym("v").unwrap(), "v");
        assert_eq!(input::keysym("pagedown").unwrap(), "Next");
        assert!(input::keysym("nonsense").is_err());
        assert_eq!(input::split_combo("ctrl+shift+v"), vec!["ctrl", "shift", "v"]);
        assert_eq!(input::split_combo("ctrl+num+"), vec!["ctrl", "num+"]);
    }
}

#[cfg(test)]
mod x11_tests {
    use super::*;

    /// Needs a running X server with a window manager and an xterm titled "Jarvis Test Terminal" whose input is
    /// written to /tmp/typed.txt (see the end-to-end script). Run with `--ignored`.
    #[test]
    #[ignore = "needs an X11 session"]
    fn x11_end_to_end() {
        // Screens.
        let screens = monitors::list();
        println!("monitors: {screens:?}");
        assert!(!screens.is_empty());
        assert_eq!((screens[0].width, screens[0].height), (1600, 900));

        // Cursor round trip and the borrowed-cursor click helper.
        monitors::set_cursor(321, 234);
        assert_eq!(monitors::cursor(), (321, 234));

        // Windows.
        let found = window::find("Jarvis Test Terminal").expect("the test terminal should be found");
        println!("found: {found:?}");
        assert!(found.bounds.right > found.bounds.left);
        assert!(window::list().iter().any(|w| w.hwnd == found.hwnd));
        window::activate(found.hwnd).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let front = window::foreground();
        println!("foreground: {} / {}", front.exe, front.title);
        assert_eq!(front.hwnd, found.hwnd, "activate should bring the terminal to the front");
        assert_eq!(front.title, "Jarvis Test Terminal");

        // Typing, a chord, and pasting via the clipboard.
        input::type_text("hello from jarvis").unwrap();
        input::chord("Enter").unwrap();
        clipboard::set_text("pasted text").unwrap();
        assert_eq!(clipboard::get_text().as_deref(), Some("pasted text"));
        clipboard::paste(found.hwnd, "pasted by jarvis", None).unwrap();
        input::chord("Enter").unwrap();
        input::chord("ctrl+d").unwrap(); // ends `cat`, so the file is flushed

        // A borrowed-cursor scroll leaves the real cursor where it was.
        monitors::set_cursor(500, 400);
        input::virtual_mouse(700, 300, "up", "").unwrap();
        assert_eq!(monitors::cursor(), (500, 400));
        input::virtual_mouse(10, 10, "move", "").unwrap();
        assert_eq!(monitors::cursor(), (500, 400));
        println!("all x11 checks passed");
    }
}

#[cfg(test)]
mod x11_paste_tests {
    use super::*;

    /// Needs a GTK entry dialog titled "Jarvis Paste Test" (zenity --entry) writing its answer to /tmp/zen.txt.
    #[test]
    #[ignore = "needs an X11 session"]
    fn x11_paste_into_a_gtk_entry() {
        let found = window::find("Jarvis Paste Test").expect("the entry dialog should be found");
        window::activate(found.hwnd).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        clipboard::set_text("previous clipboard").unwrap();
        clipboard::paste(found.hwnd, "pasted by jarvis ✓ ünïcode", None).unwrap();
        // The user's clipboard comes back after the paste.
        assert_eq!(clipboard::get_text().as_deref(), Some("previous clipboard"));
        input::chord("Enter").unwrap();
    }
}
