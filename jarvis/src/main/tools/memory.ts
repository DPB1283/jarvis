import { z } from "zod";
import { defineTool } from "./types";
import { clearMemory, deleteMemory, listMemory, upsertMemory } from "../memory/db";
import { getSettings, updateSettings } from "../config";
import type { MemoryItem } from "../../shared/settings";

const fmt = (items: MemoryItem[]) =>
  items.length
    ? items
        .map(
          (m) =>
            `#${m.id} [${m.kind}${m.kind === "todo" ? (m.done ? ", done" : ", open") : ""}${m.dueAt ? `, due ${new Date(m.dueAt).toLocaleString("en-GB")}` : ""}] ${m.text}${m.imagePath ? ` [screenshot: ${m.imagePath}]` : ""}`,
        )
        .join("\n")
    : "Nothing stored.";

export const remember = defineTool({
  name: "remember",
  description:
    "Save something to memory. Whenever the user says 'remember ...', call this right away with their words, tidied up (kind=note for things to jot down, fact for lasting facts about them). " +
    "For 'remember this/that' about what's on screen, call remember_screen first and pass its image_path here.",
  schema: z.object({
    text: z.string(),
    kind: z.enum(["fact", "note"]).optional(),
    image_path: z.string().optional().describe("Screenshot from remember_screen"),
  }),
  risk: "safe",
  summary: (i) => `remember "${i.text}"`,
  run: async ({ text, kind, image_path }) =>
    `Saved #${upsertMemory({ kind: kind ?? (image_path ? "note" : "fact"), text, imagePath: image_path ?? null }).id}`,
});

export const recall = defineTool({
  name: "recall",
  description: "Search saved facts, notes, todos and reminders. Omit query to list everything of a kind.",
  schema: z.object({
    query: z.string().optional(),
    kind: z.enum(["fact", "note", "todo", "reminder"]).optional(),
  }),
  risk: "safe",
  summary: () => "check memory",
  run: async ({ query, kind }) => fmt(listMemory({ query, kind })),
});

export const editMemory = defineTool({
  name: "edit_memory",
  description: "Change the text of a saved item, mark a todo done/undone, or move a reminder's time. Use the #id from recall.",
  schema: z.object({
    id: z.number().int(),
    text: z.string().optional(),
    done: z.boolean().optional(),
    due_at: z.string().optional().describe("New ISO 8601 local time for a reminder"),
  }),
  risk: "safe",
  summary: (i) => `update memory #${i.id}`,
  run: async ({ id, text, done, due_at }) => {
    const cur = listMemory().find((m) => m.id === id);
    if (!cur) return `No item #${id}.`;
    const m = upsertMemory({
      id,
      kind: cur.kind,
      text: text ?? cur.text,
      done: done ?? cur.done,
      dueAt: due_at ? new Date(due_at).toISOString() : cur.dueAt,
    });
    return `Updated: ${fmt([m])}`;
  },
});

export const forget = defineTool({
  name: "forget",
  description: "Delete saved items by id, or wipe a whole kind with all=true (requires the user to have explicitly asked).",
  schema: z.object({
    ids: z.array(z.number().int()).optional(),
    all: z.boolean().optional(),
    kind: z.enum(["fact", "note", "todo", "reminder"]).optional(),
  }),
  risk: (i) => (i.all ? "unsafe" : "safe"),
  summary: (i) => (i.all ? `erase ALL ${i.kind ?? "memory"}` : `forget #${(i.ids ?? []).join(", #")}`),
  run: async ({ ids, all, kind }) => {
    if (all) return `Erased ${clearMemory(kind ? [kind] : undefined)} items.`;
    const n = (ids ?? []).filter((id) => deleteMemory(id)).length;
    return `Forgot ${n} item(s).`;
  },
});

