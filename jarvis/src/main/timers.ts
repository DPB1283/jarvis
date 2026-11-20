// Named timers ("pizza timer, 12 minutes") and alarms ("wake me at 7"). Persisted so they survive restarts.
// The HUD draws the soonest one as a ring that goes yellow → orange → red (renderer, Codex).
import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import type { TimerInfo } from "../shared/settings";
import { bus } from "./bus";
import { DATA_DIR, getSettings } from "./config";

const FILE = path.join(DATA_DIR, "timers.json");
let timers: TimerInfo[] = [];
const handles = new Map<string, ReturnType<typeof setTimeout>>();

function save() {
  fs.writeFileSync(FILE, JSON.stringify(timers, null, 2));
  bus.emit("timers", { timers: listTimers() });
}

export function listTimers(): TimerInfo[] {
  return [...timers].sort((a, b) => a.endsAt - b.endsAt);
}

function schedule(t: TimerInfo) {
  clearTimeout(handles.get(t.id));
  // setTimeout caps at ~24.8 days; re-arm for long alarms.
  const ms = Math.min(t.endsAt - Date.now(), 2 ** 31 - 1);
  handles.set(
    t.id,
    setTimeout(() => (Date.now() >= t.endsAt - 50 ? fire(t.id) : schedule(t)), Math.max(0, ms)),
  );
}

function fire(id: string) {
  const t = timers.find((x) => x.id === id);
  if (!t) return;
  handles.delete(id);
  timers = timers.filter((x) => x.id !== id);
  save();
  bus.emit("timer_done", { timer: t }); // voice plays the alarm sound; the HUD flashes
  const who = getSettings().userTitle;
  const text =
    t.kind === "alarm"
      ? `It's ${new Date(t.endsAt).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" })}, ${who}. ${t.label === "Alarm" ? "Your alarm." : t.label + "."}`
      : `${who[0].toUpperCase() + who.slice(1)}, your ${/timer$/i.test(t.label) ? t.label : `${t.label} timer`} is done.`;
  // Speak after the ~2 s alarm sound so they don't overlap.
  setTimeout(() => {
    bus.emit("speak", { text });
    bus.emit("speak_end", {});
  }, 2200);
}

export function addTimer(label: string, endsAt: number, kind: TimerInfo["kind"]): TimerInfo {
  const t: TimerInfo = { id: randomUUID(), label: label.trim() || (kind === "alarm" ? "Alarm" : "Timer"), kind, endsAt, durationMs: Math.max(1000, endsAt - Date.now()) };
  timers.push(t);
  schedule(t);
  save();
  return t;
}

export function cancelTimer(idOrLabel: string): TimerInfo | null {
  const q = idOrLabel.trim().toLowerCase();
  const t = timers.find((x) => x.id === idOrLabel) ?? timers.find((x) => x.label.toLowerCase() === q) ?? timers.find((x) => x.label.toLowerCase().includes(q));
  if (!t) return null;
  clearTimeout(handles.get(t.id));
  handles.delete(t.id);
  timers = timers.filter((x) => x.id !== t.id);
  save();
  return t;
}

/** Load saved timers at startup; ones that expired while Jarvis was closed fire now (alarms only if < 10 min late). */
export function startTimers(): () => void {
  try {
    timers = JSON.parse(fs.readFileSync(FILE, "utf8")) as TimerInfo[];
  } catch {
    timers = [];
  }
  const now = Date.now();
  timers = timers.filter((t) => t.endsAt > now - 10 * 60_000);
  for (const t of timers) schedule(t);
  save();
  return () => {
    for (const h of handles.values()) clearTimeout(h);
    handles.clear();
  };
}
