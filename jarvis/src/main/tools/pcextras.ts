// Clipboard brain, "clip that" (Medal / NVIDIA / Xbox Game Bar), and find-anything via the Windows Search index.
import type Anthropic from "@anthropic-ai/sdk";
import { keyboard } from "@nut-tree-fork/nut-js";
import { z } from "zod";
import { getSettings } from "../config";
import { defineTool } from "./types";
import { ps, psq, runPs } from "./ps";
import { parseCombo } from "./computer";

// --- Clipboard ---------------------------------------------------------------------------

export const readClipboard = defineTool({
  name: "read_clipboard",
  description:
    "Read what the user just copied: text, an image (you'll see it), or copied files. Use for 'summarise / fix / translate / explain what I copied'. " +
    "To put a result back, use type_text (pastes where their cursor is) or set_clipboard.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "read the clipboard",
  run: async () => {
    const out = await ps(`Add-Type -AssemblyName System.Windows.Forms, System.Drawing
      if ([System.Windows.Forms.Clipboard]::ContainsText()) { "TEXT"; [System.Windows.Forms.Clipboard]::GetText(); exit }
      if ([System.Windows.Forms.Clipboard]::ContainsFileDropList()) { "FILES"; [System.Windows.Forms.Clipboard]::GetFileDropList(); exit }
      if ([System.Windows.Forms.Clipboard]::ContainsImage()) {
        $img = [System.Windows.Forms.Clipboard]::GetImage()
        $scale = [Math]::Min([double]1.0, [Math]::Min([double]1568 / [double][Math]::Max($img.Width, $img.Height), [Math]::Sqrt([double]1150000 / ([double]$img.Width * [double]$img.Height))))
        $w = [int]($img.Width * $scale); $h = [int]($img.Height * $scale)
        $bmp = New-Object System.Drawing.Bitmap $img, $w, $h
        $ms = New-Object System.IO.MemoryStream; $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Jpeg)
        "IMAGE"; [Convert]::ToBase64String($ms.ToArray()); exit }
      "EMPTY"`, 20_000);
    const [kind, ...rest] = out.split(/\r?\n/);
    const body = rest.join("\n");
    if (kind === "TEXT") return body.length > 30_000 ? body.slice(0, 30_000) + "\n…[truncated]" : body || "(empty text)";
    if (kind === "FILES") return `Copied files:\n${body}`;
    if (kind === "IMAGE") {
      const img: Anthropic.ImageBlockParam = { type: "image", source: { type: "base64", media_type: "image/jpeg", data: body.trim() } };
      return [{ type: "text", text: "The clipboard holds this image:" }, img];
    }
    return "The clipboard is empty.";
  },
});

export const setClipboard = defineTool({
  name: "set_clipboard",
  description: "Put text on the clipboard so the user can paste it themselves (e.g. 'copy that for me').",
  schema: z.object({ text: z.string() }),
  risk: "safe",
  summary: () => "copy text to the clipboard",
  run: async ({ text }) => {
    const r = await runPs(`Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::SetText(${psq(text || " ")})`);
    return r.code === 0 ? "Copied to the clipboard." : `Couldn't copy: ${r.stderr}`;
  },
});

// --- Clip that ---------------------------------------------------------------------------

const CLIP_DEFAULTS = { medal: "F8", nvidia: "alt+F10", xbox: "super+alt+g" } as const;
const CLIP_NAMES = { medal: "Medal", nvidia: "NVIDIA Instant Replay", xbox: "Xbox Game Bar" } as const;

export const clipThat = defineTool({
  name: "clip_that",
  description:
    "Save a clip of the last moments of gameplay/screen ('clip that'), using the user's recorder: Medal, NVIDIA Instant Replay, or Xbox Game Bar. Presses that app's save-clip hotkey.",
  schema: z.object({ app: z.enum(["auto", "medal", "nvidia", "xbox"]).optional() }),
  risk: "safe",
  summary: () => "clip that",
  run: async ({ app }) => {
    const cfg = getSettings().clips ?? { app: "auto", hotkey: "" };
    let which = app && app !== "auto" ? app : cfg.app;
    if (which === "auto") {
      const running = await ps(`Get-Process -Name Medal, 'NVIDIA Overlay' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty ProcessName -Unique`).catch(() => "");
      which = /medal/i.test(running) ? "medal" : /nvidia/i.test(running) ? "nvidia" : "xbox";
    }
    const combo = (app === undefined || app === "auto" || app === cfg.app) && cfg.hotkey.trim() ? cfg.hotkey.trim() : CLIP_DEFAULTS[which];
    const keys = parseCombo(combo);
    await keyboard.pressKey(...keys);
    await new Promise((r) => setTimeout(r, 60));
    await keyboard.releaseKey(...keys);
    const hint =
      which === "xbox" ? " (needs Game Bar's background recording turned on)" :
      which === "nvidia" ? " (needs Instant Replay turned on in the NVIDIA overlay)" : "";
    return `Pressed ${CLIP_NAMES[which]}'s clip hotkey (${combo})${hint}.`;
  },
});

// --- Find anything (Windows Search index: names + file contents) ---------------------------

export { findFilesSmart } from "./file-search";
