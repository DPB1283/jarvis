//! Renderer request/response IPC: the non-window, non-voice channels of `InvokeApi` (shared/events.ts).
//! Same channel names, arguments (positional, as `window.jarvis.invoke(channel, ...args)` sends them) and
//! JSON results as the Electron app's `main/ipc.ts`.

use anyhow::{Context as _, anyhow};
use jarvis_core::Event;
use jarvis_core::model::{ChatAppProfile, MemoryKind, MemoryUpsert, RoutineUpsert};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Ctx;

#[derive(Debug, thiserror::Error)]
pub enum InvokeError {
    /// Not a brain channel (window:*, voice:*, audio:*, voices:*): the app should route it elsewhere.
    #[error("unhandled channel")]
    Unhandled,
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}

fn arg<T: DeserializeOwned>(args: &[Value], i: usize, what: &str) -> anyhow::Result<T> {
    let v = args.get(i).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).with_context(|| format!("bad argument {i} ({what})"))
}

fn opt_arg<T: DeserializeOwned>(args: &[Value], i: usize) -> Option<T> {
    args.get(i).filter(|v| !v.is_null()).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Handle one `jarvis_invoke(channel, args)` call.
pub async fn handle_invoke(ctx: &Ctx, channel: &str, args: Vec<Value>) -> Result<Value, InvokeError> {
    let db = &ctx.db;
    let out = match channel {
        "browser:status" => ctx.browser.status(),
        "settings:get" => serde_json::to_value(ctx.settings.get()).map_err(anyhow::Error::from)?,
        "settings:update" => {
            let patch: Value = arg(&args, 0, "settings patch")?;
            serde_json::to_value(ctx.settings.update(&patch)?).map_err(anyhow::Error::from)?
        }

        "memory:list" => {
            #[derive(serde::Deserialize, Default)]
            struct Filter {
                kind: Option<MemoryKind>,
                query: Option<String>,
            }
            let f: Filter = opt_arg(&args, 0).unwrap_or_default();
            json!(db.list_memory(f.kind, f.query.as_deref())?)
        }
        "memory:upsert" => {
            let mut item: MemoryUpsert = arg(&args, 0, "memory item")?;
            // Reminders from the editor may arrive as local datetime strings; normalise to ISO (UTC).
            if let Some(Some(due)) = item.due_at.clone() {
                item.due_at = Some(Some(normalise_datetime(&due)?));
            }
            json!(db.upsert_memory(&item)?)
        }
        "memory:delete" => json!(db.delete_memory(arg(&args, 0, "id")?)?),
        "memory:clear" => {
            let kinds: Vec<MemoryKind> = opt_arg(&args, 0).unwrap_or_default();
            json!(db.clear_memory(&kinds)?)
        }
        "memory:open-image" => {
            let id: i64 = arg(&args, 0, "id")?;
            let path = db.get_memory(id)?.and_then(|m| m.image_path);
            match path {
                Some(p) if std::path::Path::new(&p).exists() => json!(open_path(&p).is_ok()),
                _ => json!(false),
            }
        }

        "routines:list" => json!(db.list_routines()?),
        "routines:upsert" => {
            let r: RoutineUpsert = arg(&args, 0, "routine")?;
            let saved = db.upsert_routine(&r)?;
            ctx.bus.emit(Event::RoutinesChanged {});
            json!(saved)
        }
        "routines:delete" => {
            let ok = db.delete_routine(arg(&args, 0, "id")?)?;
            if ok {
                ctx.bus.emit(Event::RoutinesChanged {});
            }
            json!(ok)
        }
        "routines:run" => {
            let routine = db.get_routine(arg(&args,0,"routine id")?)?.ok_or_else(|| anyhow!("Routine no longer exists"))?;
            ctx.bus.emit(Event::UserText{text:format!("Run my {} routine: {}",routine.name,routine.instructions)});
            Value::Null
        }

        // Updates
        "update:status" => crate::update::status(),
        "update:check" => crate::update::check().await?,
        "update:install" => {
            crate::update::install().await?;
            Value::Null
        }

        // The "morning briefing" routine: created once, and updated if it already exists.
        "routines:add-briefing" => {
            let time: String = arg(&args, 0, "time")?;
            let days: Vec<u8> = arg(&args, 1, "days")?;
            let existing = db.list_routines()?.into_iter().find(|r| r.name == "Morning briefing").map(|r| r.id);
            let routine = RoutineUpsert {
                id: existing,
                name: "Morning briefing".into(),
                triggers: vec!["morning briefing".into(), "brief me".into(), "how does my day look".into()],
                instructions: "Give me my morning briefing, short and spoken: the weather for my home city today, my calendar for today (and how long it takes to get to my first event if it has a place), and the top few headlines. If something is not set up, skip it without fuss.".into(),
                schedule: Some(jarvis_core::model::Schedule { time, days }),
                enabled: true,
            };
            let saved = db.upsert_routine(&routine)?;
            ctx.bus.emit(Event::RoutinesChanged {});
            json!(saved)
        }

        "chatapps:list" => json!({ "builtin": crate_builtin_chat_apps(), "custom": db.list_custom_chat_apps()? }),
        "chatapps:upsert" => {
            let p: ChatAppProfile = arg(&args, 0, "chat app profile")?;
            json!(db.upsert_custom_chat_app(&p, validate_key_combo)?)
        }
        "chatapps:delete" => json!(db.delete_custom_chat_app(&arg::<String>(&args, 0, "id")?)?),

        // API keys: the renderer can say a key is set, never read it back.
        "secrets:status" => json!(jarvis_core::secrets::status()),
        // The private home address (encrypted). The renderer may read it back so the user can check it, but it
        // is never part of settings or of what the model sees.
        "address:get" => json!(jarvis_core::secrets::get("HOME_ADDRESS")),
        "address:set" => {
            jarvis_core::secrets::set("HOME_ADDRESS", &arg::<String>(&args, 0, "address")?)?;
            Value::Null
        }
        "address:clear" => {
            jarvis_core::secrets::clear("HOME_ADDRESS")?;
            Value::Null
        }
        "secrets:set" => {
            jarvis_core::secrets::set(&arg::<String>(&args, 0, "key name")?, &arg::<String>(&args, 1, "key value")?)?;
            Value::Null
        }
        "secrets:clear" => {
            jarvis_core::secrets::clear(&arg::<String>(&args, 0, "key name")?)?;
            Value::Null
        }

        "timers:list" => json!(ctx.timers.list()),
        "timers:cancel" => json!(ctx.timers.cancel(&arg::<String>(&args, 0, "timer id")?,false)? > 0),

        _ => return Err(InvokeError::Unhandled),
    };
    Ok(out)
}

/// Parses ISO 8601 with or without a zone ("2026-11-21T09:00" = local time) into local time.
pub fn parse_local_datetime(s: &str) -> anyhow::Result<chrono::DateTime<chrono::Local>> {
    use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Local));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            if let Some(local) = Local.from_local_datetime(&naive).earliest() {
                return Ok(local);
            }
        }
    }
    Err(anyhow!("Couldn't understand the date/time \"{s}\"."))
}

