// Direct Win32 calls via koffi (no PowerShell spawn): cheap enough to poll every few seconds.
// Maps 1:1 to the `windows` crate in the Rust port.
import koffi from "koffi";

const user32 = koffi.load("user32.dll");
const kernel32 = koffi.load("kernel32.dll");
const shell32 = koffi.load("shell32.dll");

const RECT = koffi.struct("JRECT", { left: "int32", top: "int32", right: "int32", bottom: "int32" });
const MONITORINFO = koffi.struct("JMONITORINFO", { cbSize: "uint32", rcMonitor: RECT, rcWork: RECT, dwFlags: "uint32" });
export type Rect = { left: number; top: number; right: number; bottom: number };

const GetForegroundWindow = user32.func("void* __stdcall GetForegroundWindow()");
const GetWindowThreadProcessId = user32.func("uint32 __stdcall GetWindowThreadProcessId(void* hWnd, _Out_ uint32* pid)");
const GetWindowRect = user32.func("bool __stdcall GetWindowRect(void* hWnd, _Out_ JRECT* rect)");
const MonitorFromWindow = user32.func("void* __stdcall MonitorFromWindow(void* hWnd, uint32 flags)");
const GetMonitorInfoW = user32.func("bool __stdcall GetMonitorInfoW(void* hMonitor, _Inout_ JMONITORINFO* info)");
const GetClassNameW = user32.func("int __stdcall GetClassNameW(void* hWnd, void* buf, int max)");
const OpenProcess = kernel32.func("void* __stdcall OpenProcess(uint32 access, bool inherit, uint32 pid)");
const QueryFullProcessImageNameW = kernel32.func("bool __stdcall QueryFullProcessImageNameW(void* h, uint32 flags, void* buf, _Inout_ uint32* size)");
const CloseHandle = kernel32.func("bool __stdcall CloseHandle(void* h)");
const SHQueryUserNotificationState = shell32.func("int __stdcall SHQueryUserNotificationState(_Out_ int* state)");

// --- Monitors & cursor in physical pixels (per-monitor DPI aware, so mixed scaling works) ---------------
const POINT = koffi.struct("JPOINT", { x: "int32", y: "int32" });
const SetThreadDpiAwarenessContext = user32.func("intptr_t __stdcall SetThreadDpiAwarenessContext(intptr_t ctx)");
const SetCursorPos = user32.func("bool __stdcall SetCursorPos(int x, int y)");
const GetCursorPos = user32.func("bool __stdcall GetCursorPos(_Out_ JPOINT* p)");
const MonitorEnumProc = koffi.proto("bool __stdcall JMonitorEnumProc(void* hMon, void* hdc, JRECT* rect, intptr_t data)");
const EnumDisplayMonitors = user32.func("bool __stdcall EnumDisplayMonitors(void* hdc, void* clip, JMonitorEnumProc* cb, intptr_t data)");
const PER_MONITOR_AWARE_V2 = -4;

function perMonitor<T>(fn: () => T): T {
  const prev = SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2);
  try {
    return fn();
  } finally {
    if (prev) SetThreadDpiAwarenessContext(prev);
  }
}

export interface Monitor {
  /** 1-based, primary first then left-to-right. */
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
  primary: boolean;
}

/** All monitors in physical pixels on the virtual desktop. */
export function listMonitors(): Monitor[] {
  const found: Omit<Monitor, "index">[] = [];
  perMonitor(() =>
    EnumDisplayMonitors(null, null, (hMon: unknown) => {
      const info = { cbSize: koffi.sizeof(MONITORINFO), rcMonitor: { left: 0, top: 0, right: 0, bottom: 0 }, rcWork: { left: 0, top: 0, right: 0, bottom: 0 }, dwFlags: 0 };
      if (GetMonitorInfoW(hMon, info)) {
        const m = info.rcMonitor as Rect;
        found.push({ x: m.left, y: m.top, width: m.right - m.left, height: m.bottom - m.top, primary: (info.dwFlags & 1) === 1 });
      }
      return true;
    }, 0),
  );
  found.sort((a, b) => Number(b.primary) - Number(a.primary) || a.x - b.x || a.y - b.y);
  return found.map((m, i) => ({ index: i + 1, ...m }));
}

export function setCursorPhysical(x: number, y: number): void {
  perMonitor(() => SetCursorPos(Math.round(x), Math.round(y)));
}

