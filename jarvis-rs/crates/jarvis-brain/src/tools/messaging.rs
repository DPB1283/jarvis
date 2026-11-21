use super::{
    ToolDef, ToolOutput,
    ps::{WIN_NATIVE, ps, psq},
    s, unsafe_,
};
use crate::Ctx;
use anyhow::{Context, Result, bail, ensure};
use jarvis_core::model::{ChatAppKind, ChatAppProfile};
use jarvis_desktop::{
    input,
    window::{self, Foreground},
};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::time::sleep;

struct Checked {
    app: String,
    recipient: String,
    message: String,
    attachment: Option<String>,
    send: bool,
    hwnd: usize,
    pid: u32,
    title: String,
    at: Instant,
}
static CHECKED: Mutex<Option<Checked>> = Mutex::new(None);

fn profile(ctx: &Ctx, name: &str) -> Result<ChatAppProfile> {
    let name = name.trim().to_lowercase();
    ctx.db
        .list_custom_chat_apps()?
        .into_iter()
        .chain(crate::ipc::crate_builtin_chat_apps())
        .find(|p| {
            p.id.eq_ignore_ascii_case(&name)
                || p.name.eq_ignore_ascii_case(&name)
                || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(&name))
        })
        .context("No matching messaging profile")
}
/// Chat apps decorate their window title as things happen: an unread count "(3) ", a leading dot, a typing or
/// notification badge. Those are not a different conversation, so compare the title without them.
fn chat_title(title: &str) -> String {
    let mut t = title.trim();
    loop {
        // An unread count like "(3) " or "(9+) ".
        if let Some((count, tail)) = t.strip_prefix('(').and_then(|rest| rest.split_once(')')) {
            if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit() || c == '+') {
                t = tail.trim_start();
                continue;
            }
        }
        // A leading dot, bullet or other decoration.
        match t.chars().next() {
            Some(c) if !c.is_alphanumeric() && c != '@' && c != '#' => t = t[c.len_utf8()..].trim_start(),
            _ => return t.to_lowercase(),
        }
    }
}
fn same_chat(a: &str, b: &str) -> bool {
    chat_title(a) == chat_title(b)
}
fn target(app: &ChatAppProfile) -> Result<Foreground> {
    let fg = window::foreground();
    ensure!(
        fg.hwnd != 0
            && app.process_names.iter().any(|p| p
                .trim_end_matches(".exe")
                .eq_ignore_ascii_case(fg.exe.trim_end_matches(".exe"))),
        "The target app is not in front"
    );
    Ok(fg)
}
fn same(app: &ChatAppProfile, window: usize) -> Result<Foreground> {
    let fg = target(app)?;
    ensure!(fg.hwnd == window, "The active window changed");
    Ok(fg)
}
async fn press(app: &ChatAppProfile, window: usize, keys: &[String]) -> Result<()> {
    for key in keys {
        same(app, window)?;
        input::chord(key)?;
        sleep(Duration::from_millis(120)).await;
    }
    Ok(())
}
async fn open(app: &ChatAppProfile) -> Result<Foreground> {
    let web = app.launch.starts_with("https://") || app.launch.starts_with("http://");
    if !web && let Ok(fg) = target(app) {
        return Ok(fg);
    }
    let mut focused = false;
    if !web {
        let names = app
            .process_names
            .iter()
            .map(|n| psq(n.trim_end_matches(".exe")))
            .collect::<Vec<_>>()
            .join(",");
        if !names.is_empty() {
            let script = format!(
                "{WIN_NATIVE}\n$p=Get-Process -Name @({names}) -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1\nif($p){{[JarvisNative]::Focus($p.MainWindowHandle)}}"
            );
            focused = ps(&script).await?.eq_ignore_ascii_case("true");
        }
    }
    if !focused {
        super::system::open_app(&app.launch, None).await?;
        sleep(Duration::from_millis(
            app.launch_wait_ms.unwrap_or(1800).min(15000),
        ))
        .await;
    }
    target(app)
}

