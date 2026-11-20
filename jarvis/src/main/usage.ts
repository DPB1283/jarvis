// Learns routines: once a minute, note which app is in front (exe name only, never window titles),
// skipping when the user is away. Jarvis can look at the patterns and suggest routines.
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { z } from "zod";
import { DATA_DIR } from "./config";
import { defineTool } from "./tools/types";
import { foregroundPid, idleSeconds, processName } from "./win32";

const db = new DatabaseSync(path.join(DATA_DIR, "usage.sqlite"));
db.exec("CREATE TABLE IF NOT EXISTS usage (ts INTEGER NOT NULL, app TEXT NOT NULL); CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts);");

const IGNORE = /^(electron|explorer|searchhost|shellexperiencehost|lockapp|applicationframehost|startmenuexperiencehost)$/i;

export function startUsageTracking(): () => void {
  const tick = () => {
    try {
      if (idleSeconds() > 300) return; // away from the PC
      const app = processName(foregroundPid()).replace(/\.exe$/i, "");
      if (!app || IGNORE.test(app)) return;
      db.prepare("INSERT INTO usage (ts, app) VALUES (?, ?)").run(Date.now(), app);
    } catch {
      /* never let tracking break anything */
    }
  };
  const timer = setInterval(tick, 60_000);
  // Keep 60 days.
  db.prepare("DELETE FROM usage WHERE ts < ?").run(Date.now() - 60 * 86_400_000);
  return () => clearInterval(timer);
}

export const usagePatterns = defineTool({
  name: "usage_patterns",
  description:
    "How the user tends to use their PC (from minute-by-minute samples of the app in front; no window titles): time per app, usual hours, and which apps they open together at the start of a session. " +
    "Use to suggest routines ('you usually open Discord and Spotify around 8, shall I make that a routine?') or answer 'how much did I play today?'.",
  schema: z.object({ days: z.number().int().min(1).max(60).optional() }),
  risk: "safe",
  summary: () => "look at usage patterns",
  run: async ({ days = 14 }) => {
    const since = Date.now() - days * 86_400_000;
    const rows = db.prepare("SELECT ts, app FROM usage WHERE ts >= ? ORDER BY ts").all(since) as { ts: number; app: string }[];
    if (rows.length < 30) return `Only ${rows.length} minutes recorded so far; patterns need a few days of use.`;
    const perApp = new Map<string, { mins: number; hours: number[] }>();
    for (const r of rows) {
      const e = perApp.get(r.app) ?? { mins: 0, hours: Array(24).fill(0) };
      e.mins++;
      e.hours[new Date(r.ts).getHours()]++;
      perApp.set(r.app, e);
    }
    const top = [...perApp.entries()].sort((a, b) => b[1].mins - a[1].mins).slice(0, 10);
    const peak = (h: number[]) => {
      const max = Math.max(...h);
      return h.map((v, i) => [v, i] as const).filter(([v]) => v >= max * 0.6).map(([, i]) => `${i}:00`).join(", ");
    };
    // Session starts: first 3 distinct apps after a 2h+ gap.
    const starts = new Map<string, number>();
    for (let i = 0; i < rows.length; i++) {
      if (i > 0 && rows[i].ts - rows[i - 1].ts < 2 * 3_600_000) continue;
      const seq: string[] = [];
      for (let j = i; j < rows.length && seq.length < 3 && rows[j].ts - rows[i].ts < 30 * 60_000; j++) if (!seq.includes(rows[j].app)) seq.push(rows[j].app);
      const key = `${seq.join(" → ")} (around ${new Date(rows[i].ts).getHours()}:00)`;
      starts.set(key, (starts.get(key) ?? 0) + 1);
    }
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const todayMins = new Map<string, number>();
    for (const r of rows) if (r.ts >= today.getTime()) todayMins.set(r.app, (todayMins.get(r.app) ?? 0) + 1);
    const hm = (m: number) => (m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`);
    return [
      `Last ${days} days (${hm(rows.length)} of active use):`,
      ...top.map(([app, e]) => `- ${app}: ${hm(e.mins)}, mostly around ${peak(e.hours)}`),
      `Today: ${[...todayMins.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([a, m]) => `${a} ${hm(m)}`).join(", ") || "nothing yet"}`,
      `Common session starts: ${[...starts.entries()].filter(([, n]) => n >= 2).sort((a, b) => b[1] - a[1]).slice(0, 4).map(([k, n]) => `${k} x${n}`).join("; ") || "no repeated pattern yet"}`,
    ].join("\n");
  },
});
