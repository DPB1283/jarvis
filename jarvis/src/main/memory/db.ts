import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { ChatAppProfile, MemoryItem, MemoryKind, MemoryUpsert, Routine, RoutineUpsert } from "../../shared/settings";
import { DATA_DIR } from "../config";

const db = new DatabaseSync(path.join(DATA_DIR, "jarvis.sqlite"));

db.exec(`
  CREATE TABLE IF NOT EXISTS memory (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('fact','note','todo','reminder')),
    text TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    due_at TEXT,
    fired INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS action_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    tool TEXT NOT NULL,
    input TEXT NOT NULL,
    decision TEXT NOT NULL,
    result TEXT
  );
`);

// Added later: screenshot attached to a memory.
if (!(db.prepare("PRAGMA table_info(memory)").all() as { name: string }[]).some((c) => c.name === "image_path")) {
  db.exec("ALTER TABLE memory ADD COLUMN image_path TEXT");
}

type Row = {
  id: number;
  kind: MemoryKind;
  text: string;
  done: number;
  due_at: string | null;
  image_path: string | null;
  created_at: string;
  updated_at: string;
};

const toItem = (r: Row): MemoryItem => ({
  id: r.id,
  kind: r.kind,
  text: r.text,
  done: !!r.done,
  dueAt: r.due_at,
  imagePath: r.image_path ?? null,
  createdAt: r.created_at,
  updatedAt: r.updated_at,
});

export function listMemory(filter: { kind?: MemoryKind; query?: string } = {}): MemoryItem[] {
  const where: string[] = [];
  const args: string[] = [];
  if (filter.kind) {
    where.push("kind = ?");
    args.push(filter.kind);
  }
  if (filter.query) {
    where.push("text LIKE ?");
    args.push(`%${filter.query}%`);
  }
  const sql = `SELECT * FROM memory ${where.length ? "WHERE " + where.join(" AND ") : ""} ORDER BY kind, id DESC`;
  return (db.prepare(sql).all(...args) as Row[]).map(toItem);
}

export function getMemory(id: number): MemoryItem | null {
  const r = db.prepare("SELECT * FROM memory WHERE id = ?").get(id) as Row | undefined;
  return r ? toItem(r) : null;
}

