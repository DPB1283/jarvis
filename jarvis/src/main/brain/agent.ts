import ultron from "../../shared/ultron.json";
import Anthropic from "@anthropic-ai/sdk";
import { bus } from "../bus";
import { getSettings, MODEL } from "../config";
import { listMemory, listRoutines, logAction } from "../memory/db";
import { describeSchedule } from "../tools/routines";
import { isGameMode } from "../gamemode";
import type { Routine } from "../../shared/settings";
import { approvalPending, authorize } from "../safety/policy";
import { apiToolDefs, getTool, riskOf } from "../tools/registry";
import { autoSelectScreen, COMPUTER_TOOLSET, describeComputerAction, runComputerAction, screens } from "../tools/computer";
import { SYSTEM_PROMPT } from "./persona";

const SESSION_IDLE_MS = 10 * 60_000;
const SESSION_MAX_INPUT_TOKENS = 150_000;
const MAX_STEPS = 60;

type Block = Anthropic.ContentBlock & { toolset_name?: string };
type ResultParam = Anthropic.ToolResultBlockParam & { toolset_name?: string };

/** Web tools: try the dynamic-filtering versions first, fall back to the basic ones if the model rejects them. */
const WEB_TOOL_VARIANTS: Anthropic.ToolUnion[][] = [
  // Kept small: web results dominate context (and latency) in a voice assistant.
  [
    { type: "web_search_20260209", name: "web_search", max_uses: 5 } as unknown as Anthropic.ToolUnion,
    { type: "web_fetch_20260209", name: "web_fetch", max_uses: 2, max_content_tokens: 8000 } as unknown as Anthropic.ToolUnion,
  ],
  [
    { type: "web_search_20250305", name: "web_search", max_uses: 5 } as unknown as Anthropic.ToolUnion,
    { type: "web_fetch_20250910", name: "web_fetch", max_uses: 2, max_content_tokens: 8000 } as unknown as Anthropic.ToolUnion,
  ],
  [],
];

/** Splits streamed text into speakable sentences. */
class SentenceChunker {
  private buf = "";
  constructor(private emit: (s: string) => void) {}
  push(delta: string) {
    this.buf += delta;
    const re = /^([\s\S]*?[.!?…:;](?=\s)|[\s\S]*?\n)/;
    let m: RegExpMatchArray | null;
    while ((m = this.buf.match(re)) && m[1].trim().length >= 2) {
      this.emit(m[1].trim());
      this.buf = this.buf.slice(m[1].length);
    }
  }
  flush() {
    if (this.buf.trim()) this.emit(this.buf.trim());
    this.buf = "";
  }
}

export class Brain {
  private client = new Anthropic();
  private messages: Anthropic.MessageParam[] = [];
  private lastActivity = 0;
  private lastInputTokens = 0;
  private webVariant = 0;
  private abort: AbortController | null = null;
  private running: Promise<void> | null = null;
  /** Called with each reply sentence. Default: bus "speak". */
  onSentence: (s: string) => void = (text) => bus.emit("speak", { text });
  /** Set by main to voice's speechPending(), so we don't flip the orb to idle while audio is still queued. */
  isSpeaking: () => boolean = () => false;

  constructor() {
    bus.on("interrupt", ({ reason }) => {
      if (reason === "barge_in" && approvalPending()) return; // "Jarvis, yes" while a question is open
      this.abort?.abort();
    });
  }

  get busy() {
    return this.running !== null;
  }

  /** Start a turn. If one is running, it is interrupted first. */
  async handle(text: string): Promise<void> {
    if (this.running) {
      bus.emit("interrupt", { reason: "new_turn" });
      await this.running.catch(() => {});
    }
    this.running = this.turn(text).finally(() => (this.running = null));
    return this.running;
  }

  /** Run a routine now: on request ("Run now" button) or because its schedule fired. */
  runRoutine(routine: Routine, scheduled: boolean): Promise<void> {
    return this.handle(
      scheduled
        ? `(Scheduled routine "${routine.name}" is due now. The user didn't just speak, so open with a brief greeting, then do it.)\n${routine.instructions}`
        : `Run my "${routine.name}" routine: ${routine.instructions}`,
    );
  }

  resetSession() {
    this.messages = [];
    this.lastInputTokens = 0;
  }

