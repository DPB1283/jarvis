import { PvRecorder } from "@picovoice/pvrecorder-node";
import type { JarvisState } from "../../shared/events";
import type { Settings } from "../../shared/settings";
import { bus } from "../bus";
import { openTranscription } from "./stt";
import { SpeechOutput } from "./tts";
import { createWakeWord } from "./wakeword";
import { isGameMode } from "../gamemode";

export { listVoices } from "./tts";

let output: SpeechOutput | undefined;

export function speechPending() {
  return output?.pending ?? false;
}

/** Falling two-note chime: "understood, standing down". */
export function playDismissChime() {
  void output?.chime("dismiss").catch(() => {});
}

export async function previewVoice(voiceId: string) {
  if (!output) throw new Error("Voice output has not started.");
  await output.preview(voiceId);
}

export function startVoice(getSettings: () => Settings) {
  const speech = new SpeechOutput(getSettings);
  output = speech;
  void speech.warm().catch(() => {}); // load the hidden player now so the first chime is instant
  let detector: ReturnType<typeof createWakeWord> | undefined;
  let transcription: ReturnType<typeof openTranscription> | undefined;
  let listeningTimer: ReturnType<typeof setTimeout> | undefined;
  let state: JarvisState = "idle";
  let capturing = false;
  let disposed = false;
  let micFailed = false;
  let wakeConfig = "";
  let captureAfter = 0;

  function notify(text: string, level: "warn" | "error" = "warn") {
    bus.emit("toast", { level, text });
  }

  function stopListening() {
    if (listeningTimer) clearTimeout(listeningTimer);
    if (!transcription) return;
    const session = transcription;
    transcription = undefined;
    session.close();
    bus.emit("audio_level", { source: "mic", level: 0 });
  }

  function listeningFailed(message: string) {
    stopListening();
    notify(message, "error");
    if (state === "listening") bus.emit("state", { state: "idle" });
  }

  function handleFrame(frame: Int16Array) {
    if (transcription) {
      if (performance.now() < captureAfter) return;
      let energy = 0;
      for (const sample of frame) energy += (sample / 32768) ** 2;
      bus.emit("audio_level", { source: "mic", level: Math.min(1, 4 * Math.sqrt(energy / frame.length)) });
      transcription?.push(frame);
      return;
    }
    if (!detector || performance.now() < captureAfter) return;
    // Keep ~1.3 s of recent audio: follow-up speech is detected a moment after it starts, so the
    // transcription gets this pre-roll first and the user's opening words aren't lost.
    preroll.push(frame.slice());
    if (preroll.length > PREROLL_FRAMES) preroll.shift();
    let kind: number;
    try {
      kind = detector.process(frame);
    } catch {
      detector.release();
      detector = undefined;
      notify("The Jarvis wake word stopped. Toggle it in settings to retry.", "error");
      return;
    }
    if (kind < 0) return;
    bus.emit("wake", { source: kind === 1 ? "followup" : "wakeword" });
  }

  // --- Conversation window: after a reply, listen for a follow-up without the wake word. ---
  const PREROLL_FRAMES = 40; // 40 x 32 ms
  const preroll: Int16Array[] = [];
  let followUntil = 0;
  let followTimer: ReturnType<typeof setTimeout> | undefined;
  let followOn = false;

  /** Detector listens for any speech only while Jarvis is idle inside the window. */
  function syncFollowMode() {
    const want = followUntil > Date.now() && state === "idle" && !transcription;
    if (want === followOn) return;
    followOn = want;
    detector?.setFollowUp(want);
    preroll.length = 0;
  }

  function openFollowUp() {
    const secs = getSettings().conversation?.followUpSeconds ?? 60;
    if (secs <= 0 || !detector || isGameMode()) return; // no open mic while gaming (game audio / team chat)
    followUntil = Date.now() + secs * 1000;
    clearTimeout(followTimer);
    followTimer = setTimeout(closeFollowUp, secs * 1000);
    bus.emit("followup", { active: true, until: followUntil });
    syncFollowMode();
  }

  function closeFollowUp() {
    clearTimeout(followTimer);
    const wasOpen = followUntil > 0;
    followUntil = 0;
    syncFollowMode();
    if (wasOpen) bus.emit("followup", { active: false, until: 0 });
  }

  // Bumped when the chosen microphone changes; the running capture loop exits and a new one opens the new device.
  let micGeneration = 0;
  let inputDevice = getSettings().audio?.inputDevice ?? "";
  let warnedMissingInput = "";

  /** PvRecorder index for the microphone chosen in Settings; -1 = Windows default. */
  function inputIndex(): number {
    if (!inputDevice) return -1;
    const index = PvRecorder.getAvailableDevices().indexOf(inputDevice);
    if (index < 0 && warnedMissingInput !== inputDevice) {
      warnedMissingInput = inputDevice;
      notify(`Microphone "${inputDevice}" isn't connected, so I'm using the default one.`);
    }
    return index;
  }

  async function capture() {
    if (disposed || capturing || micFailed || (!detector && !transcription)) return;
    capturing = true;
    const generation = micGeneration;
    let mic: PvRecorder | undefined;
    try {
      mic = new PvRecorder(detector?.frameLength ?? 512, inputIndex());
      if (mic.sampleRate !== 16000) throw new Error("Unexpected microphone sample rate.");
      mic.start();
      while (!disposed && generation === micGeneration && (detector || transcription)) {
        const frame = await mic.read();
        if (!disposed && generation === micGeneration) handleFrame(frame);
      }
    } catch {
      if (!disposed) {
        micFailed = true;
        listeningFailed(
          `Could not read the ${inputDevice ? `"${inputDevice}"` : "default"} microphone. Check the input device in Settings and Windows microphone access.`,
        );
      }
    } finally {
      if (mic) {
        if (mic.isRecording) mic.stop();
        mic.release();
      }
      capturing = false;
      if (!disposed && generation !== micGeneration) void capture(); // reopen on the newly chosen device
    }
  }

  function updateInputDevice(name: string) {
    if (name === inputDevice) return;
    inputDevice = name;
    warnedMissingInput = "";
    micFailed = false;
    micGeneration++;
    if (!capturing) void capture();
  }

  function updateWakeWord() {
    const { enabled, sensitivity } = getSettings().wakeWord;
    const config = enabled ? String(sensitivity) : "off";
    if (wakeConfig === config) return;
    wakeConfig = config;
    detector?.release();
    detector = undefined;
    micFailed = false;
    if (!enabled) return;
    try {
      detector = createWakeWord(sensitivity);
      followOn = false; // new detector starts in wake-word mode
      syncFollowMode();
      void capture();
    } catch (error) {
      notify(error instanceof Error ? `Wake word unavailable: ${error.message}` : "Could not start the wake word.");
    }
  }

  const unsubscribe = [
    bus.on("state", event => {
      const prev = state;
      state = event.state;
      // A reply just finished (spoken, or text-only when muted): open / extend the conversation window.
      if (event.state === "idle" && (prev === "speaking" || prev === "thinking")) openFollowUp();
      else syncFollowMode();
    }),
    bus.on("settings_changed", event => {
      updateWakeWord();
      updateInputDevice(event.settings.audio?.inputDevice ?? "");
      if (event.settings.voice.muted && state === "speaking") {
        speech.interrupt();
        bus.emit("state", { state: "idle" });
      }
    }),
    bus.on("wake", ({ source }) => {
      if (transcription) return;
      if (state === "speaking" || state === "thinking") {
        bus.emit("interrupt", { reason: source === "wakeword" ? "barge_in" : "new_turn" });
      }
      micFailed = false;
      const followUp = source === "followup";
      const opening = followUp ? preroll.splice(0) : [];
      bus.emit("state", { state: "listening" });
      try {
        // Listening stays open as long as the user keeps talking: the give-up timer restarts on every bit of
        // transcribed speech (it used to be a fixed 20 s, which cut people off mid-sentence). Hard cap 5 minutes.
        const giveUp = () => {
          stopListening();
          // Follow-up triggers can be stray noise; give up quietly and keep the window open.
          if (!followUp && !heardAnything) notify("I didn't catch anything. Say Jarvis or use the talk hotkey to try again.");
          if (state === "listening") bus.emit("state", { state: "idle" });
        };
        let heardAnything = false;
        const armIdle = (ms: number) => {
          if (listeningTimer) clearTimeout(listeningTimer);
          listeningTimer = setTimeout(giveUp, ms);
        };
        const startedAt = Date.now();
        transcription = openTranscription({
          onPartial: text => {
            heardAnything = true;
            if (Date.now() - startedAt < 5 * 60_000) armIdle(10_000); // still talking: keep the mic open
            bus.emit("user_said", { text, final: false });
          },
          onFinal(text) {
            stopListening();
            if (/^(?:jarvis[ ,]+)?(?:stop|cancel)(?:[ ,]+please)?[.!]?$/i.test(text)) {
              bus.emit("interrupt", { reason: "user" });
              if (state === "listening") bus.emit("state", { state: "idle" });
            } else {
              bus.emit("user_said", { text, final: true });
              if (state === "listening") bus.emit("state", { state: "idle" });
            }
          },
          onError: text => listeningFailed(text),
          silenceSecs: getSettings().conversation?.endOfSpeechSeconds ?? 1.2,
        });
        // Soft chime on the chosen speaker (shell.beep() played the Windows error sound).
        // Skip ~380 ms of mic audio so the chime itself isn't transcribed.
        if (source === "wakeword" && !isGameMode()) void speech.chime().catch(() => {}); // quiet in game mode
        captureAfter = performance.now() + (source === "wakeword" ? 380 : 0);
        for (const frame of opening) transcription.push(frame); // follow-up: send the opening words too
        armIdle(followUp ? 10_000 : 15_000); // time to start talking
        void capture();
      } catch (error) {
        listeningFailed(error instanceof Error ? error.message : "Could not start listening.");
      }
    }),
    bus.on("user_text", () => {
      stopListening();
      if (speech.pending) speech.interrupt();
    }),
    bus.on("speak", ({ text }) => speech.enqueue(text)),
    bus.on("speak_end", () => speech.end()),
    bus.on("timer_done", () => void speech.chime("alarm").catch(() => {})), // alarms ring even in game mode
    bus.on("game_mode", ({ active }) => {
      if (active) closeFollowUp();
    }),
    bus.on("interrupt", ({ reason }) => {
      stopListening();
      speech.interrupt();
      if (reason === "user") closeFollowUp(); // "never mind", "stop", Esc: end the conversation
      if (state === "listening" || state === "speaking") bus.emit("state", { state: "idle" });
    }),
  ];
  updateWakeWord();

  return () => {
    if (disposed) return;
    disposed = true;
    for (const off of unsubscribe) off();
    clearTimeout(followTimer);
    stopListening();
    detector?.release();
    speech.interrupt();
    if (output === speech) output = undefined;
  };
}
