import { z } from "zod";
import { defineTool } from "./types";
import { addTimer, cancelTimer, listTimers } from "../timers";

const left = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return [h && `${h}h`, m && `${m}m`, !h && sec && `${sec}s`].filter(Boolean).join(" ") || "0s";
};

export const setTimer = defineTool({
  name: "set_timer",
  description:
    "Start a named countdown timer ('pizza timer, 12 minutes') or set an alarm for a clock time ('wake me at 7', 'alarm for 6:45'). " +
    "Timers show as a ring around the orb and ring + speak when done. For 'remind me about X' use set_reminder instead.",
  schema: z.object({
    label: z.string().optional().describe("e.g. 'pizza', 'laundry', 'Wake up'"),
    hours: z.number().min(0).max(48).optional(),
    minutes: z.number().min(0).max(24 * 60).optional(),
    seconds: z.number().min(0).max(24 * 3600).optional(),
    alarm_at: z.string().optional().describe("ISO 8601 local date-time for an alarm, e.g. 2026-11-21T07:00"),
  }),
  risk: "safe",
  summary: (i) => (i.alarm_at ? `set an alarm${i.label ? ` (${i.label})` : ""}` : `start a ${i.label ?? ""} timer`.replace(/\s+/g, " ")),
  run: async ({ label = "", hours = 0, minutes = 0, seconds = 0, alarm_at }) => {
    if (alarm_at) {
      const at = new Date(alarm_at);
      if (Number.isNaN(at.getTime()) || at.getTime() <= Date.now()) return "That alarm time isn't in the future.";
      const t = addTimer(label || "Alarm", at.getTime(), "alarm");
      return `Alarm "${t.label}" set for ${at.toLocaleString("en-GB", { weekday: "short", hour: "numeric", minute: "2-digit" })} (in ${left(t.durationMs)}).`;
    }
    const ms = ((hours * 60 + minutes) * 60 + seconds) * 1000;
    if (ms < 1000) return "How long should the timer be?";
    const t = addTimer(label || "Timer", Date.now() + ms, "timer");
    return `${t.label} timer started: ${left(ms)}.`;
  },
});

export const listTimersTool = defineTool({
  name: "list_timers",
  description: "Running timers and alarms with time left ('how long left on the pizza?').",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check timers",
  run: async () => {
    const all = listTimers();
    return all.length ? all.map((t) => `${t.kind} "${t.label}": ${left(t.endsAt - Date.now())} left`).join("\n") : "No timers or alarms running.";
  },
});

export const cancelTimerTool = defineTool({
  name: "cancel_timer",
  description: "Cancel a timer or alarm by its label (or 'all').",
  schema: z.object({ label: z.string() }),
  risk: "safe",
  summary: (i) => `cancel the ${i.label} timer`,
  run: async ({ label }) => {
    if (label.trim().toLowerCase() === "all") {
      const all = listTimers();
      for (const t of all) cancelTimer(t.id);
      return `Cancelled ${all.length}.`;
    }
    const t = cancelTimer(label);
    return t ? `Cancelled "${t.label}".` : `No timer called "${label}".`;
  },
});
