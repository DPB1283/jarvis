// Phase 1: talk to Jarvis's brain in the terminal (no Electron, no voice).
//   npm run cli
import readline from "node:readline/promises";
import { stdin, stdout } from "node:process";
import { bus } from "./bus";
import { getSettings } from "./config";
import { Brain } from "./brain/agent";
import { setApprover } from "./safety/policy";
import { startReminderScheduler } from "./memory/reminders";

const rl = readline.createInterface({ input: stdin, output: stdout });
const AQUA = "\x1b[38;2;63;224;208m";
const DIM = "\x1b[2m";
const RESET = "\x1b[0m";

setApprover(async ({ summary, risk }) => {
  const a = await rl.question(`\n${risk === "unsafe" ? "\x1b[33m⚠" : "?"} Jarvis wants to: ${summary}. Allow? (y/n) ${RESET}`);
  return /^y(es)?$/i.test(a.trim());
});

bus.on("assistant_delta", ({ text }) => stdout.write(AQUA + text + RESET));
bus.on("toast", ({ level, text }) => stdout.write(`\n${DIM}[${level}] ${text}${RESET}\n`));
bus.on("reminder_due", ({ text }) => stdout.write(`\n${AQUA}⏰ Reminder: ${text}${RESET}\n`));

startReminderScheduler();
const brain = new Brain();
brain.onSentence = () => {}; // text only in the CLI

async function main() {
  console.log(`${AQUA}J.A.R.V.I.S.${RESET} ${DIM}(safety: ${getSettings().safetyMode}; type "exit" to quit, "reset" for a new session)${RESET}`);
  for (;;) {
    const line = (await rl.question(`\n${DIM}you ›${RESET} `)).trim();
    if (!line) continue;
    if (line === "exit") break;
    if (line === "reset") {
      brain.resetSession();
      continue;
    }
    stdout.write(`${AQUA}jarvis ›${RESET} `);
    await brain.handle(line);
    stdout.write("\n");
  }
  rl.close();
  process.exit(0);
}

void main();
