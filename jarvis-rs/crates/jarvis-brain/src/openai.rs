//! The conversation loop for any OpenAI-compatible chat-completions service (OpenAI, Google Gemini's
//! OpenAI-compatible endpoint, OpenRouter, Groq, Together, a local Ollama or LM Studio...). It uses all of
//! Jarvis's own tools the same way the Claude path does. Claude-only extras are not available here: no direct
//! click-and-type computer use, and no built-in web search (the context block says `computer_use: false`).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use futures_util::StreamExt;
use jarvis_core::Event;
use serde_json::{Value, json};

use crate::agent::{Busy, Session, system_for, user_turn};
use crate::tools::{self, ToolOutput};
use crate::{Ctx, chunker::SentenceChunker, policy};

/// Our tool definitions in OpenAI's function format.
fn tool_defs() -> Vec<Value> {
    tools::api_defs()
        .into_iter()
        .map(|t| json!({"type":"function","function":{"name":t["name"],"description":t["description"],"parameters":t["input_schema"]}}))
        .collect()
}

/// A tool call being assembled from streamed fragments.
#[derive(Default, Debug)]
pub(crate) struct Call {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default, Debug)]
pub(crate) struct Accumulated {
    pub text: String,
    pub calls: BTreeMap<usize, Call>,
    pub finished: bool,
}

/// Apply one streamed chunk. Returns the text it added, if any.
pub(crate) fn apply_chunk(acc: &mut Accumulated, chunk: &Value) -> Option<String> {
    let choice = chunk["choices"].get(0)?;
    if choice["finish_reason"].is_string() {
        acc.finished = true;
    }
    let delta = &choice["delta"];
    for call in delta["tool_calls"].as_array().into_iter().flatten() {
        let index = call["index"].as_u64().unwrap_or(0) as usize;
        let entry = acc.calls.entry(index).or_default();
        if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
            entry.id = id.to_owned();
        }
        if let Some(name) = call["function"]["name"].as_str() {
            entry.name.push_str(name);
        }
        if let Some(args) = call["function"]["arguments"].as_str() {
            entry.arguments.push_str(args);
        }
    }
    let text = delta["content"].as_str().filter(|t| !t.is_empty())?;
    acc.text.push_str(text);
    Some(text.to_owned())
}

/// What a tool returned, as (text for the tool message, images to show the model as `data:` URLs).
fn flatten(output: &ToolOutput) -> (String, Vec<String>) {
    match output {
        ToolOutput::Text(t) => (t.clone(), Vec::new()),
        ToolOutput::Blocks(blocks) => {
            let mut text = Vec::new();
            let mut images = Vec::new();
            for block in blocks {
                match block["type"].as_str() {
                    Some("text") => text.push(block["text"].as_str().unwrap_or_default().to_owned()),
                    Some("image") => {
                        let source = &block["source"];
                        if let (Some(kind), Some(data)) = (source["media_type"].as_str(), source["data"].as_str()) {
                            images.push(format!("data:{kind};base64,{data}"));
                        }
                    }
                    _ => {}
                }
            }
            (text.join("\n"), images)
        }
    }
}

