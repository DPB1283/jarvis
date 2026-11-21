use super::{ToolDef, ToolOutput, s};
use crate::Ctx;
use anyhow::{Result, ensure};
use jarvis_core::events::Risk;
use serde_json::{Value, json};

async fn run(ctx: Ctx, v: Value) -> Result<ToolOutput> {
    let action = s(&v, "action").unwrap_or_default().to_owned();
    let result = ctx.browser.call(&action, v).await?;
    if action == "screenshot" {
        let data = result["data"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No screenshot returned"))?;
        ensure!(data.len() < 8_000_000, "Screenshot is too large");
        return Ok(ToolOutput::Blocks(vec![
            json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":data}}),
        ]));
    }
    Ok(if result.is_string() {
        result.as_str().unwrap_or_default().to_string()
    } else {
        result.to_string()
    }
    .into())
}
pub fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "browser",
        description: "Use only when asked to work in Jarvis's own browser tabs. Open and navigate web pages, list and inspect owned tabs, interact with elements, take screenshots, close tabs or expand the group. Never accesses the user's other tabs.",
        schema: || {
            json!({"type":"object","required":["action"],"properties":{
                "action":{"type":"string","enum":["open","navigate","list_tabs","read_page","click","type","press_key","scroll","screenshot","close_tab","expand_group"]},
                "tab_id":{"type":"integer"},"url":{"type":"string"},"selector":{"type":"string"},"text":{"type":"string"},
                "key":{"type":"string"},"direction":{"type":"string","enum":["up","down","left","right"]},"amount":{"type":"integer","minimum":1,"maximum":3000}
            }})
        },
        risk: |_, v| {
            if matches!(s(v, "action"), Some("click" | "type" | "press_key")) {
                Risk::Unsafe
            } else {
                Risk::Safe
            }
        },
        summary: |v| format!("{} in Jarvis's browser", s(v, "action").unwrap_or("work")),
        run: |ctx, v| Box::pin(run(ctx, v)),
    }]
}
