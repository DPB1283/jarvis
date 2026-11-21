//! Optional local histories, both OFF by default (Settings → History) and kept only on this PC:
//!  - clipboard: text you copy, so "what was that link I copied?" works;
//!  - screens: a small snapshot of your screen every few minutes, so "what was I looking at at 3pm?" works.
//! Turning one off deletes what was stored. Password managers' copies are skipped on Windows, and anything that
//! looks like a key or token is never recorded.

use crate::Ctx;
use crate::tools::{ToolDef, ToolOutput, n, s, safe};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Local, Utc};
use jarvis_core::paths::data_dir;
use serde_json::json;
use std::time::Duration;

const CLIP_KEEP: usize = 300;
const CLIP_DAYS: i64 = 7;
const SCREEN_KEEP: usize = 96;
const SCREEN_HOURS: i64 = 24;
const SCREEN_EVERY: Duration = Duration::from_secs(300);

/// Text that looks like a secret is never stored: well-known key prefixes, or one long unbroken token.
pub(crate) fn looks_secret(text: &str) -> bool {
    let t = text.trim();
    let prefixes = ["sk-", "sk_", "pk.", "ghp_", "gho_", "github_pat_", "xox", "AKIA", "AIza", "eyJ", "-----BEGIN"];
    if prefixes.iter().any(|p| t.starts_with(p)) {
        return true;
    }
    t.len() >= 28 && !t.contains(char::is_whitespace) && t.chars().any(|c| c.is_ascii_digit()) && t.chars().any(|c| c.is_ascii_alphabetic()) && !t.contains("://")
}

fn screens_dir() -> std::path::PathBuf {
    let dir = data_dir().join("screens");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn local_time(ts: &str) -> String {
    DateTime::parse_from_rfc3339(ts).map(|t| t.with_timezone(&Local).format("%a %-d %b, %-I:%M %p").to_string()).unwrap_or_else(|_| ts.to_owned())
}

fn delete_files(paths: Vec<String>) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

async fn record_clipboard(ctx: &Ctx, last_sequence: &mut u32) {
    let sequence = tokio::task::spawn_blocking(jarvis_desktop::clipboard::sequence).await.unwrap_or(0);
    if sequence != 0 && sequence == *last_sequence {
        return;
    }
    *last_sequence = sequence;
    let text = tokio::task::spawn_blocking(|| (!jarvis_desktop::clipboard::is_sensitive()).then(jarvis_desktop::clipboard::get_text).flatten()).await.ok().flatten();
    let Some(text) = text.map(|t| t.trim().to_owned()) else { return };
    if text.is_empty() || text.chars().count() > 2000 || looks_secret(&text) {
        return;
    }
    if ctx.db.add_clip(&text).unwrap_or(false) {
        let cutoff = (Utc::now() - chrono::Duration::days(CLIP_DAYS)).to_rfc3339();
        let _ = ctx.db.prune_clips(CLIP_KEEP, &cutoff);
    }
}

async fn record_screen(ctx: &Ctx) -> Result<()> {
    let foreground = tokio::task::spawn_blocking(jarvis_desktop::window::foreground).await?;
    if foreground.fullscreen {
        return Ok(()); // a game or video in full screen: leave it alone
    }
    let rect = jarvis_desktop::monitors::list()
        .into_iter()
        .map(|m| jarvis_desktop::window::Rect { left: m.x, top: m.y, right: m.x + m.width, bottom: m.y + m.height })
        .reduce(|a, b| jarvis_desktop::window::Rect { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) })
        .context("No screen")?;
    let image = crate::tools::screens::snapshot(rect).await?;
    let data = image["source"]["data"].as_str().context("No image data")?;
    let path = screens_dir().join(format!("{}.jpg", Utc::now().format("%Y%m%d-%H%M%S")));
    std::fs::write(&path, STANDARD.decode(data)?)?;
    ctx.db.add_screen(&path.to_string_lossy(), &foreground.title.chars().take(120).collect::<String>())?;
    let cutoff = (Utc::now() - chrono::Duration::hours(SCREEN_HOURS)).to_rfc3339();
    delete_files(ctx.db.prune_screens(SCREEN_KEEP, &cutoff)?);
    Ok(())
}

/// The background recorder. Does nothing until a history is switched on, and erases it when it is switched off.
pub async fn watch(ctx: Ctx) {
    let mut last_sequence = 0;
    let mut next_screen = tokio::time::Instant::now();
    let (mut had_clips, mut had_screens) = (false, false);
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let history = ctx.settings.get().history;
        if history.clipboard {
            had_clips = true;
            record_clipboard(&ctx, &mut last_sequence).await;
        } else if had_clips {
            had_clips = false;
            let _ = ctx.db.clear_clips();
        }
        if history.screens {
            had_screens = true;
            if tokio::time::Instant::now() >= next_screen {
                next_screen = tokio::time::Instant::now() + SCREEN_EVERY;
                if let Err(error) = record_screen(&ctx).await {
                    tracing::debug!(%error, "Screen history skipped a snapshot");
                }
            }
        } else if had_screens {
            had_screens = false;
            if let Ok(paths) = ctx.db.clear_screens() {
                delete_files(paths);
            }
        }
    }
}