/// Run one tool call with the same checks as the Claude path: validation, the safety policy, logging.
async fn execute(ctx: &Ctx, name: &str, input: &Value) -> Result<ToolOutput> {
    let tool = tools::find(name).ok_or_else(|| anyhow!("Unknown tool {name}"))?;
    tools::validate(&(tool.schema)(), input).map_err(anyhow::Error::msg)?;
    if !policy::authorize(ctx, name, &(tool.summary)(input), (tool.risk)(ctx, input)).await {
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
    result
}

/// Keep the history from growing without end: drop the oldest whole turns (never split a tool exchange).
fn trim(messages: &mut Vec<Value>) {
    while messages.len() > 60 {
        let next = messages.iter().skip(1).position(|m| m["role"] == "user").map(|i| i + 1);
        match next {
            Some(i) => {
                messages.drain(..i);
            }
            None => break,
        }
    }
}

fn error_message(status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .or_else(|| v["message"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let hint = match status.as_u16() {
        401 | 403 => " Check the API key in Settings, API keys.",
        404 => " Check the base URL and the model name in Settings, API keys.",
        429 => " The service is rate limiting or out of credit.",
        _ => "",
    };
    format!("The AI service answered {status}: {detail}.{hint}")
}

pub(crate) async fn reply(ctx: Ctx, mut session: Session, input: String, game: bool) -> Result<Session> {
    if session.last_reply.is_some_and(|t| t.elapsed() >= Duration::from_secs(600)) {
        session = Session::default();
    }
    trim(&mut session.oa_messages);
    tools::screens::reset();
    let settings = ctx.settings.get();
    let provider = settings.provider.clone();
    let key = jarvis_core::paths::secret("AI_API_KEY");
    let url = format!("{}/chat/completions", provider.base_url.trim().trim_end_matches('/'));
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build()?;

    let content = user_turn(&ctx, session.oa_messages.is_empty(), game, &input)?;
    session.oa_messages.push(json!({"role":"user","content":content}));
    let system = system_for(&settings);
    let definitions = tool_defs();
    let mut spoken = String::new();
    let mut chunker = SentenceChunker::new();

    for _ in 0..40 {
        let mut messages = vec![json!({"role":"system","content":system})];
        messages.extend(session.oa_messages.iter().cloned());
        let body = json!({"model":provider.model,"messages":messages,"tools":definitions,"stream":true});
        let mut request = http.post(&url).json(&body);
        if let Some(key) = &key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|e| anyhow!("Could not reach the AI service at {url}: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("{}", error_message(status, &text));
        }

        let mut acc = Accumulated::default();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        while let Some(bytes) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&bytes?));
            while let Some(newline) = buffer.find('\n') {
                let line: String = buffer.drain(..=newline).collect();
                let Some(data) = line.trim().strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" || data.is_empty() {
                    continue;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
                if let Some(delta) = apply_chunk(&mut acc, &chunk) {
                    spoken.push_str(&delta);
                    ctx.bus.emit(Event::AssistantDelta { text: delta.clone() });
                    for text in chunker.push(&delta) {
                        ctx.speak(text);
                    }
                }
            }
        }

        if acc.calls.is_empty() {
            session.oa_messages.push(json!({"role":"assistant","content":acc.text}));
            if let Some(text) = chunker.flush() {
                ctx.speak(text);
            }
            ctx.bus.emit(Event::AssistantDone { text: spoken });
            ctx.bus.emit(Event::SpeakEnd {});
            session.last_reply = Some(Instant::now());
            return Ok(session);
        }

        // He said something before reaching for tools ("Right away, sir."): speak it now.
        if let Some(text) = chunker.flush() {
            ctx.speak(text);
        }
        if !spoken.is_empty() && !spoken.ends_with(char::is_whitespace) {
            spoken.push(' ');
        }
        let calls: Vec<(String, String, String)> = acc
            .calls
            .into_values()
            .map(|c| (if c.id.is_empty() { format!("call_{}", uuid::Uuid::new_v4().simple()) } else { c.id }, c.name, c.arguments))
            .collect();
        session.oa_messages.push(json!({
            "role":"assistant",
            "content": if acc.text.is_empty() { Value::Null } else { json!(acc.text) },
            "tool_calls": calls.iter().map(|(id, name, args)| json!({"id":id,"type":"function","function":{"name":name,"arguments":args}})).collect::<Vec<_>>(),
        }));
        let mut images = Vec::new();
        for (id, name, args) in &calls {
            let input: Value = if args.trim().is_empty() { json!({}) } else { serde_json::from_str(args).unwrap_or(Value::Null) };
            let outcome = if input.is_object() { execute(&ctx, name, &input).await } else { Err(anyhow!("The arguments were not valid JSON")) };
            let text = match outcome {
                Ok(output) => {
                    let (text, found) = flatten(&output);
                    images.extend(found);
                    if text.is_empty() { "OK".to_owned() } else { text }
                }
                Err(error) => format!("Error: {error}"),
            };
            session.oa_messages.push(json!({"role":"tool","tool_call_id":id,"content":text}));
        }
        if !images.is_empty() {
            // Tool messages are text only, so what the tools produced as pictures arrives as a user message.
            let mut parts = vec![json!({"type":"text","text":"(Images produced by the tools above.)"})];
            parts.extend(images.into_iter().map(|url| json!({"type":"image_url","image_url":{"url":url}})));
            session.oa_messages.push(json!({"role":"user","content":parts}));
        }
    }
    bail!("Conversation reached the tool step limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn streamed_text_and_tool_calls_are_assembled() {
        let mut acc = Accumulated::default();
        let chunks = [
            json!({"choices":[{"delta":{"content":"Right "}}]}),
            json!({"choices":[{"delta":{"content":"away."}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"time","arguments":"{\"a\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}},{"index":1,"id":"call_2","function":{"name":"x","arguments":"{}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        ];
        let added: Vec<_> = chunks.iter().filter_map(|c| apply_chunk(&mut acc, c)).collect();
        assert_eq!(added, vec!["Right ", "away."]);
        assert_eq!(acc.text, "Right away.");
        assert!(acc.finished);
        assert_eq!(acc.calls[&0].name, "get_time");
        assert_eq!(acc.calls[&0].arguments, "{\"a\":1}");
        assert_eq!(acc.calls[&0].id, "call_1");
        assert_eq!(acc.calls[&1].name, "x");
    }

    #[test]
    fn tool_outputs_become_text_and_images() {
        let (text, images) = flatten(&ToolOutput::Blocks(vec![
            json!({"type":"text","text":"Saved it"}),
            json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AAAA"}}),
        ]));
        assert_eq!(text, "Saved it");
        assert_eq!(images, vec!["data:image/jpeg;base64,AAAA"]);
        assert_eq!(flatten(&ToolOutput::Text("hi".into())), ("hi".to_owned(), vec![]));
    }

    #[test]
    fn history_is_trimmed_on_turn_boundaries() {
        let mut messages: Vec<Value> = (0..40)
            .flat_map(|i| [json!({"role":"user","content":format!("q{i}")}), json!({"role":"assistant","content":"a"}), json!({"role":"user","content":"x"})])
            .collect();
        trim(&mut messages);
        assert!(messages.len() <= 60);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn errors_explain_what_to_fix() {
        let text = error_message(reqwest::StatusCode::UNAUTHORIZED, r#"{"error":{"message":"Incorrect API key provided"}}"#);
        assert!(text.contains("Incorrect API key") && text.contains("Settings"), "{text}");
        assert!(error_message(reqwest::StatusCode::NOT_FOUND, "nope").contains("model name"));
    }

    /// A tiny OpenAI-compatible server: answers the first request with a tool call, the second with text.
    async fn mock_server(seen: Arc<Mutex<Vec<Value>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for round in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut chunk = [0u8; 8192];
                let body = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    data.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split].to_lowercase().split("content-length:").nth(1).and_then(|v| v.lines().next()).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                        if data.len() >= split + 4 + length {
                            break text[split + 4..split + 4 + length].to_owned();
                        }
                    }
                };
                seen.lock().unwrap().push(serde_json::from_str(&body).unwrap());
                let events = if round == 0 {
                    vec![
                        json!({"choices":[{"delta":{"content":"Checking now, sir. "}}]}),
                        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"get_time","arguments":"{}"}}]}}]}),
                        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
                    ]
                } else {
                    vec![
                        json!({"choices":[{"delta":{"content":"It is "}}]}),
                        json!({"choices":[{"delta":{"content":"done, sir."}}]}),
                        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
                    ]
                };
                let mut payload = String::new();
                for event in events {
                    payload.push_str(&format!("data: {event}\n\n"));
                }
                payload.push_str("data: [DONE]\n\n");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{payload}", payload.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    #[tokio::test]
    async fn a_full_turn_with_a_tool_call_against_a_mock_service() {
        let dir = std::env::temp_dir().join(format!("jarvis-openai-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("JARVIS_DATA_DIR", &dir) };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = mock_server(seen.clone()).await;

        let bus = jarvis_core::Bus::new();
        let mut events = bus.subscribe();
        let ctx = Ctx::new(bus).unwrap();
        ctx.settings.update(&json!({"provider":{"kind":"openai","baseUrl":base,"model":"test-model"}})).unwrap();

        let session = reply(ctx.clone(), Session::default(), "what time is it".into(), false).await.unwrap();

        let mut spoken = Vec::new();
        let mut done = None;
        let mut busy = false;
        while let Ok(event) = events.try_recv() {
            match event {
                Event::Speak { text } => spoken.push(text),
                Event::AssistantDone { text } => done = Some(text),
                Event::Activity { active: true, .. } => busy = true,
                _ => {}
            }
        }
        let joined = spoken.join(" ");
        assert!(joined.contains("Checking now"), "the acknowledgement is spoken before the tool runs: {joined}");
        assert!(joined.contains("done, sir"), "{joined}");
        assert!(done.unwrap().contains("It is done, sir."));
        assert!(busy, "the activity chip is shown while the tool runs");

        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["model"], "test-model");
        assert_eq!(requests[0]["stream"], true);
        assert!(requests[0]["tools"].as_array().unwrap().iter().any(|t| t["function"]["name"] == "get_time"));
        // The second request carries the tool's result back.
        let second = requests[1]["messages"].as_array().unwrap();
        let tool_message = second.iter().find(|m| m["role"] == "tool").expect("tool result sent back");
        assert_eq!(tool_message["tool_call_id"], "call_a");
        assert!(!tool_message["content"].as_str().unwrap().is_empty());
        assert_eq!(session.oa_messages.last().unwrap()["content"], "It is done, sir.");
        std::fs::remove_dir_all(dir).ok();
    }
}