/// Accepts ISO with or without zone; returns UTC ISO like JS toISOString().
fn normalise_datetime(s: &str) -> anyhow::Result<String> {
    Ok(parse_local_datetime(s)?.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// Key combos like "ctrl+k", "shift+Escape", "Return", "F5". Full key table lives with the input code (M4).
pub fn validate_key_combo(combo: &str) -> anyhow::Result<()> {
    const NAMED: &[&str] = &[
        "ctrl", "control", "shift", "alt", "super", "win", "windows", "meta", "cmd", "return", "enter", "tab", "escape", "esc", "space",
        "backspace", "delete", "del", "insert", "home", "end", "page_up", "pageup", "page_down", "pagedown", "up", "down", "left", "right",
        "minus", "equal", "plus", "comma", "period", "slash", "backslash", "semicolon", "apostrophe", "grave", "bracketleft", "bracketright",
    ];
    let parts: Vec<&str> = combo.split('+').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(anyhow!("empty"));
    }
    for p in parts {
        let l = p.trim().to_lowercase();
        let ok = NAMED.contains(&l.as_str())
            || (l.len() == 1 && l.chars().all(|c| c.is_ascii_alphanumeric() || "-=,./\\;'`[]".contains(c)))
            || (l.starts_with('f') && l[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
        if !ok {
            return Err(anyhow!("unknown key {p}"));
        }
    }
    Ok(())
}

pub(crate) fn crate_builtin_chat_apps() -> Vec<ChatAppProfile> {
    serde_json::from_str(include_str!("../../../../jarvis/src/shared/chat-apps.json"))
        .expect("built-in chat profiles must match ChatAppProfile")
}

fn open_path(path: &str) -> anyhow::Result<()> {
    std::process::Command::new("explorer.exe").arg(jarvis_core::paths::native(path)).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_combos() {
        for ok in ["ctrl+k", "shift+Escape", "Return", "F5", "ctrl+shift+o", "alt+F10", "super+alt+g"] {
            assert!(validate_key_combo(ok).is_ok(), "{ok}");
        }
        for bad in ["", "ctrl+banana", "F99"] {
            assert!(validate_key_combo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn datetimes() {
        assert!(normalise_datetime("2026-11-21T09:00").unwrap().ends_with('Z'));
        assert_eq!(normalise_datetime("2026-11-21T17:00:00.000Z").unwrap(), "2026-11-21T17:00:00.000Z");
        assert!(normalise_datetime("tomorrow").is_err());
    }
}