const OFF: &str = "That history is off. The user can turn it on in Settings, History (it stays on this PC).";

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "search_clipboard_history",
            description: "Search the text the user recently copied (needs Settings, History, clipboard to be on). Use for 'what was that link I copied', 'the address I copied earlier'. Returns newest first.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string","description":"Words to look for; omit for the latest copies"},"limit":{"type":"integer","minimum":1,"maximum":30}}}),
            risk: safe,
            summary: |_| "look through what you copied".into(),
            run: |ctx, v| {
                Box::pin(async move {
                    if !ctx.settings.get().history.clipboard {
                        return Ok(OFF.into());
                    }
                    let limit = n(&v, "limit").unwrap_or(10.0).clamp(1.0, 30.0) as usize;
                    let rows = ctx.db.recent_clips(s(&v, "query"), limit)?;
                    Ok(if rows.is_empty() {
                        "Nothing matching was copied recently.".to_owned()
                    } else {
                        rows.iter().map(|(ts, text)| format!("{}: {}", local_time(ts), text.chars().take(300).collect::<String>())).collect::<Vec<_>>().join("\n")
                    }
                    .into())
                })
            },
        },
        ToolDef {
            name: "screen_history",
            description: "Look back at what was on the user's screen (needs Settings, History, screen snapshots to be on; one small snapshot about every 5 minutes for the last day). action list shows the times and window titles with an index; action view shows one snapshot by index so you can read it.",
            schema: || json!({"type":"object","required":["action"],"properties":{"action":{"type":"string","enum":["list","view"]},"index":{"type":"integer","minimum":0,"maximum":95,"description":"For view: the index from list (0 is the newest)"}}}),
            risk: safe,
            summary: |_| "look back at the screen".into(),
            run: |ctx, v| {
                Box::pin(async move {
                    if !ctx.settings.get().history.screens {
                        return Ok(OFF.into());
                    }
                    let rows = ctx.db.recent_screens(SCREEN_KEEP)?;
                    if rows.is_empty() {
                        return Ok("No snapshots yet. They are taken about every five minutes.".into());
                    }
                    if s(&v, "action") == Some("view") {
                        let index = n(&v, "index").unwrap_or(0.0).max(0.0) as usize;
                        let (ts, path, title) = rows.get(index).context("There is no snapshot with that index")?;
                        let bytes = std::fs::read(path).context("That snapshot has been removed")?;
                        return Ok(ToolOutput::Blocks(vec![
                            json!({"type":"text","text":format!("Snapshot from {} (window: {title})", local_time(ts))}),
                            json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":STANDARD.encode(bytes)}}),
                        ]));
                    }
                    Ok(rows.iter().enumerate().map(|(i, (ts, _, title))| format!("{i}: {} · {title}", local_time(ts))).collect::<Vec<_>>().join("\n").into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_tokens_are_never_recorded() {
        for secret in [
            concat!("sk-", "ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            "pk.eyJ1IjoiZXhhbXBsZSJ9.abcdef",
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
            "AKIAIOSFODNN7EXAMPLE",
            "-----BEGIN PRIVATE KEY-----\nabc",
            "a8Kd93jfLq0Zx7TnV2mP4wRs6YbH1cEu",
        ] {
            assert!(looks_secret(secret), "{secret}");
        }
        for fine in ["https://example.com/a/very/long/path/that/is/over/twentyeight/characters", "Meeting at 3pm with Sam about the budget", "12 High Street, Leeds", "hello"] {
            assert!(!looks_secret(fine), "{fine}");
        }
    }

    #[test]
    fn clipboard_history_stores_prunes_and_searches() {
        let dir = std::env::temp_dir().join(format!("jarvis-hist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("JARVIS_DATA_DIR", &dir) };
        let db = jarvis_core::db::Db::open().unwrap();
        assert!(db.add_clip("first link https://a.example").unwrap());
        assert!(!db.add_clip("first link https://a.example").unwrap(), "an immediate repeat is skipped");
        assert!(db.add_clip("second thing").unwrap());
        assert!(db.add_clip("third link https://c.example").unwrap());
        let found = db.recent_clips(Some("link"), 10).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].1.starts_with("third"), "newest first");
        assert_eq!(db.recent_clips(Some("100%"), 10).unwrap().len(), 0, "wildcards in the query are not special");
        db.prune_clips(2, "1970-01-01T00:00:00Z").unwrap();
        assert_eq!(db.recent_clips(None, 10).unwrap().len(), 2);
        db.prune_clips(10, "2999-01-01T00:00:00Z").unwrap();
        assert_eq!(db.recent_clips(None, 10).unwrap().len(), 0, "old entries expire");

        db.add_screen("/tmp/a.jpg", "Editor").unwrap();
        db.add_screen("/tmp/b.jpg", "Browser").unwrap();
        assert_eq!(db.recent_screens(5).unwrap()[0].2, "Browser");
        assert_eq!(db.prune_screens(1, "1970-01-01T00:00:00Z").unwrap(), vec!["/tmp/a.jpg"]);
        assert_eq!(db.clear_screens().unwrap(), vec!["/tmp/b.jpg"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn times_are_shown_in_local_time() {
        let shown = local_time("2026-11-20T15:30:00.000Z");
        assert!(shown.contains("Nov") && (shown.contains("AM") || shown.contains("PM")), "{shown}");
        assert_eq!(local_time("not a time"), "not a time");
    }
}
