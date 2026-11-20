//! Mirrors `jarvis/src/shared/events.ts` (BusEvents). Serialized JSON is identical:
//! `{"type":"user_said","text":"hi","final":true}`, `{"type":"game_mode","active":true,"app":"cs2","hideHud":false}`.

use serde::{Deserialize, Serialize};

use crate::model::TimerInfo;
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JarvisState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WakeSource {
    Wakeword,
    Hotkey,
    Followup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    User,
    BargeIn,
    NewTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LevelSource {
    Mic,
    Tts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Safe,
    Unsafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastLevel {
    Info,
    Warn,
    Error,
}

/// Everything that happens in Jarvis. Variant and field names match the TypeScript contract exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Event {
    /// voice → brain: wake word heard, talk hotkey pressed, or speech during the follow-up window.
    Wake { source: WakeSource },
    /// voice → renderer: the post-reply conversation window (`until` = epoch ms).
    Followup { active: bool, until: i64 },
    /// voice → brain: live transcript; `final` ends the utterance.
    UserSaid {
        text: String,
        #[serde(rename = "final")]
        is_final: bool,
    },
    /// renderer → brain: typed input.
    UserText { text: String },
    /// brain → voice: a sentence to speak.
    Speak { text: String },
    /// brain → voice: no more text for this reply.
    SpeakEnd {},
    /// anyone → everyone: stop talking / cancel the turn.
    Interrupt { reason: InterruptReason },
    /// voice → renderer: loudness 0..1 (mic while listening, tts while speaking), ~25 Hz.
    AudioLevel { level: f32, source: LevelSource },
    /// orb state.
    State { state: JarvisState },
    /// brain → renderer + voice: needs a yes/no.
    ApprovalRequest { id: String, tool: String, summary: String, risk: Risk },
    ApprovalResponse { id: String, ok: bool },
    /// brain → renderer: streaming reply text for captions.
    AssistantDelta { text: String },
    AssistantDone { text: String },
    ReminderDue { id: i64, text: String },
    Toast { level: ToastLevel, text: String },
    SettingsChanged { settings: Box<Settings> },
    RoutinesChanged {},
    VirtualPointer { x: i32, y: i32 },
    /// Full list of running timers whenever it changes.
    Timers { timers: Vec<TimerInfo> },
    TimerDone { timer: TimerInfo },
    /// Quiet behaviour follows `active`; the HUD hides only when `hide_hud` (orb on the game's screen).
    GameMode { active: bool, app: Option<String>, hide_hud: bool },
}

impl Event {
    /// The `type` string, e.g. "user_said".
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Wake { .. } => "wake",
            Event::Followup { .. } => "followup",
            Event::UserSaid { .. } => "user_said",
            Event::UserText { .. } => "user_text",
            Event::Speak { .. } => "speak",
            Event::SpeakEnd {} => "speak_end",
            Event::Interrupt { .. } => "interrupt",
            Event::AudioLevel { .. } => "audio_level",
            Event::State { .. } => "state",
            Event::ApprovalRequest { .. } => "approval_request",
            Event::ApprovalResponse { .. } => "approval_response",
            Event::AssistantDelta { .. } => "assistant_delta",
            Event::AssistantDone { .. } => "assistant_done",
            Event::ReminderDue { .. } => "reminder_due",
            Event::Toast { .. } => "toast",
            Event::SettingsChanged { .. } => "settings_changed",
            Event::RoutinesChanged {} => "routines_changed",
            Event::VirtualPointer { .. } => "virtual_pointer",
            Event::Timers { .. } => "timers",
            Event::TimerDone { .. } => "timer_done",
            Event::GameMode { .. } => "game_mode",
        }
    }

    /// Events forwarded to renderer windows (`RENDERER_EVENTS` in events.ts).
    pub fn for_renderer(&self) -> bool {
        !matches!(self, Event::VirtualPointer { .. } | Event::UserText { .. } | Event::Speak { .. } | Event::SpeakEnd {} | Event::Interrupt { .. })
    }

    /// Events a renderer is allowed to send (`RENDERER_SENDABLE` in events.ts).
    pub fn renderer_may_send(&self) -> bool {
        matches!(self, Event::UserText { .. } | Event::ApprovalResponse { .. } | Event::Interrupt { .. } | Event::Wake { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_matches_typescript_contract() {
        let e = Event::UserSaid { text: "hi".into(), is_final: true };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"user_said","text":"hi","final":true}"#);
        let e = Event::GameMode { active: true, app: Some("cs2".into()), hide_hud: false };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"game_mode","active":true,"app":"cs2","hideHud":false}"#);
        let e = Event::Interrupt { reason: InterruptReason::BargeIn };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"interrupt","reason":"barge_in"}"#);
        assert_eq!(serde_json::to_string(&Event::SpeakEnd {}).unwrap(), r#"{"type":"speak_end"}"#);
        let back: Event = serde_json::from_str(r#"{"type":"approval_response","id":"x","ok":true}"#).unwrap();
        assert!(matches!(back, Event::ApprovalResponse { ok: true, .. }));
        let back: Event = serde_json::from_str(r#"{"type":"wake","source":"hotkey"}"#).unwrap();
        assert!(back.renderer_may_send());
    }
}
