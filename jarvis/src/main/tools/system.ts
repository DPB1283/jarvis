import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { z } from "zod";
import { defineTool } from "./types";
import { ps, psq, runPs, WIN_NATIVE } from "./ps";

const HOME = os.homedir();

/** Expand ~ and %ENV% and make absolute. */
export function resolvePath(p: string): string {
  let s = p.trim().replace(/%([^%]+)%/g, (_, v) => process.env[v] ?? `%${v}%`);
  if (s === "~" || s.startsWith("~/") || s.startsWith("~\\")) s = path.join(HOME, s.slice(1));
  const known: Record<string, string> = {
    desktop: path.join(HOME, "Desktop"),
    documents: path.join(HOME, "Documents"),
    downloads: path.join(HOME, "Downloads"),
    pictures: path.join(HOME, "Pictures"),
    music: path.join(HOME, "Music"),
    videos: path.join(HOME, "Videos"),
  };
  const first = s.split(/[\\/]/)[0].toLowerCase();
  if (!path.isAbsolute(s) && known[first]) s = path.join(known[first], s.slice(first.length));
  return path.resolve(HOME, s);
}

// --- time / info -----------------------------------------------------------

export const getTime = defineTool({
  name: "get_time",
  description: "Get the current local date, time and timezone.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check the time",
  run: async () => {
    const now = new Date();
    return `${now.toLocaleString("en-GB", { dateStyle: "full", timeStyle: "short" })} (${Intl.DateTimeFormat().resolvedOptions().timeZone})`;
  },
});

export const systemInfo = defineTool({
  name: "system_info",
  description: "Battery, CPU load, memory, disk space, uptime and OS version.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check system status",
  run: async () => {
    const extra = await runPs(`
      $b = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1
      $cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
      $disks = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | ForEach-Object { "{0} {1:N0}GB free of {2:N0}GB" -f $_.DeviceID, ($_.FreeSpace/1GB), ($_.Size/1GB) }
      "battery=" + $(if ($b) { "$($b.EstimatedChargeRemaining)% status=$($b.BatteryStatus)" } else { "none" })
      "cpu_load=$cpu%"
      "disks=" + ($disks -join "; ")
    `);
    const mem = `${(os.freemem() / 2 ** 30).toFixed(1)}GB free of ${(os.totalmem() / 2 ** 30).toFixed(1)}GB`;
    return [
      `os=${os.version()} ${os.release()}`,
      `cpu=${os.cpus()[0]?.model} x${os.cpus().length}`,
      `memory=${mem}`,
      `uptime=${(os.uptime() / 3600).toFixed(1)}h`,
      extra.stdout,
    ].join("\n");
  },
});

// --- apps / windows --------------------------------------------------------

export const openApp = defineTool({
  name: "open_app",
  description:
    "Open an application, file, folder, URL or ms-settings: page. Accepts an app name as it appears in the Start menu (e.g. 'Notepad', 'Apple Music', 'Discord'), an exe name, a path, or a URL.",
  schema: z.object({
    target: z.string().describe("App name, exe, path, folder, URL or ms-settings: URI"),
    args: z.string().optional().describe("Optional command-line arguments"),
  }),
  risk: "safe",
  summary: (i) => `open ${i.target}`,
  run: async ({ target, args }) => {
    const t = target.trim();
    const isUri = /^[a-z][a-z0-9+.-]*:/i.test(t) && !/^[a-z]:[\\/]/i.test(t);
    const argPart = args ? ` -ArgumentList ${psq(args)}` : "";
    if (isUri || fs.existsSync(resolvePath(t))) {
      await ps(`Start-Process ${psq(isUri ? t : resolvePath(t))}${argPart}`);
      return `Opened ${t}`;
    }
    // Try as a command / App Paths name, then search Start menu shortcuts and Store apps.
    const r = await runPs(`
      $name = ${psq(t)}
      try { Start-Process $name${argPart} -ErrorAction Stop; "started:$name"; exit 0 } catch {}
      $dirs = @("$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs", "$env:APPDATA\\Microsoft\\Windows\\Start Menu\\Programs")
      $lnk = Get-ChildItem $dirs -Recurse -Include *.lnk,*.url -ErrorAction SilentlyContinue |
        Where-Object { $_.BaseName -like "*$name*" } | Sort-Object { $_.BaseName.Length } | Select-Object -First 1
      if ($lnk) { Start-Process $lnk.FullName${argPart}; "shortcut:$($lnk.BaseName)"; exit 0 }
      $app = Get-StartApps | Where-Object { $_.Name -like "*$name*" } | Sort-Object { $_.Name.Length } | Select-Object -First 1
      if ($app) { Start-Process "shell:AppsFolder\\$($app.AppID)"; "store:$($app.Name)"; exit 0 }
      "notfound"; exit 1
    `);
    if (r.code !== 0) return `Could not find an app called "${t}".`;
    return `Opened ${r.stdout.split(":").slice(1).join(":") || t}`;
  },
});

