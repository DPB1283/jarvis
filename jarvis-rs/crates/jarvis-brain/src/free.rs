//! Free mode: what Jarvis does with no AI key. A handful of everyday commands are understood locally and
//! answered with preset text on screen. Nothing is sent anywhere and nothing is spoken (speech needs an
//! ElevenLabs key too). Adding an AI key in Settings → API keys switches the full assistant on.

use crate::{Ctx, tools};
use anyhow::{Result, anyhow};
use chrono::Local;
use jarvis_core::model::TimerKind;
use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
pub enum Volume {
    Up,
    Down,
    Set(u32),
    Mute,
    Unmute,
}

#[derive(Debug, PartialEq)]
pub enum Intent {
    Time,
    Date,
    Timer { seconds: u64 },
    CancelTimers,
    ListTimers,
    Volume(Volume),
    Open(String),
    Screenshot,
    Lock,
    Help,
    Hello,
    Thanks,
    Unknown,
}

fn words(text: &str) -> Vec<String> {
    crate::dismiss::normalise(text).split_whitespace().map(str::to_owned).collect()
}

/// "5", "five", "a", "an", "half" as a number of units.
fn number(word: &str) -> Option<f64> {
    if let Ok(n) = word.parse::<f64>() {
        return Some(n);
    }
    Some(match word {
        "a" | "an" | "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "fifteen" => 15.0,
        "twenty" => 20.0,
        "thirty" => 30.0,
        "forty" => 40.0,
        "forty five" => 45.0,
        "sixty" => 60.0,
        "half" => 0.5,
        _ => return None,
    })
}

fn unit_seconds(word: &str) -> Option<f64> {
    Some(match word.trim_end_matches('s') {
        "second" | "sec" => 1.0,
        "minute" | "min" => 60.0,
        "hour" | "hr" => 3600.0,
        _ => return None,
    })
}

/// "timer for 5 minutes", "set a 30 second timer", "10 minute timer", "timer for an hour and a half".
fn timer_seconds(w: &[String]) -> Option<u64> {
    let mut total = 0.0;
    let mut i = 0;
    while i < w.len() {
        if let (Some(n), Some(next)) = (number(&w[i]), w.get(i + 1)) {
            if let Some(unit) = unit_seconds(next) {
                total += n * unit;
                i += 2;
                continue;
            }
        }
        // "an hour and a half"
        if w[i] == "half" && w.get(i + 1).map(String::as_str) == Some("an") && w.get(i + 2).and_then(|u| unit_seconds(u)).is_some() {
            total += 0.5 * unit_seconds(&w[i + 2]).unwrap_or(0.0);
            i += 3;
            continue;
        }
        i += 1;
    }
    // "and a half" after an hour / a minute
    if w.windows(3).any(|x| x[0] == "and" && x[1] == "a" && x[2] == "half") {
        if let Some(unit) = w.iter().filter_map(|u| unit_seconds(u)).next_back() {
            total += 0.5 * unit;
        }
    }
    (total >= 1.0).then_some(total.round() as u64)
}

