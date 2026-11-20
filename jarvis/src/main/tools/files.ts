import fs from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import { z } from "zod";
import { defineTool } from "./types";
import { ps, psq } from "./ps";
import { resolvePath } from "./system";

const MAX_READ = 20_000;

export const findFiles = defineTool({
  name: "find_files",
  description:
    "Search for files or folders by name (wildcards allowed, e.g. '*.pdf', 'report*'). Searches the user's home folder by default.",
  schema: z.object({
    pattern: z.string(),
    folder: z.string().optional().describe("Folder to search, e.g. 'Downloads', 'C:\\Projects'. Default: home folder"),
    limit: z.number().min(1).max(200).optional(),
  }),
  risk: "safe",
  summary: (i) => `search for ${i.pattern}`,
  run: async ({ pattern, folder, limit }) => {
    const root = folder ? resolvePath(folder) : os.homedir();
    const out = await ps(
      `Get-ChildItem -LiteralPath ${psq(root)} -Recurse -Filter ${psq(pattern)} -ErrorAction SilentlyContinue -Force |
        Where-Object { $_.FullName -notmatch '\\\\(node_modules|AppData|\\.git)\\\\' } |
        Sort-Object LastWriteTime -Descending | Select-Object -First ${limit ?? 30} |
        ForEach-Object { "{0}  {1}  {2}" -f $_.LastWriteTime.ToString('yyyy-MM-dd HH:mm'), $(if ($_.PSIsContainer) { '<dir>' } else { '{0:N0}KB' -f ($_.Length/1KB) }), $_.FullName }`,
      90_000,
    );
    return out || `No matches for ${pattern} under ${root}.`;
  },
});

export const listFolder = defineTool({
  name: "list_folder",
  description: "List the contents of a folder.",
  schema: z.object({ folder: z.string() }),
  risk: "safe",
  summary: (i) => `look in ${i.folder}`,
  run: async ({ folder }) => {
    const dir = resolvePath(folder);
    const entries = await fs.readdir(dir, { withFileTypes: true });
    return [dir, ...entries.slice(0, 300).map((e) => (e.isDirectory() ? `[dir] ${e.name}` : e.name))].join("\n");
  },
});

export const readFile = defineTool({
  name: "read_file",
  description: "Read a text file (first 20,000 characters).",
  schema: z.object({ file: z.string() }),
  risk: "safe",
  summary: (i) => `read ${i.file}`,
  run: async ({ file }) => {
    const text = await fs.readFile(resolvePath(file), "utf8");
    return text.length > MAX_READ ? text.slice(0, MAX_READ) + "\n…[truncated]" : text;
  },
});

export const writeFile = defineTool({
  name: "write_file",
  description: "Create a text file, or append to / overwrite an existing one.",
  schema: z.object({
    file: z.string(),
    content: z.string(),
    mode: z.enum(["create", "append", "overwrite"]).optional().describe("Default create (fails if the file exists)"),
  }),
  risk: (i) => (i.mode === "overwrite" && existsSync(resolvePath(i.file)) ? "unsafe" : "safe"),
  summary: (i) => `${i.mode ?? "create"} ${i.file}`,
  run: async ({ file, content, mode }) => {
    const p = resolvePath(file);
    await fs.mkdir(path.dirname(p), { recursive: true });
    if ((mode ?? "create") === "create" && existsSync(p)) return `${p} already exists; use append or overwrite.`;
    if (mode === "append") await fs.appendFile(p, content, "utf8");
    else await fs.writeFile(p, content, "utf8");
    return `Wrote ${p}`;
  },
});

export const movePath = defineTool({
  name: "move_path",
  description: "Move or rename a file or folder. Will not overwrite unless overwrite=true.",
  schema: z.object({ from: z.string(), to: z.string(), overwrite: z.boolean().optional() }),
  risk: (i) => (existsSync(resolvePath(i.to)) && i.overwrite ? "unsafe" : "safe"),
  summary: (i) => `move ${i.from} → ${i.to}`,
  run: async ({ from, to, overwrite }) => {
    const src = resolvePath(from);
    let dst = resolvePath(to);
    if (existsSync(dst) && (await fs.stat(dst)).isDirectory()) dst = path.join(dst, path.basename(src));
    if (existsSync(dst) && !overwrite) return `${dst} already exists.`;
    await fs.rename(src, dst).catch(async (e: NodeJS.ErrnoException) => {
      if (e.code !== "EXDEV") throw e;
      await fs.cp(src, dst, { recursive: true, force: !!overwrite });
      await fs.rm(src, { recursive: true });
    });
    return `Moved to ${dst}`;
  },
});

export const deletePath = defineTool({
  name: "delete_path",
  description: "Delete a file or folder by sending it to the Recycle Bin (recoverable).",
  schema: z.object({ target: z.string() }),
  risk: "unsafe",
  summary: (i) => `delete ${i.target} (to Recycle Bin)`,
  run: async ({ target }) => {
    const p = resolvePath(target);
    if (!existsSync(p)) return `${p} does not exist.`;
    const isDir = (await fs.stat(p)).isDirectory();
    await ps(`Add-Type -AssemblyName Microsoft.VisualBasic
      [Microsoft.VisualBasic.FileIO.FileSystem]::${isDir ? "DeleteDirectory" : "DeleteFile"}(${psq(p)}, 'OnlyErrorDialogs', 'SendToRecycleBin')`);
    return `Sent ${p} to the Recycle Bin.`;
  },
});
