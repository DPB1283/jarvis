//! Minimal Claude Messages API client over raw HTTPS + SSE (there is no official Rust SDK).
//! Reference: the claude-api skill's curl/examples.md (headers, streaming events, tool use).
//!
//! Content blocks are kept as raw JSON and passed back to the API unchanged: thinking blocks (with their
//! signatures), server-tool blocks, `toolset_name` on computer tool calls, citations — anything the API sends.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("overloaded: {0}")]
    Overloaded(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("HTTP {0}: {1}")]
    Http(u16, String),
    #[error("network error: {0}")]
    Network(String),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("cancelled")]
    Cancelled,
}

impl ApiError {
    fn retryable(&self) -> bool {
        matches!(self, ApiError::RateLimited(_) | ApiError::Overloaded(_) | ApiError::Network(_)) || matches!(self, ApiError::Http(s, _) if *s >= 500)
    }
}

/// A finished (streamed) response.
#[derive(Debug, Clone, Default)]
pub struct Message {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub stop_details: Option<Value>,
    pub usage: Value,
    /// tool_use ids whose streamed input wasn't valid JSON (never run these).
    pub invalid_inputs: Vec<String>,
}

impl Message {
    /// input + cache read + cache creation tokens: the size of the conversation the model just saw.
    pub fn context_tokens(&self) -> u64 {
        ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| self.usage.get(*k)?.as_u64()).sum()
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    key: String,
}

impl Client {
    pub fn from_env() -> Result<Self, ApiError> {
        let key = jarvis_core::paths::secret("ANTHROPIC_API_KEY").ok_or_else(|| ApiError::Auth("ANTHROPIC_API_KEY is not set (jarvis/.env.local)".into()))?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(Self { http, key })
    }

    /// Stream one request. `on_text` gets each text delta as it arrives (for captions and speech).
    /// Retries up to twice on 429 / 5xx / overloaded / network errors, but only before any text was delivered.
    pub async fn stream(&self, body: &Value, cancel: &CancellationToken, on_text: &mut (dyn FnMut(&str) + Send)) -> Result<Message, ApiError> {
        let mut body = body.clone();
        body["stream"] = json!(true);
        let mut attempt = 0;
        loop {
            let mut delivered = false;
            let mut wrapped = |t: &str| {
                delivered = true;
                on_text(t);
            };
            match self.stream_once(&body, cancel, &mut wrapped).await {
                Err(e) if e.retryable() && !delivered && attempt < 2 => {
                    attempt += 1;
                    let wait = Duration::from_millis(600 * 2u64.pow(attempt));
                    tracing::warn!("Claude API {e}; retrying in {wait:?}");
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(ApiError::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
                other => return other,
            }
        }
    }

    async fn stream_once(&self, body: &Value, cancel: &CancellationToken, on_text: &mut (dyn FnMut(&str) + Send)) -> Result<Message, ApiError> {
        let req = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(body)
            .send();
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(ApiError::Cancelled),
            r = req => r.map_err(|e| ApiError::Network(e.to_string()))?,
        };
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(match status {
                401 | 403 => ApiError::Auth(msg),
                429 => ApiError::RateLimited(msg),
                529 => ApiError::Overloaded(msg),
                400 | 404 | 413 | 422 => ApiError::BadRequest(msg),
                s => ApiError::Http(s, msg),
            });
        }

        let mut acc = Accumulator::default();
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(ApiError::Cancelled),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| ApiError::Network(e.to_string()))?;
            buf.extend_from_slice(&chunk);
            // SSE events are separated by a blank line.
            while let Some(end) = find_event_end(&buf) {
                let raw: Vec<u8> = buf.drain(..end.0).collect();
                buf.drain(..end.1 - end.0);
                let text = String::from_utf8_lossy(&raw);
                let data: String = text
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(|d| d.strip_prefix(' ').unwrap_or(d))
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    continue;
                }
                let ev: Value = serde_json::from_str(&data).map_err(|e| ApiError::Stream(format!("bad event JSON: {e}")))?;
                if acc.apply(&ev, on_text)? {
                    return Ok(acc.finish());
                }
            }
        }
        // Stream ended without message_stop: treat what we have as the message if it has a stop reason.
        if acc.stop_reason.is_some() { Ok(acc.finish()) } else { Err(ApiError::Network("stream ended early".into())) }
    }
}

/// (end of event, end including the separator)
fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, i + 2));
        }
        if i + 3 < buf.len() && &buf[i..i + 4] == b"\r\n\r\n" {
            return Some((i, i + 4));
        }
    }
    None
}

#[derive(Default)]
struct Accumulator {
    blocks: Vec<Option<Value>>,
    partial_json: HashMap<usize, String>,
    usage: Map<String, Value>,
    stop_reason: Option<String>,
    stop_details: Option<Value>,
    invalid_inputs: Vec<String>,
}

