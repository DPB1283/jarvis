//! Where things live. During the port, dev builds share the Electron app's files so nothing has to be
//! set up twice: `jarvis/data` (settings, memory, timers), `jarvis/models` (Vosk) and `jarvis/.env.local`
//! + `jarvis/.env` (API keys). Override with JARVIS_HOME (the Electron project dir) or JARVIS_DATA_DIR.
//! Never run the Electron and Rust apps at the same time against the same data.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The Electron project directory (`…/idk yet/jarvis`) that holds data, models and .env files.
pub fn jarvis_home() -> PathBuf {
    if let Ok(p) = std::env::var("JARVIS_HOME") {
        return PathBuf::from(p);
    }
    // crates/jarvis-core → jarvis-rs → workspace root → jarvis
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("jarvis");
    if cfg!(debug_assertions) && dev.exists() {
        return dev.canonicalize().unwrap_or(dev);
    }
    // Installed builds: the per-user settings folder (see `config_root`).
    config_root().map(|root| root.join("Jarvis")).unwrap_or_else(|| PathBuf::from("."))
}

/// Per-user settings folder: %APPDATA% on Windows, $XDG_CONFIG_HOME (or ~/.config) elsewhere.
fn config_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
}

pub fn data_dir() -> PathBuf {
    let dir = std::env::var("JARVIS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| jarvis_home().join("data"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn packaged_resources() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.to_owned())
}

pub fn models_dir() -> PathBuf {
    let packaged = packaged_resources().map(|dir| dir.join("models"));
    packaged
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| jarvis_home().join("models"))
}

pub fn vosk_model_dir() -> PathBuf {
    models_dir().join("vosk-model-small-en-us-0.15")
}

/// Load API keys once: `.env.local` (written by the ElevenLabs onboarding) wins over `.env`.
/// Existing environment variables are never overridden.
pub fn load_env() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let home = jarvis_home();
        for file in [".env.local", ".env"] {
            let _ = dotenvy::from_path(home.join(file));
        }
    });
}

/// An API key / secret from the environment (after [`load_env`]), or `None` if unset or empty.
pub fn secret(name: &str) -> Option<String> {
    // Keys typed into Settings win; the environment / .env files are the fallback, unless the user cleared
    // the key on purpose.
    match crate::secrets::lookup(name) {
        crate::secrets::Lookup::Value(saved) => return Some(saved),
        crate::secrets::Lookup::Cleared => return None,
        crate::secrets::Lookup::Unset => {}
    }
    load_env();
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
