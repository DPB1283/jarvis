// Let Jarvis create / edit / delete routines by voice ("every weekday at 7:30 give me the weather and news").
import { z } from "zod";
import type { Routine } from "../../shared/settings";
import { bus } from "../bus";
import { deleteRoutine, getRoutine, listRoutines, upsertRoutine } from "../memory/db";
import { defineTool } from "./types";

const DAY = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function describeSchedule(s: Routine["schedule"]): string {
  if (!s) return "on request only";
  const d = s.days.join(",");
  const days = d === "1,2,3,4,5" ? "weekdays" : d === "0,6" ? "weekends" : d === "0,1,2,3,4,5,6" ? "every day" : s.days.map((x) => DAY[x]).join("/");
  return `${days} at ${s.time}`;
}

export const listRoutinesTool = defineTool({
  name: "list_routines",
  description: "List the user's routines (with ids, triggers, instructions and schedules).",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check routines",
  run: async () => {
    const all = listRoutines();
    return all.length
      ? all
          .map((r) => `#${r.id} ${r.name}${r.enabled ? "" : " (disabled)"}: ${describeSchedule(r.schedule)}. Triggers: ${r.triggers.join(" / ") || "-"}. Does: ${r.instructions}`)
          .join("\n")
      : "No routines yet.";
  },
});

export const saveRoutine = defineTool({
  name: "save_routine",
  description:
    "Create a routine, or update one (pass its id). A routine is something to do when the user asks for it by meaning (trigger phrases are examples), and/or automatically at a time. " +
    "Write instructions as clear steps you can follow later. Days: 0=Sunday ... 6=Saturday. Omit schedule for on-request only; pass schedule: null to remove one.",
  schema: z.object({
    id: z.number().int().optional(),
    name: z.string().optional(),
    triggers: z.array(z.string()).optional(),
    instructions: z.string().optional(),
    schedule: z.object({ time: z.string().describe("24-hour HH:MM"), days: z.array(z.number().int().min(0).max(6)) }).nullable().optional(),
    enabled: z.boolean().optional(),
  }),
  risk: "safe",
  summary: (i) => `${i.id ? "update" : "create"} routine ${i.name ?? `#${i.id}`}`,
  run: async (i) => {
    const cur = i.id ? getRoutine(i.id) : null;
    if (i.id && !cur) return `No routine #${i.id}.`;
    const saved = upsertRoutine({
      id: i.id,
      name: i.name ?? cur?.name ?? "",
      triggers: i.triggers ?? cur?.triggers ?? [],
      instructions: i.instructions ?? cur?.instructions ?? "",
      schedule: i.schedule === undefined ? (cur?.schedule ?? null) : i.schedule,
      enabled: i.enabled ?? cur?.enabled ?? true,
    });
    bus.emit("routines_changed", {});
    return `Saved routine #${saved.id} "${saved.name}" (${describeSchedule(saved.schedule)}${saved.enabled ? "" : ", disabled"}).`;
  },
});

export const deleteRoutineTool = defineTool({
  name: "delete_routine",
  description: "Delete a routine by id.",
  schema: z.object({ id: z.number().int() }),
  risk: "safe",
  summary: (i) => `delete routine #${i.id}`,
  run: async ({ id }) => {
    const ok = deleteRoutine(id);
    if (ok) bus.emit("routines_changed", {});
    return ok ? `Deleted routine #${id}.` : `No routine #${id}.`;
  },
});
