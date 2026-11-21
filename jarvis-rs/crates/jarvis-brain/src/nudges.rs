//! Proactive nudges: Jarvis speaks up on his own, once per meeting, shortly before it starts. When the event
//! has a place and a home address and Mapbox are set up, the lead time includes the drive, so it is a
//! "time to leave" reminder. Opt out in Settings → Nudges. It stays quiet during full-screen apps and while he
//! is busy, and is silent in an app whose rule says so.

use crate::Ctx;
use crate::tools::{ToolOutput, utilities};
use jarvis_core::Event;
use jarvis_core::events::JarvisState;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Meeting {
    pub id: String,
    pub title: String,
    pub start_ms: i64,
    pub location: String,
}

pub(crate) fn parse_meetings(text: &str) -> Vec<Meeting> {
    serde_json::from_str::<Vec<Value>>(text)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            Some(Meeting {
                id: e["id"].as_str()?.to_owned(),
                title: e["title"].as_str()?.to_owned(),
                start_ms: e["startMs"].as_i64()?,
                location: e["location"].as_str().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// Time to speak up: the meeting is within the lead time plus any travel, and has not started.
pub(crate) fn due(minutes_until: i64, lead: u32, travel_minutes: Option<u64>) -> bool {
    minutes_until >= 0 && minutes_until <= i64::from(lead) + travel_minutes.unwrap_or(0) as i64
}

pub(crate) fn nudge_text(language: &str, title: &str, minutes: i64, travel_minutes: Option<u64>) -> String {
    let n = minutes.max(0);
    match (language, travel_minutes) {
        ("es", None) => format!("Aviso: {title} empieza en {n} minutos."),
        ("es", Some(m)) => format!("{title} empieza en {n} minutos y queda a unos {m} minutos, así que es hora de salir."),
        ("fr", None) => format!("Rappel : {title} commence dans {n} minutes."),
        ("fr", Some(m)) => format!("{title} commence dans {n} minutes et c'est à environ {m} minutes, il est temps de partir."),
        ("de", None) => format!("Hinweis: {title} beginnt in {n} Minuten."),
        ("de", Some(m)) => format!("{title} beginnt in {n} Minuten und der Weg dauert etwa {m} Minuten, es ist Zeit zu gehen."),
        ("yue", None) => format!("提提你：{title} 仲有 {n} 分鐘就開始。"),
        ("yue", Some(m)) => format!("{title} 仲有 {n} 分鐘就開始，路程大約 {m} 分鐘，係時候出發喇。"),
        (_, None) if n <= 1 => format!("{title} is starting now."),
        (_, None) => format!("Heads up: {title} starts in {n} minutes."),
        (_, Some(m)) => format!("{title} starts in {n} minutes, and it's about {m} minutes away, so it's time to leave."),
    }
}

fn nudged_file() -> std::path::PathBuf {
    jarvis_core::paths::data_dir().join("nudged.json")
}

fn load_nudged() -> HashSet<String> {
    std::fs::read_to_string(nudged_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub async fn run(ctx: Ctx) {
    let mut events = ctx.bus.subscribe();
    let mut idle = true;
    let mut nudged = load_nudged();
    let mut travel: HashMap<String, Option<u64>> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        while let Ok(event) = events.try_recv() {
            if let Event::State { state } = event {
                idle = state == JarvisState::Idle;
            }
        }
        let settings = ctx.settings.get();
        if !settings.nudges.enabled || settings.calendar.ics_url.trim().is_empty() || !idle {
            continue;
        }
        let busy = tokio::task::spawn_blocking(|| jarvis_desktop::window::foreground().fullscreen).await.unwrap_or(false);
        if busy {
            continue;
        }
        let Ok(ToolOutput::Text(text)) = utilities::run(ctx.clone(), "calendar_upcoming", json!({"hours": 3})).await else { continue };
        let now = chrono::Local::now().timestamp_millis();
        for meeting in parse_meetings(&text) {
            let minutes = (meeting.start_ms - now).div_euclid(60_000);
            if nudged.contains(&meeting.id) || minutes < 0 || minutes > 150 {
                continue;
            }
            // Work out the drive once per meeting, and only when it could matter.
            let minutes_away = if meeting.location.is_empty() {
                None
            } else if let Some(cached) = travel.get(&meeting.id) {
                *cached
            } else {
                let eta = crate::tools::travel::eta_seconds(&meeting.location).await.ok().map(|s| (s / 60.0).ceil() as u64);
                travel.insert(meeting.id.clone(), eta);
                eta
            };
            if !due(minutes, settings.nudges.lead_minutes, minutes_away) {
                continue;
            }
            nudged.insert(meeting.id.clone());
            let _ = std::fs::write(nudged_file(), serde_json::to_string(&nudged).unwrap_or_default());
            let text = nudge_text(settings.language_code(), &meeting.title, minutes, minutes_away);
            ctx.bus.emit(Event::AssistantDone { text: text.clone() });
            if !crate::rules::matching(&settings).is_some_and(|r| r.silent) {
                ctx.bus.emit(Event::Speak { text });
                ctx.bus.emit(Event::SpeakEnd {});
            }
            break; // one nudge per minute is plenty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meetings_are_read_from_the_helpers_json() {
        let list = parse_meetings(r#"[{"id":"a@1","title":"Dentist","startMs":1000,"endMs":2000,"location":"12 High Street"},{"id":"b@2","title":"Call","startMs":5000,"endMs":6000,"location":""},{"bad":true}]"#);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Meeting { id: "a@1".into(), title: "Dentist".into(), start_ms: 1000, location: "12 High Street".into() });
        assert!(parse_meetings("not json").is_empty());
    }

    #[test]
    fn the_nudge_comes_at_lead_time_plus_the_drive() {
        assert!(due(10, 10, None), "ten minutes before");
        assert!(!due(11, 10, None));
        assert!(due(25, 10, Some(15)), "a 15 minute drive: leave 25 minutes before");
        assert!(!due(26, 10, Some(15)));
        assert!(!due(-1, 10, None), "never after it has started");
        assert!(due(0, 10, None));
    }

    #[test]
    fn the_nudge_is_worded_in_each_language() {
        assert_eq!(nudge_text("en", "Dentist", 10, None), "Heads up: Dentist starts in 10 minutes.");
        assert!(nudge_text("en", "Dentist", 25, Some(15)).contains("time to leave"));
        assert!(nudge_text("en", "Dentist", 0, None).contains("starting now"));
        assert!(nudge_text("es", "Dentista", 10, None).starts_with("Aviso"));
        assert!(nudge_text("fr", "Dentiste", 25, Some(15)).contains("partir"));
        assert!(nudge_text("de", "Zahnarzt", 25, Some(15)).contains("Zeit zu gehen"));
        assert!(nudge_text("yue", "睇牙醫", 25, Some(15)).contains("出發"));
        assert!(nudge_text("xx", "A", 5, None).starts_with("Heads up"), "unknown languages fall back to English");
    }
}