export const listWindows = defineTool({
  name: "list_windows",
  description: "List visible top-level windows (process name, PID, title) and which one is in front.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "list open windows",
  run: async () =>
    ps(`
      Add-Type @"
      using System; using System.Runtime.InteropServices;
      public static class JFg { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid); }
"@
      $fgPid = 0; [void][JFg]::GetWindowThreadProcessId([JFg]::GetForegroundWindow(), [ref]$fgPid)
      Get-Process | Where-Object { $_.MainWindowTitle } | ForEach-Object {
        $mark = if ($_.Id -eq $fgPid) { "* " } else { "  " }
        "$mark$($_.ProcessName) (pid $($_.Id)): $($_.MainWindowTitle)"
      }
      "(* = foreground)"
    `),
});

export const focusWindow = defineTool({
  name: "focus_window",
  description: "Bring a window to the front by (part of) its title or process name.",
  schema: z.object({ query: z.string() }),
  risk: "safe",
  summary: (i) => `switch to ${i.query}`,
  run: async ({ query }) => {
    const r = await runPs(`${WIN_NATIVE}
      $q = ${psq(query)}
      $p = Get-Process | Where-Object { $_.MainWindowTitle -and ($_.MainWindowTitle -like "*$q*" -or $_.ProcessName -like "*$q*") } | Select-Object -First 1
      if (-not $p) { "notfound"; exit 1 }
      [void][JarvisNative]::Focus($p.MainWindowHandle)
      $p.MainWindowTitle
    `);
    return r.code === 0 ? `Focused "${r.stdout}"` : `No window matching "${query}".`;
  },
});

export const closeApp = defineTool({
  name: "close_app",
  description: "Close an app by process name or window title. Asks it to close normally; force kills only if force=true.",
  schema: z.object({ query: z.string(), force: z.boolean().optional() }),
  risk: "unsafe",
  summary: (i) => `${i.force ? "force-close" : "close"} ${i.query} (unsaved work may be lost)`,
  run: async ({ query, force }) => {
    const r = await runPs(`
      $q = ${psq(query)}
      $ps = Get-Process | Where-Object { $_.ProcessName -like $q -or ($_.MainWindowTitle -and $_.MainWindowTitle -like "*$q*") }
      if (-not $ps) { "notfound"; exit 1 }
      foreach ($p in $ps) { ${force ? "Stop-Process -Id $p.Id -Force" : "[void]$p.CloseMainWindow()"} ; $p.ProcessName }
    `);
    return r.code === 0 ? `Closed: ${r.stdout.split(/\r?\n/).join(", ")}` : `Nothing matching "${query}" is running.`;
  },
});

// --- audio / media ---------------------------------------------------------

