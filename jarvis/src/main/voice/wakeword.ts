// Offline "Jarvis" wake word using Vosk with a tiny grammar (no account or API key needed).
// Vosk runs in a worker thread so speech decoding never blocks Electron's main thread.
// Same shape as the Porcupine object it replaced: { frameLength, process(frame) -> index | -1, release() }.
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { Worker } from "node:worker_threads";
import { ROOT } from "../config";

export const VOSK_MODEL_DIR = path.join(ROOT, "models", "vosk-model-small-en-us-0.15");
const COOLDOWN_MS = 1500;

const WORKER_SOURCE = `
const { parentPort, workerData } = require("node:worker_threads");
const mod = require(workerData.voskPath);
const vosk = mod.default || mod;
vosk.setLogLevel(-1);
const model = new vosk.Model(workerData.modelDir);
// Free-form recognition (no grammar). A "jarvis"-or-[unk] grammar forces every word into one of two bins,
// so "Travis", "harvest" and "nervous" became "jarvis" (11/20 false wakes in testing; free-form: 0/20).
// A fresh recognizer instead of rec.reset(): Vosk's reset() keeps half-decoded audio, which re-fired "jarvis".
const makeRec = () => {
  const r = new vosk.Recognizer({ model, sampleRate: 16000 });
  r.setWords(true);
  return r;
};
let rec = makeRec();
let streak = 0;
// Follow-up mode (conversation window after a reply): any recognized speech counts, not just "jarvis".
let follow = false;
let loud = 0; // loud frames in the current utterance (reset after ~0.5 s of quiet with no words)
let quiet = 0;
const fresh = () => { rec.free(); rec = makeRec(); streak = 0; loud = 0; quiet = 0; };
const SPEECH_RMS = 0.004;
const sens = workerData.sensitivity;
// Higher sensitivity = wake sooner. >= 0.7: first partial match; 0.3-0.7: two consecutive partials; < 0.3: end of phrase.
const needStreak = sens >= 0.7 ? 1 : sens >= 0.3 ? 2 : Infinity;
const WORD = /\\bjarvis\\b/;
let gen = 0;
parentPort.postMessage({ type: "ready" });
parentPort.on("message", (msg) => {
  if (msg.type === "reset") {
    if (msg.gen !== gen) { gen = msg.gen; fresh(); }
  } else if (msg.type === "mode") {
    follow = !!msg.followUp;
    fresh();
  } else if (msg.type === "frame") {
    if (msg.gen !== gen) return; // audio from before a reset
    const samples = new Int16Array(msg.buf);
    let energy = 0;
    for (let i = 0; i < samples.length; i++) energy += (samples[i] / 32768) ** 2;
    const isLoud = Math.sqrt(energy / samples.length) > SPEECH_RMS;
    if (isLoud) { loud++; quiet = 0; } else quiet++;
    let hit = false;
    let speech = false;
    if (rec.acceptWaveform(Buffer.from(msg.buf))) {
      const r = rec.result();
      streak = 0;
      hit = !!(r && r.result && r.result.some((w) => w.word === "jarvis" && w.conf >= 0.6));
      // End of a phrase: catches short replies ("yes", "no") that never build a long partial.
      speech = follow && !!(r && r.text && r.text.trim()) && loud >= 2;
      loud = 0;
    } else {
      const p = rec.partialResult();
      const partial = ((p && p.partial) || "").trim();
      streak = WORD.test(partial) ? streak + 1 : 0;
      hit = streak >= needStreak;
      if (!partial && quiet >= 15) loud = 0;
      speech = follow && !!partial && loud >= 3;
    }
    if (hit || speech) { fresh(); parentPort.postMessage({ type: hit && !follow ? "wake" : "speech", gen }); }
  } else if (msg.type === "close") {
    rec.free(); model.free(); parentPort.close();
  }
});
`;

const voskPath = (() => {
  try {
    return createRequire(path.join(ROOT, "package.json")).resolve("vosk-koffi");
  } catch {
    return "vosk-koffi";
  }
})();

/**
 * sensitivity 0..1: at >= 0.5 Jarvis wakes on Vosk's partial result (fastest);
 * below that it waits for the end of the phrase and requires word confidence >= 1 - sensitivity.
 */
export function createWakeWord(sensitivity: number) {
  if (!fs.existsSync(VOSK_MODEL_DIR)) {
    throw new Error(`Vosk model missing at ${VOSK_MODEL_DIR}. Download vosk-model-small-en-us-0.15 into jarvis/models/.`);
  }
  const worker = new Worker(WORKER_SOURCE, {
    eval: true,
    workerData: { voskPath, modelDir: VOSK_MODEL_DIR, sensitivity },
  });
  let ready = false;
  let failed: Error | null = null;
  let pendingWake = false;
  let lastWake = 0;
  let released = false;
  // Each pause in feeding (e.g. while the user's request is being transcribed) starts a new generation:
  // the recognizer is reset and any detection from older audio is discarded. Without this, the tail of
  // the first "Jarvis" fired a second wake right after transcription and cancelled the reply.
  let gen = 0;
  let lastFeed = 0;
  const PAUSE_MS = 250;

  /** 0 = wake word, 1 = speech during the follow-up window. */
  let pendingKind: 0 | 1 = 0;
  worker.on("message", (m: { type: string; gen?: number }) => {
    if (m.type === "ready") ready = true;
    else if ((m.type === "wake" || m.type === "speech") && m.gen === gen) {
      pendingWake = true;
      pendingKind = m.type === "speech" ? 1 : 0;
    }
  });
  worker.on("error", (err) => {
    failed = err instanceof Error ? err : new Error(String(err));
    console.error("[wakeword]", failed.message);
  });
  worker.unref();

  return {
    frameLength: 512,
    sampleRate: 16000,
    /**
     * Follow-up (conversation) mode: any speech triggers, not just "Jarvis". Only enable while Jarvis is
     * silent, or it will hear its own voice. Switching resets the recognizer.
     */
    setFollowUp(on: boolean) {
      if (released) return;
      gen++;
      pendingWake = false;
      worker.postMessage({ type: "reset", gen });
      worker.postMessage({ type: "mode", followUp: on });
    },
    /** Queues the frame for the worker; returns 0 for a wake word, 1 for follow-up speech, -1 for nothing. */
    process(frame: Int16Array): number {
      if (released) return -1;
      if (failed) throw failed;
      const now = Date.now();
      if (now - lastFeed > PAUSE_MS) {
        gen++;
        pendingWake = false;
        worker.postMessage({ type: "reset", gen });
      }
      lastFeed = now;
      if (ready) {
        const copy = frame.slice(); // own buffer, transferable
        worker.postMessage({ type: "frame", gen, buf: copy.buffer }, [copy.buffer]);
      }
      if (!pendingWake) return -1;
      pendingWake = false;
      if (now - lastWake < COOLDOWN_MS) return -1;
      lastWake = now;
      // The detector stops being fed while the request is transcribed; start fresh when it resumes.
      gen++;
      worker.postMessage({ type: "reset", gen });
      return pendingKind;
    },
    release() {
      if (released) return;
      released = true;
      worker.postMessage({ type: "close" });
      setTimeout(() => void worker.terminate(), 500);
    },
  };
}
