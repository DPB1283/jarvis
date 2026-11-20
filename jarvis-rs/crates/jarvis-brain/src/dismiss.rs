//! "Go away" phrases: Jarvis stops silently, with no reply and no API call (port of `main/dismiss.ts`).
//! END phrases count when they finish the utterance; WHOLE phrases only when they're (nearly) all that was said.

use std::collections::HashSet;
use std::sync::OnceLock;

const END_PHRASES: &[&str] = &[
    "never mind", "nevermind", "nvm", "never mind then",
    "forget it", "forget that", "forget about it",
    "ignore me", "ignore that", "scratch that", "disregard", "disregard that", "cancel", "cancel that",
    "that's all", "that is all", "that's it", "that'll be all", "that will be all", "that's everything",
    "nothing else", "i'm good", "i am good", "i'm fine", "all good", "no thanks", "no thank you",
    "not now", "not right now", "false alarm", "wrong person", "i wasn't talking to you", "not talking to you",
    "go back to sleep", "go to sleep", "as you were", "carry on", "dismissed", "you're dismissed", "stand down",
    "don't worry about it", "dont worry about it", "don't worry", "no worries", "it's fine", "it's okay", "never you mind",
];

const WHOLE_PHRASES: &[&str] = &[
    "stop", "stop talking", "stop it", "quiet", "be quiet", "silence", "shush", "hush", "shh", "shut up",
    "nothing", "no", "nah", "nope", "thanks", "thank you", "cheers", "ta",
    "ok", "okay", "cool", "great", "perfect", "nice", "got it", "understood", "noted", "alright", "all right",
    "bye", "goodbye", "bye bye", "good night", "goodnight", "night", "see you", "see ya", "later", "see you later",
    "go away", "leave me alone", "never mind thanks",
];

const LEADING_FILLERS: &[&str] = &["oh", "ah", "um", "umm", "uh", "uhh", "er", "actually", "well", "okay", "ok", "alright", "right", "jarvis", "hey", "so"];
const TRAILING_FILLERS: &[&str] = &["jarvis", "then", "sorry", "please", "thanks", "thank you", "mate", "buddy", "bud", "man", "for now", "now"];

pub fn normalise(text: &str) -> String {
    let lower = text.to_lowercase().replace(['\u{2019}', '\u{2018}', '`'], "'");
    // drop "(laughs)" / "[music]" style tags
    let mut no_tags = String::with_capacity(lower.len());
    let mut depth = 0;
    for c in lower.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => {
                depth -= 1;
                no_tags.push(' ');
            }
            _ if depth > 0 => {}
            _ => no_tags.push(c),
        }
    }
    let cleaned: String = no_tags.chars().map(|c| if c.is_ascii_alphanumeric() || c == '\'' || c == ' ' { c } else { ' ' }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_fillers(s: &str) -> String {
    let mut out = s.to_string();
    let mut changed = true;
    while changed {
        changed = false;
        for f in LEADING_FILLERS {
            if let Some(rest) = out.strip_prefix(&format!("{f} ")) {
                out = rest.to_string();
                changed = true;
            }
        }
        for f in TRAILING_FILLERS {
            if let Some(rest) = out.strip_suffix(&format!(" {f}")) {
                out = rest.to_string();
                changed = true;
            }
        }
    }
    out.trim().to_string()
}

fn tables() -> &'static (HashSet<String>, Vec<String>) {
    static T: OnceLock<(HashSet<String>, Vec<String>)> = OnceLock::new();
    T.get_or_init(|| {
        let whole = WHOLE_PHRASES.iter().map(|p| normalise(p)).collect();
        let mut end: Vec<String> = END_PHRASES.iter().map(|p| normalise(p)).collect();
        end.sort_by_key(|p| std::cmp::Reverse(p.len()));
        (whole, end)
    })
}

pub fn is_dismissal(text: &str) -> bool {
    let (whole, end) = tables();
    let n = normalise(text);
    if n.is_empty() {
        return false;
    }
    if whole.contains(&n) {
        return true;
    }
    let core = strip_fillers(&n);
    if whole.contains(&core) {
        return true;
    }
    let ends_with = |s: &str| end.iter().any(|p| s == p || s.ends_with(&format!(" {p}")));
    ends_with(&n) || ends_with(&core)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_answers_as_the_typescript_tests() {
        for s in [
            "Never mind.", "nevermind", "30. Never mind.", "Oh, never mind.", "Actually, forget it.", "Jarvis, never mind", "nothing",
            "Never mind, sorry.", "scratch that", "Open Spotify. Actually never mind.", "Can you uh... cancel that", "NVM",
            "Forget about it, Jarvis.", "Thanks Jarvis, that's all.", "That's it for now.", "I'm good, thanks.", "Not now.",
            "Sorry, I wasn't talking to you.", "Stop.", "Shh.", "Thanks!", "Cheers mate", "Okay.", "Goodnight Jarvis.", "Don't worry about it.",
        ] {
            assert!(is_dismissal(s), "should dismiss: {s}");
        }
        for s in [
            "Never mind the weather, what time is it?", "Forget it and open Notepad", "Nothing is working on my PC",
            "Open Spotify", "Cancel my 3pm reminder", "How do I cancel Netflix?", "Make the music quiet", "Don't stop the timer",
            "Okay, what's the weather?", "Is that all the news?", "Open Discord, thanks", "Message Sam, thanks a lot",
        ] {
            assert!(!is_dismissal(s), "should keep: {s}");
        }
    }
}
