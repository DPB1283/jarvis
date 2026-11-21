use anyhow::{Result, bail};
use jarvis_desktop::window;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    ES_PASSWORD, GUITHREADINFO, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowLongW, GetWindowThreadProcessId,
};

pub enum Utterance {
    Stop,
    Scratch,
    Text(String),
}
const FILLERS: &[&str] = &[
    "hey jarvis", "jarvis", "hey ultron", "ultron", "hey", "ok", "okay", "please", "can you", "could you", "would you", "will you",
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
    let clean = strip_fillers(&normal(text));
    if matches!(clean.as_str(), "empezar dictado" | "iniciar dictado" | "commencer la dictée" | "commencer la dictee" | "diktat starten" | "開始聽寫" | "开始听写") || START.is_match(&clean) {
        return true;
    }
    // A short request that is clearly about dictation ("start voice dictation please") also counts, unless it is
    // a question, a stop, or about something else.
    const KEYWORDS: [&str; 5] = ["dictation", "dictating", "dictate", "voice typing", "voice type"];
    const REJECT: [&str; 17] = [
        "stop", "end", "finish", "exit", "quit", "cancel", "off", "disable", "what", "how", "why", "when", "who", "not",
        "windows", "settings", "about",
    ];
    let words: Vec<&str> = clean.split(' ').collect();
    words.len() <= 6 && KEYWORDS.iter().any(|k| clean.contains(k)) && !words.iter().any(|w| REJECT.contains(w))
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
    let clean = strip_fillers(&normal(text));
    matches!(clean.as_str(), "parar dictado" | "detener dictado" | "arrêter la dictée" | "arreter la dictee" | "diktat beenden" | "停止聽寫" | "停止听写") || STOP.is_match(&clean)
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
    const MARKS: &[char] = &[',', '.', ';', ':', '!', '?'];
    for found in re.find_iter(text) {
        if found.start() < start {
            continue;
        }
        output.push_str(&text[start..found.start()]);
        let word = found.as_str().to_ascii_lowercase();
        // The transcript already has its own commas and full stops. A spoken mark replaces the ones next to
        // it instead of doubling up ("going on, comma." must give "going on," and not "going on,,.").
        output.truncate(output.trim_end_matches(|c: char| c == ' ').len());
        if word == "new line" || word == "new paragraph" {
            output.push_str(if word == "new line" { "\n" } else { "\n\n" });
        } else {
            output.truncate(output.trim_end_matches(MARKS).len());
            output.push_str(match word.as_str() {
                "period" | "full stop" => ".",
                "comma" => ",",
                "question mark" => "?",
                _ => "!",
            });
        }
        start = found.end();
        start += text[start..].len() - text[start..].trim_start_matches(|c: char| c == ' ' || MARKS.contains(&c)).len();
        if !output.ends_with('\n') && start < text.len() {
            output.push(' ');
        }
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
    secure_field_check(&fg)?;
    Ok(fg.hwnd)
}

/// X11 has no portable way to ask whether the focused field is a password box, so on Linux dictation only
/// refuses windows that are clearly authentication prompts.
#[cfg(not(windows))]
fn secure_field_check(fg: &window::Foreground) -> Result<()> {
    let title = fg.title.to_lowercase();
    let program = fg.exe.to_lowercase();
    if title.contains("password") || title.contains("authenticat") || program.contains("pinentry") || program.contains("polkit") {
        bail!("Dictation is paused in a password or authentication window.");
    }
    Ok(())
}

#[cfg(windows)]
fn secure_field_check(_fg: &window::Foreground) -> Result<()> {
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
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_multilingual_commands() {
        for (begin, end) in [("empezar dictado", "parar dictado"), ("commencer la dictée", "arrêter la dictée"), ("Diktat starten", "Diktat beenden"), ("開始聽寫", "停止聽寫")] {
            assert!(start(begin), "{begin}");
            assert!(matches!(parse(end), Utterance::Stop), "{end}");
        }
    }

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
            "start voice dictation", "Start voice dictation, please.", "Jarvis start voice dictation", "begin voice dictation now", "open voice input", "Switch on speech to text", "type what I’m saying",
        ] {
            assert!(start(phrase), "should start: {phrase}");
        }
        for phrase in [
            "What is dictation?", "start the timer", "open notepad", "I will dictate a letter to my boss tomorrow",
            "start typing", "Windows dictation", "stop dictation", "stop voice dictation", "what is voice dictation",
            "how do I turn off dictation", "open dictation settings",
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

    #[test]
    fn spoken_punctuation_replaces_the_transcripts_own() {
        let text = |t: &str| match parse(t) {
            Utterance::Text(x) => x,
            _ => panic!("not text: {t}"),
        };
        // Scribe already punctuates; the spoken word must not double it ("going on,..").
        assert_eq!(
            text("There is a test going on, comma. This is the test, period. How long will it take, question mark."),
            "There is a test going on, This is the test. How long will it take? "
        );
        assert_eq!(text("Hello comma world period"), "Hello, world. ");
        assert_eq!(text("Hello, comma, world."), "Hello, world. ");
        assert_eq!(text("First line. New line. Second line."), "First line.
Second line. ");
        assert_eq!(text("Wait, exclamation mark."), "Wait! ");
        assert_eq!(text("Just plain words."), "Just plain words. ");
    }
}
