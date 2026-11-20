// Executor for Claude's computer-use toolset (computer_toolset_20260801) on the user's real screens.
// Multi-monitor: actions target ONE screen at a time ("active screen"); screenshots are of that screen only.
// The active screen defaults to the one holding the foreground window and can be switched with use_screen.
// Screenshots: PowerShell + System.Drawing (per-monitor DPI aware, downscaled JPEG).
// Mouse: positioned in physical pixels via Win32 (correct with mixed scaling); clicks/keys via nut.js.

import type Anthropic from "@anthropic-ai/sdk";
import { Button, Key, keyboard, mouse } from "@nut-tree-fork/nut-js";
import { ps } from "./ps";
import { foregroundMonitorIndex, getCursorPhysical, listMonitors, setCursorPhysical, type Monitor } from "../win32";

keyboard.config.autoDelayMs = 8;
mouse.config.autoDelayMs = 20;
mouse.config.mouseSpeed = 4000;

// Image limits for screenshots we send (safe for every toolset model).
const MAX_LONG_EDGE = 1568;
const MAX_PIXELS = 1_150_000;

interface Shot {
  data: string; // base64 jpeg
  width: number; // sent size
  height: number;
}

let activeIndex = 1;
let last: { sentW: number; sentH: number; screen: Monitor } | null = null;

export function screens(): Monitor[] {
  return listMonitors();
}

export function activeScreen(): Monitor {
  const all = listMonitors();
  return all.find((m) => m.index === activeIndex) ?? all[0];
}

/** Switch which screen computer actions and screenshots use (1-based). */
export function setActiveScreen(index: number): Monitor {
  const all = listMonitors();
  const m = all.find((s) => s.index === index);
  if (!m) throw new Error(`There's no screen ${index}; there are ${all.length}.`);
  activeIndex = index;
  last = null;
  return m;
}

/** Called at the start of each user turn: target the screen the user is working on. */
export function autoSelectScreen(): Monitor {
  const all = listMonitors();
  activeIndex = foregroundMonitorIndex(all);
  last = null;
  return all.find((m) => m.index === activeIndex) ?? all[0];
}

/** Screenshot of one screen (default: active), optionally a zoomed region of it, plus a thumbnail mode for overviews. */
export async function captureScreen(target?: Monitor, region?: [number, number, number, number], maxLong = MAX_LONG_EDGE): Promise<Shot> {
  return capture(region, target, maxLong);
}

async function capture(region?: [number, number, number, number], target?: Monitor, maxLong = MAX_LONG_EDGE): Promise<Shot> {
  const scr = target ?? activeScreen();
  // region is in the coordinate space of the last full screenshot (of this screen).
  const crop = region && last ? region.map((v, i) => v / (i % 2 === 0 ? last!.sentW : last!.sentH)) : null;
  const out = await ps(
    `
    Add-Type -AssemblyName System.Drawing, System.Windows.Forms
    Add-Type 'using System; using System.Runtime.InteropServices; public static class JDpi { [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v); }'
    [void][JDpi]::SetProcessDpiAwarenessContext([IntPtr](-4))
    $b = New-Object System.Drawing.Rectangle ${scr.x}, ${scr.y}, ${scr.width}, ${scr.height}
    $full = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($full)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $g.Dispose()
    $src = New-Object System.Drawing.Rectangle 0, 0, $b.Width, $b.Height
    ${
      crop
        ? `$x0 = [int](${crop[0]} * $b.Width); $y0 = [int](${crop[1]} * $b.Height)
           $x1 = [int](${crop[2]} * $b.Width); $y1 = [int](${crop[3]} * $b.Height)
           $src = New-Object System.Drawing.Rectangle $x0, $y0, ([Math]::Max(8, $x1 - $x0)), ([Math]::Max(8, $y1 - $y0))`
        : ""
    }
    # Doubles everywhere: [Math]::Min(1, 0.5) would pick the Int32 overload and round to 1.
    [double]$fitEdge = [double]${maxLong} / [double][Math]::Max($src.Width, $src.Height)
    [double]$fitArea = [Math]::Sqrt([double]${MAX_PIXELS} / ([double]$src.Width * [double]$src.Height))
    [double]$scale = [Math]::Min([double]${crop ? "4.0" : "1.0"}, [Math]::Min($fitEdge, $fitArea))
    $w = [int][Math]::Floor($src.Width * $scale); $h = [int][Math]::Floor($src.Height * $scale)
    $img = New-Object System.Drawing.Bitmap $w, $h
    $g2 = [System.Drawing.Graphics]::FromImage($img)
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g2.DrawImage($full, (New-Object System.Drawing.Rectangle 0, 0, $w, $h), $src, [System.Drawing.GraphicsUnit]::Pixel)
    $g2.Dispose(); $full.Dispose()
    $codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
    $ep = New-Object System.Drawing.Imaging.EncoderParameters 1
    $ep.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), 75L
    $ms = New-Object System.IO.MemoryStream
    $img.Save($ms, $codec, $ep); $img.Dispose()
    "$w|$h|" + [Convert]::ToBase64String($ms.ToArray())
  `,
    20_000,
  );
  const [w, h, data] = out.split("|");
  const shot = { width: Number(w), height: Number(h), data };
  // Only full-size shots of the active screen define the coordinate space for clicks.
  if (!region && !target && maxLong === MAX_LONG_EDGE) last = { sentW: shot.width, sentH: shot.height, screen: scr };
  return shot;
}

