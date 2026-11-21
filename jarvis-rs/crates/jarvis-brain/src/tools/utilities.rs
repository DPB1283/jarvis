use super::{NoWindow, ToolDef, ToolOutput, safe};
use crate::Ctx;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

// Keep the existing calendar recurrence parser, Steam library parser and YouTube result parser
// shared with Electron. The worker only accepts the tools explicitly listed in utilities.ts.
async fn run(ctx: Ctx, name: &str, input: Value) -> Result<ToolOutput> {
    let packaged = jarvis_core::paths::packaged_resources()
        .context("Application directory unavailable")?
        .join("utilities");
    let script = if packaged.join("utilities.cjs").is_file() {
        packaged.join("utilities.cjs")
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/utilities.cjs")
    };
    // A Node bundled next to the helpers (Windows installer), else the one on PATH.
    let node_name = if cfg!(windows) { "node.exe" } else { "node" };
    let node = if packaged.join(node_name).is_file() {
        packaged.join(node_name)
    } else {
        PathBuf::from(node_name)
    };
    ensure!(script.is_file(), "Jarvis utilities are missing");
    let calendar = if name == "calendar_events" {
        json!(ctx.settings.get().calendar)
    } else {
        Value::Null
    };
    let payload = serde_json::to_vec(&json!({"name":name,"input":input,"calendar":calendar}))?;
    let mut child = tokio::process::Command::new(node)
        .arg(script)
        .no_window()
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().context("Utility input unavailable")?;
    stdin.write_all(&payload).await?;
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(45), child.wait_with_output()).await??;
    ensure!(output.status.success(), "Utility failed");
    Ok(serde_json::from_slice::<String>(&output.stdout)?.into())
}

macro_rules! utility {
    ($name:literal,$description:literal,$schema:expr) => {
        ToolDef {
            name: $name,
            description: $description,
            risk: safe,
            schema: || $schema,
            summary: |_| $description.into(),
            run: |ctx, input| Box::pin(run(ctx, $name, input)),
        }
    };
}
pub fn tools() -> Vec<ToolDef> {
    vec![
        utility!(
            "write_document",
            "Write Markdown into a file, the active editor, or a new Google Doc.",
            json!({"type":"object","required":["title","content"],"properties":{"title":{"type":"string"},"content":{"type":"string","minLength":1},"destination":{"type":"string","enum":["google_docs","file","here"]},"file_format":{"type":"string","enum":["docx_compatible_html","markdown","text"]}}})
        ),
        utility!(
            "find_anything",
            "Search file names and contents in the Windows index.",
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"},"kind":{"type":"string","enum":["any","document","picture","music","video","email","folder","program"]},"modified_within_days":{"type":"integer","minimum":1,"maximum":3650},"extension":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50}}})
        ),
        utility!(
            "calendar_events",
            "Read upcoming calendar events.",
            json!({"type":"object","properties":{"days":{"type":"integer","minimum":1,"maximum":14}}})
        ),
        utility!(
            "steam_games",
            "List installed Steam games.",
            json!({"type":"object","properties":{"query":{"type":"string"}}})
        ),
        utility!(
            "steam_launch",
            "Launch an installed Steam game by title or app ID.",
            json!({"type":"object","required":["game"],"properties":{"game":{"type":"string"}}})
        ),
        utility!(
            "steam_deals",
            "Check the current featured Steam discounts.",
            json!({"type":"object","properties":{"count":{"type":"integer","minimum":1,"maximum":20}}})
        ),
        utility!(
            "youtube_search",
            "Search YouTube videos, optionally including Shorts and playlists.",
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"},"count":{"type":"integer","minimum":1,"maximum":10},"kind":{"type":"string","enum":["video","any"]}}})
        ),
    ]
}
