import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";

export interface PsResult {
  stdout: string;
  stderr: string;
  code: number | null;
}

/**
 * Run a Windows PowerShell script. Normally passed via -EncodedCommand (no quoting problems); scripts too big
 * for Windows' ~32k command-line limit (e.g. pasting a long report) go through a temporary .ps1 file instead.
 */
export function runPs(script: string, timeoutMs = 30_000): Promise<PsResult> {
  const full = "$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[Text.Encoding]::UTF8;\n" + script;
  const encoded = Buffer.from(full, "utf16le").toString("base64");
  let tempFile: string | null = null;
  let args: string[];
  if (encoded.length < 30_000) {
    args = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", encoded];
  } else {
    tempFile = path.join(os.tmpdir(), `jarvis-${randomUUID()}.ps1`);
    fs.writeFileSync(tempFile, "﻿" + full, "utf8"); // BOM so Windows PowerShell reads it as UTF-8
    args = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", tempFile];
  }
  const cleanup = () => {
    if (tempFile) fs.rm(tempFile, () => {});
  };
  return new Promise((resolve) => {
    const child = spawn("powershell.exe", args, { windowsHide: true });
    child.on("close", cleanup);
    child.on("error", cleanup);
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    const timer = setTimeout(() => {
      child.kill();
      stderr += `\n[timed out after ${timeoutMs} ms]`;
    }, timeoutMs);
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ stdout: stdout.trim(), stderr: stderr.trim(), code });
    });
    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({ stdout, stderr: String(err), code: -1 });
    });
  });
}

/** Single-quote a string for PowerShell. */
export const psq = (s: string) => `'${s.replace(/'/g, "''")}'`;

export async function ps(script: string, timeoutMs?: number): Promise<string> {
  const r = await runPs(script, timeoutMs);
  if (r.code !== 0) throw new Error(r.stderr || `PowerShell exited with ${r.code}`);
  return r.stdout;
}

/** C# helpers for volume (Core Audio) and media keys, compiled once per PowerShell call. */
export const WIN_NATIVE = `
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
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  public static string ForegroundTitle() { var sb = new System.Text.StringBuilder(512); GetWindowText(GetForegroundWindow(), sb, 512); return sb.ToString(); }
  [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr hWnd, out int pid);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr unused);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  /// Bring a window to the front despite Windows' foreground lock (the technique AutoHotkey's WinActivate uses):
  /// hold Alt, attach to the foreground window's input queue, raise + activate, detach.
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
`;