  private contextBlock(newSession: boolean): string {
    const s = getSettings();
    const lines = [
      `<context>`,
      `time: ${new Date().toLocaleString("en-GB", { dateStyle: "full", timeStyle: "short" })}`,
      `user_title: ${s.userTitle}`,
      ...(s.userName?.trim() ? [`user_name: ${s.userName.trim()}`] : []),
      `sarcasm_level: ${s.personality.sarcasm}/10`,
      `reply_length: ${s.personality.replyLength}`,
      `safety_mode: ${s.safetyMode}`,
      this.screensLine(),
      ...(isGameMode() ? ["game_mode: ON (user is gaming; be brief)"] : []),
      `home_city: ${s.location?.city?.trim() || "(not set)"}`,
      `units: ${s.location?.units ?? "metric"}`,
      `news_region: ${s.location?.newsRegion ?? "US"}`,
    ];
    const routines = listRoutines().filter((r) => r.enabled);
    if (routines.length) {
      lines.push(
        `routines:\n${routines
          .map((r) => `- #${r.id} "${r.name}" (e.g. ${r.triggers.map((t) => `"${t}"`).join(", ") || "-"}; ${describeSchedule(r.schedule)}): ${r.instructions}`)
          .join("\n")}`,
      );
    }
    if (newSession) {
      const facts = listMemory({ kind: "fact" });
      const todos = listMemory({ kind: "todo" }).filter((t) => !t.done);
      if (facts.length) lines.push(`remembered_facts:\n${facts.map((f) => `- #${f.id} ${f.text}`).join("\n")}`);
      if (todos.length) lines.push(`open_todos: ${todos.length}`);
    }
    lines.push(`</context>`);
    return lines.join("\n");
  }

  /** Monitors, and which one computer actions target (auto-selected each turn: where the user is working). */
  private screensLine(): string {
    try {
      const active = autoSelectScreen();
      const all = screens();
      if (all.length < 2) return `screens: 1 (${active.width}x${active.height})`;
      return `screens: ${all.length} (${all.map((m) => `${m.index}${m.primary ? " main" : ""} ${m.width}x${m.height}`).join(", ")}); computer actions target screen ${active.index}, where the user's active window is; use look_at_screens / use_screen for the others`;
    } catch {
      return "screens: unknown";
    }
  }

  private tools(): Anthropic.ToolUnion[] {
    return [
      ...apiToolDefs(),
      COMPUTER_TOOLSET as unknown as Anthropic.ToolUnion,
      ...WEB_TOOL_VARIANTS[this.webVariant],
    ];
  }

  private async turn(text: string): Promise<void> {
    const now = Date.now();
    if (now - this.lastActivity > SESSION_IDLE_MS || this.lastInputTokens > SESSION_MAX_INPUT_TOKENS) {
      this.resetSession();
    }
    this.lastActivity = now;
    const newSession = this.messages.length === 0;
    const userText = `${this.contextBlock(newSession)}\n\n${text}`;
    const prev = this.messages.at(-1);
    if (prev?.role === "user" && typeof prev.content === "string") {
      // The previous turn was interrupted before Claude replied; fold the new request into it.
      prev.content += `\n\n(The user interrupted and said instead:)\n${userText}`;
    } else {
      this.messages.push({ role: "user", content: userText });
    }

    const abort = new AbortController();
    this.abort = abort;
    bus.emit("state", { state: "thinking" });
    let fullReply = "";

    try {
      for (let step = 0; step < MAX_STEPS; step++) {
        const chunker = new SentenceChunker((s) => this.onSentence(s));
        let message: Anthropic.Message;
        try {
          const stream = this.client.messages.stream(
            {
              model: MODEL,
              max_tokens: 16_000,
              thinking: { type: "adaptive" },
              output_config: { effort: "low" },
              system: [{ type: "text", text: getSettings().ultronMode ? SYSTEM_PROMPT + "\n" + ultron.prompt : SYSTEM_PROMPT, cache_control: { type: "ephemeral" } }],
              tools: this.tools(),
              messages: this.messages,
            } as Anthropic.MessageStreamParams,
            { signal: abort.signal },
          );
          stream.on("text", (delta) => {
            fullReply += delta;
            bus.emit("assistant_delta", { text: delta });
            chunker.push(delta);
          });
          message = await stream.finalMessage();
        } catch (err) {
          if (abort.signal.aborted) return;
          if (err instanceof Anthropic.BadRequestError && /web_(search|fetch)/.test(err.message) && this.webVariant < 2) {
            this.webVariant++;
            step--;
            continue;
          }
          throw err;
        }
        chunker.flush();
        this.lastInputTokens =
          message.usage.input_tokens +
          (message.usage.cache_read_input_tokens ?? 0) +
          (message.usage.cache_creation_input_tokens ?? 0);

        this.messages.push({ role: "assistant", content: message.content as Anthropic.ContentBlockParam[] });

        if (message.stop_reason === "refusal") {
          this.say("I'm afraid I can't help with that one.");
          return;
        }
        if (message.stop_reason === "pause_turn") continue;
        const toolUses = (message.content as Block[]).filter(
          (b): b is Anthropic.ToolUseBlock & { toolset_name?: string } => b.type === "tool_use",
        );
        if (message.stop_reason === "max_tokens") {
          // A tool call may be cut off mid-input: never run it, but every tool_use needs a result.
          const cut: ResultParam[] = toolUses.map((b) => ({
            type: "tool_result",
            tool_use_id: b.id,
            ...(b.toolset_name === "computer" && { toolset_name: "computer" }),
            is_error: true,
            content: "Not executed: your response hit the length limit. Retry with a shorter input.",
          }));
          this.messages.push({
            role: "user",
            content: cut.length ? cut : "Your previous response was cut off. Continue briefly from where you stopped.",
          });
          continue;
        }
        if (message.stop_reason !== "tool_use") return;

        const results = await this.runTools(toolUses, abort.signal);
        this.messages.push({ role: "user", content: results });
        if (abort.signal.aborted) return;
        bus.emit("state", { state: "thinking" });
      }
      this.say("That's taking rather more steps than it should. I've stopped for now.");
    } catch (err) {
      if (abort.signal.aborted) return;
      const msg = err instanceof Anthropic.APIError ? `${err.status ?? ""} ${err.message}` : String(err);
      console.error("[brain]", msg);
      bus.emit("toast", { level: "error", text: msg.slice(0, 300) });
      this.say(
        err instanceof Anthropic.AuthenticationError
          ? "My API key isn't working, I'm afraid. Do check the settings."
          : err instanceof Anthropic.RateLimitError
            ? "I've been rate limited. Give me a moment."
            : "Something went wrong on my end. Apologies.",
      );
    } finally {
      bus.emit("assistant_done", { text: fullReply });
      bus.emit("speak_end", {});
      if (this.abort === abort) this.abort = null;
      if (!this.isSpeaking()) bus.emit("state", { state: "idle" });
    }
  }

