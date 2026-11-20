//! Shared data types, mirroring `jarvis/src/shared/settings.ts` (same JSON field names).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Note,
    Todo,
    Reminder,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Note => "note",
            MemoryKind::Todo => "todo",
            MemoryKind::Reminder => "reminder",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fact" => MemoryKind::Fact,
            "note" => MemoryKind::Note,
            "todo" => MemoryKind::Todo,
            "reminder" => MemoryKind::Reminder,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItem {
    pub id: i64,
    pub kind: MemoryKind,
    pub text: String,
    /// todos only
    pub done: bool,
    /// reminders only: ISO timestamp
    pub due_at: Option<String>,
    /// screenshot saved with "remember this"
    pub image_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Create (no id) or update a memory item. `None` on an update keeps the current value;
/// `due_at` / `image_path` use `Some(None)` to clear.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpsert {
    #[serde(default)]
    pub id: Option<i64>,
    pub kind: Option<MemoryKind>,
    pub text: String,
    #[serde(default)]
    pub done: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub due_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub image_path: Option<Option<String>>,
}

/// Distinguishes "field missing" (None) from "field: null" (Some(None)).
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    /// "HH:MM" 24-hour, local time
    pub time: String,
    /// 0 = Sunday … 6 = Saturday
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: i64,
    pub name: String,
    pub triggers: Vec<String>,
    pub instructions: String,
    pub schedule: Option<Schedule>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineUpsert {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    pub instructions: String,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimerKind {
    Timer,
    Alarm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerInfo {
    pub id: String,
    pub label: String,
    pub kind: TimerKind,
    /// epoch ms when it goes off
    pub ends_at: i64,
    /// total length in ms, for drawing progress
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatAppKind {
    People,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verify {
    Title,
    Screenshot,
}

/// How Jarvis drives a messaging / AI-chat app with the keyboard (see main/tools/chat-apps.ts).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatAppProfile {
    pub id: String,
    pub kind: ChatAppKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub process_names: Vec<String>,
    pub launch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_wait_ms: Option<u64>,
    #[serde(default)]
    pub search_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_wait_ms: Option<u64>,
    #[serde(default)]
    pub pick_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_chat_keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_message_keys: Option<Vec<String>>,
    pub verify: Verify,
    #[serde(default)]
    pub send_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceOption {
    pub voice_id: String,
    pub name: String,
    pub description: String,
    pub preview_url: Option<String>,
}
