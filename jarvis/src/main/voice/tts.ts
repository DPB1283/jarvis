import ultron from "../../shared/ultron.json";
import type { Settings, VoiceOption } from "../../shared/settings";
import { bus } from "../bus";
import { elevenLabs } from "./elevenlabs";
import { Playback } from "./playback";

export async function listVoices(): Promise<VoiceOption[]> {
  const client = elevenLabs();
  const voices = new Map<string, VoiceOption>();
  let nextPageToken: string | undefined;
  do {
    const page = await client.voices.search({ pageSize: 100, nextPageToken }, { timeoutInSeconds: 15 });
    for (const voice of page.voices) {
      voices.set(voice.voiceId, {
        voiceId: voice.voiceId,
        name: voice.name ?? voice.voiceId,
        description: Object.values(voice.labels ?? {}).join(", ") || voice.description || "",
        previewUrl: voice.previewUrl ?? null,
      });
    }
    const next = page.hasMore ? page.nextPageToken : undefined;
    if (next === nextPageToken) break;
    nextPageToken = next;
  } while (nextPageToken);
  function british(voice: VoiceOption) {
    return /\b(british|english uk|united kingdom|received pronunciation)\b/i.test(voice.description);
  }
  return [...voices.values()].sort((a, b) =>
    Number(british(b)) - Number(british(a)) || a.name.localeCompare(b.name),
  );
}

export class SpeechOutput {
  // Arrow defers the lookup: field initializers run before constructor parameter properties are set.
  private playback = new Playback(() => this.getSettings());
  private queue: string[] = [];
  private current?: AbortController;
  private previewing = false;
  private speaking = false;
  private ended = false;

  constructor(private getSettings: () => Settings) {}

  get pending() {
    return !this.previewing && (this.speaking || this.queue.length > 0 || Boolean(this.current && !this.current.signal.aborted));
  }

  private async stream(text: string, voiceId: string, signal: AbortSignal, showState: boolean) {
    const id = voiceId || process.env.ELEVENLABS_VOICE_ID;
    if (!id) throw new Error("Choose an ElevenLabs voice in Settings or set ELEVENLABS_VOICE_ID.");
    const { speed, stability } = this.getSettings().voice;
    const audio = await elevenLabs().textToSpeech.stream(id, {
      text,
      modelId: "eleven_v4_turbo",
      outputFormat: "pcm_24000",
      voiceSettings: { speed, stability },
    }, { abortSignal: signal, timeoutInSeconds: 20, maxRetries: 1 });
    await this.playback.play(audio, signal, level => {
      // level 0 comes from playback's "speaker ran dry" timer, which can fire after the reply has
      // finished; it must not flip the orb back to speaking (it got stuck there).
      if (showState && !this.speaking && level > 0 && this.current) {
        this.speaking = true;
        bus.emit("state", { state: "speaking" });
      }
      if (showState) bus.emit("audio_level", { source: "tts", level });
    });
  }

  enqueue(text: string) {
    if (!text.trim() || this.getSettings().voice.muted) return;
    if (this.previewing) this.interrupt();
    this.ended = false;
    this.queue.push(text);
    void this.run();
  }

  end() {
    this.ended = true;
    if (!this.current && !this.queue.length) this.finishSpeaking();
  }

  private finishSpeaking() {
    if (!this.speaking) return;
    this.speaking = false;
    bus.emit("state", { state: "idle" });
  }

  private async run() {
    if (this.current || !this.queue.length) return;
    const controller = new AbortController();
    this.current = controller;
    try {
      do {
        while (this.queue.length && !controller.signal.aborted) {
          const text = this.queue.shift()!;
          const voice = this.getSettings().voice;
          if (!voice.muted) await this.stream(text, this.getSettings().ultronMode ? ultron.voiceId : voice.voiceId, controller.signal, true);
        }
        await this.playback.drain(controller.signal);
      } while (this.queue.length);
    } catch (error) {
      if (!controller.signal.aborted) {
        this.queue.length = 0;
        this.playback.stop();
        this.ended = true;
        const message = error instanceof Error && /^(Choose|Set) /.test(error.message)
          ? error.message
          : "Could not play the ElevenLabs response. Check the selected voice and audio output.";
        bus.emit("toast", { level: "error", text: message });
      }
    } finally {
      this.current = undefined;
      if (!controller.signal.aborted) {
        bus.emit("audio_level", { source: "tts", level: 0 });
        if (this.ended) {
          if (this.speaking) this.finishSpeaking();
          else bus.emit("state", { state: "idle" });
        }
      }
      if (this.queue.length) void this.run();
    }
  }

  async preview(voiceId: string) {
    if (this.current || this.speaking) throw new Error("Wait for the current speech to finish.");
    const controller = new AbortController();
    this.current = controller;
    this.previewing = true;
    try {
      await this.stream("Good evening. How may I help you today?", voiceId, controller.signal, false);
      await this.playback.drain(controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) throw error;
    } finally {
      this.current = undefined;
      this.previewing = false;
      if (this.queue.length) void this.run();
    }
  }

  chime(kind: "listen" | "dismiss" | "alarm" = "listen") {
    return this.playback.chime(kind);
  }

  warm() {
    return this.playback.warm();
  }

  interrupt() {
    this.current?.abort();
    this.queue.length = 0;
    this.playback.stop();
    this.speaking = false;
    this.previewing = false;
    bus.emit("audio_level", { source: "tts", level: 0 });
  }
}
