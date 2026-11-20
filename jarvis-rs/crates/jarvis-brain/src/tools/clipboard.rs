use super::{
    ToolDef, ToolOutput,
    ps::ps,
    s, safe,
};
use anyhow::Result;
use jarvis_desktop::window;
use serde_json::{Value, json};

/// Native paste (see `jarvis_desktop::clipboard`): no PowerShell, which Defender flags as a trojan.
pub async fn paste(hwnd: usize, text: &str, image: Option<&str>) -> Result<()> {
    let (text, image) = (text.to_owned(), image.map(str::to_owned));
    tokio::task::spawn_blocking(move || jarvis_desktop::clipboard::paste(hwnd, &text, image.as_deref())).await?
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "type_text",
            description: "Paste Unicode or multiline text into the focused text field.",
            schema: || json!({"type":"object","properties":{"text":{"type":"string","minLength":1}},"required":["text"]}),
            risk: safe,
            summary: |_| "enter text in the focused field".into(),
            run: |_, v| {
                Box::pin(async move {
                    paste(
                        window::foreground().hwnd,
                        s(&v, "text").unwrap_or_default(),
                        None,
                    )
                    .await?;
                    Ok("Typed.".into())
                })
            },
        },
        ToolDef {
            name: "read_clipboard",
            description: "Read copied text, file names or an image.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "read the clipboard".into(),
            run: |_, _| {
                Box::pin(async {
                    let result = ps(include_str!("read-clipboard.ps1")).await?;
                    let (kind, data) = result.split_once('\n').unwrap_or((&result, ""));
                    Ok(match kind.trim() {
                        "IMAGE" => ToolOutput::Blocks(vec![
                            json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":data.trim()}}),
                        ]),
                        "TEXT" => data.chars().take(30000).collect::<String>().into(),
                        "FILES" => format!("Copied files:\n{data}").into(),
                        _ => "Clipboard is empty.".into(),
                    })
                })
            },
        },
        ToolDef {
            name: "set_clipboard",
            description: "Copy text to the clipboard.",
            schema: || json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}}}),
            risk: safe,
            summary: |_| "copy text to the clipboard".into(),
            run: |_, v| {
                Box::pin(async move {
                    let text = v["text"].as_str().unwrap_or_default().to_owned();
                    tokio::task::spawn_blocking(move || jarvis_desktop::clipboard::set_text(&text)).await??;
                    Ok("Copied.".into())
                })
            },
        },
        ToolDef {
            name: "now_playing",
            description: "Read the current Windows media session and playback status.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check what is playing".into(),
            run: |_, _| {
                Box::pin(async {
                    let result = ps(include_str!("read-media.ps1")).await?;
                    let media: Value = serde_json::from_str(&result)?;
                    Ok(if media.is_null() {
                        "No active media session.".to_owned()
                    } else {
                        media.to_string()
                    }
                    .into())
                })
            },
        },
    ]
}
