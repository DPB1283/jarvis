//! Linux versions of the tools that the Windows build does with PowerShell: same names and arguments, so the
//! model uses them the same way. Built on standard command-line tools (xdg-open, pactl, playerctl, loginctl,
//! systemctl, xclip, find...) and on `jarvis_desktop`'s X11 backend. Anything missing is reported by name so the
//! user knows what to install. Only compiled off Windows.

use super::{ToolDef, ToolOutput, b, n, s, safe, system::resolve_path, unsafe_};
use anyhow::{Context, Result, anyhow, bail, ensure};
use jarvis_core::events::Risk;
use serde_json::json;
use std::{collections::HashSet, os::unix::fs::PermissionsExt, path::{Path, PathBuf}, process::Stdio, time::Duration};
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
fn spawn_detached(program: &str, args: &[&str]) -> Result<Option<u32>> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("could not start `{program}`"))?;
    let id = child.id();
    tokio::spawn(async move { let mut child = child; let _ = child.wait().await; });
    Ok(id)
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn path_dirs() -> impl Iterator<Item = PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>().into_iter()
}

async fn have(program: &str) -> bool {
    if program.contains('/') {
        return executable(Path::new(program));
    }
    path_dirs().any(|dir| executable(&dir.join(program)))
}

#[derive(Debug)]
struct DesktopEntry {
    id: String,
    name: String,
    exec: Vec<String>,
    file: PathBuf,
    source: &'static str,
}

fn desktop_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut arg = String::new();
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => quoted = !quoted,
            '\\' => {
                if let Some(next) = chars.next() {
                    arg.push(next);
                }
            }
            '%' => match chars.next() {
                Some('%') => arg.push('%'),
                Some('f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
                _ => return Vec::new(),
            },
            ch if ch.is_whitespace() && !quoted => {
                if !arg.is_empty() {
                    args.push(std::mem::take(&mut arg));
                }
            }
            ch => arg.push(ch),
        }
    }
    if quoted {
        return Vec::new();
    }
    if !arg.is_empty() {
        args.push(arg);
    }
    args
}

fn parse_entry(text: &str, file: PathBuf, source: &'static str) -> Option<DesktopEntry> {
    let mut desktop = false;
    let mut name = "";
    let mut command = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            if desktop { break; }
            desktop = line == "[Desktop Entry]";
        } else if desktop {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "Hidden" | "NoDisplay" if value.eq_ignore_ascii_case("true") => return None,
                "Type" if value != "Application" => return None,
                "Name" => name = value,
                "Exec" => command = value,
                _ => {}
            }
        }
    }
    let id = file.file_stem()?.to_str()?.to_owned();
    let exec = desktop_exec(command);
    (!name.is_empty() && !exec.is_empty()).then(|| DesktopEntry { id, name: name.to_owned(), exec, file, source })
}

fn desktop_entries() -> Vec<DesktopEntry> {
    let user = std::env::var_os("HOME").map(PathBuf::from);
    let mut directories = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(||user.as_ref().map(|p|p.join(".local/share"))) {
        directories.push((data_home.join("applications"), "desktop menu"));
    }
    if let Some(user) = user {
        directories.push((user.join(".local/share/flatpak/exports/share/applications"), "Flatpak"));
    }
    directories.extend([
        (PathBuf::from("/var/lib/flatpak/exports/share/applications"), "Flatpak"),
        (PathBuf::from("/var/lib/snapd/desktop/applications"), "Snap"),
    ]);
    let data_dirs = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    directories.extend(std::env::split_paths(&data_dirs).map(|dir| (dir.join("applications"), "desktop menu")));
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for (dir, source) in directories {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") || !seen.insert(entry.file_name()) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Some(app) = parse_entry(&text, path, source)
            {
                found.push(app);
            }
        }
    }
    found
}

fn matches_entry(entry: &DesktopEntry, query: &str) -> bool {
    [&entry.id, &entry.name].iter().any(|value| value.to_lowercase().contains(query))
        || entry.exec.first().is_some_and(|program| program.to_lowercase().contains(query))
}

fn best_entry<'a>(query: &str, entries: &'a [DesktopEntry]) -> Option<&'a DesktopEntry> {
    let q = query.trim().strip_suffix(".desktop").unwrap_or(query.trim()).to_lowercase();
    entries.iter().filter(|item| matches_entry(item, &q))
        .min_by_key(|item| ((item.id.to_lowercase() != q && item.name.to_lowercase() != q), item.name.len()))
}

