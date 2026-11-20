//! Memory, reminders, settings, confirmation and routines (port of `tools/memory.ts` + `tools/routines.ts`).

use chrono::{DateTime, Duration as CDuration, Local, Utc};
use jarvis_core::Event;
use jarvis_core::events::Risk;
use jarvis_core::model::{MemoryItem, MemoryKind, MemoryUpsert, RoutineUpsert, Schedule};
use jarvis_core::settings::SafetyMode;
use serde_json::{Value, json};

use super::{ToolDef, b, n, s, safe};
use crate::ipc::parse_local_datetime;

fn fmt_items(items: &[MemoryItem]) -> String {
    if items.is_empty() {
        return "Nothing stored.".into();
    }
    items
        .iter()
        .map(|m| {
            let mut tag = m.kind.as_str().to_string();
            if m.kind == MemoryKind::Todo {
                tag.push_str(if m.done { ", done" } else { ", open" });
            }
            if let Some(due) = m.due_at.as_deref().and_then(|d| DateTime::parse_from_rfc3339(d).ok()) {
                tag.push_str(&format!(", due {}", due.with_timezone(&Local).format("%a %-d %b %H:%M")));
            }
            let shot = m.image_path.as_deref().map(|p| format!(" [screenshot: {p}]")).unwrap_or_default();
            format!("#{} [{tag}] {}{shot}", m.id, m.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn kind_of(v: &Value) -> Option<MemoryKind> {
    s(v, "kind").and_then(MemoryKind::parse)
}

fn when(d: DateTime<Local>) -> String {
    d.format("%a %-d %b, %H:%M").to_string()
}
fn clock(d: DateTime<Local>) -> String {
    d.format("%-I:%M %p").to_string()
}
fn iso(d: DateTime<Local>) -> String {
    d.with_timezone(&Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn describe_schedule(sch: Option<&Schedule>) -> String {
    let Some(sc) = sch else { return "on request only".into() };
    const DAY: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let d: Vec<u8> = sc.days.clone();
    let days = match d.as_slice() {
        [1, 2, 3, 4, 5] => "weekdays".to_string(),
        [0, 6] => "weekends".to_string(),
        [0, 1, 2, 3, 4, 5, 6] => "every day".to_string(),
        _ => d.iter().map(|x| DAY[*x as usize % 7]).collect::<Vec<_>>().join("/"),
    };
    format!("{days} at {}", sc.time)
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "remember",
            description: "Save something to memory. Whenever the user says 'remember ...', call this right away with their words, tidied up (kind=note for things to jot down, fact for lasting facts about them). For 'remember this/that' about what's on screen, call remember_screen first and pass its image_path here.",
            schema: || json!({"type":"object","properties":{"text":{"type":"string"},"kind":{"type":"string","enum":["fact","note"]},"image_path":{"type":"string","description":"Screenshot from remember_screen"}},"required":["text"]}),
            risk: safe,
            summary: |i| format!("remember \"{}\"", s(i, "text").unwrap_or("")),
            run: |ctx, i| {
                Box::pin(async move {
                    let image = s(&i, "image_path").map(String::from);
                    let kind = kind_of(&i).unwrap_or(if image.is_some() { MemoryKind::Note } else { MemoryKind::Fact });
                    let m = ctx.db.upsert_memory(&MemoryUpsert { kind: Some(kind), text: s(&i, "text").unwrap_or("").into(), image_path: Some(image), ..Default::default() })?;
                    Ok(format!("Saved #{}", m.id).into())
                })
            },
        },
        ToolDef {
            name: "recall",
            description: "Search saved facts, notes, todos and reminders. Omit query to list everything of a kind.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string","enum":["fact","note","todo","reminder"]}}}),
            risk: safe,
            summary: |_| "check memory".into(),
            run: |ctx, i| Box::pin(async move { Ok(fmt_items(&ctx.db.list_memory(kind_of(&i), s(&i, "query"))?).into()) }),
        },
        ToolDef {
            name: "edit_memory",
            description: "Change the text of a saved item, mark a todo done/undone, or move a reminder's time. Use the #id from recall.",
            schema: || json!({"type":"object","properties":{"id":{"type":"integer"},"text":{"type":"string"},"done":{"type":"boolean"},"due_at":{"type":"string","description":"New ISO 8601 local time for a reminder"}},"required":["id"]}),
            risk: safe,
            summary: |i| format!("update memory #{}", n(i, "id").unwrap_or(0.0)),
            run: |ctx, i| {
                Box::pin(async move {
                    let id = n(&i, "id").unwrap_or(0.0) as i64;
                    let Some(cur) = ctx.db.get_memory(id)? else { return Ok(format!("No item #{id}.").into()) };
                    let due = match s(&i, "due_at") {
                        Some(d) => Some(Some(iso(parse_local_datetime(d)?))),
                        None => None,
                    };
                    let m = ctx.db.upsert_memory(&MemoryUpsert {
                        id: Some(id),
                        kind: Some(cur.kind),
                        text: s(&i, "text").map(String::from).unwrap_or(cur.text),
                        done: b(&i, "done"),
                        due_at: due,
                        image_path: None,
                    })?;
                    Ok(format!("Updated: {}", fmt_items(&[m])).into())
                })
            },
        },
        ToolDef {
            name: "forget",
            description: "Delete saved items by id, or wipe a whole kind with all=true (requires the user to have explicitly asked).",
            schema: || json!({"type":"object","properties":{"ids":{"type":"array","items":{"type":"integer"}},"all":{"type":"boolean"},"kind":{"type":"string","enum":["fact","note","todo","reminder"]}}}),
            risk: |_, i| if b(i, "all") == Some(true) { Risk::Unsafe } else { Risk::Safe },
            summary: |i| if b(i, "all") == Some(true) { format!("erase ALL {}", s(i, "kind").unwrap_or("memory")) } else { format!("forget {}", i["ids"]) },
            run: |ctx, i| {
                Box::pin(async move {
                    if b(&i, "all") == Some(true) {
                        let kinds: Vec<MemoryKind> = kind_of(&i).into_iter().collect();
                        return Ok(format!("Erased {} items.", ctx.db.clear_memory(&kinds)?).into());
                    }
                    let mut count = 0;
                    for id in i["ids"].as_array().into_iter().flatten().filter_map(|v| v.as_i64()) {
                        if ctx.db.delete_memory(id)? {
                            count += 1;
                        }
                    }
                    Ok(format!("Forgot {count} item(s).").into())
                })
            },
        },
        ToolDef {
            name: "add_todo",
            description: "Add an item to the user's to-do list.",
            schema: || json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
            risk: safe,
            summary: |i| format!("add to-do \"{}\"", s(i, "text").unwrap_or("")),
            run: |ctx, i| {
                Box::pin(async move {
                    let m = ctx.db.upsert_memory(&MemoryUpsert { kind: Some(MemoryKind::Todo), text: s(&i, "text").unwrap_or("").into(), ..Default::default() })?;
                    Ok(format!("Added to-do #{}", m.id).into())
                })
            },
        },
        ToolDef {
            name: "set_reminder",
            description: "Set a reminder. Two kinds:\n- A task at a time: 'remind me in 20 minutes to stretch' / 'remind me at 9 to call mum': give in_minutes or at.\n- An EVENT with a time ('remind me I have dinner at 7', 'I've got a meeting at 3'): give event_at; Jarvis reminds them remind_before_minutes ahead (default 30, or what they ask for), and also_at_event_time if they want a second nudge when it starts.\nTimes are ISO 8601 local date-time (e.g. 2026-11-21T19:00). Today's date and time are in the context block; if a bare time like '7' is ambiguous, pick the next sensible one (7 pm for dinner).",
            schema: || json!({"type":"object","properties":{
                "text":{"type":"string","description":"What it's about, e.g. 'Dinner' or 'Call mum'"},
                "in_minutes":{"type":"number","minimum":0},
                "at":{"type":"string","description":"When to remind (task reminders)"},
                "event_at":{"type":"string","description":"When the event happens (event reminders)"},
                "remind_before_minutes":{"type":"number","minimum":0,"maximum":10080,"description":"Lead time for event reminders, default 30"},
                "also_at_event_time":{"type":"boolean"}},"required":["text"]}),
            risk: safe,
            summary: |i| format!("reminder: {}", s(i, "text").unwrap_or("")),
            run: |ctx, i| {
                Box::pin(async move {
                    let text = s(&i, "text").unwrap_or("").to_string();
                    let now = Local::now();
                    let save = |txt: String, at: DateTime<Local>| {
                        ctx.db.upsert_memory(&MemoryUpsert { kind: Some(MemoryKind::Reminder), text: txt, due_at: Some(Some(iso(at))), ..Default::default() })
                    };
                    if let Some(ev) = s(&i, "event_at") {
                        let event = parse_local_datetime(ev)?;
                        let lead = n(&i, "remind_before_minutes").unwrap_or(30.0);
                        let ahead = event - CDuration::seconds((lead * 60.0) as i64);
                        let lead_text = if lead >= 60.0 && lead % 60.0 == 0.0 {
                            let h = lead / 60.0;
                            format!("{h} hour{}", if h == 1.0 { "" } else { "s" })
                        } else {
                            format!("{lead} minutes")
                        };
                        let mut saved = Vec::new();
                        if lead > 0.0 && ahead > now {
                            let m = save(format!("{text} at {}, in {lead_text}", clock(event)), ahead)?;
                            saved.push(format!("#{} at {} ({lead_text} before)", m.id, when(ahead)));
                        }
                        if b(&i, "also_at_event_time") == Some(true) || lead == 0.0 || ahead <= now {
                            if event <= now {
                                return Ok(format!("{} has already passed.", when(event)).into());
                            }
                            let m = save(format!("{text}, starting now ({})", clock(event)), event)?;
                            saved.push(format!("#{} at {} (when it starts)", m.id, when(event)));
                        }
                        return Ok(format!("Event \"{text}\" at {}. Reminders: {}.", when(event), saved.join("; ")).into());
                    }
                    let due = match (n(&i, "in_minutes"), s(&i, "at")) {
                        (Some(m), _) => now + CDuration::seconds((m * 60.0) as i64),
                        (_, Some(a)) => parse_local_datetime(a)?,
                        _ => return Ok("I need a valid time: in_minutes, at, or event_at.".into()),
                    };
                    let m = save(text, due)?;
                    Ok(format!("Reminder #{} set for {}.", m.id, when(due)).into())
                })
            },
        },
        ToolDef {
            name: "change_settings",
            description: "Change Jarvis's own settings when the user asks: how to address them (user_title, e.g. 'sir'), their name (user_name, e.g. when they say 'my name is ...' or 'call me ...' with a name), home city / units / news region, how long to keep listening after replies, captions on/off, mute your voice, sarcasm 0-10, reply length, safety mode, wake word on/off.",
            schema: || json!({"type":"object","properties":{
                "user_title":{"type":"string"},"user_name":{"type":"string"},
                "home_city":{"type":"string"},"units":{"type":"string","enum":["metric","imperial"]},
                "news_region":{"type":"string","minLength":2},"news_sources":{"type":"string","enum":["neutral","all"]},
                "follow_up_seconds":{"type":"integer","minimum":0,"maximum":300},"end_of_speech_seconds":{"type":"number","minimum":0.4,"maximum":3},
                "captions":{"type":"boolean"},"voice_muted":{"type":"boolean"},"sarcasm":{"type":"integer","minimum":0,"maximum":10},
                "reply_length":{"type":"string","enum":["brief","normal"]},"safety_mode":{"type":"string","enum":["ask_all","ask_unsafe","full_access"]},
                "wake_word":{"type":"boolean"}}}),
            // Loosening safety always needs the user's explicit OK (unless already full access).
            risk: |ctx, i| {
                let order = |m: &str| match m { "ask_all" => 0, "ask_unsafe" => 1, _ => 2 };
                let cur = match ctx.settings.get().safety_mode { SafetyMode::AskAll => 0, SafetyMode::AskUnsafe => 1, SafetyMode::FullAccess => 2 };
                if s(i, "safety_mode").is_some_and(|m| order(m) > cur) { Risk::Unsafe } else { Risk::Safe }
            },
            summary: |i| format!("change settings: {}", i.as_object().map(|o| o.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
            run: |ctx, i| {
                Box::pin(async move {
                    let mut patch = serde_json::Map::new();
                    let mut put = |section: &str, key: &str, v: Value| {
                        let e = patch.entry(section.to_string()).or_insert_with(|| json!({}));
                        e[key] = v;
                    };
                    if let Some(v) = s(&i, "user_title") { patch_top(&mut put, "userTitle", json!(v)); }
                    if let Some(v) = i.get("user_name").and_then(|v| v.as_str()) { patch_top(&mut put, "userName", json!(v)); }
                    if let Some(v) = s(&i, "home_city") { put("location", "city", json!(v)); }
                    if let Some(v) = s(&i, "units") { put("location", "units", json!(v)); }
                    if let Some(v) = s(&i, "news_region") { put("location", "newsRegion", json!(v.to_uppercase())); }
                    if let Some(v) = s(&i, "news_sources") { put("location", "newsSources", json!(v)); }
                    if let Some(v) = n(&i, "follow_up_seconds") { put("conversation", "followUpSeconds", json!(v)); }
                    if let Some(v) = n(&i, "end_of_speech_seconds") { put("conversation", "endOfSpeechSeconds", json!(v)); }
                    if let Some(v) = b(&i, "captions") { put("captions", "enabled", json!(v)); }
                    if let Some(v) = b(&i, "voice_muted") { put("voice", "muted", json!(v)); }
                    if let Some(v) = n(&i, "sarcasm") { put("personality", "sarcasm", json!(v)); }
                    if let Some(v) = s(&i, "reply_length") { put("personality", "replyLength", json!(v)); }
                    if let Some(v) = s(&i, "safety_mode") { patch_top(&mut put, "safetyMode", json!(v)); }
                    if let Some(v) = b(&i, "wake_word") { put("wakeWord", "enabled", json!(v)); }
                    // Top-level keys were stashed under a "__top" section; lift them out.
                    let mut patch = Value::Object(patch);
                    if let Some(top) = patch.as_object_mut().and_then(|o| o.remove("__top")) {
                        for (k, v) in top.as_object().cloned().unwrap_or_default() {
                            patch[k] = v;
                        }
                    }
                    ctx.settings.update(&patch)?;
                    Ok("Settings updated.".into())
                })
            },
        },
        ToolDef {
            name: "confirm_with_user",
            description: "Ask the user for a yes/no before a sensitive step you are about to take on screen: sending a message or email, submitting a form, purchasing or paying, entering a password, posting publicly, or changing account/security settings. Returns whether they approved.",
            schema: || json!({"type":"object","properties":{"action":{"type":"string","description":"What you are about to do, in plain words"}},"required":["action"]}),
            risk: |_, _| Risk::Unsafe,
            summary: |i| s(i, "action").unwrap_or("continue").to_string(),
            // Reaching run() means the policy layer already approved (or safety mode is full access).
            run: |_ctx, _i| Box::pin(async move { Ok("Approved. Go ahead.".into()) }),
        },
        ToolDef {
            name: "list_routines",
            description: "List the user's routines (with ids, triggers, instructions and schedules).",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check routines".into(),
            run: |ctx, _i| {
                Box::pin(async move {
                    let all = ctx.db.list_routines()?;
                    if all.is_empty() {
                        return Ok("No routines yet.".into());
                    }
                    Ok(all
                        .iter()
                        .map(|r| format!("#{} {}{}: {}. Triggers: {}. Does: {}", r.id, r.name, if r.enabled { "" } else { " (disabled)" }, describe_schedule(r.schedule.as_ref()), if r.triggers.is_empty() { "-".into() } else { r.triggers.join(" / ") }, r.instructions))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into())
                })
            },
        },
        ToolDef {
            name: "save_routine",
            description: "Create a routine, or update one (pass its id). A routine is something to do when the user asks for it by meaning (trigger phrases are examples), and/or automatically at a time. Write instructions as clear steps you can follow later. Days: 0=Sunday ... 6=Saturday. Omit schedule for on-request only; pass schedule: null to remove one.",
            schema: || json!({"type":"object","properties":{
                "id":{"type":"integer"},"name":{"type":"string"},"triggers":{"type":"array","items":{"type":"string"}},"instructions":{"type":"string"},
                "schedule":{"type":"object","properties":{"time":{"type":"string","description":"24-hour HH:MM"},"days":{"type":"array","items":{"type":"integer"}}}},
                "enabled":{"type":"boolean"}}}),
            risk: safe,
            summary: |i| format!("{} routine {}", if i.get("id").is_some() { "update" } else { "create" }, s(i, "name").unwrap_or("")),
            run: |ctx, i| {
                Box::pin(async move {
                    let cur = match i.get("id").and_then(|v| v.as_i64()) {
                        Some(id) => match ctx.db.get_routine(id)? {
                            Some(r) => Some(r),
                            None => return Ok(format!("No routine #{id}.").into()),
                        },
                        None => None,
                    };
                    let schedule = match i.get("schedule") {
                        None => cur.as_ref().and_then(|r| r.schedule.clone()),
                        Some(Value::Null) => None,
                        Some(v) => Some(serde_json::from_value::<Schedule>(v.clone())?),
                    };
                    let saved = ctx.db.upsert_routine(&RoutineUpsert {
                        id: cur.as_ref().map(|r| r.id),
                        name: s(&i, "name").map(String::from).or_else(|| cur.as_ref().map(|r| r.name.clone())).unwrap_or_default(),
                        triggers: i.get("triggers").and_then(|t| serde_json::from_value(t.clone()).ok()).or_else(|| cur.as_ref().map(|r| r.triggers.clone())).unwrap_or_default(),
                        instructions: s(&i, "instructions").map(String::from).or_else(|| cur.as_ref().map(|r| r.instructions.clone())).unwrap_or_default(),
                        schedule,
                        enabled: b(&i, "enabled").or(cur.as_ref().map(|r| r.enabled)).unwrap_or(true),
                    })?;
                    ctx.bus.emit(Event::RoutinesChanged {});
                    Ok(format!("Saved routine #{} \"{}\" ({}{}).", saved.id, saved.name, describe_schedule(saved.schedule.as_ref()), if saved.enabled { "" } else { ", disabled" }).into())
                })
            },
        },
        ToolDef {
            name: "delete_routine",
            description: "Delete a routine by id.",
            schema: || json!({"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}),
            risk: safe,
            summary: |i| format!("delete routine #{}", n(i, "id").unwrap_or(0.0)),
            run: |ctx, i| {
                Box::pin(async move {
                    let id = n(&i, "id").unwrap_or(0.0) as i64;
                    let ok = ctx.db.delete_routine(id)?;
                    if ok {
                        ctx.bus.emit(Event::RoutinesChanged {});
                    }
                    Ok(if ok { format!("Deleted routine #{id}.") } else { format!("No routine #{id}.") }.into())
                })
            },
        },
    ]
}

/// Settings fields that live at the top level of the patch (not inside a section).
fn patch_top(put: &mut impl FnMut(&str, &str, Value), key: &str, v: Value) {
    put("__top", key, v);
}