/** Screenshot coordinates → physical desktop pixels on the active screen. */
async function moveTo([x, y]: [number, number]): Promise<void> {
  if (!last) await capture();
  const s = last!.screen;
  setCursorPhysical(s.x + (x / last!.sentW) * s.width, s.y + (y / last!.sentH) * s.height);
  await new Promise((r) => setTimeout(r, 15));
}

async function cursorInShot(): Promise<[number, number] | null> {
  if (!last) await capture();
  const s = last!.screen;
  const p = getCursorPhysical();
  if (p.x < s.x || p.y < s.y || p.x >= s.x + s.width || p.y >= s.y + s.height) return null;
  return [Math.round(((p.x - s.x) / s.width) * last!.sentW), Math.round(((p.y - s.y) / s.height) * last!.sentH)];
}

// xdotool-style key names -> nut.js keys.
const KEY_ALIASES: Record<string, Key> = {
  ctrl: Key.LeftControl, control: Key.LeftControl, shift: Key.LeftShift, alt: Key.LeftAlt,
  super: Key.LeftSuper, win: Key.LeftSuper, windows: Key.LeftSuper, meta: Key.LeftSuper, cmd: Key.LeftSuper,
  return: Key.Enter, enter: Key.Enter, kp_enter: Key.Enter, tab: Key.Tab, escape: Key.Escape, esc: Key.Escape,
  space: Key.Space, backspace: Key.Backspace, delete: Key.Delete, del: Key.Delete, insert: Key.Insert,
  home: Key.Home, end: Key.End, page_up: Key.PageUp, pageup: Key.PageUp, prior: Key.PageUp,
  page_down: Key.PageDown, pagedown: Key.PageDown, next: Key.PageDown,
  up: Key.Up, down: Key.Down, left: Key.Left, right: Key.Right,
  caps_lock: Key.CapsLock, print: Key.Print, menu: Key.Menu,
  minus: Key.Minus, equal: Key.Equal, plus: Key.Equal, comma: Key.Comma, period: Key.Period,
  slash: Key.Slash, backslash: Key.Backslash, semicolon: Key.Semicolon, apostrophe: Key.Quote,
  grave: Key.Grave, bracketleft: Key.LeftBracket, bracketright: Key.RightBracket,
  audiovolumeup: Key.AudioVolUp, audiovolumedown: Key.AudioVolDown, audiomute: Key.AudioMute,
  audioplay: Key.AudioPlay, audionext: Key.AudioNext, audioprev: Key.AudioPrev,
};
const PUNCT: Record<string, Key> = {
  "-": Key.Minus, "=": Key.Equal, ",": Key.Comma, ".": Key.Period, "/": Key.Slash, "\\": Key.Backslash,
  ";": Key.Semicolon, "'": Key.Quote, "`": Key.Grave, "[": Key.LeftBracket, "]": Key.RightBracket,
};

function parseKey(name: string): Key {
  const n = name.trim();
  const lower = n.toLowerCase().replace(/^super_l$|^super_r$/, "super");
  if (KEY_ALIASES[lower] !== undefined) return KEY_ALIASES[lower];
  if (PUNCT[n] !== undefined) return PUNCT[n];
  if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(n)) return Key[`F${n.slice(1)}` as keyof typeof Key];
  if (/^[a-z]$/i.test(n)) return Key[n.toUpperCase() as keyof typeof Key];
  if (/^[0-9]$/.test(n)) return Key[`Num${n}` as keyof typeof Key];
  throw new Error(`Unknown key "${name}"`);
}
export const parseCombo = (combo: string) => combo.split("+").filter(Boolean).map(parseKey);

