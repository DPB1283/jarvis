import path from "node:path";
import { app, BrowserWindow, globalShortcut, Menu, nativeImage, Notification, screen, session, Tray } from "electron";
import { PALETTE, type OrbCorner, type SafetyMode, type Settings } from "../shared/settings";
import { bus } from "./bus";
import { getSettings, updateSettings } from "./config";
import { Brain } from "./brain/agent";
import { registerIpc } from "./ipc";
import { startReminderScheduler } from "./memory/reminders";
import { playDismissChime, speechPending, startVoice } from "./voice";
import { ultronCommand } from "../shared/ultron";
import { isDismissal } from "./dismiss";
import { getRoutine } from "./memory/db";
import { startRoutineScheduler } from "./memory/routines-scheduler";
import { listTimers, startTimers } from "./timers";
import { gameModeState, setHudLocator, startGameModeWatcher } from "./gamemode";
import { startHealthWatch } from "./health";
import { startUsageTracking } from "./usage";
import type { JarvisState } from "../shared/events";

if (!app.requestSingleInstanceLock()) app.quit();

const PRELOAD = path.join(__dirname, "../preload/index.js");
let hud: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let tray: Tray | null = null;
const cleanups: (() => void)[] = [];

function loadRenderer(win: BrowserWindow, hash = "") {
  const dev = process.env.ELECTRON_RENDERER_URL;
  if (dev) void win.loadURL(dev + hash);
  else void win.loadFile(path.join(__dirname, "../renderer/index.html"), { hash: hash.replace(/^#/, "") });
}

// The HUD is a small window anchored to the orb's corner. A full-screen transparent always-on-top
// window makes Windows recomposite the whole desktop every animation frame (and slows fullscreen games).
const HUD_W = 520;
const HUD_H = 640;
const HUD_PAD = 14; // .hud padding in style.css: the orb sits this far in from its corner of the window

/** Where the orb's top-left sits inside the HUD window for a given layout corner. */
function orbOffset(corner: OrbCorner, size: number) {
  return {
    x: corner.endsWith("right") ? HUD_W - HUD_PAD - size : HUD_PAD,
    y: corner.startsWith("bottom") ? HUD_H - HUD_PAD - size : HUD_PAD,
  };
}

/** Keeps the whole orb inside the work area of the display it's on (the transparent window may hang off-screen). */
function clampOrb(p: { x: number; y: number }, size: number) {
  const wa = screen.getDisplayNearestPoint({ x: Math.round(p.x + size / 2), y: Math.round(p.y + size / 2) }).workArea;
  return {
    x: Math.round(Math.min(Math.max(p.x, wa.x), wa.x + wa.width - size)),
    y: Math.round(Math.min(Math.max(p.y, wa.y), wa.y + wa.height - size)),
  };
}

function hudBounds(s: Settings): Electron.Rectangle {
  if (s.orb.position) {
    const orb = clampOrb(s.orb.position, s.orb.size);
    const off = orbOffset(s.orb.corner, s.orb.size);
    return { x: orb.x - off.x, y: orb.y - off.y, width: HUD_W, height: HUD_H };
  }
  const wa = screen.getPrimaryDisplay().workArea;
  const right = s.orb.corner.endsWith("right");
  const bottom = s.orb.corner.startsWith("bottom");
  return {
    x: right ? wa.x + wa.width - HUD_W : wa.x,
    y: bottom ? wa.y + wa.height - HUD_H : wa.y,
    width: HUD_W,
    height: HUD_H,
  };
}

// --- Dragging the orb: the window follows the cursor; on drop, captions flip to the side with room. ---
let dragTimer: ReturnType<typeof setInterval> | undefined;
let dragOffset = { x: 0, y: 0 };

function dragHud(phase: "start" | "end") {
  if (!hud) return;
  if (phase === "start") {
    hud.setIgnoreMouseEvents(false); // keep receiving the pointer until the drop
    const c = screen.getCursorScreenPoint();
    const b = hud.getBounds();
    dragOffset = { x: c.x - b.x, y: c.y - b.y };
    clearInterval(dragTimer);
    const startedAt = Date.now();
    dragTimer = setInterval(() => {
      if (!hud || Date.now() - startedAt > 60_000) return dragHud("end"); // safety net if "end" never arrives
      const p = screen.getCursorScreenPoint();
      hud.setPosition(p.x - dragOffset.x, p.y - dragOffset.y);
    }, 16);
    return;
  }
  if (!dragTimer) return;
  clearInterval(dragTimer);
  dragTimer = undefined;
  const s = getSettings();
  const b = hud.getBounds();
  const off = orbOffset(s.orb.corner, s.orb.size);
  const orb = clampOrb({ x: b.x + off.x, y: b.y + off.y }, s.orb.size);
  // Pick the layout that grows captions toward the middle of the screen.
  const wa = screen.getDisplayNearestPoint({ x: orb.x + s.orb.size / 2, y: orb.y + s.orb.size / 2 }).workArea;
  const right = orb.x + s.orb.size / 2 > wa.x + wa.width / 2;
  const bottom = orb.y + s.orb.size / 2 > wa.y + wa.height / 2;
  const corner = `${bottom ? "bottom" : "top"}_${right ? "right" : "left"}` as OrbCorner;
  updateSettings({ orb: { corner, position: orb } });
}

function createHud() {
  hud = new BrowserWindow({
    ...hudBounds(getSettings()),
    transparent: true,
    frame: false,
    resizable: false,
    movable: false,
    skipTaskbar: true,
    hasShadow: false,
    alwaysOnTop: true,
    backgroundColor: "#00000000",
    show: false,
    webPreferences: { preload: PRELOAD, contextIsolation: true, sandbox: false, nodeIntegration: false },
  });
  hud.setAlwaysOnTop(true, "screen-saver");
  hud.setVisibleOnAllWorkspaces(true);
  hud.setContentProtection(true); // keeps the orb out of Jarvis's own screenshots
  hud.setIgnoreMouseEvents(true, { forward: true });
  hud.once("ready-to-show", () => hud?.showInactive());
  // After any (re)load, tell the HUD the current game-mode and timer state so it doesn't reappear mid-game.
  hud.webContents.on("did-finish-load", () => {
    bus.emit("game_mode", gameModeState());
    bus.emit("timers", { timers: listTimers() });
  });
  hud.on("closed", () => (hud = null));
  loadRenderer(hud);

  screen.on("display-metrics-changed", () => hud?.setBounds(hudBounds(getSettings())));
}

function openSettings() {
  if (settingsWin && !settingsWin.isDestroyed()) {
    settingsWin.show();
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    width: 760,
    height: 820,
    minWidth: 520,
    minHeight: 480,
    title: "Jarvis settings",
    backgroundColor: "#191b1f",
    titleBarStyle: "hidden",
    titleBarOverlay: { color: "#191b1f", symbolColor: "#d5d9e0", height: 36 },
    autoHideMenuBar: true,
    webPreferences: { preload: PRELOAD, contextIsolation: true, sandbox: false, nodeIntegration: false },
  });
  settingsWin.on("closed", () => (settingsWin = null));
  loadRenderer(settingsWin, "#settings");
}

function talk() {
  if (!hud) return;
  hud.show();
  hud.focus(); // lets the renderer focus its text box
  bus.emit("wake", { source: "hotkey" });
}

let registeredKeys: string[] = [];
function registerHotkeys(s: Settings) {
  for (const k of registeredKeys) globalShortcut.unregister(k);
  registeredKeys = [];
  const add = (accel: string, fn: () => void) => {
    try {
      if (accel && globalShortcut.register(accel, fn)) registeredKeys.push(accel);
      else bus.emit("toast", { level: "warn", text: `Hotkey ${accel} is taken by another app.` });
    } catch {
      bus.emit("toast", { level: "warn", text: `Invalid hotkey "${accel}".` });
    }
  };
  add(s.hotkeys.talk, talk);
  add(s.hotkeys.settings, openSettings);
}

function trayIcon() {
  // 16x16 aquamarine dot, drawn in code (BGRA).
  const size = 16;
  const buf = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x - 7.5, y - 7.5);
      const a = Math.max(0, Math.min(1, 7 - d));
      const i = (y * size + x) * 4;
      buf[i] = 0xd0; buf[i + 1] = 0xe0; buf[i + 2] = 0x3f; buf[i + 3] = Math.round(a * 255);
    }
  return nativeImage.createFromBitmap(buf, { width: size, height: size });
}

