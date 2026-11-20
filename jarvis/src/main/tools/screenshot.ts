// "Take a screenshot": full-resolution PNG saved to Pictures\Jarvis Screenshots (the HUD is excluded from
// captures by content protection), then open / copy / paste it wherever the user wants.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { z } from "zod";
import { defineTool } from "./types";
import { ps, psq, runPs, WIN_NATIVE } from "./ps";
import { foregroundPid, pasteImage } from "./desktop";
import { activeScreen, runComputerAction, setActiveScreen } from "./computer";

export const SCREENSHOT_DIR = path.join(os.homedir(), "Pictures", "Jarvis Screenshots");
let lastScreenshot: string | null = null;

export const takeScreenshot = defineTool({
  name: "take_screenshot",
  description:
    "Save a full-resolution screenshot for the user (not for your own viewing; use the computer screenshot for that). " +
    "Afterwards ask where they'd like it: opened on screen, copied to the clipboard, or pasted into an app/chat, then use use_screenshot (or send_message with image_path for a chat).",
  schema: z.object({
    area: z.enum(["screen", "all_screens", "active_window"]).optional().describe("Default: the screen they're working on"),
    screen: z.number().int().min(1).optional().describe("Which monitor for area 'screen' (1 = main); default the one with their active window"),
    delay_seconds: z.number().min(0).max(10).optional().describe("Wait before capturing, e.g. to open a menu"),
  }),
  risk: "safe",
  summary: (i) => `take a screenshot${i.area === "active_window" ? " of the active window" : i.screen ? ` of screen ${i.screen}` : ""}`,
  run: async ({ area = "screen", screen, delay_seconds }) => {
    if (screen) setActiveScreen(screen);
    if (delay_seconds) await new Promise((r) => setTimeout(r, delay_seconds * 1000));
    const file = await captureToFile(area, SCREENSHOT_DIR, "Screenshot");
    lastScreenshot = file;
    return `Saved screenshot: ${file}. Now ask where they'd like it (open it, copy it, or paste it somewhere).`;
  },
});

/** Full-resolution PNG of the screen / all screens / active window, saved in `dir`. Returns the file path. */
export async function captureToFile(area: "screen" | "all_screens" | "active_window", dir: string, prefix: string): Promise<string> {
  {
    fs.mkdirSync(dir, { recursive: true });
    const stamp = new Date().toISOString().replace(/[:T]/g, "-").slice(0, 19);
    const file = path.join(dir, `${prefix} ${stamp}.png`);
    const bounds =
      area === "active_window"
        ? `Add-Type 'using System; using System.Runtime.InteropServices; public struct JRect { public int L, T, R, B; } public static class JWin { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out JRect r, int s); }'
           $r = New-Object JRect; [void][JWin]::DwmGetWindowAttribute([JWin]::GetForegroundWindow(), 9, [ref]$r, 16)
           $b = New-Object System.Drawing.Rectangle $r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T)`
        : area === "all_screens"
          ? `$b = [System.Windows.Forms.SystemInformation]::VirtualScreen`
          : (() => {
              const m = activeScreen(); // the screen the user is working on (or the one chosen with use_screen)
              return `$b = New-Object System.Drawing.Rectangle ${m.x}, ${m.y}, ${m.width}, ${m.height}`;
            })();
    await ps(`${WIN_NATIVE}
      Add-Type -AssemblyName System.Drawing, System.Windows.Forms
      Add-Type 'using System; using System.Runtime.InteropServices; public static class JDpi2 { [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v); }'
      [void][JDpi2]::SetProcessDpiAwarenessContext([IntPtr](-4))
      ${bounds}
      if ($b.Width -le 0 -or $b.Height -le 0) { throw "Couldn't find the window to capture" }
      $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
      $g.Dispose()
      $bmp.Save(${psq(file)}, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()`);
    return file;
  }
}

export const MEMORY_SHOT_DIR = path.join(SCREENSHOT_DIR, "Memories");

export const rememberScreen = defineTool({
  name: "remember_screen",
  description:
    "For 'remember this/that' about something on screen: saves a full-resolution screenshot and shows it to you. " +
    "Then call remember with a short, specific note of what matters on screen (names, numbers, dates, links) and image_path set to the returned path.",
  schema: z.object({ area: z.enum(["screen", "all_screens", "active_window"]).optional() }),
  risk: "safe",
  summary: () => "save what's on screen to memory",
  run: async ({ area = "screen" }) => {
    const file = await captureToFile(area, MEMORY_SHOT_DIR, "Memory");
    const view = await runComputerAction("screenshot", {}).catch(() => null);
    const note = `Saved screen to ${file}. Now call remember with a concise note of what's on screen and image_path: ${JSON.stringify(file)}.`;
    return Array.isArray(view) ? [{ type: "text", text: note }, ...view] : note;
  },
});

export const useScreenshot = defineTool({
  name: "use_screenshot",
  description:
    "Do something with a saved screenshot (default: the most recent one): open it on screen, copy it to the clipboard, show it in its folder, " +
    "or paste it into the window that's currently active (focus/click the right place first). For sending it to a person in a chat app, use send_message with image_path instead.",
  schema: z.object({
    action: z.enum(["open", "copy", "show_in_folder", "paste_here"]),
    path: z.string().optional().describe("Screenshot file; defaults to the latest one"),
  }),
  risk: "safe",
  summary: (i) => `${i.action.replace(/_/g, " ")} the screenshot`,
  run: async ({ action, path: p }) => {
    const file = p ?? lastScreenshot;
    if (!file || !fs.existsSync(file)) return "No screenshot to use yet. Take one first.";
    switch (action) {
      case "open":
        await ps(`Start-Process ${psq(file)}`);
        return "Opened it.";
      case "show_in_folder":
        await ps(`Start-Process explorer.exe -ArgumentList ${psq(`/select,"${file}"`)}`);
        return "Showing it in its folder.";
      case "copy": {
        const r = await runPs(`Add-Type -AssemblyName System.Windows.Forms, System.Drawing
          $img = [System.Drawing.Image]::FromFile(${psq(file)}); [System.Windows.Forms.Clipboard]::SetImage($img); $img.Dispose()`);
        return r.code === 0 ? "Copied to the clipboard; the user can paste it anywhere with Ctrl+V." : `Couldn't copy: ${r.stderr}`;
      }
      case "paste_here": {
        await pasteImage(file, await foregroundPid());
        return "Pasted into the active window.";
      }
    }
  },
});
