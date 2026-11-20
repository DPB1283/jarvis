// Chat app profiles: how to reach a conversation in each messaging app with the keyboard.
// Pure data, so it ports 1:1 to Rust (serde). The send_message engine lives in desktop.ts.
//
// OWNERSHIP: Codex extends BUILTIN_CHAT_APPS (Slack, Signal, Phone Link, Steam, web apps...).
// Keep each profile honest: if a shortcut isn't verified, prefer verify: "screenshot".

import type { ChatAppProfile } from "../../shared/settings";
import { listCustomChatApps } from "../memory/db";
export type { ChatAppProfile };

import profiles from "../../shared/chat-apps.json";

export const BUILTIN_CHAT_APPS = profiles as ChatAppProfile[];

/** User-added profiles (Settings → Messaging apps) override built-ins with the same id. */
export function allChatApps(): ChatAppProfile[] {
  const customApps = listCustomChatApps();
  const custom = new Set(customApps.map((a) => a.id));
  return [...customApps, ...BUILTIN_CHAT_APPS.filter((a) => !custom.has(a.id))];
}

/** Resolve "discord", "Slack", "texts"... to a profile. */
export function findChatApp(name: string): ChatAppProfile | null {
  const n = name.trim().toLowerCase();
  const apps = allChatApps();
  return (
    apps.find((a) => a.id === n || a.name.toLowerCase() === n || a.aliases.includes(n)) ??
    apps.find((a) => a.aliases.some((al) => n.includes(al)) || n.includes(a.name.toLowerCase())) ??
    null
  );
}