pub fn parse(text: &str) -> Intent {
    let w = words(text);
    if w.is_empty() {
        return Intent::Unknown;
    }
    let has = |word: &str| w.iter().any(|x| x == word);
    let joined = w.join(" ");
    let starts = |prefix: &str| joined == prefix || joined.starts_with(&format!("{prefix} "));

    // Timers first: "timer" / "alarm" with a duration.
    if has("timer") || has("timers") || has("alarm") {
        if has("cancel") || has("stop") || has("clear") || has("delete") || has("remove") {
            return Intent::CancelTimers;
        }
        if has("left") || has("remaining") || starts("how long") || starts("list") || starts("show") || starts("what") || starts("check") {
            return Intent::ListTimers;
        }
        if let Some(seconds) = timer_seconds(&w) {
            return Intent::Timer { seconds };
        }
        return Intent::ListTimers;
    }
    // "remind me in 10 minutes" is a reminder, which needs the AI; keep it out of here.
    if joined.contains("what time") || joined == "time" || joined == "the time" || starts("time now") || joined.contains("current time") || joined.contains("time is it") {
        return Intent::Time;
    }
    if has("date") || joined.contains("what day") || joined.contains("day is it") || joined.contains("day of the week") {
        return Intent::Date;
    }
    if has("volume") || has("louder") || has("quieter") || has("softer") || has("mute") || has("unmute") {
        if has("unmute") {
            return Intent::Volume(Volume::Unmute);
        }
        if has("mute") {
            return Intent::Volume(Volume::Mute);
        }
        if has("louder") || (has("up") && !has("down")) || has("raise") || has("increase") {
            return Intent::Volume(Volume::Up);
        }
        if has("quieter") || has("softer") || has("down") || has("lower") || has("decrease") || has("reduce") {
            return Intent::Volume(Volume::Down);
        }
        if let Some(level) = w.iter().filter_map(|x| x.parse::<u32>().ok()).next() {
            return Intent::Volume(Volume::Set(level.min(100)));
        }
    }
    if has("screenshot") || joined.contains("screen shot") || joined.contains("capture the screen") {
        return Intent::Screenshot;
    }
    if (starts("lock") || joined.contains("lock my") || joined.contains("lock the")) && (has("pc") || has("computer") || has("screen") || has("laptop") || joined == "lock") {
        return Intent::Lock;
    }
    if ["open", "launch", "start", "run"].iter().any(|v| w[0] == *v) && w.len() >= 2 {
        let target = w[1..].join(" ");
        let target = target.strip_prefix("the ").unwrap_or(&target).strip_suffix(" app").unwrap_or(&target).trim().to_owned();
        if !target.is_empty() {
            return Intent::Open(target);
        }
    }
    if joined == "help" || starts("help me") || joined.contains("what can you do") || joined.contains("what do you do") || has("commands") || joined.contains("what can i say") {
        return Intent::Help;
    }
    if matches!(w[0].as_str(), "hello" | "hi" | "hey" | "morning" | "evening" | "yo" | "greetings") && w.len() <= 3 {
        return Intent::Hello;
    }
    if w[0] == "thanks" || w[0] == "thank" || joined == "cheers" {
        return Intent::Thanks;
    }
    Intent::Unknown
}

const HELP: &str = "Free mode, no AI key. I understand:\n\
• what time / what date it is\n\
• \"set a timer for 5 minutes\", \"cancel the timers\", \"how long is left\"\n\
• \"volume up\", \"volume down\", \"set volume to 30\", \"mute\", \"unmute\"\n\
• \"open Notepad\" (any app), \"take a screenshot\", \"lock my PC\"\n\
Add an AI key in Settings → API keys for the full assistant: conversation, voice, the web, and everything else.";

fn spoken_seconds(seconds: u64) -> String {
    let part = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(part(h, "hour"));
    }
    if m > 0 {
        parts.push(part(m, "minute"));
    }
    if s > 0 && h == 0 {
        parts.push(part(s, "second"));
    }
    parts.join(" ")
}

async fn call(ctx: &Ctx, name: &str, args: Value) -> Result<String> {
    let tool = tools::find(name).ok_or_else(|| anyhow!("That tool is not available here"))?;
    let output = (tool.run)(ctx.clone(), args).await?;
    Ok(output.log_text())
}