export const addTodo = defineTool({
  name: "add_todo",
  description: "Add an item to the user's to-do list.",
  schema: z.object({ text: z.string() }),
  risk: "safe",
  summary: (i) => `add to-do "${i.text}"`,
  run: async ({ text }) => `Added to-do #${upsertMemory({ kind: "todo", text }).id}`,
});

const fmtWhen = (d: Date) => d.toLocaleString("en-GB", { dateStyle: "medium", timeStyle: "short" });
const fmtTime = (d: Date) => d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });

export const setReminder = defineTool({
  name: "set_reminder",
  description:
    "Set a reminder. Two kinds:\n" +
    "- A task at a time: 'remind me in 20 minutes to stretch' / 'remind me at 9 to call mum': give in_minutes or at.\n" +
    "- An EVENT with a time ('remind me I have dinner at 7', 'I've got a meeting at 3'): give event_at; Jarvis reminds them " +
    "remind_before_minutes ahead (default 30, or what they ask for), and also_at_event_time if they want a second nudge when it starts.\n" +
    "Times are ISO 8601 local date-time (e.g. 2026-11-21T19:00). Today's date and time are in the context block; if a bare time like '7' is ambiguous, pick the next sensible one (7 pm for dinner).",
  schema: z.object({
    text: z.string().describe("What it's about, e.g. 'Dinner' or 'Call mum'"),
    in_minutes: z.number().positive().optional(),
    at: z.string().optional().describe("When to remind (task reminders)"),
    event_at: z.string().optional().describe("When the event happens (event reminders)"),
    remind_before_minutes: z.number().min(0).max(7 * 24 * 60).optional().describe("Lead time for event reminders, default 30"),
    also_at_event_time: z.boolean().optional(),
  }),
  risk: "safe",
  summary: (i) => `reminder: ${i.text}`,
  run: async ({ text, in_minutes, at, event_at, remind_before_minutes, also_at_event_time }) => {
    if (event_at) {
      const event = new Date(event_at);
      if (Number.isNaN(event.getTime())) return "I need a valid event time.";
      const lead = remind_before_minutes ?? 30;
      const ahead = new Date(event.getTime() - lead * 60_000);
      const leadText = lead >= 60 && lead % 60 === 0 ? `${lead / 60} hour${lead === 60 ? "" : "s"}` : `${lead} minutes`;
      const saved: string[] = [];
      if (lead > 0 && ahead.getTime() > Date.now()) {
        const m = upsertMemory({ kind: "reminder", text: `${text} at ${fmtTime(event)}, in ${leadText}`, dueAt: ahead.toISOString() });
        saved.push(`#${m.id} at ${fmtWhen(ahead)} (${leadText} before)`);
      }
      if (also_at_event_time || lead === 0 || ahead.getTime() <= Date.now()) {
        if (event.getTime() <= Date.now()) return `${fmtWhen(event)} has already passed.`;
        const m = upsertMemory({ kind: "reminder", text: `${text}, starting now (${fmtTime(event)})`, dueAt: event.toISOString() });
        saved.push(`#${m.id} at ${fmtWhen(event)} (when it starts)`);
      }
      return `Event "${text}" at ${fmtWhen(event)}. Reminders: ${saved.join("; ")}.`;
    }
    const due = in_minutes ? new Date(Date.now() + in_minutes * 60_000) : at ? new Date(at) : null;
    if (!due || Number.isNaN(due.getTime())) return "I need a valid time: in_minutes, at, or event_at.";
    const m = upsertMemory({ kind: "reminder", text, dueAt: due.toISOString() });
    return `Reminder #${m.id} set for ${fmtWhen(due)}.`;
  },
});

