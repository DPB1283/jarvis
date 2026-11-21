//! Installing software: `find_package` (safe) and `install_app` (asks first unless he has full access).
//! Runs winget / pip / npm directly with an argument list, never through a shell or an encoded PowerShell
//! command, so a package name can't smuggle in other commands (and Defender doesn't see a script).

use super::{ToolDef, safe, unsafe_};
use anyhow::{Result, bail, ensure};
use jarvis_core::Event;
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A package id or name: starts with a letter or digit, then only the characters real package names use.
fn valid(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 120
        // A scoped npm package may start with a single '@' (@scope/name), never with an option dash.
        && name.strip_prefix('@').unwrap_or(name).chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || " ._+-@/".contains(c))
        && !name.contains("..")
}

/// Run a program with arguments; returns (exit code, cleaned output, timed out).
async fn run(
    program: &str,
    args: &[&str],
    timeout: Duration,
    on_percent: Option<&(dyn Fn(f64) + Send + Sync)>,
) -> Result<(Option<i32>, String, bool)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| anyhow::anyhow!("Could not start {program}: {e}"))?;
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let read_out = async {
        let mut chunk = [0u8; 4096];
        loop {
            let n = out.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            so.extend_from_slice(&chunk[..n]);
            if let (Some(report), Some(percent)) = (on_percent, last_percent(&String::from_utf8_lossy(&chunk[..n]))) {
                report(percent);
            }
        }
    };
    let work = async { tokio::join!(read_out, err.read_to_end(&mut se), child.wait()).2 };
    let (code, timed_out) = match tokio::time::timeout(timeout, work).await {
        Ok(status) => (status?.code(), false),
        Err(_) => (None, true),
    };
    let text = format!("{}\n{}", String::from_utf8_lossy(&so), String::from_utf8_lossy(&se));
    Ok((code, tidy(&text), timed_out))
}

/// The last "NN%" in a piece of installer output, 0..=100.
fn last_percent(text: &str) -> Option<f64> {
    let mut found = None;
    for (at, _) in text.match_indices('%') {
        let digits: String = text[..at].chars().rev().skip_while(|c| *c == ' ').take_while(char::is_ascii_digit).collect();
        if let Ok(value) = digits.chars().rev().collect::<String>().parse::<f64>() {
            if value <= 100.0 {
                found = Some(value);
            }
        }
    }
    found
}

/// Drop spinners and progress bars, keep the last useful lines.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|l| l.chars().any(char::is_alphanumeric) && !l.contains('█') && !l.contains('▒'))
        .collect();
    let start = lines.len().saturating_sub(14);
    lines[start..].join("\n")
}

