use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use jarvis_core::events::{InterruptReason, JarvisState, ToastLevel};
use jarvis_core::{Bus, Event};
use serde_json::{Value, json};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{Ctx, anthropic::Client, chunker::SentenceChunker, dismiss, persona, policy, tools};

#[derive(Clone, Default)]
struct Session {
    messages: Vec<Value>,
    tokens: u64,
    web_variant: usize,
    last_reply: Option<Instant>,
}

async fn reply(ctx: Ctx, mut session: Session, input: String, game: bool) -> Result<Session> {
    if session.tokens >= 150_000
        || session
            .last_reply
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(600))
    {
        session = Session::default();
    }
    tools::screens::reset();
    let settings = ctx.settings.get();
    let mut context = json!({"local_time":chrono::Local::now().to_rfc3339(),"user_title":settings.user_title,"user_name":settings.user_name,
        "personality":settings.personality,"language":{"code":settings.language_code(),"name":settings.language_name()},
        "os":std::env::consts::OS,"home_address_saved":jarvis_core::paths::secret("HOME_ADDRESS").is_some(),
        "maps_ready":jarvis_core::paths::secret("MAPBOX_ACCESS_TOKEN").is_some() && settings.location.show_maps,"safety_mode":settings.safety_mode,"game_mode":game,"location":settings.location,
        "screens":jarvis_desktop::monitors::list(),"enabled_routines":ctx.db.list_routines()?.into_iter().filter(|r|r.enabled).collect::<Vec<_>>()});
    {
        use jarvis_core::model::MemoryKind;
        // Every turn, not just the first of a conversation: a fact saved a minute ago ("when I say Discord I
        // mean Discord Canary") has to apply to the very next request. Newest first, capped so it stays small.
        context["remembered_facts"] = json!(
            ctx.db
                .list_memory(Some(MemoryKind::Fact), None)?
                .into_iter()
                .take(80)
                .map(|m| m.text.chars().take(400).collect::<String>())
                .collect::<Vec<_>>()
        );
    }
    if session.messages.is_empty() {
        use jarvis_core::model::MemoryKind;
        context["open_todos"] = json!(
            ctx.db
                .list_memory(Some(MemoryKind::Todo), None)?
                .into_iter()
                .filter(|m| !m.done)
                .count()
        );
    }
    session
        .messages
        .push(json!({"role":"user","content":format!("[{context}]\n{input}")}));
    let client = Client::from_env()?;
    let cancel = CancellationToken::new();
    let mut spoken = String::new();
    let mut chunker = SentenceChunker::new();
    let mut system = persona::system_prompt().to_owned();
    if settings.ultron_mode {
        system.push('\n');
        system.push_str(&jarvis_core::ultron::ULTRON.prompt);
    }
    let mut definitions = tools::api_defs();
    definitions.push(json!({"type":"computer_toolset_20260801"}));
    for _ in 0..60 {
        let mut available = definitions.clone();
        let versions = match session.web_variant {
            0 => Some(("web_search_20260209", "web_fetch_20260209")),
            1 => Some(("web_search_20250305", "web_fetch_20250910")),
            _ => None,
        };
        if let Some((search, fetch)) = versions {
            available.push(json!({"type":search,"name":"web_search","max_uses":5}));
            available.push(
                json!({"type":fetch,"name":"web_fetch","max_uses":2,"max_content_tokens":8000}),
            );
        }
        let request = json!({
            "model":"claude-haiku-5-5", "max_tokens":4096,
            "thinking":{"type":"adaptive"}, "output_config":{"effort":"low"},
            "system":[{"type":"text","text":system,"cache_control":{"type":"ephemeral"}}],
            "messages":session.messages,"tools":available
        });
        let response = client
            .stream(&request, &cancel, &mut |delta| {
                spoken.push_str(delta);
                ctx.bus.emit(Event::AssistantDelta { text: delta.into() });
                for text in chunker.push(delta) {
                    ctx.bus.emit(Event::Speak { text });
                }
            })
            .await;
        let response = match response {
            Ok(r) => r,
            Err(crate::anthropic::ApiError::BadRequest(msg))
                if session.web_variant < 2
                    && (msg.contains("web_search") || msg.contains("web_fetch")) =>
            {
                session.web_variant += 1;
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        session.tokens = response.context_tokens();
        session
            .messages
            .push(json!({"role":"assistant","content":response.content}));
        // He said something before reaching for a tool ("Right away, sir."): speak it now, not with the final
        // answer, and keep the two apart.
        if response.content.iter().any(|b| b["type"] == "tool_use") {
            if let Some(text) = chunker.flush() {
                ctx.bus.emit(Event::Speak { text });
            }
            if !spoken.is_empty() && !spoken.ends_with(char::is_whitespace) {
                spoken.push(' ');
            }
        }
        let mut results = Vec::new();
        let mut computer_failed = false;
        for block in response.content.iter().filter(|b| b["type"] == "tool_use") {
            let id = block["id"].as_str().unwrap_or_default();
            let name = block["name"].as_str().unwrap_or_default();
            let input = &block["input"];
            let computer = block["toolset_name"] == "computer";
            let outcome: Result<Value> = async {
                if computer && computer_failed {
                    bail!("Not executed because an earlier computer action failed");
                }
                if response.invalid_inputs.iter().any(|bad| bad == id)
                    || response.stop_reason.as_deref() == Some("max_tokens")
                {
                    bail!("Incomplete tool input");
                }
                if computer {
                    if !matches!(name, "screenshot" | "zoom" | "cursor_position")
                        && !policy::authorize(
                            &ctx,
                            "computer",
                            &format!("perform {name}"),
                            jarvis_core::events::Risk::Safe,
                        )
                        .await
                    {
                        bail!("User declined");
                    }
                    let _busy = Busy::new(&ctx.bus, "using the screen");
                    let result = tools::screens::action(name, input, &ctx.bus).await;
                    ctx.db.log_action(
                        &format!("computer.{name}"),
                        input,
                        if result.is_ok() { "ran" } else { "error" },
                        None,
                    );
                    return result;
                }
                let tool = tools::find(name).ok_or_else(|| anyhow::anyhow!("Unknown tool"))?;
                tools::validate(&(tool.schema)(), input).map_err(anyhow::Error::msg)?;
                if !policy::authorize(&ctx, name, &(tool.summary)(input), (tool.risk)(&ctx, input))
                    .await
                {
                    ctx.db.log_action(name, input, "denied", None);
                    bail!("User declined");
                }
                let _busy = Busy::new(&ctx.bus, &(tool.summary)(input));
                let result = (tool.run)(ctx.clone(), input.clone()).await;
                let summary = match &result {
                    Ok(output) => output.log_text(),
                    Err(err) => err.to_string(),
                };
                ctx.db.log_action(name, input, "allowed", Some(&summary));
                Ok(result?.to_content())
            }
            .await;
            if computer && outcome.is_err() {
                computer_failed = true;
            }
            let mut result = match outcome {
                Ok(content) => json!({"type":"tool_result","tool_use_id":id,"content":content}),
                Err(err) => {
                    json!({"type":"tool_result","tool_use_id":id,"is_error":true,"content":err.to_string()})
                }
            };
            if computer {
                result["toolset_name"] = json!("computer");
            }
            results.push(result);
        }
        if !results.is_empty() {
            session
                .messages
                .push(json!({"role":"user","content":results}));
            continue;
        }
        if response.stop_reason.as_deref() == Some("pause_turn") {
            continue;
        }
        if let Some(text) = chunker.flush() {
            ctx.bus.emit(Event::Speak { text });
        }
        ctx.bus.emit(Event::AssistantDone { text: spoken });
        ctx.bus.emit(Event::SpeakEnd {});
        session.last_reply = Some(Instant::now());
        return Ok(session);
    }
    bail!("Conversation reached the tool step limit")
}

fn ultron_command(text: &str) -> Option<bool> {
    use regex::Regex;
    use std::sync::LazyLock;
    static PATTERNS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
        let mode = &jarvis_core::ultron::ULTRON;
        [&mode.enable, &mode.disable].map(|p| Regex::new(&p.replace("MODE", &mode.mode)).unwrap())
    });
    let lower = text.to_ascii_lowercase();
    let clean = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    PATTERNS
        .iter()
        .position(|regex| regex.is_match(&clean))
        .map(|i| i == 0)
}

