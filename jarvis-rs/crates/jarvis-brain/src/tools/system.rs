//! Apps, windows, audio, media, power, shell, files (port of `tools/system.ts` + `tools/files.ts`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use jarvis_core::events::Risk;
use serde_json::{Value, json};

use super::ps::{WIN_NATIVE, ps, ps_t, psq, run_ps};
use super::{ToolDef, ToolOutput, b, n, s, safe, unsafe_};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into()))
}

/// Expand ~, %ENV% and known folder names ("Downloads/report.pdf"); relative paths are under the home folder.
pub fn resolve_path(p: &str) -> PathBuf {
    let mut s = p.trim().to_string();
    // %VAR%
    while let Some(start) = s.find('%') {
        let Some(len) = s[start + 1..].find('%') else { break };
        let var = &s[start + 1..start + 1 + len];
        let val = std::env::var(var).unwrap_or_default();
        if val.is_empty() {
            break;
        }
        s.replace_range(start..start + len + 2, &val);
    }
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        s = format!("{}{}", home().display(), &s[1..]);
    }
    let path = PathBuf::from(&s);
    if path.is_absolute() {
        return path;
    }
    let first = s.split(['/', '\\']).next().unwrap_or("").to_lowercase();
    let known = ["desktop", "documents", "downloads", "pictures", "music", "videos"];
    if let Some(k) = known.iter().find(|k| **k == first) {
        let mut cap = k.to_string();
        cap[..1].make_ascii_uppercase();
        return home().join(cap).join(s[first.len()..].trim_start_matches(['/', '\\']));
    }
    home().join(s)
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "get_time",
            description: "Get the current local date, time and timezone.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check the time".into(),
            run: |_ctx, _i| Box::pin(async move { Ok(chrono::Local::now().format("%A %-d %B %Y, %H:%M (%Z)").to_string().into()) }),
        },
        ToolDef {
            name: "system_info",
            description: "Battery, CPU load, memory, disk space, uptime and OS version.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "check system status".into(),
            run: |_ctx, _i| {
                Box::pin(async move {
                    let out = run_ps(
                        r#"$b = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1
                        $cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
                        $os = Get-CimInstance Win32_OperatingSystem
                        $disks = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | ForEach-Object { "{0} {1:N0}GB free of {2:N0}GB" -f $_.DeviceID, ($_.FreeSpace/1GB), ($_.Size/1GB) }
                        "os=$($os.Caption) $($os.Version)"
                        "memory={0:N1}GB free of {1:N1}GB" -f ($os.FreePhysicalMemory/1MB), ($os.TotalVisibleMemorySize/1MB)
                        "uptime={0:N1}h" -f ((Get-Date) - $os.LastBootUpTime).TotalHours
                        "battery=" + $(if ($b) { "$($b.EstimatedChargeRemaining)% status=$($b.BatteryStatus)" } else { "none" })
                        "cpu_load=$cpu%"
                        "disks=" + ($disks -join "; ")"#,
                        Duration::from_secs(30),
                    )
                    .await;
                    Ok(out.stdout.into())
                })
            },
        },
        ToolDef {
            name: "open_app",
            description: "Open an application, file, folder, URL or ms-settings: page. Accepts an app name as it appears in the Start menu (e.g. 'Notepad', 'Apple Music', 'Discord'), an exe name, a path, or a URL. Also finds apps that are only in the installed-programs list. If it can't find one, call find_apps to see what is installed and use the 'how to open' value it gives.",
            schema: || json!({"type":"object","properties":{"target":{"type":"string","description":"App name, exe, path, folder, URL or ms-settings: URI"},"args":{"type":"string","description":"Optional command-line arguments"},"background":{"type":"boolean","description":"Open it without bringing it to the front: it starts minimised and whatever the user was using stays in front"}},"required":["target"]}),
            risk: safe,
            summary: |i| format!("open {}{}", s(i, "target").unwrap_or("?"), if b(i, "background") == Some(true) { " in the background" } else { "" }),
            run: |_ctx, i| {
                Box::pin(async move {
                    let background = b(&i, "background") == Some(true);
                    #[cfg(windows)]
                    let watch = background.then(BackgroundLaunch::start);
                    let result = open_app_with(s(&i, "target").unwrap_or(""), s(&i, "args"), background).await?;
                    #[cfg(windows)]
                    if let Some(watch) = watch.filter(|_| result.starts_with("Opened")) {
                        watch.keep_out_of_the_way();
                        return Ok(format!("{result} in the background (minimised; the user's window stays in front).").into());
                    }
                    Ok(result.into())
                })
            },
        },
        ToolDef {
            name: "find_apps",
            description: "List or search the apps installed on this PC: Start menu shortcuts, Store apps, the installed-programs list (Settings > Apps), registered app paths and running programs. Use it when unsure whether something is installed (\"do I have Steam?\"), when open_app can't find an app, or to answer \"what games/apps do I have\". Each line is: name | how to open (pass it to open_app as target) | where it was found.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string","description":"Part of the app's name; omit to list everything"},"limit":{"type":"integer","minimum":1,"maximum":300}}}),
            risk: safe,
            summary: |i| match s(i, "query") { Some(q) => format!("look for {q} on this PC"), None => "list the installed apps".into() },
            run: |_ctx, i| Box::pin(async move { find_apps(s(&i, "query").unwrap_or(""), n(&i, "limit").map(|l| l.clamp(1.0, 300.0) as usize).unwrap_or(80)).await.map(Into::into) }),
        },
        ToolDef {
            name: "open_url",
            description: "Open a web page in the user's default browser.",
            schema: || json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
            risk: safe,
            summary: |i| format!("open {}", s(i, "url").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let url = s(&i, "url").unwrap_or("");
                    if !(url.starts_with("http://") || url.starts_with("https://")) {
                        return Ok("Only http(s) web addresses can be opened this way.".into());
                    }
                    ps(&format!("Start-Process {}", psq(url))).await?;
                    Ok(format!("Opened {url}").into())
                })
            },
        },
        ToolDef {
            name: "list_windows",
            description: "List visible top-level windows (process name, PID, title) and which one is in front.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "list open windows".into(),
            run: |_ctx, _i| {
                Box::pin(async move {
                    let fg = jarvis_desktop::window::foreground();
                    let out = ps(r#"Get-Process | Where-Object { $_.MainWindowTitle } | ForEach-Object { "$($_.ProcessName)|$($_.Id)|$($_.MainWindowTitle)" }"#).await?;
                    let lines: Vec<String> = out
                        .lines()
                        .filter_map(|l| {
                            let mut p = l.splitn(3, '|');
                            let (name, pid, title) = (p.next()?, p.next()?.parse::<u32>().ok()?, p.next()?);
                            Some(format!("{}{name} (pid {pid}): {title}", if pid == fg.pid { "* " } else { "  " }))
                        })
                        .collect();
                    Ok(format!("{}\n(* = foreground)", lines.join("\n")).into())
                })
            },
        },
        ToolDef {
            name: "focus_window",
            description: "Bring a window to the front by (part of) its title or process name.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
            risk: safe,
            summary: |i| format!("switch to {}", s(i, "query").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let q = s(&i, "query").unwrap_or("");
                    let r = run_ps(
                        &format!(
                            r#"{WIN_NATIVE}
                            $q = {q}
                            $p = Get-Process | Where-Object {{ $_.MainWindowTitle -and ($_.MainWindowTitle -like "*$q*" -or $_.ProcessName -like "*$q*") }} | Select-Object -First 1
                            if (-not $p) {{ "notfound"; exit 1 }}
                            [void][JarvisNative]::Focus($p.MainWindowHandle)
                            $p.MainWindowTitle"#,
                            q = psq(q)
                        ),
                        Duration::from_secs(30),
                    )
                    .await;
                    Ok(if r.code == Some(0) { format!("Focused \"{}\"", r.stdout) } else { format!("No window matching \"{q}\".") }.into())
                })
            },
        },
        ToolDef {
            name: "close_app",
            description: "Close an app by process name or window title. Asks it to close normally; force kills only if force=true.",
            schema: || json!({"type":"object","properties":{"query":{"type":"string"},"force":{"type":"boolean"}},"required":["query"]}),
            risk: unsafe_,
            summary: |i| format!("{} {} (unsaved work may be lost)", if b(i, "force") == Some(true) { "force-close" } else { "close" }, s(i, "query").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let q = s(&i, "query").unwrap_or("");
                    let action = if b(&i, "force") == Some(true) { "Stop-Process -Id $p.Id -Force" } else { "[void]$p.CloseMainWindow()" };
                    let r = run_ps(
                        &format!(
                            r#"$q = {q}
                            $ps = Get-Process | Where-Object {{ $_.ProcessName -like $q -or ($_.MainWindowTitle -and $_.MainWindowTitle -like "*$q*") }}
                            if (-not $ps) {{ "notfound"; exit 1 }}
                            foreach ($p in $ps) {{ {action} ; $p.ProcessName }}"#,
                            q = psq(q)
                        ),
                        Duration::from_secs(30),
                    )
                    .await;
                    Ok(if r.code == Some(0) { format!("Closed: {}", r.stdout.lines().collect::<Vec<_>>().join(", ")) } else { format!("Nothing matching \"{q}\" is running.") }.into())
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
            run: |_ctx, i| {
                Box::pin(async move {
                    let set = match (n(&i, "level"), n(&i, "change")) {
                        (Some(l), _) => format!("$new = {l}"),
                        (_, Some(c)) => format!("$new = [Math]::Max(0, [Math]::Min(100, $cur + ({c})))"),
                        _ => "$new = $cur".into(),
                    };
                    let mute = b(&i, "mute").map(|m| format!("[JarvisNative]::SetMute(${m})")).unwrap_or_default();
                    let out = ps(&format!(
                        r#"{WIN_NATIVE}
                        $cur = [JarvisNative]::GetVolume() * 100
                        {set}
                        [JarvisNative]::SetVolume([float]($new / 100))
                        {mute}
                        "{{0:N0}}|{{1}}" -f ([JarvisNative]::GetVolume() * 100), [JarvisNative]::GetMute()"#
                    ))
                    .await?;
                    let (vol, muted) = out.split_once('|').unwrap_or((&out, "False"));
                    Ok(format!("Volume {vol}%{}", if muted == "True" { " (muted)" } else { "" }).into())
                })
            },
        },
        ToolDef {
            name: "media",
            description: "Control whatever media is playing (Apple Music, browser video, etc.) using Windows media keys: play_pause, next, previous, stop. To play a specific song, open Apple Music and use the screen instead.",
            schema: || json!({"type":"object","properties":{"action":{"type":"string","enum":["play_pause","next","previous","stop"]}},"required":["action"]}),
            risk: safe,
            summary: |i| format!("media {}", s(i, "action").unwrap_or("?").replace('_', "/")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let vk = match s(&i, "action").unwrap_or("") {
                        "play_pause" => 0xB3,
                        "next" => 0xB0,
                        "previous" => 0xB1,
                        _ => 0xB2,
                    };
                    ps(&format!("{WIN_NATIVE}\n[JarvisNative]::Key({vk})")).await?;
                    Ok(format!("Sent {}", s(&i, "action").unwrap_or("")).into())
                })
            },
        },
        ToolDef {
            name: "power",
            description: "Lock, sleep, sign out, restart or shut down the PC.",
            schema: || json!({"type":"object","properties":{"action":{"type":"string","enum":["lock","sleep","sign_out","restart","shutdown"]}},"required":["action"]}),
            risk: |_, i| if s(i, "action") == Some("lock") { Risk::Safe } else { Risk::Unsafe },
            summary: |i| format!("{} the PC", s(i, "action").unwrap_or("?").replace('_', " ")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let action = s(&i, "action").unwrap_or("");
                    let cmd = match action {
                        "lock" => format!("{WIN_NATIVE}\n[void][JarvisNative]::LockWorkStation()"),
                        "sleep" => "Add-Type -AssemblyName System.Windows.Forms; [void][System.Windows.Forms.Application]::SetSuspendState('Suspend', $false, $false)".into(),
                        "sign_out" => "shutdown.exe /l".into(),
                        "restart" => "shutdown.exe /r /t 5".into(),
                        "shutdown" => "shutdown.exe /s /t 5".into(),
                        _ => return Err(anyhow!("unknown power action")),
                    };
                    ps(&cmd).await?;
                    Ok(format!("Done: {action}").into())
                })
            },
        },
        ToolDef {
            name: "run_powershell",
            description: "Run a PowerShell script on the user's PC and return its output. Use only when no dedicated tool fits. Keep scripts short and explain what they do in `purpose`.",
            schema: || json!({"type":"object","properties":{"script":{"type":"string"},"purpose":{"type":"string","description":"Plain-English description of what the script does, shown to the user"},"timeout_seconds":{"type":"number","minimum":1,"maximum":600}},"required":["script","purpose"]}),
            risk: unsafe_,
            summary: |i| format!("run PowerShell: {}", s(i, "purpose").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let t = n(&i, "timeout_seconds").unwrap_or(60.0);
                    let r = run_ps(s(&i, "script").unwrap_or(""), Duration::from_secs_f64(t)).await;
                    let mut out = Vec::new();
                    if !r.stdout.is_empty() {
                        out.push(format!("stdout:\n{}", r.stdout));
                    }
                    if !r.stderr.is_empty() {
                        out.push(format!("stderr:\n{}", r.stderr));
                    }
                    out.push(format!("exit code: {:?}", r.code));
                    let mut text = out.join("\n");
                    if text.len() > 12_000 {
                        text.truncate(text.floor_char_boundary(12_000));
                        text.push_str("\n…[truncated]");
                    }
                    Ok(text.into())
                })
            },
        },
        // --- Files ----------------------------------------------------------------------------------
        ToolDef {
            name: "find_files",
            description: "Search for files or folders by name (wildcards allowed, e.g. '*.pdf', 'report*'). Searches the user's home folder by default.",
            schema: || json!({"type":"object","properties":{"pattern":{"type":"string"},"folder":{"type":"string","description":"Folder to search, e.g. 'Downloads', 'C:\\Projects'. Default: home folder"},"limit":{"type":"number","minimum":1,"maximum":200}},"required":["pattern"]}),
            risk: safe,
            summary: |i| format!("search for {}", s(i, "pattern").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let root = s(&i, "folder").map(resolve_path).unwrap_or_else(home);
                    let pattern = s(&i, "pattern").unwrap_or("*");
                    let limit = n(&i, "limit").unwrap_or(30.0) as u32;
                    let out = ps_t(
                        &format!(
                            r#"Get-ChildItem -LiteralPath {root} -Recurse -Filter {pat} -ErrorAction SilentlyContinue -Force |
                            Where-Object {{ $_.FullName -notmatch '\\(node_modules|AppData|\.git)\\' }} |
                            Sort-Object LastWriteTime -Descending | Select-Object -First {limit} |
                            ForEach-Object {{ "{{0}}  {{1}}  {{2}}" -f $_.LastWriteTime.ToString('yyyy-MM-dd HH:mm'), $(if ($_.PSIsContainer) {{ '<dir>' }} else {{ '{{0:N0}}KB' -f ($_.Length/1KB) }}), $_.FullName }}"#,
                            root = psq(&root.display().to_string()),
                            pat = psq(pattern)
                        ),
                        Duration::from_secs(90),
                    )
                    .await?;
                    Ok(if out.is_empty() { format!("No matches for {pattern} under {}.", root.display()) } else { out }.into())
                })
            },
        },
        ToolDef {
            name: "list_folder",
            description: "List the contents of a folder.",
            schema: || json!({"type":"object","properties":{"folder":{"type":"string"}},"required":["folder"]}),
            risk: safe,
            summary: |i| format!("look in {}", s(i, "folder").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let dir = resolve_path(s(&i, "folder").unwrap_or("~"));
                    let mut lines = vec![dir.display().to_string()];
                    for e in std::fs::read_dir(&dir)?.flatten().take(300) {
                        let name = e.file_name().to_string_lossy().to_string();
                        lines.push(if e.path().is_dir() { format!("[dir] {name}") } else { name });
                    }
                    Ok(lines.join("\n").into())
                })
            },
        },
        ToolDef {
            name: "read_file",
            description: "Read a text file (first 20,000 characters).",
            schema: || json!({"type":"object","properties":{"file":{"type":"string"}},"required":["file"]}),
            risk: safe,
            summary: |i| format!("read {}", s(i, "file").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let bytes = std::fs::read(resolve_path(s(&i, "file").unwrap_or("")))?;
                    let mut text = String::from_utf8_lossy(&bytes).to_string();
                    if text.len() > 20_000 {
                        text.truncate(text.floor_char_boundary(20_000));
                        text.push_str("\n…[truncated]");
                    }
                    Ok(text.into())
                })
            },
        },
        ToolDef {
            name: "write_file",
            description: "Create a text file, or append to / overwrite an existing one.",
            schema: || json!({"type":"object","properties":{"file":{"type":"string"},"content":{"type":"string"},"mode":{"type":"string","enum":["create","append","overwrite"],"description":"Default create (fails if the file exists)"}},"required":["file","content"]}),
            risk: |_, i| if s(i, "mode") == Some("overwrite") && resolve_path(s(i, "file").unwrap_or("")).exists() { Risk::Unsafe } else { Risk::Safe },
            summary: |i| format!("{} {}", s(i, "mode").unwrap_or("create"), s(i, "file").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let p = resolve_path(s(&i, "file").unwrap_or(""));
                    let content = i.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    if let Some(dir) = p.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    match s(&i, "mode").unwrap_or("create") {
                        "create" if p.exists() => return Ok(format!("{} already exists; use append or overwrite.", p.display()).into()),
                        "append" => {
                            use std::io::Write;
                            std::fs::OpenOptions::new().create(true).append(true).open(&p)?.write_all(content.as_bytes())?;
                        }
                        _ => std::fs::write(&p, content)?,
                    }
                    Ok(format!("Wrote {}", p.display()).into())
                })
            },
        },
        ToolDef {
            name: "move_path",
            description: "Move or rename a file or folder. Will not overwrite unless overwrite=true.",
            schema: || json!({"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"},"overwrite":{"type":"boolean"}},"required":["from","to"]}),
            risk: |_, i| if b(i, "overwrite") == Some(true) && resolve_path(s(i, "to").unwrap_or("")).exists() { Risk::Unsafe } else { Risk::Safe },
            summary: |i| format!("move {} → {}", s(i, "from").unwrap_or("?"), s(i, "to").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let src = resolve_path(s(&i, "from").unwrap_or(""));
                    let mut dst = resolve_path(s(&i, "to").unwrap_or(""));
                    if dst.is_dir() {
                        dst = dst.join(src.file_name().unwrap_or_default());
                    }
                    if dst.exists() && b(&i, "overwrite") != Some(true) {
                        return Ok(format!("{} already exists.", dst.display()).into());
                    }
                    if std::fs::rename(&src, &dst).is_err() {
                        // Different drive: copy then delete.
                        copy_recursive(&src, &dst)?;
                        if src.is_dir() { std::fs::remove_dir_all(&src)? } else { std::fs::remove_file(&src)? }
                    }
                    Ok(format!("Moved to {}", dst.display()).into())
                })
            },
        },
        ToolDef {
            name: "delete_path",
            description: "Delete a file or folder by sending it to the Recycle Bin (recoverable).",
            schema: || json!({"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}),
            risk: unsafe_,
            summary: |i| format!("delete {} (to Recycle Bin)", s(i, "target").unwrap_or("?")),
            run: |_ctx, i| {
                Box::pin(async move {
                    let p = resolve_path(s(&i, "target").unwrap_or(""));
                    if !p.exists() {
                        return Ok(format!("{} does not exist.", p.display()).into());
                    }
                    let method = if p.is_dir() { "DeleteDirectory" } else { "DeleteFile" };
                    ps(&format!(
                        "Add-Type -AssemblyName Microsoft.VisualBasic\n[Microsoft.VisualBasic.FileIO.FileSystem]::{method}({}, 'OnlyErrorDialogs', 'SendToRecycleBin')",
                        psq(&p.display().to_string())
                    ))
                    .await?;
                    Ok(format!("Sent {} to the Recycle Bin.", p.display()).into())
                })
            },
        },
    ]
}

fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

pub(super) async fn open_app(target: &str, args: Option<&str>) -> Result<String> {
    open_app_with(target, args, false).await
}

/// Where Windows records installed apps, searched in this order (first match wins): Start menu shortcuts, Store and
/// Start apps, the installed-programs list, registered app paths, and (with a query) running programs. Prints
/// "name<TAB>how to open<TAB>source" lines.
const FIND_APPS: &str = r#"
$q = $env:JARVIS_APP_QUERY
function Emit($name, $open, $src) {
  if (-not $name -or -not $open) { return }
  if ($q -and $name -notlike "*$q*" -and (Split-Path $open -Leaf) -notlike "*$q*") { return }
  "$name`t$open`t$src"
}
$dirs = @("$env:ProgramData\Microsoft\Windows\Start Menu\Programs", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs")
Get-ChildItem $dirs -Recurse -Include *.lnk,*.url -ErrorAction SilentlyContinue |
  Where-Object { $_.BaseName -notmatch '(?i)uninstall|readme|help|manual|website|release notes' } |
  ForEach-Object { Emit $_.BaseName $_.FullName 'Start menu' }
try { Get-StartApps | ForEach-Object { Emit $_.Name "shell:AppsFolder\$($_.AppID)" 'Start apps' } } catch {}
$keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -and -not $_.SystemComponent -and -not $_.ParentKeyName } | ForEach-Object {
  $exe = if ($_.DisplayIcon) { ($_.DisplayIcon -split ',')[0].Trim().Trim('"') } else { '' }
  if ($exe -notlike '*.exe' -or $exe -match '(?i)unins|setup|install' -or -not (Test-Path -LiteralPath $exe)) { $exe = '' }
  if (-not $exe -and $_.InstallLocation -and (Test-Path -LiteralPath $_.InstallLocation)) { $exe = $_.InstallLocation.Trim('"') }
  Emit $_.DisplayName $exe 'Installed programs'
}
foreach ($root in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths') {
  Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {
    $path = $_.GetValue('')
    if ($path) { Emit ($_.PSChildName -replace '\.exe$', '') $path.Trim('"') 'App paths' }
  }
}
if ($q) { Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.MainWindowTitle } | ForEach-Object { Emit $_.ProcessName $_.Path 'Running now' } }
"#;

