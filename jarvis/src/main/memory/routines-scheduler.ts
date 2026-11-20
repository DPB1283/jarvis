import type { Routine } from "../../shared/settings";
import { bus } from "../bus";
import { listRoutines, markRoutineRun } from "./db";

const CHECK_MS = 30_000;
/** A scheduled run more than this late (PC asleep, app closed) is skipped rather than fired at a surprising time. */
const MAX_LATE_MS = 15 * 60_000;

/** The most recent scheduled time at or before `now` for today, or null if it isn't scheduled today. */
export function dueTimeToday(r: Routine, now = new Date()): Date | null {
  if (!r.schedule || !r.schedule.days.includes(now.getDay())) return null;
  const [h, m] = r.schedule.time.split(":").map(Number);
  const due = new Date(now);
  due.setHours(h, m, 0, 0);
  return due <= now ? due : null;
}

/**
 * Fires scheduled routines. Waits while Jarvis is busy (talking/listening/working) and retries on the next
 * check, up to 15 minutes late.
 */
export function startRoutineScheduler(run: (r: Routine) => void, isBusy: () => boolean): () => void {
  const tick = () => {
    const now = new Date();
    for (const r of listRoutines()) {
      if (!r.enabled) continue;
      const due = dueTimeToday(r, now);
      if (!due || now.getTime() - due.getTime() > MAX_LATE_MS) continue;
      if (r.lastRunAt && new Date(r.lastRunAt) >= due) continue; // already done today
      if (isBusy()) return; // try again next check
      markRoutineRun(r.id, now);
      bus.emit("routines_changed", {});
      run(r);
      return; // one routine at a time
    }
  };
  const timer = setInterval(tick, CHECK_MS);
  setTimeout(tick, 5_000);
  return () => clearInterval(timer);
}
