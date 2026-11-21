// Shared settings + memory types. Owned by Claude; Codex reads these.
// Persisted to data/settings.json (settings) and data/jarvis.sqlite (memory).

export type SafetyMode = "ask_all" | "ask_unsafe" | "full_access";
export type ReplyLength = "brief" | "middle" | "normal";
export type Language = "en" | "es" | "fr" | "de" | "yue";
export const LANGUAGES: ReadonlyArray<readonly [Language, string]> = [
  ["en", "English"],
  ["es", "Español (Spanish)"],
  ["fr", "Français (French)"],
  ["de", "Deutsch (German)"],
  ["yue", "廣東話 (Cantonese)"],
];
export type CaptionPosition = "below_orb" | "bottom_center";
export type OrbCorner = "top_right" | "top_left" | "bottom_right" | "bottom_left";

export interface Settings {
  /** How Jarvis usually addresses the user, e.g. "sir", "ma'am", "boss". */
  ultronMode: boolean;
  /** Language Jarvis speaks and understands (Rust app): en, es, fr, de, yue (Cantonese). */
  language?: Language;
  userTitle: string;
  /** The user's name; used now and then alongside the title. "" = not set. */
  userName: string;
  captions: {
    enabled: boolean;
    position: CaptionPosition;
    /** Font size in px. */
    size: number;
  };
  voice: {
    /** ElevenLabs voice id. Empty = use ELEVENLABS_VOICE_ID from .env. */
    voiceId: string;
    /** 0.7 - 1.2 */
    speed: number;
    /** 0 - 1 */
    stability: number;
    volume: number;
    /** Mute spoken replies (text only). */
    muted: boolean;
    /** false = text only: no microphone, wake word, speech recognition or spoken replies (no ElevenLabs). Rust app. */
    enabled?: boolean;
  };
  personality: {
    /** 0 = polite and straight, 10 = very cheeky. */
    sarcasm: number;
    replyLength: ReplyLength;
  };
  audio: {
    /** Microphone name as reported by PvRecorder. "" = Windows default. */
    inputDevice: string;
    inputGain: number;
    /** Speaker label as reported by the browser (stable across origins). "" = Windows default. */
    outputDevice: string;
    /** Browser deviceId for outputDevice (origin-specific; label is the fallback). */
    outputDeviceId: string;
  };
  conversation: {
    /** After Jarvis replies, keep listening this long without the wake word. 0 = off. */
    followUpSeconds: number;
    /** How long a pause (seconds) counts as "finished talking" before Jarvis processes it. */
    endOfSpeechSeconds: number;
    /** Starting to talk while Jarvis is speaking cuts him off. Rust app. */
    interruptBySpeaking?: boolean;
  };
  /** Home location: default for weather, news region and "this morning" briefings. */
  location: {
    /** e.g. "Los Angeles" or "Austin, Texas". "" = not set (Jarvis will ask). */
    city: string;
    units: "metric" | "imperial";
    /** Two-letter country code for news, e.g. "US", "GB". */
    newsRegion: string;
    /** "neutral": national/world headlines from wire services (Reuters, AP, AFP). "all": every outlet. */
    newsSources: "neutral" | "all";
    /** Let Jarvis show maps (Mapbox) on screen. Rust app. */
    showMaps?: boolean;
  };
  /** Game mode: quiet while a fullscreen game runs (no chimes, orb hidden, no follow-up listening). Reminders and timers still speak. */
  gameMode: { mode: "auto" | "on" | "off" };
  /** PC health watch: warn about heat, full disks, RAM pressure, runaway apps. */
  health: { warnings: boolean };
  /** Google Calendar's "Secret address in iCal format" (Calendar settings → your calendar → Integrate). "" = not connected. */
  calendar: { icsUrl: string };
  /**
   * "Jarvis, clip that": which recorder saves the clip. auto = Medal if running, else NVIDIA if its overlay runs, else Xbox Game Bar.
   * hotkey overrides the app's default (Medal F8, NVIDIA alt+F10, Xbox super+alt+g), xdotool-style.
   */
  clips: { app: "auto" | "medal" | "nvidia" | "xbox"; hotkey: string };
  safetyMode: SafetyMode;
  wakeWord: {
    enabled: boolean;
    /** Porcupine sensitivity 0 - 1. */
    sensitivity: number;
  };
  hotkeys: {
    /** Push-to-talk / open text box. Electron accelerator syntax. */
    talk: string;
    settings: string;
    /** Mute / unmute spoken replies. Rust app. */
    mute?: string;
  };
  orb: {
    showInCaptures?: boolean;
    /** Layout anchor: which side captions/chips grow from. Also the snap position when `position` is null. */
    corner: OrbCorner;
    /** Where the user dragged the orb (screen DIP, top-left of the orb). null = snap to `corner`. */
    position: { x: number; y: number } | null;
    /** Orb diameter in px. */
    size: number;
    /** 0.2 - 1 */
    opacity: number;
  };
  launchAtStartup: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  ultronMode: false,
  language: "en",
  userTitle: "sir",
  userName: "",
  captions: { enabled: true, position: "below_orb", size: 15 },
  // Daniel: ElevenLabs premade voice, British male, formal and steady - the closest stock match for Jarvis.
  voice: { voiceId: "onwK4e9ZLuTAKqWW03F9", speed: 1.0, stability: 0.55, volume: 100, muted: false },
  personality: { sarcasm: 5, replyLength: "brief" },
  audio: { inputDevice: "", inputGain: 100, outputDevice: "", outputDeviceId: "" },
  conversation: { followUpSeconds: 60, endOfSpeechSeconds: 1.2 },
  location: (() => {
    // Region via Intl.Locale (locale.split("-")[1] would give "Hans" for zh-Hans-CN).
    let region = "US";
    try {
      region = new Intl.Locale(Intl.DateTimeFormat().resolvedOptions().locale).maximize().region ?? "US";
    } catch {
      /* keep US */
    }
    return {
      city: "",
      units: ["US", "LR", "MM"].includes(region) ? "imperial" : "metric",
      newsRegion: region,
      newsSources: "neutral",
    } as const;
  })(),
  gameMode: { mode: "auto" },
  health: { warnings: true },
  calendar: { icsUrl: "" },
  clips: { app: "auto", hotkey: "" },
  safetyMode: "ask_unsafe",
  wakeWord: { enabled: true, sensitivity: 0.6 },
  hotkeys: { talk: "Control+Space", settings: "Control+Alt+J", mute: "Control+Alt+M" },
  orb: { corner: "top_right", position: null, size: 96, opacity: 0.95, showInCaptures: true },
  launchAtStartup: false,
};

