// Game mode: while a fullscreen game (or any fullscreen app) is in front, Jarvis keeps quiet:
// no chimes, no follow-up listening, health warnings held until afterwards.
// Reminders and timers still speak (user's choice). "Jarvis" still wakes him.
// The orb only hides if it's on the SAME screen as the game; on another monitor it stays visible.
import { bus } from "./bus";
import { getSettings } from "./config";
import { foregroundFullscreen, type Rect } from "./win32";

let active = false;
let app: string | null = null;
let hideHud = false;

export const isGameMode = () => active;
export const gameModeState = () => ({ active, app, hideHud });

/** Supplied by the window layer: the orb's centre in physical screen pixels (null if unknown). */
let hudLocator: () => { x: number; y: number } | null = () => null;
export function setHudLocator(fn: () => { x: number; y: number } | null): void {
  hudLocator = fn;
}

const prettyName = (exe: string) => exe.replace(/\.exe$/i, "") || null;

function hudOnMonitor(monitor: Rect | null): boolean {
  if (!monitor) return true; // unknown: be safe and hide
  const p = hudLocator();
  if (!p) return true;
  return p.x >= monitor.left && p.x < monitor.right && p.y >= monitor.top && p.y < monitor.bottom;
}

export function startGameModeWatcher(): () => void {
  let fullscreenSince = 0;
  const tick = () => {
    const mode = getSettings().gameMode?.mode ?? "auto";
    let want = false;
    let name: string | null = null;
    let hide = false;
    if (mode === "on") {
      want = true;
      name = "manual";
      hide = true;
    } else if (mode === "auto") {
      try {
        const fg = foregroundFullscreen();
        // Ignore Jarvis's own windows, and need it to stay fullscreen for 2 checks (skips brief fullscreen flickers).
        const ours = /^electron\.exe$|^jarvis/i.test(fg.exe);
        if (fg.fullscreen && !ours) {
          fullscreenSince ||= Date.now();
          want = Date.now() - fullscreenSince >= 2500;
          name = prettyName(fg.exe);
          hide = want && hudOnMonitor(fg.monitor);
        } else {
          fullscreenSince = 0;
        }
      } catch {
        want = false;
      }
    }
    // Re-checked every tick, so dragging the orb to the other screen mid-game shows it again.
    if (want !== active || hide !== hideHud || (want && name !== app)) {
      active = want;
      app = want ? name : null;
      hideHud = hide;
      bus.emit("game_mode", { active, app, hideHud });
    }
  };
  tick();
  const timer = setInterval(tick, 3000);
  return () => clearInterval(timer);
}