export const setVolume = defineTool({
  name: "set_volume",
  description: "Set or adjust the system master volume (0-100), or mute/unmute. Returns the new level.",
  schema: z.object({
    level: z.number().min(0).max(100).optional().describe("Absolute level 0-100"),
    change: z.number().min(-100).max(100).optional().describe("Relative change, e.g. +10 or -20"),
    mute: z.boolean().optional(),
  }),
  risk: "safe",
  summary: (i) =>
    i.mute !== undefined ? (i.mute ? "mute" : "unmute") : i.level !== undefined ? `set volume to ${i.level}%` : `change volume by ${i.change}`,
  run: async ({ level, change, mute }) => {
    const out = await ps(`${WIN_NATIVE}
      $cur = [JarvisNative]::GetVolume() * 100
      ${level !== undefined ? `$new = ${level}` : change !== undefined ? `$new = [Math]::Max(0, [Math]::Min(100, $cur + (${change})))` : `$new = $cur`}
      [JarvisNative]::SetVolume([float]($new / 100))
      ${mute !== undefined ? `[JarvisNative]::SetMute($${mute})` : ""}
      "{0:N0}|{1}" -f ([JarvisNative]::GetVolume() * 100), [JarvisNative]::GetMute()
    `);
    const [vol, muted] = out.split("|");
    return `Volume ${vol}%${muted === "True" ? " (muted)" : ""}`;
  },
});

const MEDIA_KEYS = { play_pause: 0xb3, next: 0xb0, previous: 0xb1, stop: 0xb2 } as const;

export const media = defineTool({
  name: "media",
  description:
    "Control whatever media is playing (Apple Music, browser video, etc.) using Windows media keys: play_pause, next, previous, stop. To play a specific song, open Apple Music and use the screen instead.",
  schema: z.object({ action: z.enum(["play_pause", "next", "previous", "stop"]) }),
  risk: "safe",
  summary: (i) => `media ${i.action.replace("_", "/")}`,
  run: async ({ action }) => {
    await ps(`${WIN_NATIVE}\n[JarvisNative]::Key(${MEDIA_KEYS[action]})`);
    return `Sent ${action}`;
  },
});

// --- power -----------------------------------------------------------------

export const power = defineTool({
  name: "power",
  description: "Lock, sleep, sign out, restart or shut down the PC.",
  schema: z.object({ action: z.enum(["lock", "sleep", "sign_out", "restart", "shutdown"]) }),
  risk: (i) => (i.action === "lock" ? "safe" : "unsafe"),
  summary: (i) => `${i.action.replace("_", " ")} the PC`,
  run: async ({ action }) => {
    const cmd = {
      lock: `${WIN_NATIVE}\n[void][JarvisNative]::LockWorkStation()`,
      sleep: `Add-Type -AssemblyName System.Windows.Forms; [void][System.Windows.Forms.Application]::SetSuspendState('Suspend', $false, $false)`,
      sign_out: `shutdown.exe /l`,
      restart: `shutdown.exe /r /t 5`,
      shutdown: `shutdown.exe /s /t 5`,
    }[action];
    await ps(cmd);
    return `Done: ${action}`;
  },
});

// --- shell escape hatch ----------------------------------------------------

export const runPowershell = defineTool({
  name: "run_powershell",
  description:
    "Run a PowerShell script on the user's PC and return its output. Use only when no dedicated tool fits. Keep scripts short and explain what they do in `purpose`.",
  schema: z.object({
    script: z.string(),
    purpose: z.string().describe("Plain-English description of what the script does, shown to the user"),
    timeout_seconds: z.number().min(1).max(600).optional(),
  }),
  risk: "unsafe",
  summary: (i) => `run PowerShell: ${i.purpose}`,
  run: async ({ script, timeout_seconds }) => {
    const r = await runPs(script, (timeout_seconds ?? 60) * 1000);
    const out = [r.stdout && `stdout:\n${r.stdout}`, r.stderr && `stderr:\n${r.stderr}`, `exit code: ${r.code}`]
      .filter(Boolean)
      .join("\n");
    return out.length > 12_000 ? out.slice(0, 12_000) + "\n…[truncated]" : out;
  },
});

export const openUrl = defineTool({
  name: "open_url",
  description: "Open a web page in the user's default browser.",
  schema: z.object({ url: z.string().url() }),
  risk: "safe",
  summary: (i) => `open ${i.url}`,
  run: async ({ url }) => {
    await ps(`Start-Process ${psq(url)}`);
    return `Opened ${url}`;
  },
});