/** Deep-partial for settings updates from the UI or from Jarvis's own tools. */
export type SettingsPatch = {
  [K in keyof Settings]?: Settings[K] extends object ? Partial<Settings[K]> : Settings[K];
};

export type MemoryKind = "fact" | "note" | "todo" | "reminder";

export interface MemoryItem {
  id: number;
  kind: MemoryKind;
  text: string;
  /** todos only */
  done: boolean;
  /** reminders only: ISO timestamp */
  dueAt: string | null;
  /** Screenshot saved with this memory ("remember this" about the screen). Open via "memory:open-image". */
  imagePath: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryUpsert {
  /** Omit to create. */
  id?: number;
  kind: MemoryKind;
  text: string;
  done?: boolean;
  dueAt?: string | null;
  imagePath?: string | null;
}

/**
 * A routine: something Jarvis does when you ask for it by meaning ("how's it looking this morning")
 * and/or automatically on a schedule. Stored in data/jarvis.sqlite.
 */
export interface Routine {
  id: number;
  name: string;
  /** Example phrases. Jarvis matches by meaning, so these are hints, not exact commands. */
  triggers: string[];
  /** Plain-English steps, e.g. "Weather for my city today, top 3 headlines, my reminders and open to-dos." */
  instructions: string;
  /** Run automatically at this local time on these days (0 = Sunday ... 6 = Saturday). null = only on request. */
  schedule: { time: string; days: number[] } | null;
  enabled: boolean;
  lastRunAt: string | null;
}

export interface RoutineUpsert {
  /** Omit to create. */
  id?: number;
  name: string;
  triggers: string[];
  instructions: string;
  schedule: { time: string; days: number[] } | null;
  enabled: boolean;
}

/**
 * How Jarvis drives a messaging app with the keyboard (see main/tools/chat-apps.ts for built-ins).
 * Key combos are xdotool-style: "ctrl+k", "shift+Escape", "Return".
 */
export interface ChatAppProfile {
  /** Stable id, e.g. "discord". Lowercase, no spaces. */
  id: string;
  /** "people": message a person/channel (needs a recipient; sending asks for approval). "assistant": an AI chat (no recipient). */
  kind: "people" | "assistant";
  /** Display name, e.g. "Discord". */
  name: string;
  /** Words the user might say for this app ("discord", "dc"). Lowercase. */
  aliases: string[];
  /** Windows process names (no .exe) whose main window is the app. For web apps, the browser's (e.g. "chrome", "msedge"). */
  processNames: string[];
  /** How to start it if no window is open: Start-menu app name, .lnk name, or a URL (opens in the default browser). */
  launch: string;
  /** Extra wait after launching before the app is usable. */
  launchWaitMs?: number;
  /** People apps: keys that open the "jump to conversation" search. The engine pastes the recipient, waits, then presses pickKeys. */
  searchKeys: string[];
  searchWaitMs?: number;
  /** Keys that open the highlighted search result. Usually ["Return"]. */
  pickKeys: string[];
  /** Assistant apps: keys that start a fresh chat (used when Jarvis is asked for a new chat). */
  newChatKeys?: string[];
  /** Keys that put focus in the message box (often none needed). */
  focusMessageKeys?: string[];
  /**
   * Checking the right chat opened before typing (people apps):
   * "title" = window title contains the recipient, else falls back to a screenshot check;
   * "screenshot" = Jarvis looks at the screen and confirms.
   */
  verify: "title" | "screenshot";
  /** Keys that send. Usually ["Return"]. */
  sendKeys: string[];
  /** Short note for Jarvis about quirks. */
  notes?: string;
}

/** A running countdown ("pizza timer, 12 minutes") or an alarm at a clock time. */
export interface TimerInfo {
  id: string;
  label: string;
  kind: "timer" | "alarm";
  /** epoch ms when it goes off */
  endsAt: number;
  /** total length in ms (alarms: from when it was set), for drawing progress */
  durationMs: number;
}

export interface VoiceOption {
  voiceId: string;
  name: string;
  /** e.g. "british, calm, male" from ElevenLabs labels */
  description: string;
  previewUrl: string | null;
}

/** Colour palette - use these in the renderer (Codex) so everything matches. */
export const PALETTE = {
  bg: "#05080A",
  panel: "#0B1216",
  aqua: "#3FE0D0",
  blue: "#1E90C8",
  glow: "#7FFFE8",
  text: "#CFEFF0",
  dim: "#6B8A8E",
  warn: "#FFB547",
  danger: "#FF4D5E",
} as const;
