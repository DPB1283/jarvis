import type Anthropic from "@anthropic-ai/sdk";
import { z } from "zod";
import type { Risk } from "../../shared/events";
import type { JarvisTool } from "./types";
import * as sys from "./system";
import * as files from "./files";
import * as mem from "./memory";
import { getWeather } from "./weather";
import { getNews } from "./news";
import * as routines from "./routines";
import { sendMessage, typeText } from "./desktop";
import { rememberScreen, takeScreenshot, useScreenshot } from "./screenshot";
import { writeDocument } from "./writing";
import { cancelTimerTool, listTimersTool, setTimer } from "./timers";
import { clipThat, findFilesSmart, readClipboard, setClipboard } from "./pcextras";
import { pcHealth } from "../health";
import { usagePatterns } from "../usage";
import { listScreens, useScreen } from "./screens";
import { youtubeSearch } from "./youtube";
import { calendarEvents } from "./calendar";
import { steamDeals, steamGames, steamLaunch } from "./steam";
import { nowPlaying } from "./media-info";

export const TOOLS: JarvisTool[] = [
  sys.getTime,
  getWeather,
  getNews,
  routines.listRoutinesTool,
  routines.saveRoutine,
  routines.deleteRoutineTool,
  sys.systemInfo,
  sys.openApp,
  sys.openUrl,
  sys.listWindows,
  sys.focusWindow,
  sys.closeApp,
  typeText,
  sendMessage,
  takeScreenshot,
  useScreenshot,
  rememberScreen,
  writeDocument,
  setTimer,
  listTimersTool,
  cancelTimerTool,
  readClipboard,
  setClipboard,
  clipThat,
  findFilesSmart,
  pcHealth,
  usagePatterns,
  listScreens,
  useScreen,
  youtubeSearch,
  calendarEvents,
  steamGames,
  steamLaunch,
  steamDeals,
  nowPlaying,
  sys.setVolume,
  sys.media,
  sys.power,
  sys.runPowershell,
  files.findFiles,
  files.listFolder,
  files.readFile,
  files.writeFile,
  files.movePath,
  files.deletePath,
  mem.remember,
  mem.recall,
  mem.editMemory,
  mem.forget,
  mem.addTodo,
  mem.setReminder,
  mem.changeSettings,
  mem.confirmWithUser,
] as JarvisTool[];

const byName = new Map(TOOLS.map((t) => [t.name, t]));

export function getTool(name: string): JarvisTool | undefined {
  return byName.get(name);
}

export function riskOf(tool: JarvisTool, input: unknown): Risk {
  return typeof tool.risk === "function" ? tool.risk(input) : tool.risk;
}

/** Tool definitions for the API, in a fixed order (keeps the prompt cache stable). */
export function apiToolDefs(): Anthropic.ToolUnion[] {
  return TOOLS.map((t) => {
    const schema = z.toJSONSchema(t.schema) as Record<string, unknown>;
    delete schema.$schema;
    return {
      name: t.name,
      description: t.description,
      input_schema: schema as Anthropic.Tool.InputSchema,
      eager_input_streaming: true,
    } as Anthropic.Tool;
  });
}