export function getCursorPhysical(): { x: number; y: number } {
  const p = { x: 0, y: 0 };
  perMonitor(() => GetCursorPos(p));
  return p;
}

/** Which monitor (1-based) holds the foreground window's centre; 1 if unknown. */
export function foregroundMonitorIndex(monitors = listMonitors()): number {
  const h = GetForegroundWindow();
  if (!h) return 1;
  const r: Rect = { left: 0, top: 0, right: 0, bottom: 0 };
  if (!perMonitor(() => GetWindowRect(h, r))) return 1;
  const cx = (r.left + r.right) / 2;
  const cy = (r.top + r.bottom) / 2;
  return monitors.find((m) => cx >= m.x && cx < m.x + m.width && cy >= m.y && cy < m.y + m.height)?.index ?? 1;
}

const LASTINPUTINFO = koffi.struct("JLASTINPUTINFO", { cbSize: "uint32", dwTime: "uint32" });
const GetLastInputInfo = user32.func("bool __stdcall GetLastInputInfo(_Inout_ JLASTINPUTINFO* info)");
const GetTickCount = kernel32.func("uint32 __stdcall GetTickCount()");

/** Seconds since the last keyboard/mouse input. */
export function idleSeconds(): number {
  const info = { cbSize: koffi.sizeof(LASTINPUTINFO), dwTime: 0 };
  if (!GetLastInputInfo(info)) return 0;
  return ((GetTickCount() - info.dwTime) >>> 0) / 1000;
}

const QUNS_RUNNING_D3D_FULL_SCREEN = 3;

export function foregroundPid(): number {
  const h = GetForegroundWindow();
  if (!h) return 0;
  const pid = [0];
  GetWindowThreadProcessId(h, pid);
  return pid[0];
}

/** Executable name (e.g. "cs2.exe") for a process id, or "" if it can't be read. */
export function processName(pid: number): string {
  if (!pid) return "";
  const h = OpenProcess(0x1000 /* PROCESS_QUERY_LIMITED_INFORMATION */, false, pid);
  if (!h) return "";
  try {
    const buf = Buffer.alloc(2048);
    const size = [1024];
    if (!QueryFullProcessImageNameW(h, 0, buf, size)) return "";
    const full = buf.toString("utf16le", 0, size[0] * 2);
    return full.slice(full.lastIndexOf("\\") + 1);
  } finally {
    CloseHandle(h);
  }
}

function className(h: unknown): string {
  const buf = Buffer.alloc(512);
  const n = GetClassNameW(h, buf, 256);
  return buf.toString("utf16le", 0, n * 2);
}

const DESKTOP_CLASSES = new Set(["Progman", "WorkerW", "Shell_TrayWnd", "Windows.UI.Core.CoreWindow"]);

/**
 * Is the foreground app fullscreen (exclusive D3D, or a borderless window covering its whole monitor)?
 * `monitor` is that window's monitor rectangle in physical pixels (to tell which screen the game is on).
 */
export function foregroundFullscreen(): { fullscreen: boolean; pid: number; exe: string; monitor: Rect | null } {
  const h = GetForegroundWindow();
  const pid = foregroundPid();
  const exe = processName(pid);
  if (!h) return { fullscreen: false, pid, exe, monitor: null };
  return perMonitor(() => {
    const mon = MonitorFromWindow(h, 2 /* MONITOR_DEFAULTTONEAREST */);
    const info = { cbSize: koffi.sizeof(MONITORINFO), rcMonitor: { left: 0, top: 0, right: 0, bottom: 0 }, rcWork: { left: 0, top: 0, right: 0, bottom: 0 }, dwFlags: 0 };
    const monitor = mon && GetMonitorInfoW(mon, info) ? ({ ...(info.rcMonitor as Rect) } as Rect) : null;
    const state = [0];
    if (SHQueryUserNotificationState(state) === 0 && state[0] === QUNS_RUNNING_D3D_FULL_SCREEN) return { fullscreen: true, pid, exe, monitor };
    if (DESKTOP_CLASSES.has(className(h)) || !monitor) return { fullscreen: false, pid, exe, monitor };
    const r: Rect = { left: 0, top: 0, right: 0, bottom: 0 };
    if (!GetWindowRect(h, r)) return { fullscreen: false, pid, exe, monitor };
    const covers = r.left <= monitor.left && r.top <= monitor.top && r.right >= monitor.right && r.bottom >= monitor.bottom;
    return { fullscreen: covers, pid, exe, monitor };
  });
}
