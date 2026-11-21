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

/// Where the user's real cursor was before Jarvis borrowed it for a click or drag.
static RESTORE: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

fn borrow_cursor(x: i32, y: i32) {
    let mut saved = RESTORE.lock().unwrap();
    if saved.is_none() {
        *saved = Some(crate::monitors::cursor());
    }
    crate::monitors::set_cursor(x, y);
}

fn return_cursor() {
    if let Some((x, y)) = RESTORE.lock().unwrap().take() {
        crate::monitors::set_cursor(x, y);
    }
}

/// Click, drag or scroll at a screen point as Jarvis's virtual pointer.
///
/// Posting window messages to the window under the point looked like a separate mouse, but most modern apps
/// (Chrome, Electron, Store apps) ignore posted mouse messages and the click never focuses them, so typing
/// then goes nowhere. This sends real input instead: the cursor jumps to the point for the click and goes
/// straight back to where the user had it when the press is released. The on-screen Jarvis pointer is drawn
/// separately by the app.
pub fn virtual_mouse(x: i32, y: i32, message: &str, modifiers: &str) -> Result<()> {
    // Modifiers (ctrl/shift/alt) are held only around this one event.
    let _held = if modifiers.trim().is_empty() {
        None
    } else {
        let combo = modifiers
            .to_ascii_lowercase()
            .split(['+', ' ', ','])
            .filter(|k| matches!(*k, "ctrl" | "control" | "shift" | "alt"))
            .collect::<Vec<_>>()
            .join("+");
        if combo.is_empty() { None } else { held(&combo).ok() }
    };
    match message {
        // Moving Jarvis's pointer does not touch the real one.
        "move" => {}
        "drag_move" => crate::monitors::set_cursor(x, y),
        "left_down" | "right_down" | "middle_down" => {
            borrow_cursor(x, y);
            std::thread::sleep(std::time::Duration::from_millis(12));
            mouse(message.trim_end_matches("_down"), true)?;
        }
        "left_up" | "right_up" | "middle_up" => {
            crate::monitors::set_cursor(x, y);
            let result = mouse(message.trim_end_matches("_up"), false);
            std::thread::sleep(std::time::Duration::from_millis(12));
            return_cursor();
            result?;
        }
        "up" | "down" | "left" | "right" => {
            borrow_cursor(x, y);
            std::thread::sleep(std::time::Duration::from_millis(12));
            let result = scroll(message, 1);
            return_cursor();
            result?;
        }
        _ => bail!("Unsupported virtual mouse action"),
    }
    Ok(())
}

#[cfg(test)]
mod pointer_tests {
    #[test]
    #[ignore = "moves the real cursor briefly"]
    fn cursor_is_returned_after_a_borrowed_scroll() {
        let before = crate::monitors::cursor();
        super::virtual_mouse(before.0 + 120, before.1 + 80, "up", "").unwrap();
        assert_eq!(crate::monitors::cursor(), before);
        super::virtual_mouse(before.0 + 50, before.1 + 50, "move", "").unwrap();
        assert_eq!(crate::monitors::cursor(), before, "a virtual move must not touch the real cursor");
    }
}