function buildTrayMenu() {
  if (!tray) return;
  const s = getSettings();
  const mode = (m: SafetyMode, label: string): Electron.MenuItemConstructorOptions => ({
    label,
    type: "radio",
    checked: s.safetyMode === m,
    click: () => updateSettings({ safetyMode: m }),
  });
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: "Talk to Jarvis", accelerator: s.hotkeys.talk, click: talk },
      { label: "Settings & memory…", click: openSettings },
      { type: "separator" },
      { label: "Safety", enabled: false },
      mode("ask_all", "Ask for everything"),
      mode("ask_unsafe", "Ask only for unsafe actions"),
      mode("full_access", "Full access"),
      { type: "separator" },
      { label: s.voice.muted ? "Unmute voice" : "Mute voice", click: () => updateSettings({ voice: { muted: !s.voice.muted } }) },
      { label: "Quit Jarvis", click: () => app.quit() },
    ]),
  );
  tray.setToolTip(`Jarvis — ${s.safetyMode.replace("_", " ")}`);
}

/** Lightweight activity log in the terminal, so problems are visible without devtools. */
function wireDebugLog() {
  const t = () => new Date().toLocaleTimeString("en-GB");
  bus.on("user_text", ({ text }) => console.log(`[${t()}] typed: ${text}`));
  bus.on("user_said", ({ text, final }) => final && console.log(`[${t()}] heard: ${text}`));
  bus.on("wake", ({ source }) => console.log(`[${t()}] wake (${source})`));
  bus.on("state", ({ state }) => console.log(`[${t()}] state: ${state}`));
  bus.on("assistant_done", ({ text }) => console.log(`[${t()}] jarvis: ${text}`));
  bus.on("toast", ({ level, text }) => console.log(`[${t()}] ${level}: ${text}`));
}

