import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { JarvisApi, RendererEvent } from "../shared/events";

if ("__TAURI_INTERNALS__" in window && !window.jarvis) {
  const eventListeners = new Set<(event: RendererEvent) => void>();
  let listening = false;
  let unlisten: (() => void) | undefined;
  function startListening() {
    if (listening) return;
    listening = true;
    void listen<RendererEvent>("jarvis:event", ({ payload }) => {
      for (const callback of eventListeners) callback(payload);
    }).then(stop => {
      if (eventListeners.size) {
        unlisten = stop;
        void invoke("jarvis_invoke", { channel: "window:ready", args: [] }).catch(() => {});
      }
      else {
        stop();
        listening = false;
      }
    }).catch(error => {
      listening = false;
      console.error("Jarvis event connection could not be opened.", error);
    });
  }

  const api: JarvisApi = {
    on(callback) {
      eventListeners.add(callback);
      startListening();
      return () => {
        eventListeners.delete(callback);
        if (!eventListeners.size && unlisten) {
          unlisten();
          unlisten = undefined;
          listening = false;
        }
      };
    },
    send(event) {
      void invoke("jarvis_send", { event }).catch(error => {
        console.error("Jarvis could not send the HUD event.", error);
      });
    },
    invoke(channel, ...args) {
      return invoke("jarvis_invoke", { channel, args });
    },
  };
  window.jarvis = api;

  if (location.hash !== "#settings") {
    type Cursor = { x: number; y: number };
    void listen<Cursor | null>("jarvis:cursor", ({ payload }) => {
      const target = payload ? document.elementFromPoint(payload.x, payload.y) : null;
      if (target) {
        target.dispatchEvent(new MouseEvent("mousemove", {
          bubbles: true, clientX: payload!.x, clientY: payload!.y,
        }));
      } else {
        document.documentElement.dispatchEvent(new MouseEvent("mouseleave"));
      }
    }).then(stop => window.addEventListener("pagehide", stop, { once: true })).catch(() => {});
  }
}
