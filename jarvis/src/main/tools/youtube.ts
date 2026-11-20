import { z } from "zod";
import { defineTool } from "./types";

type Data = Record<string, unknown>;
const object = (value: unknown): Data => value && typeof value === "object" ? value as Data : {};

function text(value: unknown): string {
  if (typeof value === "string") return value.replace(/\s+/g, " ").trim();
  const { simpleText, runs } = object(value);
  if (typeof simpleText === "string") return text(simpleText);
  return Array.isArray(runs) ? text(runs.map(run => object(run).text ?? "").join("")) : "";
}

export function parseYouTubeResults(html: string, count: number, includeAll: boolean): string[] {
  const marker = /(?:\bytInitialData|["']ytInitialData["']\s*\])\s*=\s*(\{)/.exec(html);
  if (!marker) throw new Error("Search data is unavailable.");
  const start = marker.index + marker[0].length - 1;
  let depth = 0;
  let quoted = false;
  let escaped = false;
  let end = start;
  for (; end < html.length; end++) {
    const char = html[end];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') quoted = false;
    } else if (char === '"') quoted = true;
    else if (char === "{") depth++;
    else if (char === "}" && --depth === 0) break;
  }
  const data: unknown = JSON.parse(html.slice(start, end + 1));
  const pending = [object(data).contents ?? data];
  const seen = new Set<string>();
  const results: string[] = [];
  function add(url: string, fields: unknown[]) {
    if (seen.has(url)) return;
    seen.add(url);
    results.push([...fields.map(value => text(value) || "—"), url].join(" | "));
  }
  while (pending.length && results.length < count) {
    const value = pending.pop();
    if (Array.isArray(value)) {
      for (let index = value.length - 1; index >= 0; index--) pending.push(value[index]);
      continue;
    }
    const record = object(value);
    if (Object.keys(record).some(key => /promoted|adSlot|searchPyv|inFeedAd/i.test(key))) continue;
    if (record.videoRenderer) {
      const video = object(record.videoRenderer);
      const id = video.videoId;
      const endpoint = object(object(video.navigationEndpoint).commandMetadata).webCommandMetadata;
      const short = String(object(endpoint).url ?? "").startsWith("/shorts/") ||
        JSON.stringify(video.thumbnailOverlays ?? []).includes('"SHORTS"');
      if ((!short || includeAll) && typeof id === "string" && /^[\w-]{11}$/.test(id)) {
        add(`https://www.youtube.com/watch?v=${id}`, [
          video.title, video.ownerText ?? video.longBylineText, video.lengthText,
          video.viewCountText ?? video.shortViewCountText, video.publishedTimeText,
        ]);
      }
      continue;
    }
    if (includeAll && record.playlistRenderer) {
      const list = object(record.playlistRenderer);
      if (typeof list.playlistId === "string" && /^[\w-]+$/.test(list.playlistId)) {
        add(`https://www.youtube.com/playlist?list=${list.playlistId}`, [list.title, list.longBylineText, "Playlist", "", ""]);
      }
      continue;
    }
    const lockup = object(record.lockupViewModel);
    if (includeAll && lockup.contentType === "LOCKUP_CONTENT_TYPE_PLAYLIST" &&
        typeof lockup.contentId === "string" && /^[\w-]+$/.test(lockup.contentId)) {
      const title = object(object(object(lockup.metadata).lockupMetadataViewModel).title).content;
      add(`https://www.youtube.com/playlist?list=${lockup.contentId}`, [title, "", "Playlist", "", ""]);
      continue;
    }
    if (includeAll && record.shortsLockupViewModel) {
      const short = object(record.shortsLockupViewModel);
      const endpoint = object(object(object(short.onTap).innertubeCommand).reelWatchEndpoint);
      const overlay = object(short.overlayMetadata);
      if (typeof endpoint.videoId === "string" && /^[\w-]{11}$/.test(endpoint.videoId)) {
        add(`https://www.youtube.com/watch?v=${endpoint.videoId}`, [
          object(overlay.primaryText).content, "", "Short", object(overlay.secondaryText).content, "",
        ]);
      }
      continue;
    }
    for (const [key, child] of Object.entries(record).reverse()) {
      if (!includeAll && /shelf|shorts|reel|playlist/i.test(key)) continue;
      if (child && typeof child === "object") pending.push(child);
    }
  }
  return results;
}

export const youtubeSearch = defineTool({
  name: "youtube_search",
  description: "Search YouTube for videos. Use kind any to include Shorts and playlists. Returns links without opening them.",
  schema: z.object({ query: z.string().trim().min(1).max(300), count: z.number().int().min(1).max(10).optional(), kind: z.enum(["video", "any"]).optional() }),
  risk: "safe",
  summary: ({ query }) => `search YouTube for "${query}"`,
  run: async ({ query, count = 5, kind = "video" }) => {
    const url = new URL("https://www.youtube.com/results");
    url.search = new URLSearchParams({ search_query: query, hl: "en", gl: "US" }).toString();
    try {
      const response = await fetch(url, {
        signal: AbortSignal.timeout(10_000),
        headers: { "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36", "Accept-Language": "en-US,en;q=0.9" },
      });
      if (!response.ok) return "YouTube search is currently unavailable.";
      const results = parseYouTubeResults(await response.text(), count, kind === "any");
      return results.length ? results.join("\n") : "No matching YouTube videos were found.";
    } catch {
      return "YouTube search could not be read. YouTube may be showing a consent page or a different results layout.";
    }
  },
});
