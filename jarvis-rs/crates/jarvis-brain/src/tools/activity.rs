use super::{ToolDef, ps::ps, s, safe};
use crate::Ctx;
use anyhow::Result;
use chrono::{Local, TimeZone, Timelike};
use jarvis_core::{Bus, Event, events::ToastLevel};
use jarvis_desktop::{input, window};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

fn usage_db() -> Result<Connection> {
    let conn = Connection::open(jarvis_core::paths::data_dir().join("usage.sqlite"))?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS usage(ts INTEGER NOT NULL,app TEXT NOT NULL);CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts)")?;
    Ok(conn)
}
pub async fn usage() {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        let record = || -> Result<()> {
            if window::idle_seconds() > 300. {
                return Ok(());
            }
            let exe = window::foreground().exe;
            let name = exe.trim_end_matches(".exe");
            if name.is_empty()
                || matches!(
                    name.to_ascii_lowercase().as_str(),
                    "electron"
                        | "jarvis-app"
                        | "explorer"
                        | "searchhost"
                        | "shellexperiencehost"
                        | "lockapp"
                        | "applicationframehost"
                        | "startmenuexperiencehost"
                )
            {
                return Ok(());
            }
            let now = Local::now().timestamp_millis();
            let db = usage_db()?;
            db.execute("DELETE FROM usage WHERE ts < ?", [now - 60 * 86_400_000])?;
            db.execute(
                "INSERT INTO usage(ts,app) VALUES(?,?)",
                rusqlite::params![now, name],
            )?;
            Ok(())
        };
        if let Err(err) = record() {
            tracing::warn!("Usage sample unavailable: {err}");
        }
    }
}
fn report(days: i64) -> Result<String> {
    let db = usage_db()?;
    let rows = db
        .prepare("SELECT ts,app FROM usage WHERE ts>=? ORDER BY ts")?
        .query_map([Local::now().timestamp_millis() - days * 86_400_000], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() < 30 {
        return Ok(format!(
            "{} minutes recorded. Patterns need more active use.",
            rows.len()
        ));
    }
    let mut apps: HashMap<&str, (usize, [usize; 24], usize)> = HashMap::new();
    let mut starts = HashMap::new();
    for (i, (ts, app)) in rows.iter().enumerate() {
        let Some(time) = Local.timestamp_millis_opt(*ts).single() else {
            continue;
        };
        let (total, hours, today) = apps.entry(app).or_default();
        *total += 1;
        hours[time.hour() as usize] += 1;
        if time.date_naive() == Local::now().date_naive() {
            *today += 1;
        }
        if i == 0 || *ts - rows[i - 1].0 >= 7_200_000 {
            let mut seq = vec![];
            for (_, name) in rows[i..]
                .iter()
                .take_while(|(next, _)| next - ts < 1_800_000)
            {
                if !seq.contains(&name.as_str()) {
                    seq.push(name.as_str());
                }
                if seq.len() == 3 {
                    break;
                }
            }
            *starts
                .entry(format!("{} around {}:00", seq.join(" → "), time.hour()))
                .or_insert(0) += 1;
        }
    }
    let mut top = apps.into_iter().collect::<Vec<_>>();
    top.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    let mut result = vec![format!("Active minutes over {days} days: {}", rows.len())];
    for (name, (total, hours, today)) in top.into_iter().take(10) {
        let max = *hours.iter().max().unwrap();
        let peak = hours
            .iter()
            .enumerate()
            .filter(|(_, n)| **n * 10 >= max * 6)
            .map(|(h, _)| format!("{h}:00"))
            .collect::<Vec<_>>()
            .join(", ");
        result.push(format!(
            "{name}: {total} minutes, {today} today, usually {peak}"
        ));
    }
    let mut seq = starts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .collect::<Vec<_>>();
    seq.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (label, n) in seq.into_iter().take(4) {
        result.push(format!("Repeated session start: {label} ({n} times)"));
    }
    Ok(result.join("\n"))
}
async fn health() -> Result<Value> {
    Ok(serde_json::from_str(
        &ps(include_str!("health.ps1")).await?,
    )?)
}
fn issues(v: &Value) -> Vec<(String, String, u8)> {
    let mut issues = vec![];
    let ram = v["ramUsedPct"].as_f64().unwrap_or_default();
    if ram >= 92. {
        issues.push(("ram".into(), format!("Memory is {ram}% full."), 2));
    }
    let gpu = v["gpu"]["tempC"].as_f64().unwrap_or_default();
    if gpu >= 85. {
        issues.push((
            "gpu".into(),
            format!("Your graphics card is at {gpu} degrees Celsius."),
            2,
        ));
    }
    for disk in v["disks"].as_array().into_iter().flatten() {
        if let (Some(free), Some(size), Some(name)) = (
            disk["freeGB"].as_f64(),
            disk["sizeGB"].as_f64(),
            disk["drive"].as_str(),
        ) {
            if free < 10. || free < size * 0.05 {
                issues.push((
                    format!("disk-{name}"),
                    format!("Drive {name} has {free} gigabytes left."),
                    1,
                ));
            }
        }
    }
    for app in v["topApps"].as_array().into_iter().flatten() {
        let mem = app["memGB"].as_f64().unwrap_or_default();
        let name = app["name"].as_str().unwrap_or_default();
        if mem >= 6.
            && !matches!(
                name.to_ascii_lowercase().as_str(),
                "memory compression" | "system" | "vmmem"
            )
        {
            issues.push((
                format!("hog-{name}"),
                format!("{name} is using {mem} gigabytes of memory."),
                3,
            ));
            break;
        }
    }
    issues
}
fn warn(ctx: &Ctx, text: String) {
    ctx.bus.emit(Event::Toast {
        level: ToastLevel::Warn,
        text: text.clone(),
    });
    ctx.bus.emit(Event::Speak { text });
    ctx.bus.emit(Event::SpeakEnd {});
}
pub async fn watch(ctx: Ctx) {
    let mut inbox = ctx.bus.subscribe();
    let mut game = false;
    let mut held = Vec::new();
    let mut strikes = HashMap::<String, u8>::new();
    let mut last = HashMap::<String, Instant>::new();
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(20),
        Duration::from_secs(60),
    );
    loop {
        tokio::select! {
            Some(event)=Bus::recv(&mut inbox)=>{if let Event::GameMode{active,..}=event{game=active;if !active&&!held.is_empty(){warn(&ctx,format!("While you were playing: {}",held.join(" ")));held.clear();}}}
            _=interval.tick()=>{if !ctx.settings.get().health.warnings{continue;}
                let Ok(snapshot)=health().await else{continue;};
                let mut active=HashSet::new();
                for (key,text,n) in issues(&snapshot){active.insert(key.clone());let count=strikes.entry(key.clone()).or_default();*count=count.saturating_add(1);
                    if *count>=n && last.get(&key).is_none_or(|t|t.elapsed()>=Duration::from_secs(1800)){last.insert(key,Instant::now());if game{if !held.contains(&text){held.push(text);}}else{warn(&ctx,text);}}
                }strikes.retain(|k,_|active.contains(k));
            }
        }
    }
}
pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "usage_patterns",
            description: "Report app usage, peak hours and repeated session starts from local minute samples.",
            schema: || json!({"type":"object","properties":{"days":{"type":"integer","minimum":1,"maximum":60}}}),
            risk: safe,
            summary: |_| "check usage patterns".into(),
            run: |_, v| {
                Box::pin(async move { Ok(report(v["days"].as_i64().unwrap_or(14))?.into()) })
            },
        },
        ToolDef {
            name: "pc_health",
            description: "Read CPU, memory, GPU, drives and largest processes.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check PC health".into(),
            run: |_, _| Box::pin(async { Ok(health().await?.to_string().into()) }),
        },
        ToolDef {
            name: "clip_that",
            description: "Save a replay via the configured game recorder shortcut.",
            schema: || json!({"type":"object","properties":{"app":{"type":"string","enum":["auto","medal","nvidia","xbox"]}}}),
            risk: safe,
            summary: |_| "save a game clip".into(),
            run: |ctx, v| {
                Box::pin(async move {
                    let cfg = ctx.settings.get().clips;
                    let setting = serde_json::to_value(cfg.app)?;
                    let requested = s(&v, "app").unwrap_or("auto");
                    let mut app = if requested == "auto" {
                        setting.as_str().unwrap_or("auto")
                    } else {
                        requested
                    };
                    if app == "auto" {
                        let running=ps("Get-Process -Name Medal,'NVIDIA Overlay' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty ProcessName -Unique;exit 0").await?.to_lowercase();
                        app = if running.contains("medal") {
                            "medal"
                        } else if running.contains("nvidia") {
                            "nvidia"
                        } else {
                            "xbox"
                        };
                    }
                    let combo = if !cfg.hotkey.trim().is_empty()
                        && (requested == "auto" || setting == requested)
                    {
                        cfg.hotkey.trim()
                    } else {
                        match app {
                            "medal" => "F8",
                            "nvidia" => "alt+F10",
                            _ => "super+alt+g",
                        }
                    };
                    input::chord(combo)?;
                    Ok(format!("Pressed the {app} recording shortcut ({combo}).").into())
                })
            },
        },
    ]
}