/// The preset reply for a command, doing the work first. Never fails: a problem becomes the reply text.
pub async fn respond(ctx: &Ctx, text: &str) -> String {
    let title = ctx.settings.get().user_title;
    let result: Result<String> = async {
        Ok(match parse(text) {
            Intent::Time => format!("It's {}.", Local::now().format("%-I:%M %p")),
            Intent::Date => format!("Today is {}.", Local::now().format("%A, %-d %B %Y")),
            Intent::Timer { seconds } => {
                let ends = Local::now().timestamp_millis() + seconds as i64 * 1000;
                ctx.timers.add("Timer", TimerKind::Timer, ends)?;
                format!("Timer set for {}.", spoken_seconds(seconds))
            }
            Intent::CancelTimers => {
                let n = ctx.timers.cancel("all", true)?;
                if n == 0 { "There are no timers running.".into() } else { format!("Cancelled {n} timer{}.", if n == 1 { "" } else { "s" }) }
            }
            Intent::ListTimers => {
                let now = Local::now().timestamp_millis();
                let timers = ctx.timers.list();
                if timers.is_empty() {
                    "There are no timers running.".into()
                } else {
                    timers
                        .iter()
                        .map(|t| format!("{}: {} left", t.label, spoken_seconds(((t.ends_at - now).max(0) / 1000) as u64)))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            Intent::Volume(v) => {
                let args = match v {
                    Volume::Up => json!({"change": 10}),
                    Volume::Down => json!({"change": -10}),
                    Volume::Set(level) => json!({"level": level}),
                    Volume::Mute => json!({"mute": true}),
                    Volume::Unmute => json!({"mute": false}),
                };
                call(ctx, "set_volume", args).await?
            }
            Intent::Open(target) => call(ctx, "open_app", json!({"target": target})).await?,
            Intent::Screenshot => call(ctx, "take_screenshot", json!({})).await?,
            Intent::Lock => {
                call(ctx, "power", json!({"action": "lock"})).await?;
                "Locking the screen.".into()
            }
            Intent::Help => HELP.into(),
            Intent::Hello => format!("Hello, {title}. I'm in free mode, so I only understand a few commands. Say \"help\" to see them."),
            Intent::Thanks => "You're welcome.".into(),
            Intent::Unknown => format!(
                "I can't do that in free mode, {title}. Add an AI key in Settings → API keys to unlock conversation, voice and everything else, or say \"help\" for what works now."
            ),
        })
    }
    .await;
    result.unwrap_or_else(|error| format!("That didn't work: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyday_commands_are_recognised() {
        assert_eq!(parse("What time is it?"), Intent::Time);
        assert_eq!(parse("time"), Intent::Time);
        assert_eq!(parse("what's the date today"), Intent::Date);
        assert_eq!(parse("What day is it"), Intent::Date);
        assert_eq!(parse("set a timer for 5 minutes"), Intent::Timer { seconds: 300 });
        assert_eq!(parse("timer for 90 seconds"), Intent::Timer { seconds: 90 });
        assert_eq!(parse("10 minute timer"), Intent::Timer { seconds: 600 });
        assert_eq!(parse("start a timer for an hour"), Intent::Timer { seconds: 3600 });
        assert_eq!(parse("set a timer for 1 hour and 30 minutes"), Intent::Timer { seconds: 5400 });
        assert_eq!(parse("timer for an hour and a half"), Intent::Timer { seconds: 5400 });
        assert_eq!(parse("cancel the timers"), Intent::CancelTimers);
        assert_eq!(parse("how long is left on my timer"), Intent::ListTimers);
        assert_eq!(parse("volume up"), Intent::Volume(Volume::Up));
        assert_eq!(parse("make it louder"), Intent::Volume(Volume::Up));
        assert_eq!(parse("turn the volume down"), Intent::Volume(Volume::Down));
        assert_eq!(parse("set volume to 35"), Intent::Volume(Volume::Set(35)));
        assert_eq!(parse("volume 150"), Intent::Volume(Volume::Set(100)));
        assert_eq!(parse("mute"), Intent::Volume(Volume::Mute));
        assert_eq!(parse("unmute"), Intent::Volume(Volume::Unmute));
        assert_eq!(parse("open Notepad"), Intent::Open("notepad".into()));
        assert_eq!(parse("launch the calculator app"), Intent::Open("calculator".into()));
        assert_eq!(parse("take a screenshot"), Intent::Screenshot);
        assert_eq!(parse("lock my pc"), Intent::Lock);
        assert_eq!(parse("help"), Intent::Help);
        assert_eq!(parse("what can you do"), Intent::Help);
        assert_eq!(parse("hello"), Intent::Hello);
        assert_eq!(parse("thanks"), Intent::Thanks);
    }

    #[test]
    fn anything_else_is_unknown_not_guessed() {
        for text in ["write me a poem about cats", "what's the weather like", "remind me in ten minutes to call mum", "", "tell me a joke", "who won the game", "how are you"] {
            assert_eq!(parse(text), Intent::Unknown, "{text}");
        }
    }

    #[tokio::test]
    async fn free_replies_do_real_work_without_an_ai() {
        let dir = std::env::temp_dir().join(format!("jarvis-free-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("JARVIS_DATA_DIR", &dir) };
        let ctx = Ctx::new(jarvis_core::Bus::new()).unwrap();

        let time = respond(&ctx, "what time is it").await;
        assert!(time.starts_with("It's ") && (time.contains("AM") || time.contains("PM")), "{time}");
        assert!(respond(&ctx, "what is the date").await.starts_with("Today is "));

        assert_eq!(respond(&ctx, "set a timer for 2 minutes").await, "Timer set for 2 minutes.");
        assert_eq!(ctx.timers.list().len(), 1);
        assert!(respond(&ctx, "how long is left on the timer").await.contains("1 minute"));
        assert_eq!(respond(&ctx, "cancel the timers").await, "Cancelled 1 timer.");
        assert!(ctx.timers.list().is_empty());

        assert!(respond(&ctx, "help").await.contains("Free mode"));
        let unknown = respond(&ctx, "write me a poem").await;
        assert!(unknown.contains("free mode") && unknown.contains("API keys"), "{unknown}");
        std::fs::remove_dir_all(dir).ok();
    }
}