/// One app per name (the first source wins), sorted, as "name | how to open | source" lines.
pub(crate) fn tidy_app_list(raw: &str, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut apps: Vec<(String, String, String)> = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (name, open, source) = (parts.next()?.trim(), parts.next()?.trim(), parts.next()?.trim());
            (!name.is_empty() && !open.is_empty()).then(|| (name.to_owned(), open.to_owned(), source.to_owned()))
        })
        .filter(|(name, _, _)| seen.insert(name.to_lowercase()))
        .collect();
    apps.sort_by_key(|(name, _, _)| name.to_lowercase());
    apps.into_iter().take(limit).map(|(name, open, source)| format!("{name} | {open} | {source}")).collect()
}

async fn find_apps(query: &str, limit: usize) -> Result<String> {
    let query = query.trim().trim_end_matches(".exe");
    // The query goes in through the environment, never pasted into the script.
    let out = super::ps::run_ps_env(FIND_APPS, &[("JARVIS_APP_QUERY", query)], Duration::from_secs(45)).await;
    let apps = tidy_app_list(&out.stdout, limit);
    Ok(if apps.is_empty() {
        if query.is_empty() { "Could not read the list of installed apps.".into() } else { format!("Nothing called \"{query}\" seems to be installed (searched the Start menu, Store apps, installed programs, app paths and running programs).") }
    } else {
        format!("{} app(s){}:\n{}", apps.len(), if query.is_empty() { String::new() } else { format!(" matching \"{query}\"") }, apps.join("\n"))
    })
}

