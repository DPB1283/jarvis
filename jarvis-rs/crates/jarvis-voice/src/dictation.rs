use anyhow::{Result, bail};
use jarvis_desktop::window;
use windows::Win32::UI::WindowsAndMessaging::{
    ES_PASSWORD, GUITHREADINFO, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowLongW, GetWindowThreadProcessId,
};

pub enum Utterance {
    Stop,
    Scratch,
    Text(String),
}
pub fn start(text: &str) -> bool {
    matches!(
        normal(text).as_str(),
        "start dictation" | "dictation mode" | "voice type" | "type what i say"
    )
}
fn normal(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn parse(text: &str) -> Utterance {
    let clean = normal(text);
    if matches!(
        clean.as_str(),
        "stop dictation" | "stop typing" | "done typing" | "exit dictation"
    ) {
        return Utterance::Stop;
    }
    if clean == "scratch that" {
        return Utterance::Scratch;
    }
    let re = regex::Regex::new(r"(?i)\b(new paragraph|new line|question mark|exclamation mark|exclamation point|full stop|period|comma)\b").unwrap();
    let mut output = String::new();
    let mut start = 0;
    for found in re.find_iter(text) {
        output.push_str(&text[start..found.start()]);
        let word = found.as_str().to_ascii_lowercase();
        if word == "new line" || word == "new paragraph" {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push_str(if word == "new line" { "\n" } else { "\n\n" });
        } else {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push_str(match word.as_str() {
                "period" | "full stop" => ".",
                "comma" => ",",
                "question mark" => "?",
                _ => "!",
            });
        }
        start = found.end();
    }
    output.push_str(&text[start..]);
    let mut result = output.trim_matches(' ').to_owned();
    result.push(' ');
    Utterance::Text(result)
}
pub fn target() -> Result<usize> {
    let fg = window::foreground();
    if fg.hwnd == 0 || fg.pid == std::process::id() {
        bail!("Focus another app to dictate.");
    }
    unsafe {
        let hwnd = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(hwnd, None);
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(tid, &mut gui).is_ok() && !gui.hwndFocus.is_invalid() {
            let mut class = [0u16; 128];
            let n = GetClassNameW(gui.hwndFocus, &mut class).max(0) as usize;
            if String::from_utf16_lossy(&class[..n]).eq_ignore_ascii_case("Edit")
                && GetWindowLongW(gui.hwndFocus, GWL_STYLE) & ES_PASSWORD != 0
            {
                bail!("Dictation is paused in a password field.");
            }
        }
    }
    unsafe {
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        };
        use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let password =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .and_then(|automation| automation.GetFocusedElement())
                .and_then(|element| element.CurrentIsPassword())
                .map(|value| value.as_bool())
                .unwrap_or(false);
        if initialized {
            CoUninitialize();
        }
        if password {
            bail!("Dictation is paused in a password field.");
        }
    }
    Ok(fg.hwnd)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_and_punctuation() {
        assert!(start("Type what I say."));
        assert!(matches!(parse("stop typing"), Utterance::Stop));
        assert!(matches!(parse("scratch that"), Utterance::Scratch));
        let Utterance::Text(text) = parse("Hello comma world period") else {
            panic!()
        };
        assert_eq!(text, "Hello, world. ");
    }
}
