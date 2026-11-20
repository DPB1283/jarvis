import { AudioFormat, CommitStrategy, RealtimeEvents, type RealtimeConnection } from "@elevenlabs/elevenlabs-js";
import { elevenLabs } from "./elevenlabs";

const sampleRate = 16000;
// ~5 s of 32 ms frames: covers the follow-up pre-roll (~1.3 s) plus connection time.
const maxQueuedFrames = 160;

export function openTranscription(handlers: {
  onPartial: (text: string) => void;
  onFinal: (text: string) => void;
  onError: (message: string) => void;
  /** Pause length (s) that ends the utterance. From settings.conversation.endOfSpeechSeconds. */
  silenceSecs?: number;
}) {
  const client = elevenLabs();
  let connection: RealtimeConnection | undefined;
  let closed = false;
  let ready = false;
  const queued: string[] = [];

  function close() {
    if (closed) return;
    closed = true;
    queued.length = 0;
    connection?.close();
  }

  function fail(message: string) {
    if (closed) return;
    close();
    handlers.onError(message);
  }

  function send(audioBase64: string) {
    try {
      connection?.send({ audioBase64, sampleRate });
    } catch {
      fail("Could not stream microphone audio to ElevenLabs.");
    }
  }

  void client.speechToText.realtime.connect({
    modelId: "scribe_v3_live",
    audioFormat: AudioFormat.PCM_16000,
    sampleRate,
    commitStrategy: CommitStrategy.VAD,
    vadSilenceThresholdSecs: Math.min(3, Math.max(0.3, handlers.silenceSecs ?? 1.2)),
  }).then(session => {
    session.on(RealtimeEvents.ERROR, () => fail("ElevenLabs transcription failed. Check the API key and account."));
    if (closed) {
      session.close();
      return;
    }
    connection = session;
    session.on(RealtimeEvents.CLOSE, () => fail("ElevenLabs closed the microphone session."));
    session.on(RealtimeEvents.SESSION_STARTED, () => {
      if (closed) return;
      ready = true;
      for (const audio of queued.splice(0)) {
        if (closed) break;
        send(audio);
      }
    });
    session.on(RealtimeEvents.PARTIAL_TRANSCRIPT, ({ text }) => {
      if (!closed && text.trim()) handlers.onPartial(text);
    });
    function commit({ text }: { text: string }) {
      if (closed || !text.trim()) return;
      close();
      handlers.onFinal(text.trim());
    }
    session.on(RealtimeEvents.COMMITTED_TRANSCRIPT, commit);
  }).catch(() => fail("Could not connect to ElevenLabs transcription."));

  return {
    close,
    push(frame: Int16Array) {
      if (closed) return;
      const audio = Buffer.from(frame.buffer, frame.byteOffset, frame.byteLength).toString("base64");
      if (ready) {
        send(audio);
      } else if (queued.length >= maxQueuedFrames) {
        fail("ElevenLabs took too long to start listening. Please try again.");
      } else {
        queued.push(audio);
      }
    },
  };
}
