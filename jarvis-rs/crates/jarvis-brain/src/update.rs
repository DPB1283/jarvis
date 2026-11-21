//! Updates: look for a newer GitHub release, and on Windows download and run its installer.
//!
//! Safety: the release is read from this project's GitHub API over HTTPS, the installer must come from this
//! project's release URL, and it is only run if its SHA-256 matches the digest GitHub reports for the asset.

use crate::Ctx;
use anyhow::{Context, Result, bail, ensure};
use jarvis_core::Event;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::Duration;

const REPO: &str = "DreadBear1283/jarvis";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page_url: String,
    pub installer_url: Option<String>,
    pub sha256: Option<String>,
}

static LATEST: Mutex<Option<Release>> = Mutex::new(None);

/// "v1.2.3" or "1.2.3" (anything after a "-" is ignored) as numbers.
pub(crate) fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let core = text.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?))
}

pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// The only place an installer may be downloaded from.
pub(crate) fn trusted_installer_url(url: &str) -> bool {
    url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) && url.ends_with(".exe") && !url.contains("..")
}

pub(crate) fn parse_release(json: &Value) -> Option<Release> {
    let version = json["tag_name"].as_str()?.trim_start_matches(['v', 'V']).to_owned();
    parse_version(&version)?;
    let installer = json["assets"].as_array()?.iter().find(|a| {
        let name = a["name"].as_str().unwrap_or("").to_lowercase();
        name.contains("setup") && name.ends_with(".exe")
    });
    let installer_url = installer.and_then(|a| a["browser_download_url"].as_str()).filter(|u| trusted_installer_url(u)).map(str::to_owned);
    let sha256 = installer.and_then(|a| a["digest"].as_str()).and_then(|d| d.strip_prefix("sha256:")).filter(|d| d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_lowercase);
    Some(Release {
        version,
        notes: json["body"].as_str().unwrap_or("").chars().take(1500).collect(),
        page_url: json["html_url"].as_str().filter(|u| u.starts_with("https://github.com/")).unwrap_or("https://github.com/DreadBear1283/jarvis/releases").to_owned(),
        installer_url,
        sha256,
    })
}

fn status_json(release: Option<&Release>) -> Value {
    let available = release.is_some_and(|r| is_newer(&r.version, CURRENT));
    json!({
        "current": CURRENT,
        "latest": release.map(|r| r.version.clone()),
        "available": available,
        "notes": release.filter(|_| available).map(|r| r.notes.clone()).unwrap_or_default(),
        // Windows can install in place; elsewhere the release page is where to get it.
        "installable": available && cfg!(windows) && release.is_some_and(|r| r.installer_url.is_some() && r.sha256.is_some()),
    })
}

/// What is known without going to the network.
pub fn status() -> Value {
    status_json(LATEST.lock().unwrap().as_ref())
}

pub async fn check() -> Result<Value> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Jarvis update check")
        .build()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("Could not reach GitHub to look for updates")?;
    ensure!(response.status().is_success(), "GitHub answered {} when looking for updates", response.status());
    let release = parse_release(&response.json::<Value>().await?).context("The latest release looked unusual")?;
    *LATEST.lock().unwrap() = Some(release);
    Ok(status())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Download the installer, check its checksum, and run it. Windows only.
pub async fn install() -> Result<()> {
    ensure!(cfg!(windows), "Automatic install is only available on Windows. Download the new version from the releases page.");
    let release = LATEST.lock().unwrap().clone().context("Check for updates first")?;
    ensure!(is_newer(&release.version, CURRENT), "You already have the latest version");
    let url = release.installer_url.context("This release has no Windows installer")?;
    let expected = release.sha256.context("This release has no checksum, so it can't be installed automatically")?;
    ensure!(trusted_installer_url(&url), "The installer is not from the project's releases");

    let response = reqwest::Client::builder().timeout(Duration::from_secs(600)).user_agent("Jarvis update").build()?.get(&url).send().await?.error_for_status()?;
    ensure!(response.content_length().is_none_or(|n| n < 400 * 1024 * 1024), "The installer is unexpectedly large");
    let bytes = response.bytes().await?;
    ensure!(sha256_hex(&bytes) == expected, "The download did not match its checksum, so I did not run it");

    let path = std::env::temp_dir().join(format!("Jarvis-Setup-{}.exe", release.version));
    std::fs::write(&path, &bytes)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Detached, so it keeps going when the installer closes this app. /LAUNCH starts Jarvis again at the end.
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200 | 0x0800_0000;
        std::process::Command::new(&path)
            .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/LAUNCH"])
            .creation_flags(DETACHED)
            .spawn()
            .context("Could not start the installer")?;
    }
    #[cfg(not(windows))]
    bail!("Automatic install is only available on Windows");
    #[cfg(windows)]
    Ok(())
}