  private say(text: string) {
    bus.emit("assistant_delta", { text });
    this.onSentence(text);
  }

  /** Executes every tool_use block in order and returns one result per block. */
  private async runTools(
    blocks: (Anthropic.ToolUseBlock & { toolset_name?: string })[],
    signal: AbortSignal,
  ): Promise<ResultParam[]> {
    const results: ResultParam[] = [];
    let computerFailed = false;

    for (const b of blocks) {
      const isComputer = b.toolset_name === "computer";
      const base = { type: "tool_result" as const, tool_use_id: b.id, ...(isComputer && { toolset_name: "computer" }) };

      if (signal.aborted) {
        results.push({ ...base, is_error: true, content: "Cancelled: the user interrupted." });
        continue;
      }
      if (isComputer && computerFailed) {
        results.push({ ...base, is_error: true, content: "Not executed: an earlier computer action in this turn failed." });
        continue;
      }

      try {
        if (isComputer) {
          const input = b.input as Parameters<typeof runComputerAction>[1];
          const summary = describeComputerAction(b.name, input);
          if (b.name !== "screenshot" && b.name !== "zoom" && b.name !== "cursor_position" && !(await authorize("computer", summary, "safe"))) {
            results.push({ ...base, is_error: true, content: "Declined by the user." });
            computerFailed = true;
            continue;
          }
          const content = await runComputerAction(b.name, input);
          if (b.name !== "screenshot" && b.name !== "zoom") logAction(`computer.${b.name}`, input, "ran");
          results.push({ ...base, content });
          continue;
        }

        const tool = getTool(b.name);
        if (!tool) {
          results.push({ ...base, is_error: true, content: `Unknown tool ${b.name}` });
          continue;
        }
        const parsed = tool.schema.safeParse(b.input);
        if (!parsed.success) {
          results.push({ ...base, is_error: true, content: `INVALID_INPUT: ${parsed.error.message}` });
          continue;
        }
        const input = parsed.data;
        const risk = riskOf(tool, input);
        const ok = await authorize(tool.name, tool.summary(input), risk);
        if (!ok) {
          logAction(tool.name, input, "declined");
          results.push({ ...base, is_error: true, content: "Declined by the user." });
          continue;
        }
        const out = await tool.run(input);
        logAction(tool.name, input, "ran", typeof out === "string" ? out : "[content]");
        results.push({ ...base, content: out });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (isComputer) computerFailed = true;
        logAction(isComputer ? `computer.${b.name}` : b.name, b.input, "error", msg);
        results.push({ ...base, is_error: true, content: `Error: ${msg}` });
      }
    }
    return results;
  }
}
