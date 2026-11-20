import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { z } from "zod";
import { runPs } from "./ps";
import { defineTool } from "./types";

interface Vdf { [key: string]: Vdf | string }

function parseVdf(source: string): Vdf {
  const tokens = Array.from(source.matchAll(/\/\/[^\r\n]*|"((?:\\.|[^"\\])*)"|[{}]|[^\s{}"]+/g))
    .filter(token => !token[0].startsWith("//"));
  let i = 0;
  const value = (token: RegExpMatchArray) => token[1]?.replace(/\\([\\"])/g, "$1") ?? token[0];
  function readGroup(depth: number): Vdf {
    const result: Vdf = Object.create(null);
    if (depth > 30) throw new Error("Steam metadata is nested too deeply.");
    while (i < tokens.length) {
      const key = tokens[i++];
      if (key[0] === "}") return result;
      const item = tokens[i++];
      if (!item || item[0] === "}") throw new Error("Incomplete Steam metadata.");
      result[value(key).toLowerCase()] = item[0] === "{" ? readGroup(depth + 1) : value(item);
    }
    return result;
  }
  return readGroup(0);
}

async function metadata(file: string): Promise<Vdf> {
  try { return parseVdf(await readFile(file, "utf8")); } catch { return {}; }
}

function group(value: Vdf | string | undefined): Vdf {
  return value && typeof value === "object" ? value : {};
}

export async function readSteamGames(root: string) {
  const steamApps = path.join(root, "steamapps");
  const modern = await metadata(path.join(steamApps, "libraryfolders.vdf"));
  const libraryFolders = Object.keys(modern).length ? modern : await metadata(path.join(root, "config", "libraryfolders.vdf"));
  const libraries = new Set([steamApps]);
  for (const [key, value] of Object.entries(group(libraryFolders.libraryfolders))) {
    if (!/^\d+$/.test(key)) continue;
    const folder = typeof value === "string" ? value : value.path;
    if (typeof folder === "string" && path.isAbsolute(folder)) libraries.add(path.join(folder, "steamapps"));
  }
  const games = new Map<string, { id: string; name: string }>();
  for (const library of libraries) {
    const files = await readdir(library).catch(() => []);
    for (const file of files) {
      const id = /^appmanifest_(\d+)\.acf$/i.exec(file)?.[1];
      if (!id || games.has(id)) continue;
      const app = group((await metadata(path.join(library, file))).appstate);
      const { name, installdir, stateflags } = app;
      if (typeof name !== "string" || typeof installdir !== "string" ||
          !installdir || installdir === "." || installdir === ".." || /[\\/:]/.test(installdir)) continue;
      if (typeof stateflags === "string" && (Number(stateflags) & 4) === 0) continue;
      if (!(await stat(path.join(library, "common", installdir)).catch(() => null))?.isDirectory()) continue;
      games.set(id, { id, name });
    }
  }
  return [...games.values()].sort((a, b) => a.name.localeCompare(b.name));
}

async function installedGames() {
  const registry = await runPs(
    "(Get-ItemProperty -LiteralPath 'HKCU:\\Software\\Valve\\Steam' -ErrorAction SilentlyContinue).SteamPath",
    5_000,
  );
  const root = registry.stdout.trim();
  return readSteamGames(root && path.isAbsolute(root) ? root : path.join(process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "Steam"));
}

const normalize = (value: string) => value.toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "").replace(/[^\p{L}\p{N}]+/gu, " ").trim();

function matchScore(name: string, query: string) {
  if (name === query) return 3;
  if (name.startsWith(query)) return 2;
  if (name.includes(query) || query.split(" ").every(word => name.includes(word))) return 1;
  const limit = query.length >= 10 ? 2 : 1;
  if (query.length < 5 || Math.abs(name.length - query.length) > limit) return 0;
  let distances = Array.from({ length: query.length + 1 }, (_, index) => index);
  for (let row = 1; row <= name.length; row++) {
    const next = [row];
    for (let column = 1; column <= query.length; column++) {
      next.push(Math.min(next[column - 1] + 1, distances[column] + 1,
        distances[column - 1] + Number(name[row - 1] !== query[column - 1])));
    }
    distances = next;
  }
  const distance = distances[query.length];
  return distance <= limit ? 1 / (1 + distance) : 0;
}

export const steamGames = defineTool({
  name: "steam_games",
  description: "List games installed in the local Steam libraries, optionally filtered by title.",
  schema: z.object({ query: z.string().trim().min(1).optional() }),
  risk: "safe",
  summary: () => "list installed Steam games",
  run: async ({ query }) => {
    const name = query ? normalize(query) : "";
    const games = (await installedGames()).filter(game => !name || matchScore(normalize(game.name), name));
    return games.length ? games.map(game => `${game.name} (App ID ${game.id})`).join("\n") : "No matching installed Steam games were found.";
  },
});

export const steamLaunch = defineTool({
  name: "steam_launch",
  description: "Launch a matching installed Steam game by title. If several titles match equally well, asks for a more specific title.",
  schema: z.object({ game: z.string().trim().min(1).max(200) }),
  risk: "safe",
  summary: ({ game }) => `launch ${game} in Steam`,
  run: async ({ game }) => {
    const query = normalize(game);
    const matches = (await installedGames()).map(item => ({
      ...item, score: item.id === game ? 4 : query ? matchScore(normalize(item.name), query) : 0,
    })).filter(item => item.score).sort((a, b) => b.score - a.score);
    const [best, next] = matches;
    if (!best) return `No installed Steam game matched "${game}".`;
    if (best.score === next?.score) return `Several games match. Which one? ${matches.filter(item => item.score === best.score).map(item => item.name).join(", ")}.`;
    const result = await runPs(`Start-Process 'steam://rungameid/${best.id}' -ErrorAction Stop`, 6_000);
    return result.code === 0 ? `Launching ${best.name} in Steam.` : `Steam could not launch ${best.name}.`;
  },
});

export const steamDeals = defineTool({
  name: "steam_deals",
  description: "List currently featured Steam discounts, with prices in the currency returned by the store.",
  schema: z.object({ count: z.number().int().min(1).max(20).optional() }),
  risk: "safe",
  summary: () => "check featured Steam deals",
  run: async ({ count = 5 }) => {
    try {
      const response = await fetch("https://store.steampowered.com/api/featuredcategories?l=english", { signal: AbortSignal.timeout(10_000) });
      if (!response.ok) return "The Steam Store is currently unavailable.";
      const parsed = z.object({ specials: z.object({ items: z.array(z.unknown()) }) }).safeParse(await response.json());
      if (!parsed.success) return "The Steam Store returned an unexpected deals format.";
      const itemSchema = z.object({ name: z.string(), discount_percent: z.number().positive(), final_price: z.number().nonnegative(), currency: z.string().regex(/^[A-Z]{3}$/) });
      const items = parsed.data.specials.items.flatMap(item => {
        const parsedItem = itemSchema.safeParse(item);
        return parsedItem.success ? [parsedItem.data] : [];
      }).slice(0, count);
      return items.length ? items.map(item => {
        const price = new Intl.NumberFormat(undefined, { style: "currency", currency: item.currency }).format(item.final_price / 100);
        return `${item.name} | ${item.discount_percent}% off | ${price}`;
      }).join("\n") : "Steam did not return any featured discounts.";
    } catch {
      return "Could not read the Steam Store deals.";
    }
  },
});
