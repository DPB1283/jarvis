import { readFileSync } from "node:fs";
import { calendarEvents, calendarUpcoming } from "../../jarvis/src/main/tools/calendar";
import { steamDeals, steamGames, steamLaunch } from "../../jarvis/src/main/tools/steam";
import { youtubeSearch } from "../../jarvis/src/main/tools/youtube";
import { setCalendar } from "./utility-settings";

import { writeDocument } from "../../jarvis/src/main/tools/writing";
import { findFilesSmart } from "../../jarvis/src/main/tools/file-search";

const tools = [writeDocument, findFilesSmart, calendarEvents, calendarUpcoming, steamDeals, steamGames, steamLaunch, youtubeSearch];
async function main() {
  const { name, input, calendar } = JSON.parse(readFileSync(0, "utf8"));
  const tool = tools.find(tool => tool.name === name);
  if (!tool) throw new Error("Unknown utility");
  setCalendar(calendar);
  const result = await tool.run(tool.schema.parse(input) as never);
  process.stdout.write(JSON.stringify(result));
}
main().catch(() => {
  process.stderr.write("The requested utility could not complete.");
  process.exitCode = 1;
});
