import { BrowserWindow, ipcMain, shell } from "electron";
import { PvRecorder } from "@picovoice/pvrecorder-node";
import {
  RENDERER_EVENTS,
  RENDERER_SENDABLE,
  type InvokeApi,
  type InvokeChannel,
  type RendererSend,
} from "../shared/events";
import { bus } from "./bus";
import { getSettings, updateSettings } from "./config";
import {
  clearMemory,
  deleteCustomChatApp,
  deleteMemory,
  deleteRoutine,
  listCustomChatApps,
  listMemory,
  listRoutines,
  upsertCustomChatApp,
  upsertMemory,
  upsertRoutine,
} from "./memory/db";
import { BUILTIN_CHAT_APPS } from "./tools/chat-apps";
import { parseCombo } from "./tools/computer";
import { cancelTimer, listTimers } from "./timers";
import { listVoices, previewVoice } from "./voice";

export interface WindowHooks {
  openSettings: () => void;
  setIgnoreMouse: (win: BrowserWindow, ignore: boolean) => void;
  drag: (phase: "start" | "end") => void;
  runRoutine: (id: number) => void;
}

/** Forwards bus events to every renderer and wires the invoke channels. */
export function registerIpc(hooks: WindowHooks): void {
  for (const name of RENDERER_EVENTS) {
    bus.on(name, (payload) => {
      for (const w of BrowserWindow.getAllWindows()) {
        if (!w.isDestroyed()) w.webContents.send("jarvis:event", { type: name, ...payload });
      }
    });
  }

  const sendable = new Set<string>(RENDERER_SENDABLE);
  ipcMain.on("jarvis:send", (_e, msg: RendererSend) => {
    if (!msg || typeof msg !== "object" || !sendable.has(msg.type)) return;
    const { type, ...payload } = msg;
    bus.emit(type, payload as never);
  });

  const handle = <C extends InvokeChannel>(
    channel: C,
    fn: (e: Electron.IpcMainInvokeEvent, ...args: Parameters<InvokeApi[C]>) => ReturnType<InvokeApi[C]> | Promise<ReturnType<InvokeApi[C]>>,
  ) => ipcMain.handle(channel, (e, ...args) => fn(e, ...(args as Parameters<InvokeApi[C]>)));

  handle("settings:get", () => getSettings());
  handle("settings:update", (_e, patch) => updateSettings(patch));
  handle("memory:list", (_e, filter) => listMemory(filter ?? {}));
  handle("memory:upsert", (_e, item) => {
    // Reminders from the editor may arrive as local datetime strings; normalise to ISO.
    if (item.dueAt) item = { ...item, dueAt: new Date(item.dueAt).toISOString() };
    return upsertMemory(item);
  });
  handle("memory:delete", (_e, id) => deleteMemory(id));
  handle("memory:clear", (_e, kinds) => clearMemory(kinds));
  handle("memory:open-image", async (_e, id) => {
    const p = listMemory().find((m) => m.id === id)?.imagePath;
    if (!p) return false;
    return (await shell.openPath(p)) === "";
  });
  handle("voices:list", () => listVoices());
  handle("audio:inputs", () => PvRecorder.getAvailableDevices());
  handle("routines:list", () => listRoutines());
  handle("routines:upsert", (_e, routine) => {
    const saved = upsertRoutine(routine); // throws a readable Error for bad time/days
    bus.emit("routines_changed", {});
    return saved;
  });
  handle("routines:delete", (_e, id) => {
    const ok = deleteRoutine(id);
    if (ok) bus.emit("routines_changed", {});
    return ok;
  });
  handle("routines:run", (_e, id) => hooks.runRoutine(id));
  handle("chatapps:list", () => ({ builtin: BUILTIN_CHAT_APPS, custom: listCustomChatApps() }));
  handle("chatapps:upsert", (_e, profile) => upsertCustomChatApp(profile, (combo) => void parseCombo(combo)));
  handle("chatapps:delete", (_e, id) => deleteCustomChatApp(id));
  handle("timers:list", () => listTimers());
  handle("timers:cancel", (_e, id) => cancelTimer(id) !== null);
  handle("voice:preview", async (_e, id) => {
    await previewVoice(id);
  });
  handle("window:open-settings", () => hooks.openSettings());
  handle("window:drag", (_e, phase) => hooks.drag(phase));
  handle("window:set-ignore-mouse", (e, ignore) => {
    const win = BrowserWindow.fromWebContents(e.sender);
    if (win) hooks.setIgnoreMouse(win, ignore);
  });
}
