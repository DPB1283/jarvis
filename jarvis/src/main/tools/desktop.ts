import { ensureForeground, foregroundPid, foregroundWindow, focusWindow } from "./window-paste";
export { foregroundPid, foregroundWindow, pasteRich } from "./window-paste";
// Typing into apps and sending chat messages in any messaging app.
// Text is pasted via the clipboard (fast, handles emoji/accents/newlines; simulated typing doesn't),
// and the user's clipboard is restored afterwards.
import { existsSync } from "node:fs";
import { Key, keyboard } from "@nut-tree-fork/nut-js";
import { z } from "zod";
import type Anthropic from "@anthropic-ai/sdk";
import { defineTool } from "./types";
import { ps, psq, runPs, WIN_NATIVE } from "./ps";
import { parseCombo, runComputerAction } from "./computer";
import { allChatApps, findChatApp, type ChatAppProfile } from "./chat-apps";
import { openApp } from "./system";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function chord(...keys: Key[]) {
  await keyboard.pressKey(...keys);
  await keyboard.releaseKey(...keys);
}

/** Presses xdotool-style combos in order, e.g. ["Escape", "ctrl+k"], only while `pid` is the active app. */
async function pressAll(pid: number, combos: string[] | undefined, gapMs = 120) {
  if (!combos?.length) return;
  await ensureForeground(pid);
  for (const c of combos) {
    await chord(...parseCombo(c));
    await sleep(gapMs);
  }
}

/**
 * Paste text into whatever has keyboard focus, then put the user's clipboard back exactly as it was
 * (every format: text, images, files, rich text), or empty if it was empty. One PowerShell process does
 * save → set → Ctrl+V → restore, so nothing else can grab the clipboard in between.
 */

export async function pasteText(text: string, expectPid?: number): Promise<void> {
  // When we know which app we mean to type into, refuse to paste anywhere else.
  if (expectPid !== undefined) await ensureForeground(expectPid);
  const r = await runPs(
    `Add-Type -AssemblyName System.Windows.Forms
    $saved = New-Object System.Windows.Forms.DataObject
    $hadData = $false
    $old = [System.Windows.Forms.Clipboard]::GetDataObject()
    if ($old) { foreach ($f in $old.GetFormats($false)) { try { $d = $old.GetData($f); if ($null -ne $d) { $saved.SetData($f, $d); $hadData = $true } } catch {} } }
    [System.Windows.Forms.Clipboard]::SetText(${psq(text)})
    Start-Sleep -Milliseconds 40
    [System.Windows.Forms.SendKeys]::SendWait("^v")
    Start-Sleep -Milliseconds 300
    if ($hadData) { [System.Windows.Forms.Clipboard]::SetDataObject($saved, $true) } else { [System.Windows.Forms.Clipboard]::Clear() }`,
  );
  if (r.code !== 0) throw new Error(`Couldn't paste: ${r.stderr || "clipboard busy"}`);
}




/** Paste an image file (e.g. a screenshot) into the focused app, restoring the clipboard afterwards. */
export async function pasteImage(file: string, expectPid?: number): Promise<void> {
  if (expectPid !== undefined) await ensureForeground(expectPid);
  const r = await runPs(
    `Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $saved = New-Object System.Windows.Forms.DataObject
    $hadData = $false
    $old = [System.Windows.Forms.Clipboard]::GetDataObject()
    if ($old) { foreach ($f in $old.GetFormats($false)) { try { $d = $old.GetData($f); if ($null -ne $d) { $saved.SetData($f, $d); $hadData = $true } } catch {} } }
    $img = [System.Drawing.Image]::FromFile(${psq(file)})
    [System.Windows.Forms.Clipboard]::SetImage($img)
    Start-Sleep -Milliseconds 60
    [System.Windows.Forms.SendKeys]::SendWait("^v")
    Start-Sleep -Milliseconds 900
    $img.Dispose()
    if ($hadData) { [System.Windows.Forms.Clipboard]::SetDataObject($saved, $true) } else { [System.Windows.Forms.Clipboard]::Clear() }`,
  );
  if (r.code !== 0) throw new Error(`Couldn't paste the image: ${r.stderr || "clipboard busy"}`);
}

