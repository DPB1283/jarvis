// Headlines from Google News RSS (free, no key, ~0.3 s): local, state, national and world.
// "Neutral" mode keeps national/world headlines to the big wire services (Reuters, AP, AFP), which
// media-bias raters consistently place nearest the centre and which mostly report facts rather than opinion.
import { z } from "zod";
import { getSettings } from "../config";
import { defineTool } from "./types";

const TOPICS = ["business", "technology", "science", "health", "sports", "entertainment"] as const;
const WIRE = /^(reuters|ap news|the associated press|associated press|afp|agence france-presse)$/i;
const WIRE_SEARCH = "(site:reuters.com OR site:apnews.com OR site:afp.com)";

interface Item {
  title: string;
  source: string;
  ageH: number;
}

function decode(s: string): string {
  return s
    .replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, "$1")
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#39;|&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .trim();
}

async function feed(path: string, gl: string): Promise<Item[]> {
  const sep = path.includes("?") ? "&" : "?";
  const res = await fetch(`https://news.google.com/rss${path}${sep}hl=en-${gl}&gl=${gl}&ceid=${gl}:en`, {
    signal: AbortSignal.timeout(10_000),
    headers: { "User-Agent": "Mozilla/5.0 (Jarvis desktop assistant)" },
  });
  if (!res.ok) throw new Error(`News feed returned ${res.status}`);
  const xml = await res.text();
  return [...xml.matchAll(/<item>([\s\S]*?)<\/item>/g)].map((m) => {
    const get = (tag: string) => decode(new RegExp(`<${tag}[^>]*>([\\s\\S]*?)</${tag}>`).exec(m[1])?.[1] ?? "");
    const source = get("source");
    const esc = source.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const title = source ? get("title").replace(new RegExp(`\\s+-\\s+${esc}$`), "") : get("title");
    return { title, source, ageH: (Date.now() - new Date(get("pubDate")).getTime()) / 3_600_000 };
  });
}

/** Home state/region from the home city, via the same free geocoder the weather tool uses. */
async function homeRegion(city: string): Promise<string | null> {
  const name = city.split(",")[0].trim();
  const res = await fetch(`https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(name)}&count=5&language=en&format=json`, {
    signal: AbortSignal.timeout(8_000),
  });
  const body = (await res.json()) as { results?: { admin1?: string; country?: string }[] };
  const hint = city.split(",").slice(1).join(" ").trim().toLowerCase();
  const r = body.results?.find((x) => hint && `${x.admin1} ${x.country}`.toLowerCase().includes(hint)) ?? body.results?.[0];
  return r?.admin1 ?? null;
}

const fmt = (items: Item[]) =>
  items
    .map((i) => {
      const age = Number.isFinite(i.ageH) ? (i.ageH < 1 ? `${Math.max(1, Math.round(i.ageH * 60))}m ago` : `${Math.round(i.ageH)}h ago`) : "";
      return `- ${i.title}${i.source ? ` (${i.source}${age ? `, ${age}` : ""})` : ""}`;
    })
    .join("\n");

export const getNews = defineTool({
  name: "get_news",
  description:
    "Current news headlines. scope: local (the user's home city), state (their state/region), national, world, or top. " +
    "Or give a topic or a search query. National/world use neutral wire services (Reuters, AP, AFP) unless the user prefers all sources. " +
    "Prefer this over web search for headlines; use web search/fetch only to dig into a story. Report headlines neutrally, without adding opinion.",
  schema: z.object({
    scope: z.enum(["top", "local", "state", "national", "world"]).optional(),
    topic: z.enum(TOPICS).optional(),
    query: z.string().optional().describe("Search instead, e.g. 'Formula 1' or 'city council'"),
    sources: z.enum(["neutral", "all"]).optional().describe("Default: the user's setting (neutral = wire services for national/world)"),
    count: z.number().int().min(1).max(15).optional(),
  }),
  risk: "safe",
  summary: (i) => `check the ${i.scope ?? i.topic ?? ""} news${i.query ? ` about ${i.query}` : ""}`.replace(/\s+/g, " "),
  run: async ({ scope, topic, query, sources, count = 6 }) => {
    const s = getSettings().location;
    const gl = (s?.newsRegion ?? "US").toUpperCase();
    const neutral = (sources ?? s?.newsSources ?? "neutral") === "neutral";
    const city = s?.city?.trim() ?? "";
    let label: string;
    let items: Item[];

    if (query) {
      label = `"${query}"`;
      items = await feed(`/search?q=${encodeURIComponent(`${query} when:3d`)}`, gl);
    } else if (topic) {
      label = topic;
      items = await feed(`/headlines/section/topic/${topic.toUpperCase()}`, gl);
    } else if (scope === "local" || scope === "state") {
      if (!city) return "No home city is set, so I can't get local news. Ask for their city and save it with change_settings.";
      if (scope === "local") {
        label = `local (${city})`;
        items = await feed(`/headlines/section/geo/${encodeURIComponent(city.split(",")[0].trim())}`, gl);
      } else {
        const region = await homeRegion(city);
        if (!region) return `Couldn't work out the state/region for ${city}.`;
        label = `state (${region})`;
        // Region names that Google reads as a city get a disambiguated search ("Washington" → D.C. news).
        const AMBIGUOUS: Record<string, string> = { Washington: "Washington state", Georgia: "Georgia state", "New York": "New York state" };
        items = AMBIGUOUS[region]
          ? await feed(`/search?q=${encodeURIComponent(`"${AMBIGUOUS[region]}" when:2d`)}`, gl)
          : await feed(`/headlines/section/geo/${encodeURIComponent(region)}`, gl);
        if (items.length < 3) items = await feed(`/search?q=${encodeURIComponent(`"${region}" when:1d`)}`, gl);
      }
    } else if (scope === "national" || scope === "world" || !scope || scope === "top") {
      const section = scope === "world" ? "WORLD" : scope === "national" ? "NATION" : null;
      label = scope ?? "top";
      items = section ? await feed(`/headlines/section/topic/${section}`, gl) : await feed("", gl);
      if (neutral) {
        // Keep wire-service stories; top up from a wire-only search if the section had too few.
        const wire = items.filter((i) => WIRE.test(i.source));
        if (wire.length < count) {
          const extra = await feed(`/search?q=${encodeURIComponent(`${WIRE_SEARCH} when:1d${scope === "world" ? " world" : ""}`)}`, gl);
          for (const e of extra) if (!wire.some((w) => w.title === e.title)) wire.push(e);
        }
        items = wire;
        label += ", wire services";
      }
    } else {
      label = "top";
      items = await feed("", gl);
    }

    items = items.filter((i) => i.title).slice(0, count);
    return items.length ? `Headlines (${label}):\n${fmt(items)}` : `No headlines found (${label}).`;
  },
});