export function upsertMemory(item: MemoryUpsert): MemoryItem {
  const now = new Date().toISOString();
  if (item.id != null) {
    const existing = getMemory(item.id);
    if (!existing) throw new Error(`No memory item ${item.id}`);
    const dueAt = item.dueAt !== undefined ? item.dueAt : existing.dueAt;
    const imagePath = item.imagePath !== undefined ? item.imagePath : existing.imagePath;
    db.prepare(
      "UPDATE memory SET kind = ?, text = ?, done = ?, due_at = ?, image_path = ?, fired = CASE WHEN ? IS NOT due_at THEN 0 ELSE fired END, updated_at = ? WHERE id = ?",
    ).run(item.kind, item.text, (item.done ?? existing.done) ? 1 : 0, dueAt, imagePath, dueAt, now, item.id);
    return getMemory(item.id)!;
  }
  const res = db
    .prepare("INSERT INTO memory (kind, text, done, due_at, image_path, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
    .run(item.kind, item.text, item.done ? 1 : 0, item.dueAt ?? null, item.imagePath ?? null, now, now);
  return getMemory(Number(res.lastInsertRowid))!;
}

export function deleteMemory(id: number): boolean {
  return db.prepare("DELETE FROM memory WHERE id = ?").run(id).changes > 0;
}

export function clearMemory(kinds?: MemoryKind[]): number {
  if (!kinds || kinds.length === 0) return Number(db.prepare("DELETE FROM memory").run().changes);
  const ph = kinds.map(() => "?").join(",");
  return Number(db.prepare(`DELETE FROM memory WHERE kind IN (${ph})`).run(...kinds).changes);
}

/** Reminders that are due and not yet announced; marks them fired. */
export function takeDueReminders(now = new Date()): MemoryItem[] {
  const rows = db
    .prepare("SELECT * FROM memory WHERE kind = 'reminder' AND fired = 0 AND due_at IS NOT NULL AND due_at <= ?")
    .all(now.toISOString()) as Row[];
  for (const r of rows) db.prepare("UPDATE memory SET fired = 1 WHERE id = ?").run(r.id);
  return rows.map(toItem);
}

// --- Routines ---------------------------------------------------------------

db.exec(`
  CREATE TABLE IF NOT EXISTS routines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    triggers TEXT NOT NULL DEFAULT '[]',
    instructions TEXT NOT NULL,
    schedule TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
`);

type RoutineRow = {
  id: number;
  name: string;
  triggers: string;
  instructions: string;
  schedule: string | null;
  enabled: number;
  last_run_at: string | null;
};

const toRoutine = (r: RoutineRow): Routine => ({
  id: r.id,
  name: r.name,
  triggers: JSON.parse(r.triggers) as string[],
  instructions: r.instructions,
  schedule: r.schedule ? (JSON.parse(r.schedule) as Routine["schedule"]) : null,
  enabled: !!r.enabled,
  lastRunAt: r.last_run_at,
});

/** Throws a readable error if the routine is invalid; returns it normalised. */
export function validateRoutine(r: RoutineUpsert): RoutineUpsert {
  const name = r.name?.trim();
  if (!name) throw new Error("Give the routine a name.");
  const instructions = r.instructions?.trim();
  if (!instructions) throw new Error("Say what the routine should do.");
  const triggers = (r.triggers ?? []).map((t) => t.trim()).filter(Boolean).slice(0, 20);
  let schedule: Routine["schedule"] = null;
  if (r.schedule) {
    const m = /^(\d{1,2}):(\d{2})$/.exec(r.schedule.time?.trim() ?? "");
    if (!m || Number(m[1]) > 23 || Number(m[2]) > 59) throw new Error("Schedule time must look like 07:30 (24-hour).");
    const days = [...new Set(r.schedule.days ?? [])].filter((d) => Number.isInteger(d) && d >= 0 && d <= 6).sort();
    if (!days.length) throw new Error("Pick at least one day for the schedule.");
    schedule = { time: `${m[1].padStart(2, "0")}:${m[2]}`, days };
  }
  return { id: r.id, name, triggers, instructions, schedule, enabled: r.enabled !== false };
}

export function listRoutines(): Routine[] {
  return (db.prepare("SELECT * FROM routines ORDER BY id").all() as RoutineRow[]).map(toRoutine);
}

export function getRoutine(id: number): Routine | null {
  const r = db.prepare("SELECT * FROM routines WHERE id = ?").get(id) as RoutineRow | undefined;
  return r ? toRoutine(r) : null;
}

export function upsertRoutine(input: RoutineUpsert): Routine {
  const r = validateRoutine(input);
  const now = new Date().toISOString();
  const args = [r.name, JSON.stringify(r.triggers), r.instructions, r.schedule ? JSON.stringify(r.schedule) : null, r.enabled ? 1 : 0];
  if (r.id != null) {
    if (!getRoutine(r.id)) throw new Error(`No routine ${r.id}.`);
    db.prepare("UPDATE routines SET name = ?, triggers = ?, instructions = ?, schedule = ?, enabled = ?, updated_at = ? WHERE id = ?").run(
      ...args,
      now,
      r.id,
    );
    return getRoutine(r.id)!;
  }
  const res = db
    .prepare("INSERT INTO routines (name, triggers, instructions, schedule, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
    .run(...args, now, now);
  return getRoutine(Number(res.lastInsertRowid))!;
}

export function deleteRoutine(id: number): boolean {
  return db.prepare("DELETE FROM routines WHERE id = ?").run(id).changes > 0;
}

export function markRoutineRun(id: number, at = new Date()): void {
  db.prepare("UPDATE routines SET last_run_at = ? WHERE id = ?").run(at.toISOString(), id);
}

// Seed an evening wrap-up once (on request only until the user gives it a time).
if (!db.prepare("SELECT value FROM meta WHERE key = 'seeded_wrapup'").get()) {
  queueMicrotask(() => {
    upsertRoutine({
      name: "Evening wrap-up",
      triggers: ["wrap up my day", "how did today go", "evening wrap-up", "what's tomorrow looking like"],
      instructions:
        "A short end-of-day summary: what I spent my time on today (usage_patterns, top few apps), to-dos still open, " +
        "tomorrow's reminders and calendar events if connected, and tomorrow's weather for my home city. " +
        "If you notice a habit that would make a good routine, suggest it in one line. Under about 40 seconds spoken.",
      schedule: null,
      enabled: true,
    });
    db.prepare("INSERT INTO meta (key, value) VALUES ('seeded_wrapup', '1')").run();
  });
}

// Seed a default morning briefing once (not again if the user deletes it).
if (!db.prepare("SELECT value FROM meta WHERE key = 'seeded_routines'").get()) {
  upsertRoutine({
    name: "Morning briefing",
    triggers: ["how's it looking this morning", "morning briefing", "what's today looking like", "catch me up"],
    instructions:
      "Greet me for the time of day. Today's weather for my home city (now, high/low, chance of rain, anything notable). " +
      "Then the top 3 news headlines in one line each. Then any reminders due today and how many open to-dos I have. " +
      "Keep it under about 40 seconds when spoken.",
    schedule: null,
    enabled: true,
  });
  db.prepare("INSERT INTO meta (key, value) VALUES ('seeded_routines', '1')").run();
}

// --- Custom chat-app profiles (user-added messaging / AI apps) ------------------

db.exec(`CREATE TABLE IF NOT EXISTS chat_apps (id TEXT PRIMARY KEY, json TEXT NOT NULL, updated_at TEXT NOT NULL);`);

export function listCustomChatApps(): ChatAppProfile[] {
  return (db.prepare("SELECT json FROM chat_apps ORDER BY id").all() as { json: string }[]).map((r) => JSON.parse(r.json) as ChatAppProfile);
}

/** `validateKeys` checks key combos (injected to keep this file free of input-library imports). */
export function upsertCustomChatApp(p: ChatAppProfile, validateKeys: (combo: string) => void): ChatAppProfile {
  const id = (p.id || p.name || "").trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  if (!id) throw new Error("Give the app a name.");
  if (!p.name?.trim()) throw new Error("Give the app a name.");
  if (p.kind !== "people" && p.kind !== "assistant") throw new Error("Choose whether it's for messaging people or an AI assistant.");
  const processNames = (p.processNames ?? []).map((s) => s.trim().replace(/\.exe$/i, "")).filter(Boolean);
  if (!processNames.length) throw new Error("Add the app's process name (as in Task Manager, without .exe).");
  if (!p.launch?.trim()) throw new Error("Say how to open it: Start-menu name, shortcut name or web address.");
  const keys = (k?: string[]) => (k ?? []).map((s) => s.trim()).filter(Boolean);
  const clean: ChatAppProfile = {
    id,
    kind: p.kind,
    name: p.name.trim(),
    aliases: [...new Set([p.name.trim().toLowerCase(), ...(p.aliases ?? []).map((a) => a.trim().toLowerCase()).filter(Boolean)])],
    processNames,
    launch: p.launch.trim(),
    launchWaitMs: p.launchWaitMs,
    searchKeys: keys(p.searchKeys),
    searchWaitMs: p.searchWaitMs,
    pickKeys: keys(p.pickKeys),
    newChatKeys: keys(p.newChatKeys),
    focusMessageKeys: keys(p.focusMessageKeys),
    verify: p.verify === "title" ? "title" : "screenshot",
    sendKeys: keys(p.sendKeys).length ? keys(p.sendKeys) : ["Return"],
    notes: p.notes?.trim() || undefined,
  };
  if (clean.kind === "people" && !clean.searchKeys.length) throw new Error("Add the shortcut that opens the app's search / quick switcher (e.g. ctrl+k).");
  for (const combo of [...clean.searchKeys, ...clean.pickKeys, ...(clean.newChatKeys ?? []), ...(clean.focusMessageKeys ?? []), ...clean.sendKeys]) {
    try {
      validateKeys(combo);
    } catch {
      throw new Error(`"${combo}" isn't a key combo I understand. Use forms like ctrl+k, shift+Escape, Return.`);
    }
  }
  db.prepare("INSERT INTO chat_apps (id, json, updated_at) VALUES (?, ?, ?) ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at").run(
    id,
    JSON.stringify(clean),
    new Date().toISOString(),
  );
  return clean;
}

export function deleteCustomChatApp(id: string): boolean {
  return db.prepare("DELETE FROM chat_apps WHERE id = ?").run(id).changes > 0;
}

export function logAction(tool: string, input: unknown, decision: string, result?: string): void {
  db.prepare("INSERT INTO action_log (ts, tool, input, decision, result) VALUES (?, ?, ?, ?, ?)").run(
    new Date().toISOString(),
    tool,
    JSON.stringify(input).slice(0, 4000),
    decision,
    result?.slice(0, 4000) ?? null,
  );
}