interface Win {
  pid: number;
  title: string;
}

/** First visible top-level window of any of these processes. */
async function findWindow(processNames: string[]): Promise<Win | null> {
  const out = await ps(
    `Get-Process -Name ${processNames.map(psq).join(",")} -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle } | Select-Object -First 1 | ForEach-Object { "$($_.Id)|$($_.MainWindowTitle)" }`,
  );
  if (!out) return null;
  const [pid, ...title] = out.split("|");
  return { pid: Number(pid), title: title.join("|") };
}


async function windowTitle(pid: number): Promise<string> {
  return ps(`(Get-Process -Id ${pid} -ErrorAction SilentlyContinue).MainWindowTitle`);
}


const norm = (s: string) => s.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "");

/** Does the window title show we're in a chat with this person/channel? */
export function titleMatches(title: string, recipient: string): boolean {
  const t = norm(title);
  const r = norm(recipient.replace(/^[@#]/, ""));
  if (r && t.includes(r)) return true;
  const words = recipient.toLowerCase().split(/[^a-z0-9]+/).filter((w) => w.length >= 3);
  return words.length > 0 && words.every((w) => t.includes(w));
}

async function screenshotBlock(): Promise<Anthropic.ImageBlockParam | null> {
  const shot = await runComputerAction("screenshot", {}).catch(() => null);
  return Array.isArray(shot) && shot[0]?.type === "image" ? (shot[0] as Anthropic.ImageBlockParam) : null;
}

export const typeText = defineTool({
  name: "type_text",
  description:
    "Type (paste) text into whatever text box currently has keyboard focus on the user's PC. Handles emoji, accents and multiple lines. " +
    "Click or focus the right box first. Does not press Enter: use the computer 'key' action with 'Return' to submit, and confirm_with_user first if that sends a message or submits a form.",
  schema: z.object({ text: z.string().min(1) }),
  risk: "safe",
  summary: (i) => `type "${i.text.slice(0, 60)}${i.text.length > 60 ? "…" : ""}"`,
  run: async ({ text }) => {
    await pasteText(text);
    return "Typed.";
  },
});

// One approval per message: after the user approves a send, the follow-up call that finishes it
// (chat_confirmed after a screenshot check) doesn't ask again for 3 minutes.
const approvedSends = new Map<string, number>();
const sendKey = (app: string, recipient: string, message: string) => `${app}\u0000${recipient.toLowerCase()}\u0000${message}`;
const recentlyApproved = (k: string) => (approvedSends.get(k) ?? 0) > Date.now() - 3 * 60_000;
/** Window shown to the model for checking; the chat_confirmed call must still be on exactly this window/page. */
const verifiedWindow = new Map<string, Win & { at: number }>();

const isUrl = (s: string) => /^https?:\/\//i.test(s.trim());

async function openOrFocus(app: ChatAppProfile): Promise<Win | string> {
  // Web apps share the browser process with every other tab, so "focus the browser" could land on an
  // unrelated page. Always navigate to the app's URL (opens/activates a tab in the default browser).
  if (isUrl(app.launch)) {
    await ps(`Start-Process ${psq(app.launch.trim())}`);
    let win: Win | null = null;
    for (let waited = 0; waited < 15_000 && !win; waited += 500) {
      await sleep(500);
      win = await findWindow(app.processNames);
    }
    if (!win) return `${app.name} didn't open in the browser within 15 seconds.`;
    await sleep(app.launchWaitMs ?? 3000);
    await focusWindow(win.pid);
    await sleep(300);
    return win;
  }
  let win = await findWindow(app.processNames);
  if (!win) {
    await openApp.run({ target: app.launch });
    for (let waited = 0; waited < 30_000 && !(win && !/updat|starting|loading/i.test(win.title)); waited += 1000) {
      await sleep(1000);
      win = await findWindow(app.processNames);
    }
    if (!win) return `${app.name} didn't open within 30 seconds.`;
    await sleep(app.launchWaitMs ?? 2000);
  }
  await focusWindow(win.pid);
  await sleep(350);
  return win;
}

export const sendMessage = defineTool({
  name: "send_message",
  description:
    "Send a message in a chat app on the user's PC: to a person/channel (Discord, Slack, ...) or a prompt to an AI chat app (ChatGPT, ...). " +
    "Opens/focuses the app; for people apps it jumps to the conversation with the app's quick switcher and checks it's the right chat; then pastes the message and sends. " +
    "It asks the user for approval itself when needed, so don't also call confirm_with_user. " +
    "If the result says the chat needs checking, look at the screenshot: if the right conversation is open, call send_message again with the same app/recipient/message and chat_confirmed: true; if not, tell the user (or navigate with the computer tools first). " +
    "For AI apps, set read_reply_after_seconds to get a screenshot of the answer to read back. " +
    "For an app with no profile, drive it with the computer tools and type_text instead.",
  schema: z.object({
    app: z.string().describe("e.g. 'discord', 'slack', 'signal', 'chatgpt'"),
    recipient: z.string().optional().describe("Person or channel name as shown in the app. Not used for AI apps."),
    message: z.string().describe("Text to send; may be empty when sending just an image"),
    image_path: z.string().optional().describe("Attach an image file (e.g. the screenshot from take_screenshot)"),
    send: z.boolean().optional().describe("false = type it into the box but don't send"),
    chat_confirmed: z.boolean().optional().describe("true after you've checked the screenshot shows the right chat open"),
    new_chat: z.boolean().optional().describe("AI apps: start a fresh conversation first"),
    read_reply_after_seconds: z.number().min(1).max(90).optional().describe("AI apps: wait this long after sending, then return a screenshot of the reply"),
  }),
  risk: (i) => {
    const app = findChatApp(i.app);
    if (i.send === false || app?.kind === "assistant") return "safe"; // prompting your own AI app isn't messaging a person
    if (i.chat_confirmed && app && recentlyApproved(sendKey(app.id, i.recipient ?? "", i.message))) return "safe";
    return "unsafe";
  },
  summary: (i) => {
    const app = findChatApp(i.app);
    const text = i.message.trim() ? `"${i.message.slice(0, 80)}${i.message.length > 80 ? "…" : ""}"` : "";
    const msg = [text, i.image_path ? "an image" : ""].filter(Boolean).join(" with ");
    if (app?.kind === "assistant") return `ask ${app.name} ${msg}`;
    return `${i.send === false ? "write (not send)" : "send"} ${msg} to ${i.recipient ?? "?"} on ${app?.name ?? i.app}`;
  },
  run: async ({ app: appName, recipient = "", message, image_path, send = true, chat_confirmed, new_chat, read_reply_after_seconds }) => {
    if (!message.trim() && !image_path) return "Nothing to send: give a message or an image_path.";
    if (image_path && !existsSync(image_path)) return `Image not found: ${image_path}`;
    // Attach the image (if any), then the text. Apps show pasted images as an attachment on the draft.
    const writeDraft = async (pid: number) => {
      if (image_path) {
        await pasteImage(image_path, pid);
        await sleep(400);
      }
      if (message.trim()) await pasteText(message, pid);
    };
    const app = findChatApp(appName);
    if (!app) {
      return `No profile for "${appName}". Known apps: ${allChatApps().map((a) => `${a.name} (${a.kind})`).join(", ")}. Use the computer tools and type_text to do it by sight.`;
    }
    if (app.kind === "people" && !recipient.trim()) return `Who should I message on ${app.name}? (recipient is required)`;
    // Incomplete profile: don't paste blindly into whatever field has focus; hand over to sight-based control.
    const manual = (why: string) =>
      `The ${app.name} profile ${why}, so I won't type blindly. Do it by sight instead: open/focus ${app.name}, ` +
      `screenshot, click the right conversation and its message box, type_text, then confirm_with_user before pressing Return.`;
    if (app.kind === "people" && !app.searchKeys.length) return manual("has no quick-switch shortcut");
    if (app.kind === "people" && !app.pickKeys.length && !chat_confirmed) return manual("has no key to open the search result");
    if (!app.sendKeys.length) return manual("has no send key");
    const key = sendKey(app.id, recipient, message);
    if (send) approvedSends.set(key, Date.now()); // reaching run() means it was approved (or no approval needed)

    // Second call after a visual check: keep the window/tab that was verified. For web apps, re-opening the URL
    // would land on a fresh tab and lose the verified conversation, so require it to still be in front.
    let opened: Win | string;
    if (chat_confirmed && isUrl(app.launch)) {
      // Browsers: the process can't tell tabs or windows apart, so require the exact page (title) that was checked.
      const snap = verifiedWindow.get(key);
      const fg = await foregroundPid();
      const title = await windowTitle(fg);
      if (!snap || snap.pid !== fg || snap.title !== title || Date.now() - snap.at > 3 * 60_000) {
        return `The ${app.name} page I checked isn't the one in front any more, so I stopped without typing. Do it by sight: bring the right tab forward, screenshot, click the message box, type_text, then confirm_with_user before sending.`;
      }
      opened = { pid: fg, title };
    } else {
      opened = await openOrFocus(app);
    }
    if (typeof opened === "string") return opened;

    const pid = opened.pid;
    if (app.kind === "assistant") {
      if (!app.focusMessageKeys?.length && !chat_confirmed) {
        // No reliable shortcut to the prompt box: let the model click it first.
        verifiedWindow.set(key, { pid, title: await windowTitle(pid), at: Date.now() });
        const shot = await screenshotBlock();
        const note = `${app.name} is open, but its profile has no shortcut to focus the prompt box. Click the message box (and start a new chat if asked), then call send_message again with chat_confirmed: true. Nothing typed yet.`;
        return shot ? [{ type: "text", text: note }, shot] : note;
      }
      if (new_chat && !chat_confirmed) {
        await pressAll(pid, app.newChatKeys);
        await sleep(700);
      }
      if (!chat_confirmed) await pressAll(pid, app.focusMessageKeys);
      await writeDraft(pid);
      await sleep(200);
      if (!send) return `Typed the prompt into ${app.name} without sending.`;
      await pressAll(pid, app.sendKeys);
      if (!read_reply_after_seconds) return `Sent to ${app.name}.`;
      await sleep(read_reply_after_seconds * 1000);
      const shot = await screenshotBlock();
      const note = `Sent to ${app.name}. Screenshot after ${read_reply_after_seconds}s: read the reply (scroll or wait longer with the computer tools if it's still writing or cut off).`;
      return shot ? [{ type: "text", text: note }, shot] : note;
    }

    if (!chat_confirmed) {
      // Jump to the conversation.
      await pressAll(pid, app.searchKeys);
      await sleep(300);
      await pasteText(recipient.replace(/^[@#]/, ""), pid);
      await sleep(app.searchWaitMs ?? 900);
      await pressAll(pid, app.pickKeys);
      await sleep(1200);

      // Check it's the right one before typing anything.
      const title = await windowTitle(opened.pid);
      if (app.verify !== "title" || !titleMatches(title, recipient)) {
        verifiedWindow.set(key, { pid, title, at: Date.now() });
        const shot = await screenshotBlock();
        const note =
          app.verify === "title"
            ? `Opened a chat in ${app.name}, but the window title "${title}" doesn't clearly match "${recipient}". Nothing typed yet.`
            : `Opened ${app.name} and searched for "${recipient}". Nothing typed yet.`;
        const ask = ` Check the screenshot: if the conversation with ${recipient} is open, call send_message again with chat_confirmed: true.`;
        return shot ? [{ type: "text", text: note + ask }, shot] : note + ask;
      }
    }

    // Type and send.
    await pressAll(pid, app.focusMessageKeys);
    await writeDraft(pid);
    await sleep(200);
    if (send) await pressAll(pid, app.sendKeys);
    approvedSends.delete(key);
    const title = await windowTitle(opened.pid);
    return send ? `Sent to ${recipient} on ${app.name} (window: "${title}").` : `Typed into the ${recipient} chat on ${app.name} without sending.`;
  },
});