impl Accumulator {
    /// Returns Ok(true) at message_stop.
    fn apply(&mut self, ev: &Value, on_text: &mut (dyn FnMut(&str) + Send)) -> Result<bool, ApiError> {
        match ev["type"].as_str().unwrap_or("") {
            "message_start" => {
                if let Some(u) = ev["message"]["usage"].as_object() {
                    self.usage.extend(u.clone());
                }
            }
            "content_block_start" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                if self.blocks.len() <= i {
                    self.blocks.resize(i + 1, None);
                }
                self.blocks[i] = Some(ev["content_block"].clone());
            }
            "content_block_delta" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                let delta = &ev["delta"];
                let Some(Some(block)) = self.blocks.get_mut(i) else { return Ok(false) };
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = delta["text"].as_str().unwrap_or("");
                        append_str(block, "text", t);
                        on_text(t);
                    }
                    "thinking_delta" => append_str(block, "thinking", delta["thinking"].as_str().unwrap_or("")),
                    "signature_delta" => block["signature"] = delta["signature"].clone(),
                    "input_json_delta" => self.partial_json.entry(i).or_default().push_str(delta["partial_json"].as_str().unwrap_or("")),
                    "citations_delta" => {
                        let c = delta["citation"].clone();
                        match block.get_mut("citations").and_then(|v| v.as_array_mut()) {
                            Some(arr) => arr.push(c),
                            None => block["citations"] = json!([c]),
                        }
                    }
                    other => tracing::debug!("ignoring delta type {other}"),
                }
            }
            "content_block_stop" => {
                let i = ev["index"].as_u64().unwrap_or(0) as usize;
                if let (Some(raw), Some(Some(block))) = (self.partial_json.remove(&i), self.blocks.get_mut(i)) {
                    let input = if raw.trim().is_empty() {
                        json!({})
                    } else {
                        match serde_json::from_str::<Value>(&raw) {
                            Ok(v) => v,
                            Err(_) => {
                                if let Some(id) = block["id"].as_str() {
                                    self.invalid_inputs.push(id.to_string());
                                }
                                json!({})
                            }
                        }
                    };
                    block["input"] = input;
                }
            }
            "message_delta" => {
                if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
                if !ev["delta"]["stop_details"].is_null() {
                    self.stop_details = Some(ev["delta"]["stop_details"].clone());
                }
                if let Some(u) = ev["usage"].as_object() {
                    self.usage.extend(u.clone());
                }
            }
            "message_stop" => return Ok(true),
            "error" => {
                let kind = ev["error"]["type"].as_str().unwrap_or("error");
                let msg = ev["error"]["message"].as_str().unwrap_or("").to_string();
                return Err(match kind {
                    "overloaded_error" => ApiError::Overloaded(msg),
                    "rate_limit_error" => ApiError::RateLimited(msg),
                    _ => ApiError::Stream(format!("{kind}: {msg}")),
                });
            }
            _ => {} // ping and anything new
        }
        Ok(false)
    }

    fn finish(self) -> Message {
        Message {
            content: self.blocks.into_iter().flatten().collect(),
            stop_reason: self.stop_reason,
            stop_details: self.stop_details,
            usage: Value::Object(self.usage),
            invalid_inputs: self.invalid_inputs,
        }
    }
}

fn append_str(block: &mut Value, key: &str, add: &str) {
    let cur = block[key].as_str().unwrap_or("").to_string();
    block[key] = Value::String(cur + add);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_text_thinking_and_tool_input() {
        let events = [
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":5}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig123"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello, "}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"sir."}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"left_click","toolset_name":"computer","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"coordinate\":[1"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"0,20]}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
            json!({"type":"message_stop"}),
        ];
        let mut acc = Accumulator::default();
        let mut said = String::new();
        let mut on = |t: &str| said.push_str(t);
        let mut done = false;
        for e in &events {
            done = acc.apply(e, &mut on).unwrap();
        }
        assert!(done);
        let m = acc.finish();
        assert_eq!(said, "Hello, sir.");
        assert_eq!(m.content[0]["signature"], "sig123");
        assert_eq!(m.content[1]["text"], "Hello, sir.");
        assert_eq!(m.content[2]["input"]["coordinate"], json!([10, 20]));
        assert_eq!(m.content[2]["toolset_name"], "computer", "unknown fields survive untouched");
        assert_eq!(m.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(m.context_tokens(), 15);
        assert!(m.invalid_inputs.is_empty());
    }

    /// Live call (costs a fraction of a cent). Run with: cargo test -p jarvis-brain live_ -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_streams_a_reply_with_a_tool_call() {
        let client = Client::from_env().expect("ANTHROPIC_API_KEY");
        let body = json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 2000,
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "low"},
            "tools": [{"name": "get_time", "description": "Current local time", "input_schema": {"type": "object", "properties": {}}, "eager_input_streaming": true}],
            "messages": [{"role": "user", "content": "Say 'Testing, sir.' then call get_time."}],
        });
        let mut text = String::new();
        let mut on = |t: &str| text.push_str(t);
        let m = client.stream(&body, &CancellationToken::new(), &mut on).await.unwrap();
        println!("text: {text:?}\nstop: {:?}\nblocks: {}\nusage: {}", m.stop_reason, serde_json::to_string(&m.content).unwrap(), m.usage);
        assert_eq!(m.stop_reason.as_deref(), Some("tool_use"));
        assert!(m.content.iter().any(|b| b["type"] == "tool_use" && b["name"] == "get_time"));
    }

    #[test]
    fn splits_sse_events() {
        assert_eq!(find_event_end(b"event: ping\ndata: {}\n\nrest"), Some((20, 22)));
        assert_eq!(find_event_end(b"data: {}\r\n\r\n"), Some((8, 12)));
        assert_eq!(find_event_end(b"data: {"), None);
    }
}
