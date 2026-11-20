// The contract between the brain (Claude), the voice pipeline (Codex) and the HUD renderer (Codex).
// Change this file only after agreeing over the bridge.

import type {
  ChatAppProfile,
  MemoryItem,
  MemoryKind,
  MemoryUpsert,
  Routine,
  RoutineUpsert,
  Settings,
  SettingsPatch,
  TimerInfo,
  VoiceOption,
} from "./settings";

export type JarvisState = "idle" | "listening" | "thinking" | "speaking" | "error";
export type Risk = "safe" | "unsafe";

// ---------------------------------------------------------------------------
// 1) In-process bus inside the Electron main process (src/main/bus.ts).
//    voice/ (Codex) <-> brain/ (Claude). Typed EventEmitter: bus.emit(name, payload).
// ---------------------------------------------------------------------------
export interface BusEvents {
  /** voice -> brain: wake word heard, talk hotkey pressed, or speech during the follow-up window; start listening. */
  wake: { source: "wakeword" | "hotkey" | "followup" };
  /**
   * voice -> renderer: the post-reply conversation window. While active, speaking (no wake word) starts a turn.
   * `until` is epoch ms when it closes; `active: false` when it ends early or times out.
   */
  followup: { active: boolean; until: number };
  /** voice -> brain: live transcript. final=true ends the utterance and starts a turn. */
  user_said: { text: string; final: boolean };
  /** renderer -> brain (relayed by main): typed text box input. */
  user_text: { text: string };
  /** brain -> voice: a chunk of reply text to speak (sentence-sized). */
  speak: { text: string };
  /** brain -> voice: no more text for this reply. */
  speak_end: Record<string, never>;
  /** anyone -> everyone: stop talking and cancel the current turn. */
  interrupt: { reason: "user" | "barge_in" | "new_turn" };
  /** voice -> renderer: mic/tts loudness 0..1 for the orb, ~30 Hz max. */
  audio_level: { level: number; source: "mic" | "tts" };
  /** brain/voice -> renderer: orb state. */
  state: { state: JarvisState };
  /** brain -> renderer + voice: needs a yes/no from the user. Voice may answer it too. */
  approval_request: { id: string; tool: string; summary: string; risk: Risk };
  /** renderer/voice -> brain */
  approval_response: { id: string; ok: boolean };
  /** brain -> renderer: streaming reply text for captions. */
  assistant_delta: { text: string };
  assistant_done: { text: string };
  /** brain -> renderer + voice: a reminder fired. */
  reminder_due: { id: number; text: string };
  /** anyone -> renderer */
  toast: { level: "info" | "warn" | "error"; text: string };
  settings_changed: { settings: Settings };
  /** Routines were created/edited/deleted (from Settings or by voice); UIs should re-list. */
  routines_changed: Record<string, never>;
  dictation: { active: boolean };
  /** Full list of running timers/alarms whenever it changes (empty = none). Renderer animates progress locally. */
  timers: { timers: TimerInfo[] };
  /** A timer/alarm just went off (HUD can flash; voice plays the alarm). */
  timer_done: { timer: TimerInfo };
  /**
   * Game mode on/off (auto-detected fullscreen app, or forced in settings). Quiet behaviour follows `active`;
   * the HUD hides only when `hideHud` (the orb is on the same screen as the game).
   */
  game_mode: { active: boolean; app: string | null; hideHud: boolean };
}

export type BusEventName = keyof BusEvents;

// ---------------------------------------------------------------------------
// 2) Main <-> renderer IPC. Exposed by src/preload/index.ts as window.jarvis.
// ---------------------------------------------------------------------------

/** Events the main process forwards to the renderer on channel "jarvis:event". */
export const RENDERER_EVENTS = [
  "wake",
  "state",
  "user_said",
  "assistant_delta",
  "assistant_done",
  "approval_request",
  "approval_response",
  "audio_level",
  "reminder_due",
  "toast",
  "settings_changed",
  "followup",
  "routines_changed",
  "dictation",
  "timers",
  "timer_done",
  "game_mode",
] as const satisfies readonly BusEventName[];

