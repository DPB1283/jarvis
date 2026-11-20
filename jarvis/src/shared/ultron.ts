import mode from "./ultron.json";

const pattern = (source: string) => new RegExp(source.replaceAll("MODE", mode.mode));
const enable = pattern(mode.enable);
const disable = pattern(mode.disable);

export function ultronCommand(input: string): boolean | undefined {
  const command = input.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
  if (enable.test(command)) return true;
  if (disable.test(command)) return false;
}
