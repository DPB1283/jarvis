//! API keys the user types into Settings. Stored per Windows user, encrypted with DPAPI, in
//! `<data dir>/secrets.bin`; never in the repo, never sent to the renderer. Environment / `.env` values
//! are only a fallback (and are imported once so existing installs just keep working).

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The only names that can be stored.
pub const NAMES: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "ELEVENLABS_API_KEY",
    "XWEATHER_CLIENT_ID",
    "XWEATHER_CLIENT_SECRET",
];

static STORE: Mutex<Option<BTreeMap<String, String>>> = Mutex::new(None);

fn file() -> PathBuf {
    crate::paths::data_dir().join("secrets.bin")
}

#[cfg(windows)]
fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if encrypt {
            CryptProtectData(&input, None, None, None, None, 0, &mut output)?;
        } else {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut output)?;
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(bytes)
    }
}

#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>> {
    bail!("Secure key storage needs Windows")
}

fn read() -> BTreeMap<String, String> {
    std::fs::read(file())
        .ok()
        .and_then(|bytes| protect(&bytes, false).ok())
        .and_then(|plain| serde_json::from_slice(&plain).ok())
        .unwrap_or_default()
}

fn with<T>(f: impl FnOnce(&mut BTreeMap<String, String>) -> T) -> T {
    let mut guard = STORE.lock();
    f(guard.get_or_insert_with(read))
}

fn save(map: &BTreeMap<String, String>) -> Result<()> {
    let encrypted = protect(&serde_json::to_vec(map)?, true)?;
    let path = file();
    let temp = path.with_extension("bin.tmp");
    std::fs::write(&temp, encrypted).context("Could not write the key store")?;
    std::fs::rename(&temp, &path).context("Could not save the key store")?;
    Ok(())
}

fn known(name: &str) -> Result<()> {
    if NAMES.contains(&name) {
        Ok(())
    } else {
        bail!("Unknown key name")
    }
}

/// What Settings knows about a key. `Cleared` (an empty entry) means the user removed it on purpose, so the
/// environment / `.env` fallback stays off until they set a key again.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    Value(String),
    Cleared,
    Unset,
}

pub fn lookup(name: &str) -> Lookup {
    with(|map| match map.get(name) {
        Some(value) if !value.trim().is_empty() => Lookup::Value(value.clone()),
        Some(_) => Lookup::Cleared,
        None => Lookup::Unset,
    })
}

/// A key saved in Settings (not the environment).
pub fn get(name: &str) -> Option<String> {
    match lookup(name) {
        Lookup::Value(value) => Some(value),
        _ => None,
    }
}

fn update(name: &str, value: String) -> Result<()> {
    with(|map| {
        let mut next = map.clone();
        next.insert(name.to_owned(), value);
        save(&next)?;
        *map = next;
        Ok(())
    })
}

pub fn set(name: &str, value: &str) -> Result<()> {
    known(name)?;
    let value = value.trim();
    if value.is_empty() || value.len() > 500 || value.chars().any(char::is_control) {
        bail!("That does not look like a valid key");
    }
    update(name, value.to_owned())
}

/// Remove a key and keep the environment fallback off for it until it is set again.
pub fn clear(name: &str) -> Result<()> {
    known(name)?;
    update(name, String::new())
}

/// Which keys are available, from Settings or the environment. Never returns the values.
pub fn status() -> BTreeMap<String, bool> {
    NAMES.iter().map(|name| ((*name).to_owned(), crate::paths::secret(name).is_some())).collect()
}

/// Copy keys found in the environment / `.env` files into the encrypted store once, so a user who set
/// them up the old way sees them in Settings. Keys already saved or deliberately cleared are left alone.
pub fn import_env() {
    crate::paths::load_env();
    for name in NAMES {
        if lookup(name) == Lookup::Unset
            && let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty())
        {
            if let Err(error) = set(name, &value) {
                tracing::warn!(%error, name, "Could not import a key");
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_hides_the_value() {
        let secret = b"sk-test-1234567890";
        let encrypted = protect(secret, true).unwrap();
        assert!(!encrypted.windows(secret.len()).any(|w| w == secret));
        assert_eq!(protect(&encrypted, false).unwrap(), secret);
    }

    #[test]
    fn rejects_unknown_names_and_junk() {
        assert!(set("PATH", "x").is_err());
        assert!(set("ANTHROPIC_API_KEY", "  ").is_err());
        assert!(set("ANTHROPIC_API_KEY", "bad\nkey").is_err());
        assert!(clear("PATH").is_err());
    }

    #[test]
    fn cleared_keys_do_not_fall_back_to_the_environment() {
        let dir = std::env::temp_dir().join(format!("jarvis-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("JARVIS_DATA_DIR", &dir);
            std::env::set_var("XWEATHER_CLIENT_ID", "from-env");
        }
        *STORE.lock() = None;
        assert_eq!(crate::paths::secret("XWEATHER_CLIENT_ID").as_deref(), Some("from-env"));
        set("XWEATHER_CLIENT_ID", "saved").unwrap();
        assert_eq!(crate::paths::secret("XWEATHER_CLIENT_ID").as_deref(), Some("saved"));
        clear("XWEATHER_CLIENT_ID").unwrap();
        assert_eq!(crate::paths::secret("XWEATHER_CLIENT_ID"), None, "env fallback must stay off");
        import_env();
        assert_eq!(crate::paths::secret("XWEATHER_CLIENT_ID"), None, "import must not undo a clear");
        set("XWEATHER_CLIENT_ID", "again").unwrap();
        assert_eq!(crate::paths::secret("XWEATHER_CLIENT_ID").as_deref(), Some("again"));
        std::fs::remove_dir_all(dir).ok();
    }
}
