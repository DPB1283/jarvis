// "Write me a report/doc/letter": Jarvis composes it (markdown) and puts it in a new Google Doc, a file,
// or wherever the cursor is — with real formatting (headings, bold, lists) via a rich-text paste.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { z } from "zod";
import { defineTool } from "./types";
import { ps, psq } from "./ps";
import { foregroundWindow, pasteRich } from "./window-paste";

export const DOCS_DIR = path.join(os.homedir(), "Documents", "Jarvis Documents");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function inline(s: string): string {
  return esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<i>$2</i>")
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2">$1</a>');
}

/** Small markdown → HTML converter: headings, paragraphs, bold/italic/code/links, bullet & numbered lists, quotes, rules, simple tables. */
export function markdownToHtml(md: string): string {
  const out: string[] = [];
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  let list: "ul" | "ol" | null = null;
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) out.push(`<p>${inline(para.join(" "))}</p>`);
    para = [];
  };
  const closeList = () => {
    if (list) out.push(`</${list}>`);
    list = null;
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const t = line.trim();
    let m: RegExpMatchArray | null;
    if (!t) {
      flushPara();
      closeList();
    } else if ((m = /^(#{1,6})\s+(.*)$/.exec(t))) {
      flushPara();
      closeList();
      out.push(`<h${m[1].length}>${inline(m[2])}</h${m[1].length}>`);
    } else if (/^(-{3,}|\*{3,}|_{3,})$/.test(t)) {
      flushPara();
      closeList();
      out.push("<hr>");
    } else if ((m = /^[-*+]\s+(.*)$/.exec(t)) || (m = /^\d+[.)]\s+(.*)$/.exec(t))) {
      flushPara();
      const kind = /^\d/.test(t) ? "ol" : "ul";
      if (list !== kind) {
        closeList();
        out.push(`<${kind}>`);
        list = kind;
      }
      out.push(`<li>${inline(m[1])}</li>`);
    } else if ((m = /^>\s?(.*)$/.exec(t))) {
      flushPara();
      closeList();
      out.push(`<blockquote>${inline(m[1])}</blockquote>`);
    } else if (t.startsWith("|") && lines[i + 1]?.trim().match(/^\|?[\s:-]+\|/)) {
      flushPara();
      closeList();
      const cells = (row: string) => row.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      const head = cells(t);
      i++; // skip the --- separator row
      const rows: string[][] = [];
      while (i + 1 < lines.length && lines[i + 1].trim().startsWith("|")) rows.push(cells(lines[++i]));
      out.push(
        `<table border="1" style="border-collapse:collapse"><tr>${head.map((h) => `<th>${inline(h)}</th>`).join("")}</tr>` +
          rows.map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`).join("") +
          "</table>",
      );
    } else {
      closeList();
      para.push(t);
    }
  }
  flushPara();
  closeList();
  return out.join("\n");
}

/** Plain-text version (for apps that don't take formatting). */
const plain = (md: string) =>
  md
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1$2")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, "$1 ($2)");

const safeName = (s: string) => s.replace(/[<>:"/\\|?*\x00-\x1f]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 80) || "Document";

export const writeDocument = defineTool({
  name: "write_document",
  description:
    "Put a piece of writing you've composed (report, essay, letter, email draft, notes, plan...) somewhere for the user. " +
    "Write the full text yourself first, in markdown (# headings, **bold**, - bullets, numbered lists, | tables |). " +
    "destination: 'google_docs' (default: opens a new Google Doc in their browser and pastes it with formatting), " +
    "'file' (saves to Documents\\Jarvis Documents and opens it), or 'here' (pastes into whatever app/box is active now, e.g. Word or an email).",
  schema: z.object({
    title: z.string().describe("Short title, used for the file name"),
    content: z.string().min(1).describe("The full document in markdown"),
    destination: z.enum(["google_docs", "file", "here"]).optional(),
    file_format: z.enum(["docx_compatible_html", "markdown", "text"]).optional().describe("For destination 'file'. Default: docx_compatible_html (opens in Word with formatting)"),
  }),
  risk: "safe",
  summary: (i) => `write "${i.title}"${i.destination === "file" ? " to a file" : i.destination === "here" ? " here" : " in Google Docs"}`,
  run: async ({ title, content, destination = "google_docs", file_format = "docx_compatible_html" }) => {
    const html = markdownToHtml(content);
    const text = plain(content);
    const words = content.split(/\s+/).filter(Boolean).length;

    if (destination === "file") {
      fs.mkdirSync(DOCS_DIR, { recursive: true });
      const stamp = new Date().toISOString().slice(0, 10);
      const ext = file_format === "markdown" ? "md" : file_format === "text" ? "txt" : "doc";
      const file = path.join(DOCS_DIR, `${safeName(title)} ${stamp}.${ext}`);
      const body =
        ext === "doc"
          ? `<html><head><meta charset="utf-8"><title>${esc(title)}</title></head><body style="font-family:Calibri,Arial;font-size:11pt">${html}</body></html>`
          : ext === "md"
            ? content
            : text;
      fs.writeFileSync(file, ext === "doc" ? "﻿" + body : body, "utf8");
      await ps(`Start-Process ${psq(file)}`).catch(() => {});
      return `Saved ${words} words to ${file} and opened it.`;
    }

    if (destination === "here") {
      const fg = await foregroundWindow();
      await pasteRich(html, text, fg.pid);
      return `Pasted ${words} words into "${fg.title}".`;
    }

    // Google Docs: docs.new creates a blank doc in the default browser (must be signed in to Google).
    await ps(`Start-Process 'https://docs.new'`);
    let fg = { pid: 0, title: "" };
    for (let waited = 0; waited < 25_000; waited += 500) {
      await sleep(500);
      fg = await foregroundWindow();
      if (/sign in|accounts\.google|choose an account/i.test(fg.title)) {
        return "Google Docs is asking you to sign in to Google in your browser. Sign in, then ask me again, or I can save it as a file instead.";
      }
      if (/Untitled document|Google Docs/i.test(fg.title)) break;
    }
    if (!/Untitled document|Google Docs/i.test(fg.title)) {
      return `A new Google Doc didn't come up in front (the active window is "${fg.title}"), so I didn't type anything. I can save it as a file instead.`;
    }
    await sleep(2500); // let the editor finish loading and take focus
    const now = await foregroundWindow();
    if (now.pid !== fg.pid || !/Untitled document|Google Docs/i.test(now.title)) {
      return "The Google Doc lost focus before I could type, so I stopped. Ask me again or I can save it as a file.";
    }
    await pasteRich(html, text, fg.pid);
    return `Wrote ${words} words into a new Google Doc ("${title}"). It's saved in their Google Drive as "Untitled document" until renamed.`;
  },
});