export const changeSettings = defineTool({
  name: "change_settings",
  description:
    "Change Jarvis's own settings when the user asks: how to address them (user_title, e.g. 'sir'), their name (user_name, e.g. when they say 'my name is ...' or 'call me ...' with a name), home city / units / news region, how long to keep listening after replies, captions on/off, mute your voice, sarcasm 0-10, reply length, safety mode, wake word on/off.",
  schema: z.object({
    user_title: z.string().optional(),
    user_name: z.string().optional().describe("The user's name; Jarvis uses it now and then alongside the title"),
    home_city: z.string().optional().describe("Home city for weather and briefings, e.g. 'Los Angeles'"),
    units: z.enum(["metric", "imperial"]).optional(),
    news_region: z.string().length(2).optional().describe("Two-letter country code for news, e.g. US, GB"),
    news_sources: z.enum(["neutral", "all"]).optional().describe("neutral = national/world news from wire services only"),
    follow_up_seconds: z.number().int().min(0).max(300).optional().describe("How long to keep listening after a reply (0 = off)"),
    end_of_speech_seconds: z.number().min(0.4).max(3).optional().describe("Pause that counts as 'finished talking'; raise it if Jarvis cuts them off"),
    captions: z.boolean().optional(),
    voice_muted: z.boolean().optional(),
    sarcasm: z.number().int().min(0).max(10).optional(),
    reply_length: z.enum(["brief", "normal"]).optional(),
    safety_mode: z.enum(["ask_all", "ask_unsafe", "full_access"]).optional(),
    wake_word: z.boolean().optional(),
  }),
  // Loosening safety always needs the user's explicit OK (unless already full access).
  risk: (i) => {
    const order = { ask_all: 0, ask_unsafe: 1, full_access: 2 } as const;
    return i.safety_mode && order[i.safety_mode] > order[getSettings().safetyMode] ? "unsafe" : "safe";
  },
  summary: (i) =>
    "change settings: " +
    Object.entries(i)
      .map(([k, v]) => `${k}=${v}`)
      .join(", "),
  run: async (i) => {
    const s = getSettings();
    updateSettings({
      ...(i.user_title !== undefined && { userTitle: i.user_title }),
      ...(i.user_name !== undefined && { userName: i.user_name }),
      ...((i.home_city !== undefined || i.units !== undefined || i.news_region !== undefined || i.news_sources !== undefined) && {
        location: {
          city: i.home_city ?? s.location.city,
          units: i.units ?? s.location.units,
          newsRegion: (i.news_region ?? s.location.newsRegion).toUpperCase(),
          newsSources: i.news_sources ?? s.location.newsSources ?? "neutral",
        },
      }),
      ...((i.follow_up_seconds !== undefined || i.end_of_speech_seconds !== undefined) && {
        conversation: {
          followUpSeconds: i.follow_up_seconds ?? s.conversation.followUpSeconds,
          endOfSpeechSeconds: i.end_of_speech_seconds ?? s.conversation.endOfSpeechSeconds ?? 1.2,
        },
      }),
      ...(i.captions !== undefined && { captions: { ...s.captions, enabled: i.captions } }),
      ...(i.voice_muted !== undefined && { voice: { ...s.voice, muted: i.voice_muted } }),
      ...((i.sarcasm !== undefined || i.reply_length !== undefined) && {
        personality: {
          sarcasm: i.sarcasm ?? s.personality.sarcasm,
          replyLength: i.reply_length ?? s.personality.replyLength,
        },
      }),
      ...(i.safety_mode !== undefined && { safetyMode: i.safety_mode }),
      ...(i.wake_word !== undefined && { wakeWord: { ...s.wakeWord, enabled: i.wake_word } }),
    });
    return "Settings updated.";
  },
});

export const confirmWithUser = defineTool({
  name: "confirm_with_user",
  description:
    "Ask the user for a yes/no before a sensitive step you are about to take on screen: sending a message or email, submitting a form, purchasing or paying, entering a password, posting publicly, or changing account/security settings. Returns whether they approved.",
  schema: z.object({ action: z.string().describe("What you are about to do, in plain words") }),
  risk: "unsafe",
  summary: (i) => i.action,
  // Reaching run() means the policy layer already approved (or safety mode is full access).
  run: async () => "Approved. Go ahead.",
});
