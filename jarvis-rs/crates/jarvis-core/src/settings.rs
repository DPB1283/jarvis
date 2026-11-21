//! Mirrors `Settings` / `DEFAULT_SETTINGS` in `jarvis/src/shared/settings.ts` (same JSON, same file).
//! Loading is lenient: a section that fails to parse falls back to its default instead of losing the file.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bus::Bus;
use crate::events::Event;
use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SafetyMode {
    AskAll,
    #[default]
    AskUnsafe,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReplyLength {
    #[default]
    Brief,
    /// One to two short sentences.
    Middle,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    #[default]
    BelowOrb,
    BottomCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrbCorner {
    #[default]
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Metric,
    #[default]
    Imperial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NewsSources {
    #[default]
    Neutral,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GameModeSetting {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClipApp {
    #[default]
    Auto,
    Medal,
    Nvidia,
    Xbox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Captions {
    pub enabled: bool,
    pub position: CaptionPosition,
    pub size: f64,
}
impl Default for Captions {
    fn default() -> Self {
        Self { enabled: true, position: CaptionPosition::BelowOrb, size: 15.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Voice {
    /// ElevenLabs voice id. Empty = ELEVENLABS_VOICE_ID from .env.
    pub voice_id: String,
    pub speed: f64,
    pub stability: f64,
    pub volume: f64,
    pub muted: bool,
    /// false = text only: no microphone, wake word, speech recognition or spoken replies (no ElevenLabs).
    pub enabled: bool,
}
impl Default for Voice {
    fn default() -> Self {
        // Daniel: ElevenLabs premade British male voice.
        Self { voice_id: "onwK4e9ZLuTAKqWW03F9".into(), speed: 1.0, stability: 0.55, volume: 100.0, muted: false, enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Personality {
    pub sarcasm: f64,
    pub reply_length: ReplyLength,
}
impl Default for Personality {
    fn default() -> Self {
        Self { sarcasm: 5.0, reply_length: ReplyLength::Brief }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Audio {
    pub input_gain: f64,
    /// Microphone name. "" = Windows default.
    pub input_device: String,
    /// Speaker label. "" = Windows default.
    pub output_device: String,
    /// Browser deviceId from the Electron app (ignored by native playback; label is used).
    pub output_device_id: String,
}

impl Default for Audio {
    fn default() -> Self { Self { input_gain: 100.0, input_device: String::new(), output_device: String::new(), output_device_id: String::new() } }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Conversation {
    pub follow_up_seconds: f64,
    pub end_of_speech_seconds: f64,
    /// Starting to talk while Jarvis is speaking cuts him off.
    pub interrupt_by_speaking: bool,
}
impl Default for Conversation {
    fn default() -> Self {
        Self { follow_up_seconds: 60.0, end_of_speech_seconds: 1.2, interrupt_by_speaking: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Location {
    pub city: String,
    pub units: Units,
    pub news_region: String,
    pub news_sources: NewsSources,
    /// Allow Jarvis to show maps (Mapbox) on screen.
    pub show_maps: bool,
}
impl Default for Location {
    fn default() -> Self {
        let region = std::env::var("JARVIS_REGION").unwrap_or_else(|_| "US".into()).to_uppercase();
        let units = if ["US", "LR", "MM"].contains(&region.as_str()) { Units::Imperial } else { Units::Metric };
        Self { city: String::new(), units, news_region: region, news_sources: NewsSources::Neutral, show_maps: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GameMode {
    pub mode: GameModeSetting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Health {
    pub warnings: bool,
}
impl Default for Health {
    fn default() -> Self {
        Self { warnings: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Calendar {
    /// Google Calendar "secret address in iCal format". Never log it.
    pub ics_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Clips {
    pub app: ClipApp,
    /// Overrides the app's default hotkey (xdotool-style). "" = default.
    pub hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WakeWord {
    pub enabled: bool,
    pub sensitivity: f64,
}
impl Default for WakeWord {
    fn default() -> Self {
        Self { enabled: true, sensitivity: 0.6 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    /// Electron accelerator syntax, e.g. "Control+Space".
    pub talk: String,
    pub settings: String,
    /// Mute / unmute Jarvis's spoken replies.
    pub mute: String,
}
impl Default for Hotkeys {
    fn default() -> Self {
        Self { talk: "Control+Space".into(), settings: "Control+Alt+J".into(), mute: "Control+Alt+M".into() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Orb {
    pub corner: OrbCorner,
    /// Where the user dragged the orb (screen DIP, top-left). None = snap to `corner`.
    pub position: Option<Point>,
    pub size: f64,
    pub opacity: f64,
    /// true = Jarvis is visible in screen shares, recordings and screenshots; false hides him from them.
    pub show_in_captures: bool,
}
impl Default for Orb {
    fn default() -> Self {
        Self { corner: OrbCorner::TopRight, position: None, size: 96.0, opacity: 0.95, show_in_captures: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub ultron_mode: bool,
    pub user_title: String,
    pub user_name: String,
    pub captions: Captions,
    pub voice: Voice,
    pub personality: Personality,
    pub audio: Audio,
    pub conversation: Conversation,
    pub location: Location,
    pub game_mode: GameMode,
    pub health: Health,
    pub calendar: Calendar,
    pub clips: Clips,
    pub safety_mode: SafetyMode,
    pub wake_word: WakeWord,
    pub hotkeys: Hotkeys,
    pub orb: Orb,
    pub launch_at_startup: bool,
    /// Which AI answers: Claude (default) or any OpenAI-compatible service.
    pub provider: Provider,
    /// Language Jarvis speaks and understands: en, es, fr, de or yue (Cantonese).
    pub language: String,
}

/// The AI behind the assistant. `kind` "anthropic" uses the Anthropic key; "openai" talks to any
/// OpenAI-compatible chat-completions service (OpenAI, Google Gemini, OpenRouter, Groq, a local Ollama...) at
/// `base_url` with `model`, using the key saved as AI_API_KEY (local servers need none).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    pub kind: String,
    pub base_url: String,
    pub model: String,
}
impl Default for Provider {
    fn default() -> Self {
        Self { kind: "anthropic".into(), base_url: String::new(), model: String::new() }
    }
}
impl Provider {
    pub fn is_openai(&self) -> bool {
        self.kind == "openai"
    }
}

/// The languages Jarvis supports: (code, English name). The same codes ElevenLabs speech recognition and
/// `eleven_v4_turbo` accept.
pub const LANGUAGES: [(&str, &str); 5] = [("en", "English"), ("es", "Spanish"), ("fr", "French"), ("de", "German"), ("yue", "Cantonese")];

impl Settings {
    /// The language code, falling back to English for anything unknown.
    pub fn language_code(&self) -> &'static str {
        LANGUAGES.iter().find(|(code, _)| *code == self.language).map_or("en", |(code, _)| code)
    }

    pub fn language_name(&self) -> &'static str {
        LANGUAGES.iter().find(|(code, _)| *code == self.language).map_or("English", |(_, name)| name)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ultron_mode: false,
            user_title: "sir".into(),
            user_name: String::new(),
            captions: Captions::default(),
            voice: Voice::default(),
            personality: Personality::default(),
            audio: Audio::default(),
            conversation: Conversation::default(),
            location: Location::default(),
            game_mode: GameMode::default(),
            health: Health::default(),
            calendar: Calendar::default(),
            clips: Clips::default(),
            safety_mode: SafetyMode::AskUnsafe,
            wake_word: WakeWord::default(),
            hotkeys: Hotkeys::default(),
            orb: Orb::default(),
            launch_at_startup: false,
            provider: Provider::default(),
            language: "en".into(),
        }
    }
}

/// Same merge as the TypeScript `merge()`: object sections merge one level deep; everything else replaces.
pub fn merge(base: &mut Value, patch: &Value) {
    let (Some(base), Some(patch)) = (base.as_object_mut(), patch.as_object()) else { return };
    for (k, v) in patch {
        if v.is_null() && k != "position" {
            continue;
        }
        match (base.get_mut(k), v) {
            (Some(Value::Object(cur)), Value::Object(p)) => {
                for (pk, pv) in p {
                    cur.insert(pk.clone(), pv.clone());
                }
            }
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Parse settings JSON leniently: start from defaults, apply each section on its own, skip sections that don't fit.
pub fn from_json_lenient(raw: &Value) -> Settings {
    let defaults = serde_json::to_value(Settings::default()).expect("defaults serialize");
    let mut good = defaults.clone();
    if let Some(obj) = raw.as_object() {
        for (k, v) in obj {
            let mut attempt = good.clone();
            let mut one = Map::new();
            one.insert(k.clone(), v.clone());
            merge(&mut attempt, &Value::Object(one));
            if serde_json::from_value::<Settings>(attempt.clone()).is_ok() {
                good = attempt;
            } else {
                tracing::warn!("settings: ignoring invalid section '{k}'");
            }
        }
    }
    serde_json::from_value(good).unwrap_or_default()
}

/// Thread-safe settings with persistence and change events.
#[derive(Clone)]
pub struct SettingsStore {
    inner: Arc<RwLock<Settings>>,
    path: PathBuf,
    bus: Bus,
}

impl SettingsStore {
    pub fn load(bus: Bus) -> Self {
        let path = paths::data_dir().join("settings.json");
        let settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok())
            .map(|v| from_json_lenient(&v))
            .unwrap_or_default();
        Self { inner: Arc::new(RwLock::new(settings)), path, bus }
    }

    /// A snapshot of the current settings.
    pub fn get(&self) -> Settings {
        self.inner.read().clone()
    }

    /// Apply a `SettingsPatch` (same JSON the renderer sends), save, and emit `settings_changed`.
    pub fn update(&self, patch: &Value) -> anyhow::Result<Settings> {
        let next = {
            let mut cur = serde_json::to_value(&*self.inner.read())?;
            merge(&mut cur, patch);
            serde_json::from_value::<Settings>(cur)?
        };
        std::fs::write(&self.path, serde_json::to_string_pretty(&next)?)?;
        *self.inner.write() = next.clone();
        self.bus.emit(Event::SettingsChanged { settings: Box::new(next.clone()) });
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_electron_settings_shape() {
        let raw = serde_json::json!({
            "userTitle": "boss", "voice": { "voiceId": "abc" }, "orb": { "corner": "bottom_left", "position": { "x": 10, "y": 20 } },
            "safetyMode": "full_access", "location": { "city": "Seattle", "units": "metric" }, "captions": { "position": "nonsense" }
        });
        let s = from_json_lenient(&raw);
        assert_eq!(s.user_title, "boss");
        assert_eq!(s.voice.voice_id, "abc");
        assert_eq!(s.voice.speed, 1.0); // missing field → default
        assert_eq!(s.orb.corner, OrbCorner::BottomLeft);
        assert_eq!(s.orb.position, Some(Point { x: 10.0, y: 20.0 }));
        assert_eq!(s.safety_mode, SafetyMode::FullAccess);
        assert_eq!(s.location.units, Units::Metric);
        assert_eq!(s.captions.position, CaptionPosition::BelowOrb); // invalid section ignored
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["orb"]["position"]["x"], 10.0);
        assert_eq!(json["userTitle"], "boss");
    }

    #[test]
    fn patch_merges_one_level() {
        let mut base = serde_json::to_value(Settings::default()).unwrap();
        merge(&mut base, &serde_json::json!({ "voice": { "muted": true }, "orb": { "position": null } }));
        let s: Settings = serde_json::from_value(base).unwrap();
        assert!(s.voice.muted);
        assert_eq!(s.voice.voice_id, "onwK4e9ZLuTAKqWW03F9");
        assert_eq!(s.orb.position, None);
    }

    #[test]
    fn old_settings_files_get_the_default_mute_hotkey() {
        let s: Settings = serde_json::from_str(r#"{"hotkeys":{"talk":"Control+Space","settings":"Control+Alt+J"}}"#).unwrap();
        assert_eq!(s.hotkeys.mute, "Control+Alt+M");
        assert_eq!(s.hotkeys.talk, "Control+Space");
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn language_defaults_to_english_and_rejects_unknown_codes() {
        let mut s = Settings::default();
        assert_eq!((s.language_code(), s.language_name()), ("en", "English"));
        for (code, name) in [("es", "Spanish"), ("fr", "French"), ("de", "German"), ("yue", "Cantonese")] {
            s.language = code.into();
            assert_eq!((s.language_code(), s.language_name()), (code, name));
        }
        s.language = "xx".into();
        assert_eq!(s.language_code(), "en");
        // Older settings files without the field still load.
        let old: Settings = serde_json::from_str(r#"{"userTitle":"sir"}"#).unwrap();
        assert_eq!(old.language, "en");
    }
}