function wireBrain() {
  wireDebugLog();
  const brain = new Brain();
  brain.isSpeaking = speechPending;

  // "Never mind", "that's all", "thanks", "shush"...: stop silently. No reply, no API call (see dismiss.ts).
  // The falling chime only plays when the user deliberately addressed Jarvis (wake word, hotkey, typing).
  // Utterances caught by the hands-free conversation window are often noise that speech-to-text turns into
  // "Thank you." — winding down from those stays silent.
  let lastWake: "wakeword" | "hotkey" | "followup" = "wakeword";
  cleanups.push(bus.on("wake", ({ source }) => (lastWake = source)));
  const dismiss = (chime: boolean) => {
    bus.emit("interrupt", { reason: "user" }); // stops speech, cancels any running turn, ends the conversation window
    bus.emit("state", { state: "idle" });
    if (chime) playDismissChime();
  };

  // Voice approvals: while a chip is pending, "yes"/"no" answers it instead of starting a turn.
  function switchMode(input: string) {
    const enabled = ultronCommand(input);
    if (enabled === undefined) return false;
    bus.emit("interrupt", { reason: "user" });
    updateSettings({ ultronMode: enabled });
    const text = enabled ? "Ultron online." : "Jarvis online.";
    bus.emit("assistant_done", { text });
    bus.emit("speak", { text });
    bus.emit("speak_end", {});
    return true;
  }
  const pending: string[] = [];
  cleanups.push(
    bus.on("approval_request", ({ id, summary }) => {
      pending.push(id);
      bus.emit("speak", { text: `Shall I ${summary}, ${getSettings().userTitle}?` });
      bus.emit("speak_end", {});
    }),
    bus.on("approval_response", ({ id }) => {
      const i = pending.indexOf(id);
      if (i >= 0) pending.splice(i, 1);
    }),
    bus.on("user_said", ({ text, final }) => {
      if (!final || !text.trim()) return;
      // Follow-up listening can catch background noise; transcripts like "(music)" or "[laughs]" aren't requests.
      if (!/[a-z0-9]/i.test(text.replace(/[([][^)\]]*[)\]]/g, ""))) return;
      // An open yes/no question gets first say, so "okay" / "no" answer it rather than dismissing.
      if (pending.length) {
        const t = text.trim().toLowerCase();
        if (/^(yes|yeah|yep|sure|go ahead|do it|proceed|confirm|ok(ay)?|please do)\b/.test(t)) {
          bus.emit("approval_response", { id: pending[0], ok: true });
          return;
        }
        if (/^(no|nope|nah|don'?t|stop|cancel|deny|negative)\b/.test(t)) {
          bus.emit("approval_response", { id: pending[0], ok: false });
          return;
        }
      }
      if (switchMode(text)) return;
      if (isDismissal(text)) return dismiss(lastWake !== "followup");
      void brain.handle(text);
    }),
    bus.on("user_text", ({ text }) => {
      if (!text.trim()) return;
      if (switchMode(text)) return;
      if (isDismissal(text)) return dismiss(true);
      void brain.handle(text);
    }),
  );
  return brain;
}