async fn flatpak_apps() -> Vec<(String, String)> {
    let Ok(rows) = run("flatpak", &["list", "--app", "--columns=application,name"]).await else { return Vec::new() };
    rows.lines().filter_map(|line| {
        let (id, name) = line.split_once('\t')?;
        let id = id.trim();
        (id.contains('.') && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
            .then(|| (id.to_owned(), name.trim().to_owned()))
    }).collect()
}

async fn find_apps(query: Option<&str>, limit: usize) -> String {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let mut apps = Vec::<(String, String, &'static str)>::new();
    for entry in desktop_entries() {
        if matches_entry(&entry, &query) {
            apps.push((entry.name, entry.id, entry.source));
        }
    }
    for (id, name) in flatpak_apps().await {
        if id.to_lowercase().contains(&query) || name.to_lowercase().contains(&query) {
            apps.push((name, id, "Flatpak"));
        }
    }
    if !query.is_empty() {
        for dir in path_dirs() {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.to_lowercase().contains(&query) && executable(&entry.path()) {
                    apps.push((name.clone(), name, "PATH"));
                }
            }
        }
    }
    apps.sort_by_cached_key(|(name, _, source)| (*source == "PATH", name.to_lowercase()));
    let mut names = HashSet::new();
    let mut targets = HashSet::new();
    let mut matches: Vec<_> = apps.into_iter().filter(|(name, target, _)| {
        let (name, target) = (name.to_lowercase(), target.to_lowercase());
        if names.contains(&name) || targets.contains(&target) { return false; }
        names.insert(name);
        targets.insert(target);
        true
    }).collect();
    matches.sort_by_cached_key(|(name, _, _)| name.to_lowercase());
    if matches.is_empty() {
        return "No matching applications found.".into();
    }
    matches.into_iter().take(limit).map(|(name, target, source)|format!("{name} | {target} | {source}")).collect::<Vec<_>>().join("\n")
}

async fn x11_windows() -> HashSet<String> {
    let output = run("xdotool", &["search", "--onlyvisible", "--maxdepth", "2", "--class", ".*"]).await.unwrap_or_default();
    output.lines().filter(|id| id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty()).map(str::to_owned).collect()
}

fn descendant_of(mut pid: u32, root: u32) -> bool {
    for _ in 0..32 {
        if pid == root { return true; }
        if pid <= 1 { break; }
        let Some(parent) = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
            .and_then(|stat| stat.rsplit_once(") ")?.1.split_whitespace().nth(1)?.parse().ok()) else { break };
        pid = parent;
    }
    false
}

async fn launch(program: &str, args: &[&str], identity: &str, background: bool) -> Result<&'static str> {
    let x11 = std::env::var_os("DISPLAY").is_some()
        && std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("wayland")
        && have("xdotool").await;
    if !background || !x11 {
        spawn_detached(program, args)?;
        return Ok(if background { " Background launch is unavailable in this session." } else { "" });
    }
    let foreground = run("xdotool", &["getactivewindow"]).await.ok();
    let before = x11_windows().await;
    let child = spawn_detached(program, args)?;
    let program_hint = identity.to_lowercase();
    tokio::spawn(async move {
        let mut handled = before;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            for id in x11_windows().await.difference(&handled).cloned().collect::<Vec<_>>() {
                let pid = run("xdotool", &["getwindowpid", &id]).await.ok().and_then(|pid|pid.parse::<u32>().ok());
                let ours = child.zip(pid).is_some_and(|(root, pid)| descendant_of(pid, root));
                let app_class = x11_class(&id).await;
                if !ours && (program_hint.is_empty() || !app_class.contains(&program_hint)) {
                    continue;
                }
                let focused = run("xdotool", &["getactivewindow"]).await.is_ok_and(|active|active == id);
                if run("xdotool", &["windowminimize", &id]).await.is_ok() {
                    handled.insert(id);
                }
                if focused && let Some(foreground) = &foreground {
                    let _ = run("xdotool", &["windowactivate", foreground]).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    Ok(" Requested background launch.")
}

async fn x11_class(id: &str) -> String {
    run("xprop", &["-id", id, "WM_CLASS"]).await.unwrap_or_default().to_lowercase()
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
            name: "find_apps",
            description: "Find installed applications by name, or list them alphabetically. Prefer the desktop ID shown in the second column when calling open_app.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":200}}}),
            risk: safe,
            summary: |_| "find installed apps".into(),
            run: |_, input| Box::pin(async move {
                let limit = n(&input, "limit").unwrap_or(80.0).clamp(1.0, 200.0) as usize;
                Ok(find_apps(s(&input, "query"), limit).await.into())
            }),
        },
        ToolDef {
            name: "open_app",
            description: "Open an application, file, folder or URL. Accepts a desktop name or ID, command, path or URL. Use find_apps if the name is unclear.",
            schema: || json!({"type":"object","properties":{"target":{"type":"string","description":"App name, desktop ID, command, path, folder or URL"},"args":{"type":"string","description":"Optional command-line arguments"},"background":{"type":"boolean","description":"Open it without bringing it to the front: it starts minimised and whatever the user was using stays in front"}},"required":["target"]}),
            risk: safe,
            summary: |i| format!("open {}", s(i, "target").unwrap_or("?")),
            run: |_, input| Box::pin(async move {
                let target = s(&input, "target").unwrap_or("").trim();
                ensure!(!target.is_empty(), "Say what to open");
                let background = b(&input, "background").unwrap_or(false);
                let path = resolve_path(target);
                if looks_like_url(target) || path.exists() {
                    let what = if looks_like_url(target) { target.to_owned() } else { path.display().to_string() };
                    let status = launch("xdg-open", &[&what], "", background).await?;
                    return Ok(format!("Opened {target}.{status}").into());
                }
                let extra: Vec<&str> = s(&input, "args").map(|a| a.split_whitespace().collect()).unwrap_or_default();
                if !target.contains(' ') && have(target).await {
                    let status = launch(target, &extra, target, background).await?;
                    return Ok(format!("Opened {target}.{status}").into());
                }
                if let Some(entry) = best_entry(target, &desktop_entries()) {
                    let exe = Path::new(&entry.exec[0]).file_name().and_then(|s|s.to_str()).unwrap_or(&entry.id);
                    let status = if extra.is_empty() && have("gio").await {
                        let path = entry.file.to_string_lossy();
                        launch("gio", &["launch", &path], exe, background).await?
                    } else if extra.is_empty() && have("gtk-launch").await {
                        launch("gtk-launch", &[&entry.id], exe, background).await?
                    } else {
                        let mut args: Vec<&str> = entry.exec.iter().skip(1).map(String::as_str).collect();
                        args.extend(extra);
                        launch(&entry.exec[0], &args, exe, background).await?
                    };
                    return Ok(format!("Opened {}.{status}", entry.name).into());
                }
                let query = target.to_lowercase();
                if let Some((id, name)) = flatpak_apps().await.into_iter().find(|(id, name)|
                    id.to_lowercase() == query || name.to_lowercase() == query
                ) {
                    let status = launch("flatpak", &["run", &id], &id, background).await?;
                    return Ok(format!("Opened {name}.{status}").into());
                }
                Ok(format!("Could not find an app called \"{target}\".").into())
            }),
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
        let entries = [
            ("firefox", "Firefox Web Browser"),
            ("org.gnome.Nautilus", "Files"),
            ("discord-canary", "Discord Canary"),
            ("discord", "Discord"),
        ].into_iter().map(|(id, name)| parse_entry(
            &format!("[Desktop Entry]\nName={name}\nExec={id}"),
            PathBuf::from(format!("{id}.desktop")), "desktop menu"
        ).unwrap()).collect::<Vec<_>>();
        assert_eq!(best_entry("Firefox", &entries).map(|app| app.id.as_str()), Some("firefox"));
        assert_eq!(best_entry("files", &entries).map(|app| app.id.as_str()), Some("org.gnome.Nautilus"));
        assert_eq!(best_entry("discord", &entries).map(|app| app.id.as_str()), Some("discord"));
        assert_eq!(best_entry("canary", &entries).map(|app| app.id.as_str()), Some("discord-canary"));
        assert!(best_entry("photoshop", &entries).is_none());
    }

    #[test]
    fn desktop_entry_ignores_hidden_apps_and_expands_no_file_arguments() {
        let parse = |text| parse_entry(text, PathBuf::from("test.desktop"), "desktop menu");
        assert!(parse("[Desktop Entry]\nName=Hidden\nExec=app\nNoDisplay=true").is_none());
        assert!(parse("[Desktop Entry]\nName=Hidden\nExec=app\nHidden=true").is_none());
        let entry = parse("[Desktop Entry]\nName=Steam\nExec=steam \"two words\" %U %f %%\n[Desktop Action Play]\nName=Play\nExec=other").unwrap();
        assert_eq!(entry.name, "Steam");
        assert_eq!(entry.exec, ["steam", "two words", "%"]);
        assert_eq!(entry.id, "test");
    }

    #[tokio::test]
    #[ignore = "requires an isolated X11 display with a window manager and xmessage"]
    async fn background_launch_minimizes_new_app() {
        let active = "Jarvis test foreground";
        let away = "Jarvis test background";
        spawn_detached("xmessage", &["-title", active, "Foreground fixture"]).unwrap();
        let foreground = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(ids) = run("xdotool", &["search", "--onlyvisible", "--name", active]).await
                    && let Some(id) = ids.lines().next()
                {
                    break id.to_owned();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.unwrap();
        run("xdotool", &["windowactivate", "--sync", &foreground]).await.unwrap();
        assert!(launch("xmessage", &["-title", away, "Background fixture"], "xmessage", true).await.unwrap().contains("Requested"));
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                if let Ok(ids) = run("xdotool", &["search", "--name", away]).await
                    && let Some(id) = ids.lines().next()
                    && run("xprop", &["-id", id, "_NET_WM_STATE"]).await.is_ok_and(|state|state.contains("_NET_WM_STATE_HIDDEN"))
                {
                    assert_eq!(run("xdotool", &["getactivewindow"]).await.unwrap(), foreground);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.unwrap();
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
