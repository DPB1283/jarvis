import ical, { type EventInstance } from "node-ical";
import { z } from "zod";
import { getSettings } from "../config";
import { defineTool } from "./types";

const dayOptions = { weekday: "short", month: "short", day: "numeric" } as const;
const dayFormat = new Intl.DateTimeFormat(undefined, dayOptions);
const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

function formatDateOnly(date: EventInstance["start"]) {
  if (date.tz) {
    try {
      return new Intl.DateTimeFormat(undefined, { ...dayOptions, timeZone: date.tz }).format(date);
    } catch {
      // A custom VTIMEZONE may not have an Intl identifier.
    }
  }
  return dayFormat.format(date);
}

export async function parseCalendarAgenda(source: string, from: Date, to: Date): Promise<string[]> {
  const calendar = await ical.async.parseICS(source);
  const events: EventInstance[] = [];
  for (const event of Object.values(calendar)) {
    if (event?.type !== "VEVENT" || event.status === "CANCELLED" || !event.start) continue;
    try {
      events.push(...ical.expandRecurringEvent(event, { from, to, expandOngoing: true })
        .filter(instance => instance.start < to && instance.event.status !== "CANCELLED"));
    } catch {
      // A malformed individual event should not hide a readable calendar.
    }
  }
  return events.sort((a, b) => +a.start - +b.start).slice(0, 100).map(event => {
    const title = (typeof event.summary === "string" ? event.summary : event.summary?.val ?? "")
      .replace(/\s+/g, " ").trim() || "Untitled event";
    if (event.isFullDay) {
      const last = new Date(event.end.getTime() - 1) as EventInstance["start"];
      last.tz = event.end.tz ?? event.start.tz;
      const firstDay = formatDateOnly(event.start);
      const lastDay = event.end > event.start ? formatDateOnly(last) : firstDay;
      return `${firstDay}${lastDay !== firstDay ? `–${lastDay}` : ""} · All day · ${title}`;
    }
    const start = `${dayFormat.format(event.start)}, ${timeFormat.format(event.start)}`;
    const end = event.end > event.start
      ? `–${dayFormat.format(event.end) === dayFormat.format(event.start) ? "" : `${dayFormat.format(event.end)}, `}${timeFormat.format(event.end)}`
      : "";
    return `${start}${end} · ${title}`;
  });
}

export const calendarEvents = defineTool({
  name: "calendar_events",
  description: "Read the connected calendar and list upcoming events in local time for the next 1–14 calendar days.",
  schema: z.object({ days: z.number().int().min(1).max(14).optional() }),
  risk: "safe",
  summary: () => "read the upcoming calendar agenda",
  run: async ({ days = 1 }) => {
    const setting = getSettings().calendar?.icsUrl.trim();
    if (!setting) return "Connect your calendar in Jarvis Settings → Calendar. In Google Calendar, open Settings → select your calendar → Integrate calendar → Secret address in iCal format.";
    try {
      const url = new URL(setting);
      if (url.protocol !== "https:" || url.username || url.password) return "Enter an HTTPS iCal address in Jarvis Settings → Calendar.";
      const response = await fetch(url, { signal: AbortSignal.timeout(10_000), redirect: "error" });
      if (!response.ok) return "Could not read the calendar. Check the secret iCal address in Settings.";
      const source = await response.text();
      if (!/^BEGIN:VCALENDAR\s*$/im.test(source)) return "The calendar address did not return an iCal feed.";
      const from = new Date();
      from.setHours(0, 0, 0, 0);
      const to = new Date(from);
      to.setDate(to.getDate() + days);
      const events = await parseCalendarAgenda(source, from, to);
      return events.length ? events.join("\n") : "No calendar events were found in that period.";
    } catch {
      // Errors from fetch can include the private calendar URL.
      return "Could not load the calendar. Check the secret iCal address and your connection.";
    }
  },
});
