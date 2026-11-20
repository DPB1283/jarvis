// Recognises "go away" phrases so Jarvis stops silently: no reply, no API call.
//
// Two lists, to avoid swallowing real requests:
// - END phrases count when they finish the utterance ("30. Never mind.", "Thanks Jarvis, that's all").
// - WHOLE phrases count only when they are (nearly) all that was said, so "make the music quiet"
//   or "don't stop the timer" still go through.

const END_PHRASES = [
  "never mind", "nevermind", "nvm", "never mind then",
  "forget it", "forget that", "forget about it",
  "ignore me", "ignore that", "scratch that", "disregard", "disregard that", "cancel", "cancel that",
  "that's all", "that is all", "that's it", "that'll be all", "that will be all", "that's everything",
  "nothing else", "i'm good", "i am good", "i'm fine", "all good", "no thanks", "no thank you",
  "not now", "not right now", "false alarm", "wrong person", "i wasn't talking to you", "not talking to you",
  "go back to sleep", "go to sleep", "as you were", "carry on", "dismissed", "you're dismissed", "stand down",
  "don't worry about it", "dont worry about it", "don't worry", "no worries", "it's fine", "it's okay", "never you mind",
];

const WHOLE_PHRASES = [
  "stop", "stop talking", "stop it", "quiet", "be quiet", "silence", "shush", "hush", "shh", "shut up",
  "nothing", "no", "nah", "nope", "thanks", "thank you", "cheers", "ta",
  "ok", "okay", "cool", "great", "perfect", "nice", "got it", "understood", "noted", "alright", "all right",
  "bye", "goodbye", "bye bye", "good night", "goodnight", "night", "see you", "see ya", "later", "see you later",
  "go away", "leave me alone", "never mind thanks",
];

const LEADING_FILLERS = ["oh", "ah", "um", "umm", "uh", "uhh", "er", "actually", "well", "okay", "ok", "alright", "right", "jarvis", "hey", "so"];
const TRAILING_FILLERS = ["jarvis", "then", "sorry", "please", "thanks", "thank you", "mate", "buddy", "bud", "man", "for now", "now"];

export function normalise(text: string): string {
  return text
    .toLowerCase()
    .replace(/[’‘`]/g, "'")
    .replace(/[([][^)\]]*[)\]]/g, " ") // drop "(laughs)" style tags
    .replace(/[^a-z0-9' ]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function stripFillers(s: string): string {
  let out = s;
  for (let changed = true; changed; ) {
    changed = false;
    for (const f of LEADING_FILLERS) {
      if (out.startsWith(f + " ")) {
        out = out.slice(f.length + 1);
        changed = true;
      }
    }
    for (const f of TRAILING_FILLERS) {
      if (out.endsWith(" " + f)) {
        out = out.slice(0, -(f.length + 1));
        changed = true;
      }
    }
  }
  return out.trim();
}

const WHOLE = new Set(WHOLE_PHRASES.map(normalise));
const END = END_PHRASES.map(normalise).sort((a, b) => b.length - a.length);

export function isDismissal(text: string): boolean {
  const n = normalise(text);
  if (!n) return false;
  if (WHOLE.has(n)) return true;
  const core = stripFillers(n);
  if (WHOLE.has(core)) return true;
  const endsWith = (s: string) => END.some((p) => s === p || s.endsWith(" " + p));
  return endsWith(n) || endsWith(core);
}