/** Our renderer pages may list audio devices (needed for device names) and pick a speaker. Nothing else. */
function allowAudioDevicePermissions() {
  const devUrl = process.env.ELECTRON_RENDERER_URL;
  const ownOrigin = (origin: string) => origin.startsWith("file://") || (!!devUrl && origin.startsWith(new URL(devUrl).origin));
  const allowed = new Set(["media", "speaker-selection"]);
  session.defaultSession.setPermissionCheckHandler((_wc, permission, origin) => allowed.has(permission) && ownOrigin(origin));
  session.defaultSession.setPermissionRequestHandler((wc, permission, callback, details) => {
    const origin = "requestingUrl" in details ? details.requestingUrl : wc.getURL();
    const videoRequested = "mediaTypes" in details && details.mediaTypes?.includes("video");
    callback(allowed.has(permission) && !videoRequested && ownOrigin(origin));
  });
}

app.whenReady().then(() => {
  app.setAppUserModelId("Jarvis");
  allowAudioDevicePermissions();
  registerIpc({
    openSettings,
    setIgnoreMouse: (win, ignore) => {
      if (win === hud && !dragTimer) win.setIgnoreMouseEvents(ignore, { forward: true });
    },
    drag: dragHud,
    runRoutine: (id) => {
      const routine = getRoutine(id);
      if (routine) void brain.runRoutine(routine, false);
    },
  });
  createHud();
  const brain = wireBrain();
  let voiceState: JarvisState = "idle";
  cleanups.push(bus.on("state", ({ state }) => (voiceState = state)));
  cleanups.push(
    startVoice(getSettings),
    startReminderScheduler(),
    startTimers(),
    (() => {
      // Tell game mode where the orb is (physical pixels), so it only hides on the game's screen.
      setHudLocator(() => {
        if (!hud || hud.isDestroyed()) return null;
        const s = getSettings();
        const b = hud.getBounds();
        const off = orbOffset(s.orb.corner, s.orb.size);
        return screen.dipToScreenPoint({ x: b.x + off.x + s.orb.size / 2, y: b.y + off.y + s.orb.size / 2 });
      });
      return startGameModeWatcher();
    })(),
    startHealthWatch(),
    startUsageTracking(),
    startRoutineScheduler(
      (routine) => void brain.runRoutine(routine, true),
      () => brain.busy || voiceState !== "idle" || speechPending(),
    ),
  );

  tray = new Tray(trayIcon());
  tray.on("click", talk);
  buildTrayMenu();
  registerHotkeys(getSettings());
  app.setLoginItemSettings({ openAtLogin: getSettings().launchAtStartup });

  let prev = getSettings();
  cleanups.push(
    bus.on("settings_changed", ({ settings }) => {
      if (JSON.stringify(settings.hotkeys) !== JSON.stringify(prev.hotkeys)) registerHotkeys(settings);
      const samePos = JSON.stringify(settings.orb.position) === JSON.stringify(prev.orb.position);
      if (settings.orb.corner !== prev.orb.corner && samePos && settings.orb.position) {
        // Corner picked in Settings (not a drag): snap back to that corner.
        prev = settings;
        updateSettings({ orb: { position: null } });
        return;
      }
      if (settings.orb.corner !== prev.orb.corner || !samePos || settings.orb.size !== prev.orb.size) {
        hud?.setBounds(hudBounds(settings));
      }
      if (settings.launchAtStartup !== prev.launchAtStartup) app.setLoginItemSettings({ openAtLogin: settings.launchAtStartup });
      prev = settings;
      buildTrayMenu();
    }),
    bus.on("reminder_due", ({ text }) => {
      if (Notification.isSupported()) new Notification({ title: "Jarvis reminder", body: text, silent: true }).show();
    }),
  );
});

app.on("second-instance", talk);
app.on("window-all-closed", () => {
  /* keep running in the tray */
});
app.on("will-quit", () => {
  globalShortcut.unregisterAll();
  for (const c of cleanups.splice(0)) {
    try {
      c();
    } catch {
      /* ignore */
    }
  }
});
