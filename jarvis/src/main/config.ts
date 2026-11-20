import fs from "node:fs";
import path from "node:path";
import dotenv from "dotenv";
import { DEFAULT_SETTINGS, type Settings, type SettingsPatch } from "../shared/settings";
import { bus } from "./bus";

/** jarvis/ project root. npm scripts and electron-vite both run from here. */
export const ROOT = process.env.JARVIS_ROOT ?? process.cwd();
export const DATA_DIR = process.env.JARVIS_DATA_DIR ?? path.join(ROOT, "data");
fs.mkdirSync(DATA_DIR, { recursive: true });

// .env.local (written by `elevenlabs onboard`) wins over .env. Main-process only; never reaches renderer code.
dotenv.config({ path: [path.join(ROOT, ".env.local"), path.join(ROOT, ".env")], quiet: true });

export const MODEL = "claude-haiku-5-5";

const SETTINGS_FILE = path.join(DATA_DIR, "settings.json");

function merge(base: Settings, patch: SettingsPatch): Settings {
  const out: Record<string, unknown> = { ...base };
  for (const [k, v] of Object.entries(patch)) {
    if (v === undefined) continue;
    const cur = (base as unknown as Record<string, unknown>)[k];
    out[k] =
      v !== null && typeof v === "object" && !Array.isArray(v) && typeof cur === "object" && cur !== null
        ? { ...(cur as object), ...(v as object) }
        : v;
  }
  return out as unknown as Settings;
}

function load(): Settings {
  try {
    const raw = JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf8")) as SettingsPatch;
    return merge(DEFAULT_SETTINGS, raw);
  } catch {
    return structuredClone(DEFAULT_SETTINGS);
  }
}

let current = load();

export function getSettings(): Settings {
  return current;
}

export function updateSettings(patch: SettingsPatch): Settings {
  current = merge(current, patch);
  fs.writeFileSync(SETTINGS_FILE, JSON.stringify(current, null, 2));
  bus.emit("settings_changed", { settings: current });
  return current;
}

export function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Add it to jarvis/.env (see .env.example).`);
  return v;
}
