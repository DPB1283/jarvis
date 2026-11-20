import os from "node:os";
import { z } from "zod";
import { psq, runPs } from "./ps";
import { defineTool } from "./types";

const KINDS = ["any", "document", "picture", "music", "video", "email", "folder", "program"] as const;

export const findFilesSmart = defineTool({
  name: "find_anything",
  description:
    "Search the user's files by name AND contents using the Windows Search index (fast). Use for 'find the PDF my teacher sent last week', " +
    "'that spreadsheet about the budget', 'photos from yesterday'. Filter by kind and how recently modified. Falls back to find_files if the index is off.",
  schema: z.object({
    query: z.string().describe("Key words that would be in the name or contents"),
    kind: z.enum(KINDS).optional(),
    modified_within_days: z.number().int().min(1).max(3650).optional(),
    extension: z.string().optional().describe("e.g. 'pdf', 'docx'"),
    limit: z.number().int().min(1).max(50).optional(),
  }),
  risk: "safe",
  summary: (i) => `search files for "${i.query}"`,
  run: async ({ query, kind = "any", modified_within_days, extension, limit = 15 }) => {
    const clean = (s: string) => s.replace(/['"%_\[\]]/g, " ").trim();
    const words = clean(query).split(/\s+/).filter((w) => w.length > 1).slice(0, 6);
    if (!words.length) return "What should I look for?";
    const contains = words.map((w) => `"${w}*"`).join(" AND ");
    const nameLike = words.map((w) => `System.FileName LIKE '%${w}%'`).join(" AND ");
    const where = [
      `SCOPE='file:${os.homedir().replace(/\\/g, "/")}'`,
      `(CONTAINS(*, '${contains.replace(/'/g, "''")}') OR (${nameLike}))`,
      kind !== "any" ? `System.Kind = '${kind}'` : "",
      extension ? `System.FileExtension = '.${clean(extension).replace(/^\./, "")}'` : "",
      modified_within_days ? `System.DateModified >= '${new Date(Date.now() - modified_within_days * 86_400_000).toISOString().slice(0, 10)} 00:00:00'` : "",
    ]
      .filter(Boolean)
      .join(" AND ");
    const sql = `SELECT TOP ${limit} System.ItemPathDisplay, System.DateModified, System.Size FROM SystemIndex WHERE ${where} ORDER BY System.DateModified DESC`;
    const r = await runPs(
      `$c = New-Object -ComObject ADODB.Connection
      $c.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
      $rs = New-Object -ComObject ADODB.Recordset
      $rs.Open(${psq(sql)}, $c)
      while (-not $rs.EOF) {
        $p = $rs.Fields.Item("System.ItemPathDisplay").Value; $d = $rs.Fields.Item("System.DateModified").Value; $s = $rs.Fields.Item("System.Size").Value
        "{0:yyyy-MM-dd HH:mm}  {1,8}  {2}" -f $d, $(if ($s) { "{0:N0}KB" -f ($s / 1KB) } else { "" }), $p
        $rs.MoveNext() }
      $rs.Close(); $c.Close()`,
      30_000,
    );
    if (r.code !== 0) return `The Windows Search index isn't available (${r.stderr.split("\n")[0]}). Use find_files instead.`;
    return r.stdout || `Nothing indexed matches "${query}". Try other words, or find_files for a plain name search.`;
  },
});