fn manager(v: &Value) -> &str {
    v["manager"].as_str().unwrap_or("winget")
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "find_package",
            description: "Search for installable software before installing it. Returns matching winget package ids (use the id with install_app). Use this when you are not sure of the exact id.",
            schema: || json!({"type":"object","required":["query"],"properties":{"query":{"type":"string","description":"What to look for, e.g. python, vscode, 7zip"}}}),
            risk: safe,
            summary: |v| format!("search for {}", v["query"].as_str().unwrap_or("software")),
            run: |_, v| {
                Box::pin(async move {
                    let query = v["query"].as_str().unwrap_or_default().trim().to_owned();
                    ensure!(valid(&query), "That is not a valid search");
                    let (_, output, timed_out) = run(
                        "winget",
                        &["search", &query, "--source", "winget", "--accept-source-agreements", "--count", "8"],
                        Duration::from_secs(60),
                        None,
                    )
                    .await?;
                    ensure!(!timed_out, "The package search timed out");
                    Ok(if output.is_empty() { "No packages found.".to_owned() } else { output }.into())
                })
            },
        },
        ToolDef {
            name: "install_app",
            description: "Install software on the user's PC properly instead of sending them to a download page. manager winget (default) installs apps by package id, e.g. Python.Python.3.12, Git.Git, Microsoft.VisualStudioCode, 7zip.7zip. manager pip installs a Python package, npm installs a global npm package. Installs silently. Use find_package first if unsure of the winget id. Only open a website instead when no package exists, and say so.",
            schema: || json!({"type":"object","required":["package"],"properties":{
                "package":{"type":"string","description":"winget id or name, pip package, or npm package"},
                "manager":{"type":"string","enum":["winget","pip","npm"]}}}),
            risk: unsafe_,
            summary: |v| format!("install {} with {}", v["package"].as_str().unwrap_or("a package"), manager(v)),
            run: |ctx, v| {
                Box::pin(async move {
                    let package = v["package"].as_str().unwrap_or_default().trim().to_owned();
                    ensure!(valid(&package), "That is not a valid package name");
                    let kind = manager(&v).to_owned();
                    let (program, args): (&str, Vec<&str>) = match kind.as_str() {
                        "winget" => {
                            let mut a = vec!["install"];
                            // "Publisher.Name" is an exact id; anything else is searched by name.
                            if package.contains('.') && !package.contains(' ') {
                                a.extend(["--id", &package, "--exact"]);
                            } else {
                                a.push(&package);
                            }
                            a.extend([
                                "--source", "winget", "--silent", "--disable-interactivity",
                                "--accept-package-agreements", "--accept-source-agreements",
                            ]);
                            ("winget", a)
                        }
                        "pip" => ("python", vec!["-m", "pip", "install", &package]),
                        "npm" => ("npm.cmd", vec!["install", "-g", &package]),
                        _ => bail!("Unknown package manager"),
                    };
                    let label = format!("Installing {package}");
                    ctx.bus.emit(Event::Activity { text: label.clone(), active: true, progress: None });
                    let bus = ctx.bus.clone();
                    let report = move |percent: f64| {
                        bus.emit(Event::Activity { text: label.clone(), active: true, progress: Some(percent / 100.0) });
                    };
                    let (code, output, timed_out) = run(program, &args, Duration::from_secs(900), Some(&report)).await?;
                    if timed_out {
                        bail!("The install is still running after 15 minutes; check the installer window");
                    }
                    let lower = output.to_lowercase();
                    let verdict = if lower.contains("already installed") || lower.contains("no available upgrade") {
                        format!("{package} is already installed.")
                    } else if lower.contains("no package found") || lower.contains("no packages found") {
                        format!("No package matched {package}. Try find_package, or open the download page if it has no package.")
                    } else if code == Some(0) {
                        format!("Installed {package}. Programs already running (and the current terminal) may not see it on PATH until restarted.")
                    } else {
                        format!("The install did not finish (exit code {code:?}).")
                    };
                    Ok(format!("{verdict}\n{output}").into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_cannot_smuggle_options_or_commands() {
        for ok in ["Python.Python.3.12", "7zip.7zip", "git", "@scope/pkg", "requests", "Microsoft.VisualStudioCode"] {
            assert!(valid(ok), "{ok}");
        }
        for bad in ["", "--source evil", "-rf", "a; del *", "a && b", "$(calc)", "a|b", "../x", "a\nb", "\"q\""] {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn percentages_are_read_from_installer_output() {
        assert_eq!(last_percent("  ██████▒▒▒▒  45%"), Some(45.0));
        assert_eq!(last_percent("10% 20 % 73%"), Some(73.0));
        assert_eq!(last_percent("12.3 MB / 30.0 MB"), None);
        assert_eq!(last_percent("100%"), Some(100.0));
        assert_eq!(last_percent("abc %"), None);
    }

    #[test]
    fn output_is_tidied() {
        let raw = "   - \r\n  \\ \r\n██████▒▒▒▒ 40%\r\nFound Python 3.12 [Python.Python.3.12]\r\nSuccessfully installed\r\n";
        assert_eq!(tidy(raw), "Found Python 3.12 [Python.Python.3.12]\nSuccessfully installed");
    }

    #[tokio::test]
    #[ignore = "uses the network via winget (read-only search)"]
    async fn winget_search_works() {
        let (code, out, _) = run("winget", &["search", "python", "--source", "winget", "--accept-source-agreements", "--count", "3"], Duration::from_secs(60), None).await.unwrap();
        println!("{code:?}\n{out}");
        assert!(out.to_lowercase().contains("python"));
    }
}
