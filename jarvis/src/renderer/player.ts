// Hidden audio player page (index.html#player), driven by main/voice/playback.ts via executeJavaScript.
// It lives on the same origin as the settings page so speaker deviceIds match the ones the user picked.

interface SinkResult {
  ok: boolean;
  /** true when a specific device was requested but couldn't be found (fell back to default). */
  missing?: boolean;
  error?: string;
}

type SinkContext = AudioContext & { setSinkId?: (id: string) => Promise<void> };

export function mountPlayer() {
  const sampleRate = 24000;
  let context: SinkContext | undefined;
  let next = 0;
  let sinkKey: string | undefined;

  async function findDevice(deviceId: string, label: string): Promise<string | null> {
    const outputs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audiooutput");
    return (outputs.find((d) => d.deviceId === deviceId) ?? outputs.find((d) => d.label === label))?.deviceId ?? null;
  }

  Object.assign(globalThis, {
    jarvisPlayer: {
      async setSink(deviceId: string, label: string): Promise<SinkResult> {
        const key = `${deviceId}|${label}`;
        context ??= new AudioContext() as SinkContext;
        if (key === sinkKey) return { ok: true };
        sinkKey = key;
        if (!context.setSinkId) return { ok: false, error: "This build can't choose speakers." };
        if (!deviceId && !label) {
          await context.setSinkId("");
          return { ok: true };
        }
        const id = await findDevice(deviceId, label);
        try {
          await context.setSinkId(id ?? "");
          return id ? { ok: true } : { ok: false, missing: true };
        } catch (error) {
          await context.setSinkId("").catch(() => {});
          return { ok: false, error: String(error) };
        }
      },

      /** Soft two-note chime, synthesized (no sound files): rising = listening, falling = dismissed. */
      async chime(kind: "listen" | "dismiss" | "alarm" = "listen") {
        context ??= new AudioContext() as SinkContext;
        await context.resume();
        const t0 = context.currentTime + 0.01;
        const alarm: number[][] = [];
        for (let round = 0; round < 3; round++) for (let k = 0; k < 3; k++) alarm.push([[1046.5, 1318.5, 1568][k], round * 0.7 + k * 0.12]);
        const notes = kind === "alarm" ? alarm : kind === "dismiss" ? [[1174.7, 0], [784, 0.09]] : [[880, 0], [1318.5, 0.09]];
        for (const [freq, at] of notes) {
          const osc = context.createOscillator();
          const gain = context.createGain();
          osc.type = "sine";
          osc.frequency.value = freq;
          gain.gain.setValueAtTime(0, t0 + at);
          gain.gain.linearRampToValueAtTime(0.12, t0 + at + 0.012);
          gain.gain.exponentialRampToValueAtTime(0.0001, t0 + at + 0.22);
          osc.connect(gain).connect(context.destination);
          osc.onended = () => { osc.disconnect(); gain.disconnect(); };
          osc.start(t0 + at);
          osc.stop(t0 + at + 0.24);
        }
      },

      async push(encoded: string, volume = 100) {
        context ??= new AudioContext() as SinkContext;
        await context.resume();
        const bytes = Uint8Array.from(atob(encoded), (char) => char.charCodeAt(0));
        const pcm = new DataView(bytes.buffer);
        const audio = context.createBuffer(1, bytes.length / 2, sampleRate);
        const samples = audio.getChannelData(0);
        let energy = 0;
        for (let i = 0; i < samples.length; i++) {
          const sample = Math.max(-1, Math.min(1, pcm.getInt16(i * 2, true) / 32768 * Math.max(0, Math.min(200, volume)) / 100));
          samples[i] = sample;
          energy += sample * sample;
        }
        const source = context.createBufferSource();
        source.buffer = audio;
        source.connect(context.destination);
        const start = Math.max(next, context.currentTime + 0.012);
        next = start + audio.duration;
        source.onended = () => source.disconnect();
        source.start(start);
        return {
          lead: (start - context.currentTime) * 1000,
          tail: (next - context.currentTime) * 1000,
          level: Math.min(1, Math.sqrt(energy / samples.length) * 3),
        };
      },
    },
  });
}