/// Check at start-up (after a short delay) and every few hours; tell the HUD once per new version.
pub async fn run(ctx: Ctx) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    let mut told = String::new();
    loop {
        if ctx.settings.get().updates.check {
            if let Ok(status) = check().await {
                let latest = status["latest"].as_str().unwrap_or_default().to_owned();
                if status["available"] == true && latest != told {
                    told = latest.clone();
                    let release = LATEST.lock().unwrap().clone();
                    ctx.bus.emit(Event::UpdateAvailable {
                        version: latest,
                        notes: status["notes"].as_str().unwrap_or_default().to_owned(),
                        url: release.map(|r| r.page_url).unwrap_or_default(),
                    });
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically_not_alphabetically() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.10.0", "0.9.0"), "10 is more than 9");
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
    }

    fn release_json() -> Value {
        json!({
            "tag_name": "v0.2.0", "body": "What is new", "html_url": "https://github.com/DreadBear1283/jarvis/releases/tag/v0.2.0",
            "assets": [
                {"name": "notes.txt", "browser_download_url": "https://github.com/DreadBear1283/jarvis/releases/download/v0.2.0/notes.txt"},
                {"name": "Jarvis-Setup-0.2.0.exe", "browser_download_url": "https://github.com/DreadBear1283/jarvis/releases/download/v0.2.0/Jarvis-Setup-0.2.0.exe",
                 "digest": "sha256:3a347c8859685ed9a7c8d45b6cf699f6720508f432e1716a7d0d134872faadd8"}
            ]
        })
    }

    #[test]
    fn a_release_is_read_with_its_installer_and_checksum() {
        let release = parse_release(&release_json()).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert!(release.installer_url.as_deref().unwrap().ends_with("Jarvis-Setup-0.2.0.exe"));
        assert_eq!(release.sha256.as_deref().unwrap().len(), 64);
        let status = status_json(Some(&release));
        assert_eq!(status["available"], true);
        assert_eq!(status["installable"], cfg!(windows));
        assert_eq!(status["notes"], "What is new");
        assert_eq!(status_json(None)["available"], false);
    }

    #[test]
    fn installers_from_anywhere_else_are_ignored() {
        let mut evil = release_json();
        evil["assets"][1]["browser_download_url"] = json!("https://evil.example/Jarvis-Setup-0.2.0.exe");
        let release = parse_release(&evil).unwrap();
        assert!(release.installer_url.is_none(), "not from this project's releases");
        assert!(status_json(Some(&release))["installable"] == false);
        assert!(!trusted_installer_url("https://github.com/someone-else/jarvis/releases/download/v1/Jarvis-Setup.exe"));
        assert!(!trusted_installer_url("http://github.com/DreadBear1283/jarvis/releases/download/v1/Jarvis-Setup.exe"));
        assert!(!trusted_installer_url("https://github.com/DreadBear1283/jarvis/releases/download/v1/../../x.exe"));

        let mut no_digest = release_json();
        no_digest["assets"][1].as_object_mut().unwrap().remove("digest");
        assert!(status_json(parse_release(&no_digest).as_ref())["installable"] == false, "no checksum, no automatic install");
        let mut bad_digest = release_json();
        bad_digest["assets"][1]["digest"] = json!("sha256:xyz");
        assert!(parse_release(&bad_digest).unwrap().sha256.is_none());
    }

    #[test]
    fn checksums_are_computed_correctly() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[tokio::test]
    #[ignore = "asks the real GitHub API"]
    async fn the_real_latest_release_can_be_read() {
        let status = check().await.unwrap();
        println!("{status}");
        assert_eq!(status["current"], CURRENT);
        assert!(status["latest"].is_string());
    }
}