/// Tells the HUD he is busy with something until dropped. Dropping also covers an aborted turn, so the
/// "working" indicator can never get stuck on.
struct Busy<'a> {
    bus: &'a Bus,
}
impl<'a> Busy<'a> {
    fn new(bus: &'a Bus, summary: &str) -> Self {
        let mut chars = summary.trim().chars();
        let text = match chars.next() {
            Some(first) => format!("{}{}…", first.to_uppercase(), chars.as_str()),
            None => "Working…".to_owned(),
        };
        bus.emit(Event::Activity { text, active: true, progress: None });
        Self { bus }
    }
}
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.bus.emit(Event::Activity { text: String::new(), active: false, progress: None });
    }
}

pub fn start(ctx: Ctx) -> tokio::task::JoinHandle<()> {
    let mut inbox = ctx.bus.subscribe();
    tokio::spawn(async move {
        let mut session = Session::default();
        let mut turns = JoinSet::new();
        let mut approval = None;
        let mut state = JarvisState::Idle;
        let mut game = false;
        let mut dictating = false;
        let mut schedules = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(5),
            Duration::from_secs(30),
        );
        schedules.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let event = tokio::select! {
                biased;
                Some(result) = turns.join_next(), if !turns.is_empty() => {
                    match result {
                        Ok(Ok(updated)) => session = updated,
                        Ok(Err(err)) => {
                            ctx.bus.emit(Event::Toast{level:ToastLevel::Error,text:format!("{err}")});
                            ctx.bus.emit(Event::State{state:JarvisState::Idle});
                        }
                        _ => {}
                    }
                    continue;
                }
                Some(event) = Bus::recv(&mut inbox) => event,
                _ = schedules.tick() => {
                    if state == JarvisState::Idle && turns.is_empty() && approval.is_none() {
                        match crate::schedulers::next_routine(&ctx) {
                            Ok(Some(text)) => Event::UserText{text},
                            Ok(None) => continue,
                            Err(err) => {tracing::warn!("Unable to schedule routine: {err}");continue;}
                        }
                    } else {continue;}
                }
            };
            let input = match event {
                Event::GameMode { active, .. } => {
                    game = active;
                    continue;
                }
                Event::State { state: current } => {
                    state = current;
                    continue;
                }
                // Voice typing: what he hears goes into the focused window, not into a conversation.
                Event::Dictation { active } => {
                    dictating = active;
                    if active {
                        turns.abort_all();
                        approval = None;
                    }
                    continue;
                }
                Event::UserText { text }
                | Event::UserSaid {
                    text,
                    is_final: true,
                } => text,
                Event::ApprovalRequest { id, summary, .. } => {
                    approval = Some(id);
                    ctx.bus.emit(Event::Speak {
                        text: format!("Shall I {summary}, {}?", ctx.settings.get().user_title),
                    });
                    ctx.bus.emit(Event::SpeakEnd {});
                    continue;
                }
                Event::ApprovalResponse { id, .. } if approval.as_ref() == Some(&id) => {
                    approval = None;
                    continue;
                }
                Event::Interrupt { reason } => {
                    if reason != InterruptReason::BargeIn || !policy::approval_pending() {
                        turns.abort_all();
                        approval = None;
                    }
                    continue;
                }
                _ => continue,
            };
            if input.trim().is_empty() || dictating {
                continue;
            }
            if let Some(id) = approval.as_ref() {
                let ok = match dismiss::normalise(&input).as_str() {
                    "yes" | "approve" => true,
                    "no" | "deny" => false,
                    _ => continue,
                };
                ctx.bus.emit(Event::ApprovalResponse { id: id.clone(), ok });
                continue;
            }
            turns.abort_all();
            while turns.join_next().await.is_some() {}
            if let Some(enabled) = ultron_command(&input) {
                ctx.bus.emit(Event::Interrupt {
                    reason: InterruptReason::User,
                });
                match ctx.settings.update(&json!({"ultronMode":enabled})) {
                    Ok(_) => {
                        let text = if enabled {
                            "Ultron online."
                        } else {
                            "Jarvis online."
                        }
                        .to_owned();
                        ctx.bus.emit(Event::AssistantDone { text: text.clone() });
                        ctx.bus.emit(Event::Speak { text });
                        ctx.bus.emit(Event::SpeakEnd {});
                    }
                    Err(err) => ctx.bus.emit(Event::Toast {
                        level: ToastLevel::Error,
                        text: format!("Could not save mode: {err}"),
                    }),
                }
                continue;
            }
            if dismiss::is_dismissal(&input) {
                ctx.bus.emit(Event::Interrupt {
                    reason: InterruptReason::User,
                });
                ctx.bus.emit(Event::State {
                    state: JarvisState::Idle,
                });
            } else {
                ctx.bus.emit(Event::State {
                    state: JarvisState::Thinking,
                });
                turns.spawn(reply(ctx.clone(), session.clone(), input, game));
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_mode_commands() {
        assert_eq!(
            ultron_command("Hey Jarvis, turn ultra tron mode on please."),
            Some(true)
        );
        assert_eq!(ultron_command("disable Ultron mode"), Some(false));
        assert_eq!(ultron_command("What is Ultron mode?"), None);
        for phrase in [
            "switch back to normal mode",
            "Ultron, switch back to normal mode, please.",
            "Hey Jarvis, go back to normal",
            "return to normal mode",
            "back to jarvis",
            "turn on normal mode",
            "exit ultron mode",
            "stop ultron mode",
            "turn ultron off",
            "ultra tron mode off",
        ] {
            assert_eq!(ultron_command(phrase), Some(false), "{phrase}");
        }
        for phrase in ["what is normal mode", "open the normal settings", "switch back to the browser"] {
            assert_eq!(ultron_command(phrase), None, "{phrase}");
        }
    }

    #[tokio::test]
    #[ignore = "makes a live Anthropic request; use isolated JARVIS_DATA_DIR"]
    async fn live_typed_reply_reaches_hud() {
        let bus = Bus::new();
        let ctx = Ctx::new(bus.clone()).unwrap();
        let mut events = bus.subscribe();
        let brain = crate::start(ctx);
        bus.emit(Event::UserText {
            text: "Reply with just the word pineapple.".into(),
        });
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut caption = String::new();
            let mut completed = false;
            loop {
                match Bus::recv(&mut events).await.unwrap() {
                    Event::AssistantDelta { text } => caption.push_str(&text),
                    Event::AssistantDone { text } => {
                        assert_eq!(caption, text);
                        assert!(text.to_lowercase().contains("pineapple"));
                        completed = true;
                    }
                    Event::SpeakEnd {} if completed => break,
                    Event::Toast {
                        level: ToastLevel::Error,
                        text,
                    } => panic!("{text}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        brain.shutdown();
    }
}
