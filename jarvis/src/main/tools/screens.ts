// Multi-monitor: see every screen separately, and choose which one the computer actions work on.
import type Anthropic from "@anthropic-ai/sdk";
import { z } from "zod";
import { defineTool } from "./types";
import { activeScreen, captureScreen, runComputerAction, screens, setActiveScreen } from "./computer";

const describe = (m: { index: number; width: number; height: number; primary: boolean; x: number }, active: number) =>
  `Screen ${m.index}${m.primary ? " (main)" : ""}: ${m.width}x${m.height}${m.index === active ? " — ACTIVE (computer actions use this one)" : ""}`;

export const listScreens = defineTool({
  name: "look_at_screens",
  description:
    "See all of the user's monitors at once: one small picture per screen, labelled, plus which is active. " +
    "Use when you're not sure which screen something is on ('on my other monitor', 'the video on the right'). Then use_screen to work on it.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "look at all screens",
  run: async () => {
    const all = screens();
    const active = activeScreen().index;
    const content: Anthropic.ToolResultBlockParam["content"] = [];
    for (const m of all) {
      const shot = await captureScreen(m, undefined, 800); // thumbnail: overview only, not for clicking
      content.push({ type: "text", text: describe(m, active) + (m.x < all[0].x ? " (left of main)" : m.x > all[0].x ? " (right of main)" : "") });
      content.push({ type: "image", source: { type: "base64", media_type: "image/jpeg", data: shot.data } });
    }
    content.push({ type: "text", text: "These are overview thumbnails. Take a normal screenshot after use_screen before clicking." });
    return content;
  },
});

export const useScreen = defineTool({
  name: "use_screen",
  description: "Switch which monitor the computer actions (screenshot, click, type...) work on. Returns a screenshot of that screen.",
  schema: z.object({ screen: z.number().int().min(1) }),
  risk: "safe",
  summary: (i) => `switch to screen ${i.screen}`,
  run: async ({ screen }) => {
    const m = setActiveScreen(screen);
    const shot = await runComputerAction("screenshot", {});
    const note = `Now working on screen ${m.index} (${m.width}x${m.height}). Coordinates refer to this screenshot.`;
    return Array.isArray(shot) ? [{ type: "text", text: note }, ...shot] : note;
  },
});
