import path from "node:path";
import { BrowserWindow } from "electron";
import { setTimeout as delay } from "node:timers/promises";
import type { Settings } from "../../shared/settings";
import { bus } from "../bus";

const sampleRate = 24000;
const chunkBytes = sampleRate * 2 * 0.04;

// The hidden player page (renderer index.html#player, see renderer/player.ts) only receives PCM bytes.
// Web Audio handles output, sample-rate conversion and the chosen speaker (AudioContext.setSinkId).
// It shares the settings page's origin so speaker deviceIds picked in Settings are valid here.
function loadPlayerPage(win: BrowserWindow) {
  const dev = process.env.ELECTRON_RENDERER_URL;
  return dev
    ? win.loadURL(`${dev}#player`)
    : win.loadFile(path.join(__dirname, "../renderer/index.html"), { hash: "player" });
}

interface ScheduledAudio {
  lead: number;
  tail: number;
  level: number;
}

export class Playback {
  private window?: BrowserWindow;
  private loading?: Promise<void>;
  private meters = new Set<ReturnType<typeof setTimeout>>();
  private playingUntil = 0;
  private warnedSink = "";
  private silenceTimer?: ReturnType<typeof setTimeout>;

  constructor(private getSettings?: () => Settings) {}

  /** Points the player at the speaker chosen in Settings (default device if none / not found). */
  private async applySink(player: Electron.WebContents) {
    const audio = this.getSettings?.().audio;
    const result: { ok: boolean; missing?: boolean; error?: string } = await player.executeJavaScript(
      `globalThis.jarvisPlayer.setSink(${JSON.stringify(audio?.outputDeviceId ?? "")}, ${JSON.stringify(audio?.outputDevice ?? "")})`,
    );
    const key = `${audio?.outputDeviceId}|${audio?.outputDevice}`;
    if (!result.ok && this.warnedSink !== key) {
      this.warnedSink = key;
      bus.emit("toast", {
        level: "warn",
        text: result.missing
          ? `Speaker "${audio?.outputDevice}" isn't connected, so I'm using the default output.`
          : `Couldn't switch speakers (${result.error ?? "unknown error"}). Using the default output.`,
      });
    } else if (result.ok) this.warnedSink = "";
  }

  private async player(signal: AbortSignal) {
    if (!this.window || this.window.isDestroyed()) {
      this.window = new BrowserWindow({
        show: false,
        skipTaskbar: true,
        webPreferences: {
          sandbox: true,
          contextIsolation: true,
          nodeIntegration: false,
          autoplayPolicy: "no-user-gesture-required",
          backgroundThrottling: false,
        },
      });
      this.window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
      this.window.webContents.on("will-navigate", event => event.preventDefault());
      this.loading = loadPlayerPage(this.window);
    }
    const window = this.window;
    try {
      await this.loading;
    } catch (error) {
      if (!window.isDestroyed()) window.destroy();
      throw error;
    }
    signal.throwIfAborted();
    return window.webContents;
  }

  async play(
    stream: ReadableStream<Uint8Array>,
    signal: AbortSignal,
    onLevel: (level: number) => void,
  ) {
    const reader = stream.getReader();
    const cancel = () => { void reader.cancel().catch(() => {}); };
    signal.addEventListener("abort", cancel, { once: true });
    let pending = Buffer.alloc(0);
    try {
      const player = await this.player(signal);
      await this.applySink(player);
      const push = async (bytes: Buffer) => {
        signal.throwIfAborted();
        const audio: ScheduledAudio = await player.executeJavaScript(
          `globalThis.jarvisPlayer.push(${JSON.stringify(bytes.toString("base64"))}, ${Number(this.getSettings?.().voice.volume ?? 100)})`,
        );
        signal.throwIfAborted();
        this.playingUntil = performance.now() + audio.tail;
        const timer = setTimeout(() => {
          this.meters.delete(timer);
          if (!signal.aborted) onLevel(audio.level);
        }, Math.max(0, audio.lead));
        this.meters.add(timer);
        // When the speaker runs dry (end, or a gap while the next sentence streams in), report silence
        // so the orb doesn't freeze mid-pulse. Each new chunk pushes this deadline back.
        clearTimeout(this.silenceTimer);
        this.silenceTimer = setTimeout(() => {
          if (!signal.aborted) onLevel(0);
        }, Math.max(0, audio.tail) + 30);
        if (audio.tail > 100) await delay(audio.tail - 100, undefined, { signal });
      };
      while (!signal.aborted) {
        const { done, value } = await reader.read();
        if (done) break;
        pending = Buffer.concat([pending, value]);
        while (pending.length >= chunkBytes) {
          await push(pending.subarray(0, chunkBytes));
          pending = pending.subarray(chunkBytes);
        }
      }
      if (pending.length % 2 !== 0 && !signal.aborted) throw new Error("Incomplete PCM sample");
      if (pending.length) await push(pending);
    } finally {
      signal.removeEventListener("abort", cancel);
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
  }

  /** Plays a short chime on the chosen speaker: rising = listening, falling = dismissed. */
  async chime(kind: "listen" | "dismiss" | "alarm" = "listen") {
    const player = await this.player(new AbortController().signal);
    await this.applySink(player);
    await player.executeJavaScript(`globalThis.jarvisPlayer.chime(${JSON.stringify(kind)})`);
  }

  /** Creates the hidden player ahead of time so the first chime/reply isn't delayed. */
  async warm() {
    await this.player(new AbortController().signal);
  }

  async drain(signal: AbortSignal) {
    await delay(Math.max(0, this.playingUntil - performance.now()), undefined, { signal });
  }

  stop() {
    this.playingUntil = 0;
    clearTimeout(this.silenceTimer);
    for (const timer of this.meters) clearTimeout(timer);
    this.meters.clear();
    // Destroying the isolated player also stops any queued audio immediately.
    if (this.window && !this.window.isDestroyed()) this.window.destroy();
    this.window = undefined;
    this.loading = undefined;
  }
}