async fn message(ctx: Ctx, v: Value) -> Result<ToolOutput> {
    let app = profile(&ctx, s(&v, "app").context("Missing app")?)?;
    let recipient = s(&v, "recipient").unwrap_or("").trim();
    let message = v["message"].as_str().context("Missing message")?;
    let attachment = s(&v, "image_path");
    ensure!(
        !message.is_empty() || attachment.is_some(),
        "Specify text or an image"
    );
    let send = v["send"].as_bool().unwrap_or(true);
    ensure!(
        app.kind != ChatAppKind::People || !recipient.is_empty(),
        "Specify the intended recipient"
    );
    if v["chat_confirmed"] == true {
        let saved = CHECKED
            .lock()
            .unwrap()
            .take()
            .context("Inspect the conversation first")?;
        let check = || -> Result<()> {
            let fg = same(&app, saved.hwnd)?;
            ensure!(
                saved.at.elapsed() < Duration::from_secs(180)
                    && saved.app == app.id
                    && saved.recipient == recipient
                    && saved.message == message
                    && saved.attachment.as_deref() == attachment
                    && saved.send == send
                    && fg.pid == saved.pid
                    && same_chat(&fg.title, &saved.title),
                "The inspected conversation or message changed"
            );
            Ok(())
        };
        check()?;
        press(
            &app,
            saved.hwnd,
            app.focus_message_keys.as_deref().unwrap_or_default(),
        )
        .await?;
        ensure!(
            !send || !app.send_keys.is_empty(),
            "This profile has no verified send shortcut. Choose draft mode or configure Send keys."
        );
        check()?;
        if let Some(image) = attachment {
            super::clipboard::paste(saved.hwnd, "", Some(image)).await?;
        }
        if !message.is_empty() {
            check()?;
            super::clipboard::paste(saved.hwnd, message, None).await?;
        }
        if send {
            sleep(Duration::from_millis(150)).await;
            check()?;
            press(&app, saved.hwnd, &app.send_keys).await?;
        }
        if send && let Some(delay) = v["read_reply_after_seconds"].as_f64() {
            sleep(Duration::from_secs_f64(delay.clamp(0., 30.))).await;
            let fg = same(&app, saved.hwnd)?;
            return Ok(ToolOutput::Blocks(vec![
                super::screens::snapshot(fg.bounds.context("Cannot inspect reply")?).await?,
            ]));
        }
        return Ok(if send {
            "Issued the configured send shortcut."
        } else {
            "Draft entered."
        }
        .into());
    }
    *CHECKED.lock().unwrap() = None;
    let fg = open(&app).await?;
    if app.kind == ChatAppKind::People && !app.search_keys.is_empty() {
        press(&app, fg.hwnd, &app.search_keys).await?;
        same(&app, fg.hwnd)?;
        input::type_text(recipient)?;
        sleep(Duration::from_millis(
            app.search_wait_ms.unwrap_or(800).min(15000),
        ))
        .await;
        press(&app, fg.hwnd, &app.pick_keys).await?;
    } else if app.kind == ChatAppKind::Assistant && v["new_chat"] == true {
        press(
            &app,
            fg.hwnd,
            app.new_chat_keys.as_deref().unwrap_or_default(),
        )
        .await?;
    }
    sleep(Duration::from_millis(300)).await;
    let fg = same(&app, fg.hwnd)?;
    let image =
        super::screens::snapshot(fg.bounds.context("Cannot inspect target window")?).await?;
    let after = same(&app, fg.hwnd)?;
    if !same_chat(&after.title, &fg.title) {
        bail!("Target changed during inspection");
    }
    *CHECKED.lock().unwrap() = Some(Checked {
        app: app.id,
        recipient: recipient.into(),
        message: message.into(),
        attachment: attachment.map(str::to_owned),
        send,
        hwnd: fg.hwnd,
        pid: fg.pid,
        title: fg.title,
        at: Instant::now(),
    });
    Ok(ToolOutput::Blocks(vec![
        json!({"type":"text","text":"Check the visible app, recipient and composer. Only when the intended conversation and text entry field are ready, repeat the exact request with chat_confirmed true. No message has been entered."}),
        image,
    ]))
}

pub fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "send_message",
        description: "Inspect a messaging app and draft or send text to the requested conversation after verifying its screenshot. Preserve the app, recipient, message and send flag on confirmation.",
        risk: |ctx, v| {
            if v["chat_confirmed"] == true {
                unsafe_(ctx, v)
            } else {
                super::safe(ctx, v)
            }
        },
        summary: |v| {
            format!(
                "{} a message in {} to {}",
                if v["send"] == false { "draft" } else { "send" },
                s(v, "app").unwrap_or("?"),
                s(v, "recipient").unwrap_or("the assistant")
            )
        },
        schema: || json!({"type":"object","required":["app","message"],"properties":{"app":{"type":"string"},"recipient":{"type":"string"},"message":{"type":"string"},"image_path":{"type":"string"},"read_reply_after_seconds":{"type":"number","minimum":0,"maximum":30},"send":{"type":"boolean"},"chat_confirmed":{"type":"boolean"},"new_chat":{"type":"boolean"}}}),
        run: |ctx, v| Box::pin(message(ctx, v)),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires a foreground local Jarvis messaging test window and isolated data"]
    async fn local_draft_and_send() {
        let fg = window::foreground();
        assert_eq!(fg.title, "Jarvis local messaging fixture");
        let ctx = Ctx::new(jarvis_core::Bus::new()).unwrap();
        let mut profile = crate::ipc::crate_builtin_chat_apps()
            .into_iter()
            .find(|p| p.id == "discord")
            .unwrap();
        profile.id = "local-fixture".into();
        profile.kind = ChatAppKind::Assistant;
        profile.name = "Local fixture".into();
        profile.process_names = vec![fg.exe];
        profile.launch = "fixture".into();
        profile.search_keys.clear();
        profile.pick_keys.clear();
        ctx.db
            .upsert_custom_chat_app(&profile, crate::ipc::validate_key_combo)
            .unwrap();
        let mut request =
            json!({"app":"local-fixture","recipient":"fixture","message":"Jarvis test ✓"});
        let inspected = message(ctx.clone(), request.clone()).await.unwrap();
        assert!(matches!(inspected, ToolOutput::Blocks(_)));
        request["chat_confirmed"] = json!(true);
        message(ctx.clone(), request.clone()).await.unwrap();
        assert!(message(ctx, request).await.is_err());
    }

    #[test]
    fn unread_badges_do_not_look_like_a_different_chat() {
        assert!(same_chat("@Snowman - Discord", "(3) @Snowman - Discord"));
        assert!(same_chat("• @Snowman - Discord", "@Snowman - Discord"));
        assert!(same_chat("(12) #general | PalaceHub - Discord", "#general | PalaceHub - Discord"));
        assert!(!same_chat("@Snowman - Discord", "@Remy - Discord"));
        assert!(!same_chat("#rules | PalaceHub - Discord", "@Snowman - Discord"));
    }
}
