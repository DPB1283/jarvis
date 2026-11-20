let calendar = { icsUrl: "" };
export function setCalendar(value: typeof calendar) { calendar = value; }
export function getSettings() { return { calendar }; }
