//! Per-app behaviour: while a given app is in front, Jarvis can stay silent (text only), keep replies short, or
//! ask before every action. The rule is chosen from the last app the user was in that is not Jarvis himself,
//! because pressing the talk hotkey puts Jarvis's own window in front.

use jarvis_core::Settings;
use jarvis_core::settings::AppRule;
use std::sync::Mutex;
use std::time::Duration;

/// (program name, window title), both lower-case, of the last app in front that was not Jarvis.
static LAST_APP: Mutex<Option<(String, String)>> = Mutex::new(None);

fn is_jarvis(exe: &str, title: &str) -> bool {
    exe.contains("jarvis") || title == "jarvis hud" || title.starts_with("jarvis")
}

/// The first rule whose `app` text appears in the program name or the window title.
pub(crate) fn find_rule<'a>(rules: &'a [AppRule], exe: &str, title: &str) -> Option<&'a AppRule> {
    let (exe, title) = (exe.to_lowercase(), title.to_lowercase());
    rules.iter().find(|r| {
        let app = r.app.trim().to_lowercase();
        !app.is_empty() && (exe.contains(&app) || title.contains(&app))
    })
}

/// The rule that applies right now, if any.
pub fn matching(settings: &Settings) -> Option<AppRule> {
    if settings.app_rules.is_empty() {
        return None;
    }
    let guard = LAST_APP.lock().unwrap();
    let (exe, title) = guard.as_ref()?;
    find_rule(&settings.app_rules, exe, title).cloned()
}

/// Watch the foreground window (once a second) so rules know which app the user was in.
pub async fn track() {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(fg) = tokio::task::spawn_blocking(jarvis_desktop::window::foreground).await {
            let (exe, title) = (fg.exe.to_lowercase(), fg.title.to_lowercase());
            if !exe.is_empty() && !is_jarvis(&exe, &title) {
                *LAST_APP.lock().unwrap() = Some((exe, title));
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn pretend_foreground(exe: &str, title: &str) {
    *LAST_APP.lock().unwrap() = Some((exe.to_lowercase(), title.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app: &str, silent: bool, length: &str, ask: bool) -> AppRule {
        AppRule { app: app.into(), silent, reply_length: length.into(), ask_first: ask }
    }

    #[test]
    fn rules_match_the_program_or_the_window_title() {
        let rules = vec![rule("cs2", true, "", false), rule("discord", false, "brief", true), rule("", true, "", false)];
        assert_eq!(find_rule(&rules, "cs2.exe", "Counter-Strike 2").unwrap().app, "cs2");
        assert_eq!(find_rule(&rules, "discordcanary.exe", "@Remy - Discord Canary").unwrap().app, "discord");
        assert_eq!(find_rule(&rules, "chrome.exe", "Watching Discord tutorial - YouTube").unwrap().app, "discord", "titles count too");
        assert!(find_rule(&rules, "notepad.exe", "Untitled").is_none(), "an empty app name never matches everything");
    }

    #[test]
    fn jarvis_himself_is_never_the_app() {
        assert!(is_jarvis("jarvis.exe", "x"));
        assert!(is_jarvis("jarvis-app", "x"));
        assert!(is_jarvis("webview", "jarvis hud"));
        assert!(!is_jarvis("chrome.exe", "a jar of jam"));
    }

    #[test]
    fn the_current_rule_follows_the_last_real_app() {
        let mut settings = Settings::default();
        assert!(matching(&settings).is_none());
        settings.app_rules = vec![rule("notepad", true, "middle", false)];
        pretend_foreground("notepad.exe", "Untitled - Notepad");
        let found = matching(&settings).unwrap();
        assert!(found.silent && found.reply_length == "middle");
        pretend_foreground("chrome.exe", "Google");
        assert!(matching(&settings).is_none());
    }
}
