//! Jarvis core: the contract every other crate builds on.
//!
//! - [`events::Event`]: everything that happens, as one serde-tagged enum whose JSON is identical to the
//!   TypeScript `BusEvents` / `RendererEvent` (so the existing renderer works unchanged).
//! - [`bus::Bus`]: in-process broadcast of events (voice ↔ brain ↔ windows).
//! - [`settings`]: the user's settings (same `settings.json` as the Electron app).
//! - [`model`]: shared data types (memory items, routines, timers, chat-app profiles, voices).
//! - [`paths`]: where data, models and API keys live.
//! - [`db`]: SQLite storage (same schema as the Electron app's `jarvis.sqlite`).

pub mod bus;
pub mod db;
pub mod events;
pub mod model;
pub mod paths;
pub mod settings;
pub mod speech;

pub use bus::Bus;
pub use events::Event;
pub use settings::{Settings, SettingsStore};
pub mod secrets;
pub mod ultron;