/// `background` starts the app minimised (the watcher below then keeps it out of the way).
pub(super) async fn open_app_with(target: &str, args: Option<&str>, background: bool) -> Result<String> {
    let t = target.trim();
    let is_uri = t.contains(':') && !t.chars().nth(1).is_some_and(|c| c == ':') && t.split(':').next().is_some_and(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)));
    let arg_part = args.map(|a| format!(" -ArgumentList {}", psq(a))).unwrap_or_default();
    let style = if background { " -WindowStyle Minimized" } else { "" };
    if is_uri || resolve_path(t).exists() {
        let what = if is_uri { t.to_string() } else { resolve_path(t).display().to_string() };
        ps(&format!("Start-Process {}{arg_part}{style}", psq(&what))).await?;
        return Ok(format!("Opened {t}"));
    }
    // A command / App Paths name, then Start-menu shortcuts, then Store apps, then the installed-programs list.
    let r = run_ps(
        &format!(
            r#"$name = {name}
            try {{ Start-Process $name{arg_part}{style} -ErrorAction Stop; "started:$name"; exit 0 }} catch {{}}
            $dirs = @("$env:ProgramData\Microsoft\Windows\Start Menu\Programs", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs")
            $lnk = Get-ChildItem $dirs -Recurse -Include *.lnk,*.url -ErrorAction SilentlyContinue |
              Where-Object {{ $_.BaseName -like "*$name*" -and $_.BaseName -notmatch '(?i)uninstall' }} | Sort-Object {{ $_.BaseName.Length }} | Select-Object -First 1
            if ($lnk) {{ Start-Process $lnk.FullName{arg_part}{style}; "shortcut:$($lnk.BaseName)"; exit 0 }}
            $app = Get-StartApps | Where-Object {{ $_.Name -like "*$name*" }} | Sort-Object {{ $_.Name.Length }} | Select-Object -First 1
            if ($app) {{ Start-Process "shell:AppsFolder\$($app.AppID)"; "store:$($app.Name)"; exit 0 }}
            $keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
            $prog = Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object {{ $_.DisplayName -like "*$name*" -and $_.DisplayIcon }} |
              ForEach-Object {{ $e = ($_.DisplayIcon -split ',')[0].Trim().Trim('"'); if ($e -like '*.exe' -and $e -notmatch '(?i)unins|setup' -and (Test-Path -LiteralPath $e)) {{ [pscustomobject]@{{ Name = $_.DisplayName; Exe = $e }} }} }} |
              Sort-Object {{ $_.Name.Length }} | Select-Object -First 1
            if ($prog) {{ Start-Process $prog.Exe{arg_part}{style}; "installed:$($prog.Name)"; exit 0 }}
            "notfound"; exit 1"#,
            name = psq(t)
        ),
        Duration::from_secs(30),
    )
    .await;
    if r.code != Some(0) {
        return Ok(format!("Could not find an app called \"{t}\". Use find_apps to look for it under another name."));
    }
    Ok(format!("Opened {}", r.stdout.trim().split_once(':').map(|(_, n)| n).unwrap_or(t)))
}

