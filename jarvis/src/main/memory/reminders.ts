import { bus } from "../bus";
import { getSettings } from "../config";
import { takeDueReminders } from "./db";

/** Checks for due reminders every 15 s; announces them over the bus (toast + speech + orb). */
export function startReminderScheduler(): () => void {
  const tick = () => {
    for (const r of takeDueReminders()) {
      bus.emit("reminder_due", { id: r.id, text: r.text });
      bus.emit("toast", { level: "info", text: `Reminder: ${r.text}` });
      bus.emit("speak", { text: `Pardon the interruption, ${getSettings().userTitle}. A reminder: ${r.text}` });
      bus.emit("speak_end", {});
    }
  };
  tick();
  const timer = setInterval(tick, 15_000);
  return () => clearInterval(timer);
}
