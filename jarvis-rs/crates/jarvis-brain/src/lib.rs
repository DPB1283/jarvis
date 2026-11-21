//! Jarvis's brain. Owns the shared [`Ctx`] (bus, settings, storage), the renderer IPC handlers
//! ([`ipc::handle_invoke`]), and (M2) the Claude agent loop and tools.
//!
//! How the app crate uses it:
//! ```ignore
//! let ctx = jarvis_brain::Ctx::new(bus.clone())?;      // loads settings + opens jarvis.sqlite
//! let brain = jarvis_brain::start(ctx.clone());         // spawns the brain's bus listeners (M2)
//! // in the `jarvis_invoke` Tauri command, for non-window channels:
//! match jarvis_brain::ipc::handle_invoke(&ctx, &channel, args).await {
//!     Ok(value) => Ok(value),
//!     Err(InvokeError::Unhandled) => /* voice:* / audio:* → jarvis_voice; window:* → app */,
//!     Err(InvokeError::Failed(e)) => Err(e.to_string()),   // shown to the user by the renderer
//! }
//! ```

mod agent;
pub mod anthropic;
pub mod browser;
pub mod chunker;
pub mod dismiss;
mod free;
pub mod ipc;
mod openai;
pub mod persona;
pub mod policy;
pub mod provider;
mod schedulers;
mod timers;
pub mod tools;

use anyhow::Result;
use jarvis_core::db::Db;
use jarvis_core::{Bus, SettingsStore};

/// Shared state handed to every part of the backend. Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    pub bus: Bus,
    pub settings: SettingsStore,
    pub db: Db,
    pub timers: timers::Timers,
    pub browser: std::sync::Arc<browser::BrowserBridge>,
}

impl Ctx {
    pub fn new(bus: Bus) -> Result<Self> {
        jarvis_core::paths::load_env();
        // Keys set up the old way (.env files) show up in Settings and keep working.
        jarvis_core::secrets::import_env();
        let settings = SettingsStore::load(bus.clone());
        let db = Db::open()?;
        let timers = timers::Timers::load(
            jarvis_core::paths::data_dir().join("timers.json"),
            bus.clone(),
        )?;
        let browser = browser::BrowserBridge::new()?;
        Ok(Self {
            browser,
            bus,
            settings,
            db,
            timers,
        })
    }
}

/// Handle to the running brain (agent loop, schedulers). Dropping it does not stop anything;
/// call [`BrainHandle::shutdown`] on app exit.
pub struct BrainHandle {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl BrainHandle {
    pub fn shutdown(self) {
        for t in self.tasks {
            t.abort();
        }
    }
}

/// Start the brain's background work on the current tokio runtime.
/// Runs conversations, timers, reminders and scheduled routines.
pub fn start(ctx: Ctx) -> BrainHandle {
    BrainHandle {
        tasks: vec![
            agent::start(ctx.clone()),
            tokio::spawn(ctx.browser.clone().run()),
            tokio::spawn(tools::activity::watch(ctx.clone())),
            tokio::spawn(tools::activity::usage()),
            tokio::spawn(timers::run(ctx.clone())),
            tokio::spawn(schedulers::reminders(ctx)),
        ],
    }
}
