// Live weather via Open-Meteo (free, no API key).
import { z } from "zod";
import { defineTool } from "./types";
import { getSettings } from "../config";

const CODES: Record<number, string> = {
  0: "clear", 1: "mainly clear", 2: "partly cloudy", 3: "overcast", 45: "fog", 48: "freezing fog",
  51: "light drizzle", 53: "drizzle", 55: "heavy drizzle", 56: "freezing drizzle", 57: "heavy freezing drizzle",
  61: "light rain", 63: "rain", 65: "heavy rain", 66: "freezing rain", 67: "heavy freezing rain",
  71: "light snow", 73: "snow", 75: "heavy snow", 77: "snow grains",
  80: "light showers", 81: "showers", 82: "violent showers", 85: "snow showers", 86: "heavy snow showers",
  95: "thunderstorm", 96: "thunderstorm with hail", 99: "severe thunderstorm with hail",
};

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
  if (!res.ok) throw new Error(`Weather service returned ${res.status}`);
  return (await res.json()) as T;
}

export const getWeather = defineTool({
  name: "get_weather",
  description:
    "Live current weather, a short daily forecast, and (when available) severe-weather alerts and air quality for a place. Prefer this over web search for anything weather-related. Always mention any active alerts.",
  schema: z.object({
    location: z.string().optional().describe("City or place name, e.g. 'London' or 'Austin, Texas'. Omit for the user's home city."),
    days: z.number().int().min(1).max(7).optional().describe("Forecast days including today (default 3)"),
    units: z.enum(["metric", "imperial"]).optional().describe("Omit to use the user's preference"),
  }),
  risk: "safe",
  summary: (i) => `check the weather${i.location ? ` in ${i.location}` : ""}`,
  run: async ({ location: requested, days, units: requestedUnits }) => {
    const home = getSettings().location;
    const location = requested?.trim() || home?.city?.trim();
    if (!location) return "No location given and no home city is set. Ask the user for their city (and offer to save it as home with change_settings).";
    const units = requestedUnits ?? home?.units;
    const [name, ...rest] = location.split(",").map((s) => s.trim());
    const geo = await getJson<{ results?: { name: string; admin1?: string; country?: string; latitude: number; longitude: number }[] }>(
      `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(name)}&count=10&language=en&format=json`,
    );
    const hint = rest.join(" ").toLowerCase();
    const place =
      geo.results?.find((r) => hint && `${r.admin1 ?? ""} ${r.country ?? ""}`.toLowerCase().includes(hint)) ?? geo.results?.[0];
    if (!place) return `Couldn't find a place called "${location}".`;
    const placeName = `${place.name}${place.admin1 ? `, ${place.admin1}` : ""}${place.country ? `, ${place.country}` : ""}`;

    // Xweather (if keys are set): adds severe-weather alerts and air quality. Falls back to Open-Meteo on any failure.
    let fallbackNote = "";
    if (process.env.XWEATHER_CLIENT_ID && process.env.XWEATHER_CLIENT_SECRET) {
      try {
        return await xweather(place.latitude, place.longitude, placeName, days ?? 3, units === "imperial");
      } catch (err) {
        fallbackNote = `\n(Xweather unavailable: ${err instanceof Error ? err.message : err}; used Open-Meteo instead.)`;
      }
    }

    const imperial = units === "imperial";
    const q = new URLSearchParams({
      latitude: String(place.latitude),
      longitude: String(place.longitude),
      current: "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,precipitation",
      daily: "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset",
      timezone: "auto",
      forecast_days: String(days ?? 3),
      ...(imperial && { temperature_unit: "fahrenheit", wind_speed_unit: "mph", precipitation_unit: "inch" }),
    });
    type W = {
      current: Record<string, number | string>;
      current_units: Record<string, string>;
      daily: Record<string, (number | string)[]>;
    };
    const w = await getJson<W>(`https://api.open-meteo.com/v1/forecast?${q}`);
    const c = w.current;
    const u = w.current_units;
    const lines = [
      `${place.name}${place.admin1 ? `, ${place.admin1}` : ""}${place.country ? `, ${place.country}` : ""} (local time ${c.time})`,
      `now: ${CODES[c.weather_code as number] ?? "unknown"}, ${c.temperature_2m}${u.temperature_2m} (feels like ${c.apparent_temperature}${u.apparent_temperature}), humidity ${c.relative_humidity_2m}%, wind ${c.wind_speed_10m} ${u.wind_speed_10m}, precipitation ${c.precipitation} ${u.precipitation}`,
      ...w.daily.time.map(
        (d, i) =>
          `${d}: ${CODES[w.daily.weather_code[i] as number] ?? "?"}, ${w.daily.temperature_2m_min[i]}–${w.daily.temperature_2m_max[i]}${u.temperature_2m}, rain chance ${w.daily.precipitation_probability_max[i]}%, sunrise ${String(w.daily.sunrise[i]).slice(11)}, sunset ${String(w.daily.sunset[i]).slice(11)}`,
      ),
    ];
    return lines.join("\n") + fallbackNote;
  },
});

