use anyhow::{Result, bail, ensure};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

fn key(code: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(code),
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}
fn emit(events: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(events, size_of::<INPUT>() as i32) };
    ensure!(
        sent as usize == events.len(),
        "Windows rejected keyboard input"
    );
    Ok(())
}
/// Numpad keys ("num/", "numpad *", "num7", "numenter") as (virtual key, extended-key flag).
/// They need their own virtual keys: "/" is the main-row slash (VK 191), not Numpad Divide (VK 0x6F).
fn numpad(token: &str) -> Option<(u16, bool)> {
    let compact = token.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let rest = compact
        .strip_prefix("numpad")
        .or_else(|| compact.strip_prefix("num"))
        .or_else(|| compact.strip_prefix("kp"))?;
    // A bare "-" was stripped above, so "num-" arrives here as "".
    let minus = token.trim().ends_with('-');
    Some(match rest {
        "/" | "divide" | "slash" => (0x6F, true),
        "*" | "multiply" | "asterisk" | "star" => (0x6A, false),
        "+" | "plus" | "add" => (0x6B, false),
        "" if minus => (0x6D, false),
        "minus" | "subtract" | "dash" => (0x6D, false),
        "." | "decimal" | "period" | "dot" => (0x6E, false),
        "enter" | "return" => (13, true),
        "lock" => (0x90, true),
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => (0x60 + u16::from(d.as_bytes()[0] - b'0'), false),
        _ => return None,
    })
}

/// Virtual key plus whether Windows treats it as an "extended" key (needed for the numpad divide/enter,
/// and for the navigation cluster).
fn parse_key(token: &str) -> Result<(u16, bool)> {
    if let Some(found) = numpad(token) {
        return Ok(found);
    }
    let code = vk(token)?;
    Ok((code, matches!(code, 33..=40 | 45 | 46)))
}
fn vk(token: &str) -> Result<u16> {
    let token = token.trim().to_ascii_lowercase();
    Ok(match token.as_str() {
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        "win" | "super" | "windows" | "meta" | "cmd" => 0x5b,
        "enter" | "return" => 13,
        "esc" | "escape" => 27,
        "tab" => 9,
        "space" => 32,
        "backspace" => 8,
        "end" => 35,
        "home" => 36,
        "left" => 37,
        "up" => 38,
        "right" => 39,
        "down" => 40,
        "del" | "delete" => 46,
        "insert" => 45,
        "pageup" | "page_up" => 33,
        "pagedown" | "page_down" => 34,
        "minus" | "-" => 189,
        "equal" | "=" => 187,
        "period" | "." => 190,
        "comma" | "," => 188,
        "slash" | "/" => 191,
        "backslash" | "\\" => 220,
        "grave" | "`" => 192,
        "semicolon" | ";" => 186,
        "apostrophe" | "'" => 222,
        "bracketleft" | "[" => 219,
        "bracketright" | "]" => 221,
        t if t.len() == 1 && t.as_bytes()[0].is_ascii_alphanumeric() => {
            t.as_bytes()[0].to_ascii_uppercase() as u16
        }
        t if t.starts_with('f') && t[1..].parse::<u16>().is_ok_and(|n| (1..=24).contains(&n)) => {
            111 + t[1..].parse::<u16>()?
        }
        _ => bail!("Unsupported shortcut key {token}"),
    })
}
fn scan(code: u16) -> u16 {
    unsafe { MapVirtualKeyW(u32::from(code), MAPVK_VK_TO_VSC) as u16 }
}
fn press(key_: (u16, bool), up: bool) -> INPUT {
    let mut flags = if key_.1 { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    key(key_.0, scan(key_.0), flags)
}
/// "num+" is a numpad key while a lone "+" splits the combo; split on '+' but keep a trailing "num+".
fn split_combo(combo: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = combo.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'+' {
            let before = combo[start..i].trim().to_ascii_lowercase();
            let numpad_plus = i + 1 == bytes.len() && (before == "num" || before == "numpad" || before.ends_with(" num") || before == "kp");
            if numpad_plus {
                continue;
            }
            parts.push(&combo[start..i]);
            start = i + 1;
        }
    }
    parts.push(&combo[start..]);
    parts
}
/// Windows voice typing (Win+H) is never opened: Jarvis has his own dictation ("start dictation").
fn forbid_windows_dictation(keys: &[(u16, bool)]) -> Result<()> {
    let win = keys.iter().any(|k| k.0 == 0x5b || k.0 == 0x5c);
    let h = keys.iter().any(|k| k.0 == 0x48);
    ensure!(
        !(win && h),
        "Windows voice typing (Win+H) is blocked. Jarvis has his own dictation: ask the user to say \"start dictation\"."
    );
    Ok(())
}
pub fn chord(combo: &str) -> Result<()> {
    let keys = split_combo(combo)
        .into_iter()
        .map(parse_key)
        .collect::<Result<Vec<_>>>()?;
    forbid_windows_dictation(&keys)?;
    let events = keys
        .iter()
        .map(|k| press(*k, false))
        .chain(keys.iter().rev().map(|k| press(*k, true)))
        .collect::<Vec<_>>();
    emit(&events)
}
pub fn type_text(text: &str) -> Result<()> {
    let events = text
        .encode_utf16()
        .flat_map(|ch| {
            [
                key(0, ch, KEYEVENTF_UNICODE),
                key(0, ch, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
            ]
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        return Ok(());
    }
    emit(&events)
}

pub struct Held(Vec<(u16, bool)>);
impl Drop for Held {
    fn drop(&mut self) {
        for k in self.0.iter().rev() {
            let _ = emit(&[press(*k, true)]);
        }
    }
}
pub fn held(combo: &str) -> Result<Held> {
    let keys = split_combo(combo)
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(parse_key)
        .collect::<Result<Vec<_>>>()?;
    forbid_windows_dictation(&keys)?;
    let mut held = Held(vec![]);
    for k in keys {
        emit(&[press(k, false)])?;
        held.0.push(k);
    }
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numpad_keys_are_distinct_from_the_main_keyboard() {
        assert_eq!(parse_key("num/").unwrap(), (0x6F, true));
        assert_eq!(parse_key("Numpad /").unwrap(), (0x6F, true));
        assert_eq!(parse_key("NumDivide").unwrap(), (0x6F, true));
        assert_eq!(parse_key("/").unwrap(), (191, false));
        assert_eq!(parse_key("num*").unwrap().0, 0x6A);
        assert_eq!(parse_key("num-").unwrap().0, 0x6D);
        assert_eq!(parse_key("num+").unwrap().0, 0x6B);
        assert_eq!(parse_key("num.").unwrap().0, 0x6E);
        assert_eq!(parse_key("num7").unwrap().0, 0x67);
        assert_eq!(parse_key("numenter").unwrap(), (13, true));
        assert_eq!(parse_key("enter").unwrap(), (13, false));
        assert!(parse_key("num").is_err());
        assert!(parse_key("num9000").is_err());
    }

    #[test]
    fn windows_voice_typing_is_blocked() {
        assert!(chord("win+h").is_err());
        assert!(chord("super+H").is_err());
        assert!(held("win+h").is_err());
    }

    #[test]
    fn combos_split_around_numpad_plus() {
        assert_eq!(split_combo("ctrl+num/"), vec!["ctrl", "num/"]);
        assert_eq!(split_combo("alt+F10"), vec!["alt", "F10"]);
        assert_eq!(split_combo("ctrl+num+"), vec!["ctrl", "num+"]);
        assert_eq!(split_combo("num+"), vec!["num+"]);
    }
}
fn mouse_event(flags: MOUSE_EVENT_FLAGS, data: i32) -> Result<()> {
    emit(&[INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                mouseData: data as u32,
                ..Default::default()
            },
        },
    }])
}
pub fn mouse(button: &str, down: bool) -> Result<()> {
    mouse_event(
        match (button, down) {
            ("left", true) => MOUSEEVENTF_LEFTDOWN,
            ("left", false) => MOUSEEVENTF_LEFTUP,
            ("right", true) => MOUSEEVENTF_RIGHTDOWN,
            ("right", false) => MOUSEEVENTF_RIGHTUP,
            ("middle", true) => MOUSEEVENTF_MIDDLEDOWN,
            ("middle", false) => MOUSEEVENTF_MIDDLEUP,
            _ => bail!("Unknown mouse button"),
        },
        0,
    )
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
    let (flags, sign) = match dir {
        "left" => (MOUSEEVENTF_HWHEEL, -1),
        "right" => (MOUSEEVENTF_HWHEEL, 1),
        "up" => (MOUSEEVENTF_WHEEL, 1),
        "down" => (MOUSEEVENTF_WHEEL, -1),
        _ => bail!("Unknown scroll direction"),
    };
    mouse_event(flags, sign * 120 * steps)
}

