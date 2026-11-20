import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import type { JarvisApi, RendererEvent } from "../shared/events";

const api: JarvisApi = {
  on(listener) {
    const handler = (_e: IpcRendererEvent, event: RendererEvent) => listener(event);
    ipcRenderer.on("jarvis:event", handler);
    return () => ipcRenderer.off("jarvis:event", handler);
  },
  send(event) {
    ipcRenderer.send("jarvis:send", event);
  },
  invoke(channel, ...args) {
    return ipcRenderer.invoke(channel, ...args);
  },
};

contextBridge.exposeInMainWorld("jarvis", api);
