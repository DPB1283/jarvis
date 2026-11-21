//! Linux versions of the tools that the Windows build does with PowerShell: same names and arguments, so the
//! model uses them the same way. Built on standard command-line tools (xdg-open, pactl, playerctl, loginctl,
//! systemctl, xclip, find...) and on `jarvis_desktop`'s X11 backend. Anything missing is reported by name so the
//! user knows what to install. Only compiled off Windows.

use super::{ToolDef, ToolOutput, b, n, s, safe, system::resolve_path, unsafe_};
use anyhow::{Context, Result, anyhow, bail, ensure};
use jarvis_core::events::Risk;
use serde_json::json;
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

/// Run a program (no shell) and return its trimmed stdout.
async fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(program).args(args).stdin(Stdio::null()).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| anyhow!("`{program}` took too long"))?
    .with_context(|| format!("`{program}` is not installed"))?;
    ensure!(output.status.success(), "`{program}` failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Start a program and let it run on its own (an app the user asked to open).
fn spawn_detached(program: &str, args: &[&str]) -> Result<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("could not start `{program}`"))?;
    Ok(())
}

async fn have(program: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {program} >/dev/null 2>&1")]).status().await.is_ok_and(|s| s.success())
}

/// Desktop entries: (file stem, display name) from the standard application folders.
fn desktop_entries() -> Vec<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = vec!["/usr/share/applications".to_owned(), "/usr/local/share/applications".to_owned(), "/var/lib/flatpak/exports/share/applications".to_owned()];
    dirs.push(format!("{home}/.local/share/applications"));
    dirs.push(format!("{home}/.local/share/flatpak/exports/share/applications"));
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if text.lines().any(|l| l.trim() == "NoDisplay=true") {
                continue;
            }
            let name = text.lines().find_map(|l| l.strip_prefix("Name=")).unwrap_or_default().to_owned();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                found.push((stem.to_owned(), name));
            }
        }
    }
    found
}

/// The desktop entry whose name or file name best matches `query` (exact first, then contains).
fn best_entry(query: &str, entries: &[(String, String)]) -> Option<String> {
    let q = query.trim().to_lowercase();
    let exact = entries.iter().find(|(stem, name)| stem.to_lowercase() == q || name.to_lowercase() == q);
    let partial = || {
        entries
            .iter()
            .filter(|(stem, name)| stem.to_lowercase().contains(&q) || name.to_lowercase().contains(&q))
            .min_by_key(|(_, name)| name.len())
    };
    exact.or_else(partial).map(|(stem, _)| stem.clone())
}

fn looks_like_url(text: &str) -> bool {
    text.contains("://") || text.starts_with("www.") || text.starts_with("mailto:")
}

/// "0:35" style CPU/memory numbers from /proc, as text lines.
fn proc_summary() -> Vec<String> {
    let mut lines = Vec::new();
    if let Ok(os) = std::fs::read_to_string("/etc/os-release") {
        if let Some(name) = os.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")) {
            lines.push(format!("os={}", name.trim_matches('"')));
        }
    }
    if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
        let kb = |key: &str| mem.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<f64>().ok());
        if let (Some(total), Some(free)) = (kb("MemTotal:"), kb("MemAvailable:")) {
            lines.push(format!("memory={:.1}GB free of {:.1}GB", free / 1_048_576.0, total / 1_048_576.0));
        }
    }
    if let Ok(up) = std::fs::read_to_string("/proc/uptime") {
        if let Some(seconds) = up.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()) {
            lines.push(format!("uptime={:.1}h", seconds / 3600.0));
        }
    }
    if let Ok(load) = std::fs::read_to_string("/proc/loadavg") {
        let cores = std::thread::available_parallelism().map_or(1, |c| c.get());
        if let Some(one) = load.split_whitespace().next() {
            lines.push(format!("load_1min={one} on {cores} cores"));
        }
    }
    if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let path = entry.path();
            if std::fs::read_to_string(path.join("type")).is_ok_and(|t| t.trim() == "Battery") {
                let capacity = std::fs::read_to_string(path.join("capacity")).unwrap_or_default();
                let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
                lines.push(format!("battery={}% status={}", capacity.trim(), status.trim()));
            }
        }
    }
    lines
}

