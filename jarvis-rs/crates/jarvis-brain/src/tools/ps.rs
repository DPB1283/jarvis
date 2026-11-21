//! Runs Windows PowerShell scripts (port of `tools/ps.ts`). Scripts go via -EncodedCommand (no quoting
//! problems) or, past Windows' ~32k command-line limit, a temporary .ps1 file. Many tools reuse the exact
//! scripts proven in the Electron app; they move to native Win32 calls piece by piece.


use super::NoWindow;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::Engine;
use tokio::io::AsyncReadExt;


pub struct PsResult {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

pub async fn run_ps(script: &str, timeout: Duration) -> PsResult {
    if cfg!(not(windows)) {
        return PsResult {
            stdout: String::new(),
            stderr: "This action needs Windows PowerShell, which is not available on Linux.".into(),
            code: Some(-1),
        };
    }
    let full = format!("$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[Text.Encoding]::UTF8;\n{script}");
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&utf16);
    let mut temp: Option<std::path::PathBuf> = None;
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]);
    if encoded.len() < 30_000 {
        cmd.args(["-EncodedCommand", &encoded]);
    } else {
        let p = std::env::temp_dir().join(format!("jarvis-{}.ps1", uuid::Uuid::new_v4()));
        let mut bytes = vec![0xEF, 0xBB, 0xBF]; // BOM so Windows PowerShell reads UTF-8
        bytes.extend_from_slice(full.as_bytes());
        if let Err(e) = std::fs::write(&p, bytes) {
            return PsResult { stdout: String::new(), stderr: e.to_string(), code: Some(-1) };
        }
        cmd.arg("-File").arg(&p);
        temp = Some(p);
    }
    cmd.no_window().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let result = async {
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
        let (mut so, mut se) = (Vec::new(), Vec::new());
        let (_, _, status) = tokio::join!(out.read_to_end(&mut so), err.read_to_end(&mut se), child.wait());
        let status = status.map_err(|e| e.to_string())?;
        Ok::<_, String>(PsResult {
            stdout: String::from_utf8_lossy(&so).trim().to_string(),
            stderr: String::from_utf8_lossy(&se).trim().to_string(),
            code: status.code(),
        })
    };
    let r = match tokio::time::timeout(timeout, result).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => PsResult { stdout: String::new(), stderr: e, code: Some(-1) },
        Err(_) => PsResult { stdout: String::new(), stderr: format!("[timed out after {timeout:?}]"), code: None },
    };
    if let Some(p) = temp {
        let _ = std::fs::remove_file(p);
    }
    r
}

/// Run and return stdout, or an error with stderr.
pub async fn ps(script: &str) -> Result<String> {
    ps_t(script, Duration::from_secs(30)).await
}

pub async fn ps_t(script: &str, timeout: Duration) -> Result<String> {
    let r = run_ps(script, timeout).await;
    if r.code != Some(0) {
        return Err(anyhow!(if r.stderr.is_empty() { format!("PowerShell exited with {:?}", r.code) } else { r.stderr }));
    }
    Ok(r.stdout)
}

/// Single-quote a string for PowerShell.
pub fn psq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// C# helpers (volume, media keys, window focus) compiled into the PowerShell session that needs them.
pub const WIN_NATIVE: &str = r#"
if (-not ('JarvisNative' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IAudioEndpointVolume {
  int f(); int g(); int h(); int i();
  int SetMasterVolumeLevelScalar(float fLevel, Guid pguidEventContext);
  int j();
  int GetMasterVolumeLevelScalar(out float pfLevel);
  int k(); int l(); int m(); int n();
  int SetMute([MarshalAs(UnmanagedType.Bool)] bool bMute, Guid pguidEventContext);
  int GetMute(out bool pbMute);
}
[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice { int Activate(ref Guid id, int clsCtx, int activationParams, out IAudioEndpointVolume aev); }
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator { int f(); int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice endpoint); }
[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumeratorComObject { }
public static class JarvisNative {
  static IAudioEndpointVolume Vol() {
    var enumerator = new MMDeviceEnumeratorComObject() as IMMDeviceEnumerator;
    IMMDevice dev = null;
    Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(0, 1, out dev));
    IAudioEndpointVolume epv = null;
    var epvid = typeof(IAudioEndpointVolume).GUID;
    Marshal.ThrowExceptionForHR(dev.Activate(ref epvid, 23, 0, out epv));
    return epv;
  }
  public static float GetVolume() { float v = -1; Marshal.ThrowExceptionForHR(Vol().GetMasterVolumeLevelScalar(out v)); return v; }
  public static void SetVolume(float v) { Marshal.ThrowExceptionForHR(Vol().SetMasterVolumeLevelScalar(v, Guid.Empty)); }
  public static bool GetMute() { bool m; Marshal.ThrowExceptionForHR(Vol().GetMute(out m)); return m; }
  public static void SetMute(bool m) { Marshal.ThrowExceptionForHR(Vol().SetMute(m, Guid.Empty)); }
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  public static void Key(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); keybd_event(vk, 0, 2, UIntPtr.Zero); }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool LockWorkStation();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr unused);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  public static bool Focus(IntPtr h) {
    IntPtr fg = GetForegroundWindow();
    if (fg == h) return true;
    uint fgThread = GetWindowThreadProcessId(fg, IntPtr.Zero);
    uint me = GetCurrentThreadId();
    keybd_event(0x12, 0, 1, UIntPtr.Zero);
    bool attached = fgThread != 0 && fgThread != me && AttachThreadInput(me, fgThread, true);
    ShowWindow(h, IsIconic(h) ? 9 : 5);
    BringWindowToTop(h);
    SetForegroundWindow(h);
    if (attached) AttachThreadInput(me, fgThread, false);
    keybd_event(0x12, 0, 3, UIntPtr.Zero);
    return GetForegroundWindow() == h;
  }
}
"@
}
"#;