async function withModifiers<T>(mods: string | undefined, fn: () => Promise<T>): Promise<T> {
  const keys = mods ? parseCombo(mods) : [];
  if (keys.length) await keyboard.pressKey(...keys);
  try {
    return await fn();
  } finally {
    if (keys.length) await keyboard.releaseKey(...keys);
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Coord = [number, number];
type Input = {
  coordinate?: Coord;
  start_coordinate?: Coord;
  region?: [number, number, number, number];
  text?: string;
  scroll_direction?: "up" | "down" | "left" | "right";
  scroll_amount?: number;
  repeat?: number;
  duration?: number;
};

/** Actions considered "safe" in ask_unsafe mode. Everything here is safe; sensitive steps go through confirm_with_user. */
export function describeComputerAction(name: string, i: Input): string {
  const at = i.coordinate ? ` at (${i.coordinate.join(", ")})` : "";
  switch (name) {
    case "type":
      return `type "${(i.text ?? "").slice(0, 60)}"`;
    case "key":
      return `press ${i.text}`;
    case "scroll":
      return `scroll ${i.scroll_direction}${at}`;
    default:
      return `${name.replace(/_/g, " ")}${at}`;
  }
}

type ResultContent = Anthropic.ToolResultBlockParam["content"];

/** Run one computer member action. Returns tool_result content. */
export async function runComputerAction(name: string, input: Input): Promise<ResultContent> {
  const image = (s: Shot): ResultContent => [
    { type: "image", source: { type: "base64", media_type: "image/jpeg", data: s.data } },
  ];
  const click = async (fn: () => Promise<unknown>) => {
    if (input.coordinate) await moveTo(input.coordinate);
    await withModifiers(input.text, fn);
    return "OK";
  };

  switch (name) {
    case "screenshot":
      return image(await capture());
    case "zoom":
      if (!input.region) throw new Error("zoom needs region");
      return image(await capture(input.region));
    case "left_click":
      return click(() => mouse.click(Button.LEFT));
    case "right_click":
      return click(() => mouse.click(Button.RIGHT));
    case "middle_click":
      return click(() => mouse.click(Button.MIDDLE));
    case "double_click":
      return click(() => mouse.doubleClick(Button.LEFT));
    case "triple_click":
      return click(async () => {
        for (let k = 0; k < 3; k++) await mouse.click(Button.LEFT);
      });
    case "mouse_move":
      if (!input.coordinate) throw new Error("mouse_move needs coordinate");
      await moveTo(input.coordinate);
      return "OK";
    case "left_click_drag":
      if (!input.start_coordinate || !input.coordinate) throw new Error("drag needs start_coordinate and coordinate");
      await withModifiers(input.text, async () => {
        await moveTo(input.start_coordinate!);
        await mouse.pressButton(Button.LEFT);
        await sleep(60);
        await moveTo(input.coordinate!);
        await sleep(60);
        await mouse.releaseButton(Button.LEFT);
      });
      return "OK";
    case "left_mouse_down":
      if (input.coordinate) await moveTo(input.coordinate);
      await mouse.pressButton(Button.LEFT);
      return "OK";
    case "left_mouse_up":
      if (input.coordinate) await moveTo(input.coordinate);
      await mouse.releaseButton(Button.LEFT);
      return "OK";
    case "cursor_position": {
      const pos = await cursorInShot();
      return pos ? `X=${pos[0]}, Y=${pos[1]}` : `The cursor is on another screen (not screen ${activeIndex}).`;
    }
    case "scroll": {
      if (input.coordinate) await moveTo(input.coordinate);
      const amount = Math.max(1, Math.round(input.scroll_amount ?? 3)) * 120; // wheel clicks -> wheel units
      await withModifiers(input.text, async () => {
        const dir = input.scroll_direction ?? "down";
        if (dir === "down") await mouse.scrollDown(amount);
        else if (dir === "up") await mouse.scrollUp(amount);
        else if (dir === "left") await mouse.scrollLeft(amount);
        else await mouse.scrollRight(amount);
      });
      return "OK";
    }
    case "type":
      await keyboard.type(input.text ?? "");
      return "OK";
    case "key": {
      const keys = parseCombo(input.text ?? "");
      for (let k = 0; k < Math.min(100, input.repeat ?? 1); k++) {
        await keyboard.pressKey(...keys);
        await keyboard.releaseKey(...keys);
      }
      return "OK";
    }
    case "hold_key": {
      const keys = parseCombo(input.text ?? "");
      await keyboard.pressKey(...keys);
      await sleep(Math.min(300, input.duration ?? 1) * 1000);
      await keyboard.releaseKey(...keys);
      return "OK";
    }
    case "wait":
      await sleep(Math.min(300, input.duration ?? 1) * 1000);
      return "OK";
    default:
      throw new Error(`Unsupported computer action "${name}"`);
  }
}

export const COMPUTER_TOOLSET = { type: "computer_toolset_20260801" } as const;