export type RendererEventName = (typeof RENDERER_EVENTS)[number];
export type RendererEvent = {
  [K in RendererEventName]: { type: K } & BusEvents[K];
}[RendererEventName];

/** Events the renderer may send to main on channel "jarvis:send". */
export const RENDERER_SENDABLE = ["user_text", "approval_response", "interrupt", "wake"] as const satisfies readonly BusEventName[];
export type RendererSendableName = (typeof RENDERER_SENDABLE)[number];
export type RendererSend = {
  [K in RendererSendableName]: { type: K } & BusEvents[K];
}[RendererSendableName];

/** Request/response IPC (ipcRenderer.invoke). Channel name = key. Main implements these. */
export interface InvokeApi {
  "secrets:status": () => Record<string, boolean>;
  "secrets:set": (name: string, value: string) => void;
  "secrets:clear": (name: string) => void;
  "settings:get": () => Settings;
  "settings:update": (patch: SettingsPatch) => Settings;
  "memory:list": (filter?: { kind?: MemoryKind; query?: string }) => MemoryItem[];
  "memory:upsert": (item: MemoryUpsert) => MemoryItem;
  "memory:delete": (id: number) => boolean;
  /** Deletes all memory items of the given kinds (all kinds if omitted). Renderer must confirm first. */
  "memory:clear": (kinds?: MemoryKind[]) => number;
  /** Opens a memory's screenshot in the default image viewer. */
  "memory:open-image": (id: number) => boolean;
  "voices:list": () => VoiceOption[];
  "routines:list": () => Routine[];
  /** Validates time "HH:MM" and days 0-6; throws with a readable message if invalid. */
  "routines:upsert": (routine: RoutineUpsert) => Routine;
  "routines:delete": (id: number) => boolean;
  /** "Run now" button: Jarvis performs the routine immediately. */
  "routines:run": (id: number) => void;
  /** Messaging / AI chat app profiles. Custom ones override built-ins with the same id. */
  "chatapps:list": () => { builtin: ChatAppProfile[]; custom: ChatAppProfile[] };
  /** Validates (keys like "ctrl+k", required fields); throws a readable Error. Returns the saved profile. */
  "chatapps:upsert": (profile: ChatAppProfile) => ChatAppProfile;
  "chatapps:delete": (id: string) => boolean;
  /** Current timers (for the HUD on load). */
  "timers:list": () => TimerInfo[];
  "timers:cancel": (id: string) => boolean;
  /** Microphone names (PvRecorder). Output devices are enumerated in the renderer via navigator.mediaDevices. */
  "audio:inputs": () => string[];
  "audio:test-input": (device: string, gain: number) => number;
  /** Native-output names in the Tauri port; Electron enumerates Web Audio devices directly. */
  "audio:outputs": () => string[];
  "voice:preview": (voiceId: string) => void;
  /** HUD windows: show/hide the settings panel, let clicks through the orb window. */
  "window:open-settings": () => void;
  "window:set-ignore-mouse": (ignore: boolean) => void;
  /** Orb drag: main moves the HUD window with the cursor between "start" and "end", then saves the spot. */
  "window:drag": (phase: "start" | "end") => void;
}

export type InvokeChannel = keyof InvokeApi;

/** Shape of window.jarvis in the renderer. */
export interface JarvisApi {
  on(listener: (event: RendererEvent) => void): () => void;
  send(event: RendererSend): void;
  invoke<C extends InvokeChannel>(
    channel: C,
    ...args: Parameters<InvokeApi[C]>
  ): Promise<ReturnType<InvokeApi[C]>>;
}

declare global {
  interface Window {
    jarvis: JarvisApi;
  }
}