// --- Xweather -------------------------------------------------------------------------------------
// Docs: https://www.xweather.com/docs/weather-api (endpoints: conditions, forecasts, alerts, airquality).
// Auth: client_id + client_secret query params. Location: "lat,long" as the :id.

type XwEnvelope<T> = { success: boolean; error: { code: string; description: string } | null; response: T[] | T };

async function xw<T>(endpoint: string, lat: number, lon: number, params: Record<string, string> = {}): Promise<T[]> {
  const q = new URLSearchParams({
    client_id: process.env.XWEATHER_CLIENT_ID!,
    client_secret: process.env.XWEATHER_CLIENT_SECRET!,
    ...params,
  });
  const res = await fetch(`https://data.api.xweather.com/${endpoint}/${lat.toFixed(4)},${lon.toFixed(4)}?${q}`, {
    signal: AbortSignal.timeout(10_000),
  });
  const body = (await res.json().catch(() => null)) as XwEnvelope<T> | null;
  if (!body) throw new Error(`HTTP ${res.status}`);
  if (!body.success) {
    if (body.error?.code === "warn_no_data") return [];
    const hint =
      body.error?.code === "invalid_client" ? " (check XWEATHER_CLIENT_ID / XWEATHER_CLIENT_SECRET)" :
      body.error?.code === "unauthorized_namespace" ? " (the app's namespace in your Xweather account doesn't allow this client)" : "";
    throw new Error(`${body.error?.code ?? res.status}${hint}`);
  }
  return Array.isArray(body.response) ? body.response : [body.response];
}

type XwPeriod = Record<string, number | string | null | undefined>;
type XwPlace = { place?: { name?: string; state?: string; country?: string }; profile?: { tz?: string }; periods?: XwPeriod[] };
type XwAlert = {
  details?: { name?: string; body?: string; priority?: number; emergency?: boolean };
  timestamps?: { beginsISO?: string; expiresISO?: string };
};

async function xweather(lat: number, lon: number, placeName: string, days: number, imperial: boolean): Promise<string> {
  const [cond, fc, alerts, aq] = await Promise.all([
    xw<XwPlace>("conditions", lat, lon),
    xw<XwPlace>("forecasts", lat, lon, { filter: "day", limit: String(days) }),
    xw<XwAlert>("alerts", lat, lon).catch(() => [] as XwAlert[]), // optional extras: never fail the whole call
    xw<XwPlace>("airquality", lat, lon).catch(() => [] as XwPlace[]),
  ]);
  const now = cond[0]?.periods?.[0];
  if (!now) throw new Error("no current conditions");
  const T = imperial ? "F" : "C";
  const wind = imperial ? "MPH" : "KPH";
  const gust = imperial ? "MPH" : "KPH";
  const pre = imperial ? "IN" : "MM";
  const n = (v: unknown, d = 0) => (typeof v === "number" ? v.toFixed(d) : "?");
  const time = (iso: unknown) => (typeof iso === "string" ? iso.slice(11, 16) : "?");

  const lines = [`${placeName} (Xweather)`];
  lines.push(
    `now: ${now.weather ?? "?"}, ${n(now[`temp${T}`])}°${T} (feels like ${n(now[`feelslike${T}`])}°${T}), humidity ${n(now.humidity)}%, ` +
      `wind ${now.windDir ?? ""} ${n(now[`windSpeed${wind}`])} ${wind.toLowerCase()}${now[`windGust${gust}`] ? ` gusting ${n(now[`windGust${gust}`])}` : ""}, ` +
      `UV ${n(now.uvi)}, chance of precipitation ${n(now.pop)}%`,
  );
  for (const d of fc[0]?.periods ?? []) {
    lines.push(
      `${String(d.dateTimeISO ?? "").slice(0, 10)}: ${d.weather ?? "?"}, ${n(d[`minTemp${T}`])}–${n(d[`maxTemp${T}`])}°${T}, ` +
        `chance of precipitation ${n(d.pop)}%${Number(d[`precip${pre}`]) > 0 ? ` (${n(d[`precip${pre}`], 2)} ${pre.toLowerCase()})` : ""}, ` +
        `sunrise ${time(d.sunriseISO)}, sunset ${time(d.sunsetISO)}`,
    );
  }
  const air = aq[0]?.periods?.[0];
  if (air?.aqi != null) lines.push(`air quality: AQI ${air.aqi} (${air.category ?? "?"}${air.dominant ? `, mainly ${air.dominant}` : ""})`);
  const active = alerts
    .filter((a) => a.details?.name)
    .sort((a, b) => (a.details?.priority ?? 99) - (b.details?.priority ?? 99))
    .slice(0, 3);
  lines.push(
    active.length
      ? `ALERTS: ${active.map((a) => `${a.details!.name}${a.details!.emergency ? " (EMERGENCY)" : ""} until ${a.timestamps?.expiresISO?.slice(0, 16).replace("T", " ") ?? "?"}: ${(a.details!.body ?? "").replace(/\s+/g, " ").slice(0, 220)}`).join(" | ")}`
      : "alerts: none active",
  );
  return lines.join("\n");
}