/// Send mouse messages to the window under Jarvis's virtual pointer.
pub fn virtual_mouse(x: i32, y: i32, message: &str, modifiers: &str) -> Result<()> {
    use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::{
        CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, ChildWindowFromPointEx, PostMessageW, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WindowFromPoint,
    };
    let screen = POINT { x, y };
    let mut point = screen;
    let window = unsafe { WindowFromPoint(screen) };
    ensure!(!window.is_invalid(), "No window under the virtual pointer");
    unsafe { ScreenToClient(window, &mut point) }.ok()?;
    let child =
        unsafe { ChildWindowFromPointEx(window, point, CWP_SKIPINVISIBLE | CWP_SKIPDISABLED) };
    let target = if child.is_invalid() { window } else { child };
    point = screen;
    unsafe { ScreenToClient(target, &mut point) }.ok()?;
    let mut keys = 0usize;
    let modifiers = modifiers.to_ascii_lowercase();
    if modifiers.contains("ctrl") || modifiers.contains("control") {
        keys |= 8;
    }
    if modifiers.contains("shift") {
        keys |= 4;
    }
    if message == "drag_move" || message == "left_down" {
        keys |= 1;
    }
    let (msg, delta, absolute) = match message {
        "move" | "drag_move" => (WM_MOUSEMOVE, 0i16, false),
        "left_down" => (WM_LBUTTONDOWN, 0, false),
        "left_up" => (WM_LBUTTONUP, 0, false),
        "right_down" => (WM_RBUTTONDOWN, 0, false),
        "right_up" => (WM_RBUTTONUP, 0, false),
        "middle_down" => (WM_MBUTTONDOWN, 0, false),
        "middle_up" => (WM_MBUTTONUP, 0, false),
        "up" => (WM_MOUSEWHEEL, 120, true),
        "down" => (WM_MOUSEWHEEL, -120, true),
        "left" => (WM_MOUSEHWHEEL, -120, true),
        "right" => (WM_MOUSEHWHEEL, 120, true),
        _ => bail!("Unsupported virtual mouse action"),
    };
    if absolute {
        point = screen;
    }
    let packed = ((point.x as u16 as u32) | ((point.y as u16 as u32) << 16)) as i32 as isize;
    let wparam = keys | ((delta as u16 as usize) << 16);
    unsafe { PostMessageW(Some(target), msg, WPARAM(wparam), LPARAM(packed)) }?;
    Ok(())
}