/// Keeps a just-launched app from taking over the screen: any window that appears in the next few seconds is
/// minimised, and the window the user was using gets the focus back.
#[cfg(windows)]
struct BackgroundLaunch {
    front: usize,
    before: std::collections::HashSet<usize>,
}

#[cfg(windows)]
impl BackgroundLaunch {
    fn start() -> Self {
        use jarvis_desktop::window;
        Self { front: window::foreground().hwnd, before: window::top_level().into_iter().map(|(h, _)| h).collect() }
    }

    fn keep_out_of_the_way(self) {
        use jarvis_desktop::window;
        tokio::spawn(async move {
            let me = std::process::id();
            let mut handled = std::collections::HashSet::new();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let fresh: Vec<usize> = window::top_level()
                    .into_iter()
                    .filter(|(h, pid)| *pid != me && !self.before.contains(h) && !handled.contains(h))
                    .map(|(h, _)| h)
                    .collect();
                for hwnd in fresh {
                    handled.insert(hwnd);
                    window::minimize(hwnd);
                }
                // Only take the focus back from a window this launch opened, never from one the user picked.
                if handled.contains(&window::foreground().hwnd) || window::foreground().hwnd == 0 {
                    window::give_focus(self.front);
                }
            }
            if handled.contains(&window::foreground().hwnd) {
                window::give_focus(self.front);
            }
        });
    }
}
/// For tools that want to send plain text built from JSON.
pub fn _unused(_: Value) -> ToolOutput {
    ToolOutput::Text(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_lists_are_deduped_sorted_and_capped() {
        let raw = "Steam\tC:\\Steam\\steam.exe\tStart menu\nsteam\tC:\\other\\steam.exe\tInstalled programs\nDiscord\tC:\\d.lnk\tStart menu\n\tC:\\nameless.exe\tApp paths\nBroken line\n7-Zip\tC:\\7z.exe\tInstalled programs";
        let apps = tidy_app_list(raw, 10);
        assert_eq!(apps, vec!["7-Zip | C:\\7z.exe | Installed programs", "Discord | C:\\d.lnk | Start menu", "Steam | C:\\Steam\\steam.exe | Start menu"]);
        assert_eq!(tidy_app_list(raw, 1).len(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "reads this PC's installed apps"]
    async fn finds_installed_apps_live() {
        let all = find_apps("", 300).await.unwrap();
        println!("{}", all.lines().take(12).collect::<Vec<_>>().join("\n"));
        assert!(all.lines().count() > 10, "{all}");
        let steam = find_apps("steam", 20).await.unwrap();
        println!("{steam}");
        assert!(steam.to_lowercase().contains("steam |"), "{steam}");
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "opens and closes Character Map on this PC"]
    async fn opens_an_app_in_the_background_live() {
        use jarvis_desktop::window;
        let front = window::foreground();
        let watch = BackgroundLaunch::start();
        let result = open_app_with("charmap", None, true).await.unwrap();
        assert!(result.starts_with("Opened"), "{result}");
        watch.keep_out_of_the_way();
        tokio::time::sleep(Duration::from_secs(4)).await;
        let after = window::foreground();
        let charmap = window::find("Character Map");
        println!("front before: {} ({}), after: {} ({}), charmap visible: {:?}", front.title, front.exe, after.title, after.exe, charmap.as_ref().map(|f| &f.title));
        let _ = std::process::Command::new("taskkill").args(["/IM", "charmap.exe", "/F"]).output();
        assert!(!after.exe.eq_ignore_ascii_case("charmap.exe"), "Character Map must not be in front");
        assert!(charmap.is_none(), "window::find skips minimised windows, so Character Map should not be found");
    }

    #[cfg(windows)]
    #[test]
    fn resolves_known_folders() {
        let h = home();
        assert_eq!(resolve_path("Downloads/x.pdf"), h.join("Downloads").join("x.pdf"));
        assert_eq!(resolve_path("~/notes.txt"), PathBuf::from(format!("{}/notes.txt", h.display())));
        assert_eq!(resolve_path("C:\\Temp\\a.txt"), PathBuf::from("C:\\Temp\\a.txt"));
    }
}

