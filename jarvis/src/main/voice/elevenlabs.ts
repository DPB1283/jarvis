import { ElevenLabsClient } from "@elevenlabs/elevenlabs-js";

export function elevenLabs() {
  const apiKey = process.env.ELEVENLABS_API_KEY;
  if (!apiKey) throw new Error("Set ELEVENLABS_API_KEY to use speech.");
  return new ElevenLabsClient({ apiKey });
}