async fn disks() -> String {
    run("df", &["-hP", "-x", "tmpfs", "-x", "devtmpfs", "-x", "squashfs", "-x", "overlay"]).await.unwrap_or_default()
}

/// Volume as a percentage (first channel) from `pactl`.
async fn pactl_volume() -> Result<(u32, bool)> {
    let volume = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await?;
    let percent = volume
        .split_whitespace()
        .find_map(|w| w.strip_suffix('%').and_then(|n| n.parse::<u32>().ok()))
        .context("could not read the volume")?;
    let mute = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).await.unwrap_or_default();
    Ok((percent, mute.contains("yes")))
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "system_info",
            description: "Battery, CPU load, memory, disk space, uptime and OS version.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check system status".into(),
            run: |_, _| {
                Box::pin(async move {
                    let mut lines = proc_summary();
                    lines.push(format!("disks:\n{}", disks().await));
                    Ok(lines.join("\n").into())
                })
            },
        },
        ToolDef {
            name: "open_app",
            description: "Open an application, file, folder or URL. Accepts an app name as it appears in the applications menu (e.g. 'Firefox', 'Files', 'Discord'), a command name, a path, or a URL.",
            schema: || json!({"type":"object","properties":{"target":{"type":"string","description":"App name, command, path, folder or URL"},"args":{"type":"string","description":"Optional command-line arguments"}},"required":["target"]}),
            risk: safe,
            summary: |i| format!("open {}", s(i, "target").unwrap_or("?")),
            run: |_, i| {
                Box::pin(async move {
                    let target = s(&i, "target").unwrap_or("").trim().to_owned();
                    ensure!(!target.is_empty(), "Say what to open");
                    if looks_like_url(&target) || resolve_path(&target).exists() {
                        let what = if looks_like_url(&target) { target.clone() } else { resolve_path(&target).display().to_string() };
                        spawn_detached("xdg-open", &[&what])?;
                        return Ok(format!("Opened {target}").into());
                    }
                    let extra: Vec<&str> = s(&i, "args").map(|a| a.split_whitespace().collect()).unwrap_or_default();
                    // 1) a program on PATH with that name, 2) a desktop entry matching the name.
                    if !target.contains(' ') && have(&target).await {
                        spawn_detached(&target, &extra)?;
                        return Ok(format!("Opened {target}").into());
                    }
                    if let Some(entry) = best_entry(&target, &desktop_entries()) {
                        spawn_detached("gtk-launch", &[&entry])?;
                        return Ok(format!("Opened {entry}").into());
                    }
                    Ok(format!("Could not find an app called \"{target}\".").into())
                })
            },
        },
        ToolDef {
            name: "open_url",
            description: "Open a web page in the user's default browser.",
            schema: || json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
            risk: safe,
            summary: |i| format!("open {}", s(i, "url").unwrap_or("a page")),
            run: |_, i| {
                Box::pin(async move {
                    let url = s(&i, "url").unwrap_or("").trim().to_owned();
                    ensure!(looks_like_url(&url), "That is not a web address");
                    spawn_detached("xdg-open", &[&url])?;
                    Ok(format!("Opened {url}").into())
                })
            },
        },
        ToolDef {
            name: "list_windows",
            description: "List visible top-level windows (process name, PID, title) and which one is in front.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "list the open windows".into(),
            run: |_, _| {
                Box::pin(async move {
                    let front = jarvis_desktop::window::foreground().hwnd;
                    let windows = tokio::task::spawn_blocking(jarvis_desktop::window::list).await?;
                    ensure!(!windows.is_empty(), "No windows found (is this an X11 session with xprop and xdotool installed?)");
                    Ok(windows
                        .iter()
                        .map(|w| format!("{}{}|{}", if w.hwnd == front { "* " } else { "" }, w.exe, w.title))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into())
                })
            },
        },
        ToolDef {
            name: "focus_window",
            description: "Bring a window to the front by (part of) its title or process name.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
            risk: safe,
            summary: |i| format!("focus {}", s(i, "query").unwrap_or("a window")),
            run: |_, i| {
                Box::pin(async move {
                    let q = s(&i, "query").unwrap_or("").to_owned();
                    let query = q.clone();
                    let found = tokio::task::spawn_blocking(move || jarvis_desktop::window::find(&query)).await?;
                    let Some(window) = found else { return Ok(format!("No window matching \"{q}\".").into()) };
                    let (hwnd, title) = (window.hwnd, window.title);
                    tokio::task::spawn_blocking(move || jarvis_desktop::window::activate(hwnd)).await??;
                    Ok(format!("Focused \"{title}\"").into())
                })
            },
        },
        ToolDef {
            name: "close_app",
            description: "Close an app by process name or window title. Asks it to close normally; force kills only if force=true.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"},"force":{"type":"boolean"}},"required":["query"]}),
            risk: unsafe_,
            summary: |i| format!("{} {} (unsaved work may be lost)", if b(i, "force") == Some(true) { "force-close" } else { "close" }, s(i, "query").unwrap_or("?")),
            run: |_, i| {
                Box::pin(async move {
                    let q = s(&i, "query").unwrap_or("").to_owned();
                    let force = b(&i, "force") == Some(true);
                    let query = q.to_lowercase();
                    let closed = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
                        let mut done = Vec::new();
                        for w in jarvis_desktop::window::list() {
                            if w.title.to_lowercase().contains(&query) || w.exe.to_lowercase().contains(&query) {
                                jarvis_desktop::window::close(w.hwnd, force)?;
                                done.push(w.exe);
                            }
                        }
                        Ok(done)
                    })
                    .await??;
                    Ok(if closed.is_empty() { format!("Nothing matching \"{q}\" is running.") } else { format!("Closed: {}", closed.join(", ")) }.into())
                })
            },
        },
        ToolDef {
            name: "set_volume",
            description: "Set or adjust the system master volume (0-100), or mute/unmute. Returns the new level.",
            schema: || json!({"type":"object","properties":{"level":{"type":"number","minimum":0,"maximum":100,"description":"Absolute level 0-100"},"change":{"type":"number","minimum":-100,"maximum":100,"description":"Relative change, e.g. +10 or -20"},"mute":{"type":"boolean"}}}),
            risk: safe,
            summary: |i| match (b(i, "mute"), n(i, "level"), n(i, "change")) {
                (Some(m), _, _) => if m { "mute".into() } else { "unmute".into() },
                (_, Some(l), _) => format!("set volume to {l}%"),
                (_, _, Some(c)) => format!("change volume by {c}"),
                _ => "check the volume".into(),
            },
            run: |_, i| {
                Box::pin(async move {
                    let (current, _) = pactl_volume().await.map_err(|e| anyhow!("{e}. Volume control needs PulseAudio or PipeWire (pactl)."))?;
                    let target = match (n(&i, "level"), n(&i, "change")) {
                        (Some(level), _) => Some(level.clamp(0.0, 100.0)),
                        (_, Some(change)) => Some((f64::from(current) + change).clamp(0.0, 100.0)),
                        _ => None,
                    };
                    if let Some(target) = target {
                        run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", target.round() as u32)]).await?;
                    }
                    if let Some(mute) = b(&i, "mute") {
                        run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", if mute { "1" } else { "0" }]).await?;
                    }
                    let (volume, muted) = pactl_volume().await?;
                    Ok(format!("Volume {volume}%{}", if muted { " (muted)" } else { "" }).into())
                })
            },
        },
        ToolDef {
            name: "media",
            description: "Control whatever media is playing (music player, browser video, etc.): play_pause, next, previous, stop. Uses playerctl when installed, otherwise the media keys.",
            schema: || json!({"type":"object","properties":{"action":{"type":"string","enum":["play_pause","next","previous","stop"]}},"required":["action"]}),
            risk: safe,
            summary: |i| format!("media {}", s(i, "action").unwrap_or("?").replace('_', "/")),
            run: |_, i| {
                Box::pin(async move {
                    let action = s(&i, "action").unwrap_or("").to_owned();
                    let (player, key) = match action.as_str() {
                        "play_pause" => ("play-pause", "XF86AudioPlay"),
                        "next" => ("next", "XF86AudioNext"),
                        "previous" => ("previous", "XF86AudioPrev"),
                        "stop" => ("stop", "XF86AudioStop"),
                        _ => bail!("Unknown media action"),
                    };
                    if have("playerctl").await {
                        run("playerctl", &[player]).await?;
                    } else {
                        tokio::task::spawn_blocking(move || jarvis_desktop::input::chord(key)).await??;
                    }
                    Ok(format!("Sent {action}").into())
                })
            },
        },
        ToolDef {
            name: "power",
            description: "Lock, sleep, sign out, restart or shut down the PC.",
            schema: || json!({"type":"object","properties":{"action":{"type":"string","enum":["lock","sleep","sign_out","restart","shutdown"]}},"required":["action"]}),
            risk: |_, i| if s(i, "action") == Some("lock") { Risk::Safe } else { Risk::Unsafe },
            summary: |i| format!("{} the PC", s(i, "action").unwrap_or("?").replace('_', " ")),
            run: |_, i| {
                Box::pin(async move {
                    let action = s(&i, "action").unwrap_or("").to_owned();
                    match action.as_str() {
                        "lock" => {
                            if run("loginctl", &["lock-session"]).await.is_err() {
                                run("xdg-screensaver", &["lock"]).await?;
                            }
                        }
                        "sleep" => drop(run("systemctl", &["suspend"]).await?),
                        "sign_out" => {
                            let user = std::env::var("USER").unwrap_or_default();
                            run("loginctl", &["terminate-user", &user]).await?;
                        }
                        "restart" => drop(run("systemctl", &["reboot"]).await?),
                        "shutdown" => drop(run("systemctl", &["poweroff"]).await?),
                        _ => bail!("unknown power action"),
                    }
                    Ok(format!("Done: {action}").into())
                })
            },
        },
        ToolDef {
            name: "run_shell",
            description: "Run a shell (bash) script on the user's PC and return its output. Use only when no dedicated tool fits. Keep scripts short and explain what they do in `purpose`.",
            schema: || json!({"type":"object","properties":{"script":{"type":"string"},"purpose":{"type":"string","description":"Plain-English description of what the script does, shown to the user"},"timeout_seconds":{"type":"number","minimum":1,"maximum":600}},"required":["script","purpose"]}),
            risk: unsafe_,
            summary: |i| format!("run a shell script: {}", s(i, "purpose").unwrap_or("?")),
            run: |_, i| {
                Box::pin(async move {
                    let seconds = n(&i, "timeout_seconds").unwrap_or(60.0).clamp(1.0, 600.0);
                    let output = tokio::time::timeout(
                        Duration::from_secs_f64(seconds),
                        Command::new("bash").args(["-c", s(&i, "script").unwrap_or("")]).stdin(Stdio::null()).kill_on_drop(true).output(),
                    )
                    .await
                    .map_err(|_| anyhow!("The script timed out after {seconds} seconds"))??;
                    let mut text = String::new();
                    let (out, err) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
                    if !out.trim().is_empty() {
                        text.push_str(&format!("stdout:\n{}\n", out.trim()));
                    }
                    if !err.trim().is_empty() {
                        text.push_str(&format!("stderr:\n{}\n", err.trim()));
                    }
                    text.push_str(&format!("exit code: {:?}", output.status.code()));
                    if text.len() > 12_000 {
                        text.truncate(text.floor_char_boundary(12_000));
                        text.push_str("\n…[truncated]");
                    }
                    Ok(text.into())
                })
            },
        },
        ToolDef {
            name: "find_files",
            description: "Search for files or folders by name (wildcards allowed, e.g. '*.pdf', 'report*'). Searches the user's home folder by default.",
            schema: || json!({"type":"object","properties":{"pattern":{"type":"string"},"folder":{"type":"string","description":"Folder to search, e.g. 'Downloads', '/srv/projects'. Default: home folder"},"limit":{"type":"number","minimum":1,"maximum":200}},"required":["pattern"]}),
            risk: safe,
            summary: |i| format!("find files matching {}", s(i, "pattern").unwrap_or("?")),
            run: |_, i| {
                Box::pin(async move {
                    let pattern = s(&i, "pattern").unwrap_or("").trim().to_owned();
                    ensure!(!pattern.is_empty() && !pattern.starts_with('-'), "Give a file name or pattern");
                    let folder = resolve_path(s(&i, "folder").unwrap_or("~"));
                    let limit = n(&i, "limit").unwrap_or(50.0).clamp(1.0, 200.0) as usize;
                    let glob = if pattern.contains(['*', '?', '[']) { pattern } else { format!("*{pattern}*") };
                    let found = run(
                        "find",
                        &[&folder.display().to_string(), "-maxdepth", "8", "-not", "-path", "*/.*", "-iname", &glob],
                    )
                    .await
                    .unwrap_or_default();
                    let lines: Vec<&str> = found.lines().take(limit).collect();
                    Ok(if lines.is_empty() { "No matches.".to_owned() } else { lines.join("\n") }.into())
                })
            },
        },
        ToolDef {
            name: "delete_path",
            description: "Move a file or folder to the Trash (recoverable).",
            schema: || json!({"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}),
            risk: unsafe_,
            summary: |i| format!("delete {} (to the Trash)", s(i, "target").unwrap_or("?")),
            run: |_, i| {
                Box::pin(async move {
                    let path = resolve_path(s(&i, "target").unwrap_or(""));
                    if !path.exists() {
                        return Ok(format!("{} does not exist.", path.display()).into());
                    }
                    let shown = path.display().to_string();
                    if have("gio").await {
                        run("gio", &["trash", &shown]).await?;
                    } else {
                        run("trash-put", &[&shown]).await.map_err(|_| anyhow!("No Trash tool found (install gio or trash-cli)"))?;
                    }
                    Ok(format!("Moved {shown} to the Trash.").into())
                })
            },
        },
        ToolDef {
            name: "pc_health",
            description: "Read CPU, memory, GPU, drives and largest processes.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check PC health".into(),
            run: |_, _| {
                Box::pin(async move {
                    let mut lines = proc_summary();
                    lines.push(format!("disks:\n{}", disks().await));
                    let top = run("ps", &["-eo", "comm,pcpu,pmem", "--sort=-pmem"]).await.unwrap_or_default();
                    lines.push(format!("largest processes:\n{}", top.lines().take(7).collect::<Vec<_>>().join("\n")));
                    if have("nvidia-smi").await {
                        if let Ok(gpu) = run("nvidia-smi", &["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader"]).await {
                            lines.push(format!("gpu: {gpu}"));
                        }
                    }
                    Ok(lines.join("\n").into())
                })
            },
        },
        ToolDef {
            name: "clip_that",
            description: "Save a replay by pressing the user's recorder's save-clip shortcut (set in Settings, Gaming, Clip hotkey).",
            schema: || json!({"type":"object","properties":{"app":{"type":"string","enum":["auto","medal","nvidia","xbox"]}}}),
            risk: safe,
            summary: |_| "save a game clip".into(),
            run: |ctx, _| {
                Box::pin(async move {
                    let hotkey = ctx.settings.get().clips.hotkey;
                    let hotkey = hotkey.trim().to_owned();
                    ensure!(!hotkey.is_empty(), "Set your recorder's clip hotkey in Settings, Gaming, Clip hotkey first");
                    let combo = hotkey.clone();
                    tokio::task::spawn_blocking(move || jarvis_desktop::input::chord(&combo)).await??;
                    Ok(format!("Pressed the clip shortcut ({hotkey}).").into())
                })
            },
        },
        ToolDef {
            name: "read_clipboard",
            description: "Read copied text, file names or an image.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "read the clipboard".into(),
            run: |_, _| {
                Box::pin(async move {
                    let targets = run("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"]).await.unwrap_or_default();
                    if targets.lines().any(|t| t == "image/png") {
                        use base64::{Engine, engine::general_purpose::STANDARD};
                        let png = Command::new("xclip").args(["-selection", "clipboard", "-t", "image/png", "-o"]).output().await?;
                        ensure!(png.status.success() && !png.stdout.is_empty(), "Could not read the copied image");
                        return Ok(ToolOutput::Blocks(vec![json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":STANDARD.encode(&png.stdout)}})]));
                    }
                    if targets.lines().any(|t| t == "text/uri-list") {
                        let files = run("xclip", &["-selection", "clipboard", "-t", "text/uri-list", "-o"]).await.unwrap_or_default();
                        return Ok(format!("Copied files:\n{files}").into());
                    }
                    let text = tokio::task::spawn_blocking(jarvis_desktop::clipboard::get_text).await?.unwrap_or_default();
                    Ok(if text.is_empty() { "Clipboard is empty.".to_owned() } else { text.chars().take(30_000).collect() }.into())
                })
            },
        },
        ToolDef {
            name: "now_playing",
            description: "Read the current media session and playback status.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check what is playing".into(),
            run: |_, _| {
                Box::pin(async move {
                    if !have("playerctl").await {
                        return Ok("Install playerctl so I can see what is playing.".into());
                    }
                    let info = run("playerctl", &["metadata", "--format", "{{status}}|{{artist}}|{{title}}|{{album}}|{{playerName}}"]).await.unwrap_or_default();
                    Ok(if info.is_empty() { "No active media session.".to_owned() } else { info }.into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_match_desktop_entries() {
        let entries = vec![
            ("firefox".to_owned(), "Firefox Web Browser".to_owned()),
            ("org.gnome.Nautilus".to_owned(), "Files".to_owned()),
            ("discord-canary".to_owned(), "Discord Canary".to_owned()),
            ("discord".to_owned(), "Discord".to_owned()),
        ];
        assert_eq!(best_entry("Firefox", &entries).as_deref(), Some("firefox"));
        assert_eq!(best_entry("files", &entries).as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(best_entry("discord", &entries).as_deref(), Some("discord"), "exact beats partial");
        assert_eq!(best_entry("canary", &entries).as_deref(), Some("discord-canary"));
        assert_eq!(best_entry("photoshop", &entries), None);
    }

    #[test]
    fn urls_and_paths_are_told_apart() {
        assert!(looks_like_url("https://example.com"));
        assert!(looks_like_url("www.example.com"));
        assert!(!looks_like_url("firefox"));
        assert!(!looks_like_url("/home/me/file.txt"));
    }

    #[tokio::test]
    async fn shell_and_os_basics_work() {
        let info = proc_summary();
        assert!(info.iter().any(|l| l.starts_with("memory=")), "{info:?}");
        assert_eq!(run("echo", &["hi"]).await.unwrap(), "hi");
        assert!(run("definitely-not-installed-xyz", &[]).await.unwrap_err().to_string().contains("not installed"));
        assert!(std::path::Path::new("/proc").exists());
    }
}
