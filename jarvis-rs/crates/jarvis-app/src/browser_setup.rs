//! Helps the user add the Jarvis browser extension. Chrome and Edge only accept an unpacked extension by hand
//! (Developer mode, then Load unpacked), so Jarvis opens the browser's extensions page and the extension folder,
//! and puts the folder's path on the clipboard to paste into the Load unpacked dialog.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The installed (or, in development, the checked-out) browser-extension folder.
pub fn extension_folder() -> Option<PathBuf> {
    let packaged = jarvis_core::paths::packaged_resources().map(|dir| dir.join("browser-extension"));
    let checkout = jarvis_core::paths::jarvis_home().join("..").join("browser-extension");
    [packaged, Some(checkout)]
        .into_iter()
        .flatten()
        .find(|dir| dir.join("manifest.json").is_file())
        .map(|dir| tidy(&std::fs::canonicalize(&dir).unwrap_or(dir)))
}

/// Drop Windows' `\\?\` long-path prefix: the browser's folder picker doesn't accept it.
fn tidy(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned())
}

#[cfg(windows)]
fn candidates(browser: &str) -> Vec<PathBuf> {
    let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].into_iter().filter_map(std::env::var_os).map(PathBuf::from);
    let tail: &[&str] = match browser {
        "edge" => &["Microsoft", "Edge", "Application", "msedge.exe"],
        "chrome" => &["Google", "Chrome", "Application", "chrome.exe"],
        _ => return Vec::new(),
    };
    roots.map(|root| tail.iter().fold(root, |path, part| path.join(part))).collect()
}

#[cfg(not(windows))]
fn candidates(browser: &str) -> Vec<PathBuf> {
    let names: &[&str] = match browser {
        "edge" => &["microsoft-edge", "microsoft-edge-stable"],
        "chrome" => &["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"],
        _ => return Vec::new(),
    };
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    names.iter().flat_map(|name| dirs.iter().map(move |dir| dir.join(name))).collect()
}

fn find(browser: &str) -> Option<PathBuf> {
    candidates(browser).into_iter().find(|path| path.is_file())
}

/// Which of Edge and Chrome are installed.
pub fn browsers() -> Vec<&'static str> {
    ["edge", "chrome"].into_iter().filter(|b| find(b).is_some()).collect()
}

/// Open the extension folder (and copy its path), and for "edge" / "chrome" that browser's extensions page.
pub fn open(target: &str) -> Result<()> {
    let folder = extension_folder().context("The browser-extension folder is missing. Reinstall Jarvis to restore it.")?;
    let page = match target {
        "edge" => Some("edge://extensions/"),
        "chrome" => Some("chrome://extensions/"),
        "folder" => None,
        _ => bail!("Unknown browser."),
    };
    if let Some(page) = page {
        let exe = find(target).with_context(|| format!("{} doesn't seem to be installed.", if target == "edge" { "Microsoft Edge" } else { "Google Chrome" }))?;
        Command::new(exe).arg(page).spawn().context("Could not open the browser.")?;
    }
    let _ = jarvis_desktop::clipboard::set_text(&folder.to_string_lossy());
    #[cfg(windows)]
    Command::new("explorer.exe").arg(&folder).spawn().context("Could not open the folder.")?;
    #[cfg(not(windows))]
    Command::new("xdg-open").arg(&folder).spawn().context("Could not open the folder.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_extension_folder_in_a_checkout() {
        let folder = extension_folder().expect("the repository has a browser-extension folder");
        assert!(folder.join("manifest.json").is_file());
        assert!(!folder.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn unknown_targets_are_refused() {
        assert!(open("firefox").is_err());
        assert!(candidates("safari").is_empty());
    }
}
