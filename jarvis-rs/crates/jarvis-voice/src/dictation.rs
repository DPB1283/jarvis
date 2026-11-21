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
const FILLERS: [&str; 17] = [
    "hey jarvis", "jarvis", "hey", "ok", "okay", "please", "can you", "could you", "would you", "will you",
    "i want you to", "i want to", "i would like to", "i d like to", "lets", "let s", "go ahead and",
];

/// Drop polite lead-ins ("hey Jarvis, could you please ...") from an already normalised utterance.
fn strip_fillers(text: &str) -> String {
    let mut out = text.to_owned();
    loop {
        let before = out.len();
        for filler in FILLERS {
            if let Some(rest) = out.strip_prefix(filler).filter(|rest| rest.is_empty() || rest.starts_with(' ')) {
                out = rest.trim_start().to_owned();
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// "start dictation", "start voice typing", "could you begin dictating", "dictate", "type what I say"...
/// The whole utterance has to be the request, so "what is dictation" or text being dictated never matches.
pub fn start(text: &str) -> bool {
    use regex::Regex;
    use std::sync::LazyLock;
    static START: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?:(?:start|begin|enable|activate|turn on|switch on|open|launch|use|enter|go into|switch to|resume)",
            r"(?: the| my| up)?(?: voice)? (?:dictation|dictating|voice typing|voice type|voice input|speech to text|voice to text)(?: mode)?",
            r"|(?:dictation|dictate|dictating|voice typing|voice type|take dictation|let me dictate|i want to dictate|i will dictate|dictation mode)",
            r"|type (?:what i say|what i m saying|what i am saying|as i speak|as i talk|for me)",
            r"|start typing (?:what i say|for me))",
            r"(?: for me| please| now)?$"
        ))
        .unwrap()
    });
    START.is_match(&strip_fillers(&normal(text)))
}
fn stop(text: &str) -> bool {
    use regex::Regex;
    use std::sync::LazyLock;
    static STOP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?:(?:stop|end|finish|exit|quit|cancel|turn off|disable|leave)(?: the)?(?: voice)? (?:dictation|dictating|typing|voice typing|voice input)(?: mode)?",
            r"|done typing|done dictating|done dictation|finished dictating|finished typing)",
            r"(?: please| now)?$"
        ))
        .unwrap()
    });
    STOP.is_match(&strip_fillers(&normal(text)))
}
fn normal(text: &str) -> String {
    text.to_lowercase()
        .replace(['’'], " ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn parse(text: &str) -> Utterance {
    let clean = normal(text);
    if stop(text) {
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

    #[test]
    fn natural_ways_to_start_and_stop() {
        for phrase in [
            "Start dictation", "Jarvis, start dictation.", "start dictating", "Start voice typing", "Could you please start voice typing?",
            "Hey Jarvis, can you begin dictation for me", "turn on dictation mode", "enable voice typing", "Voice type", "Dictate",
            "dictation mode", "I want to dictate", "Let me dictate", "Type for me", "start typing what I say", "Take dictation",
            "open voice input", "Switch on speech to text", "type what I’m saying",
        ] {
            assert!(start(phrase), "should start: {phrase}");
        }
        for phrase in [
            "What is dictation?", "start the timer", "open notepad", "I will dictate a letter to my boss tomorrow",
            "start typing", "Windows dictation", "stop dictation",
        ] {
            assert!(!start(phrase), "must not start: {phrase}");
        }
        for phrase in [
            "stop dictation", "Stop dictating.", "Jarvis stop dictation please", "end voice typing", "turn off dictation mode",
            "done typing", "Finished dictating", "exit dictation", "stop typing",
        ] {
            assert!(matches!(parse(phrase), Utterance::Stop), "should stop: {phrase}");
        }
        assert!(!matches!(parse("stop the music"), Utterance::Stop));
    }
}
