export const settingsTabs = [
  { id: "general", title: "General", preference: true },
  { id: "voice", title: "Voice & audio", preference: true },
  { id: "display", title: "Display", preference: true },
  { id: "controls", title: "Controls", preference: true },
  { id: "routines", title: "Routines", preference: false },
  { id: "timers", title: "Timers", preference: false },
  { id: "memory", title: "Memory", preference: false },
  { id: "messaging", title: "Messaging", preference: false },
  { id: "api-keys", title: "API keys", preference: false },
  { id: "browser", title: "Browser", preference: false },
] as const;

export type SettingsTab = (typeof settingsTabs)[number]["id"];
export type PreferenceTab = Extract<(typeof settingsTabs)[number], { preference: true }>["id"];

export function mountSettingsTabs(
  tablist: HTMLElement,
  form: HTMLFormElement,
  managed: HTMLElement,
  onSelect: (tab: SettingsTab, preference: boolean) => void,
) {
  const panels = new Map<SettingsTab, HTMLElement>();
  const buttons = new Map<SettingsTab, HTMLButtonElement>();
  let current: SettingsTab | undefined;

  function activate(id: SettingsTab, focus = false) {
    for (const tab of settingsTabs) {
      const selected = tab.id === id;
      panels.get(tab.id)!.hidden = !selected;
      const button = buttons.get(tab.id)!;
      button.tabIndex = selected ? 0 : -1;
      button.setAttribute("aria-selected", String(selected));
    }
    if (focus) buttons.get(id)?.focus();
    if (current && current !== id) window.scrollTo(0, 0);
    current = id;
    onSelect(id, settingsTabs.find(tab => tab.id === id)!.preference);
  }

  settingsTabs.forEach((tab, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.id = `tab-${tab.id}`;
    button.textContent = tab.title;
    button.setAttribute("role", "tab");
    button.setAttribute("aria-controls", `panel-${tab.id}`);
    button.addEventListener("click", () => activate(tab.id));
    button.addEventListener("keydown", event => {
      const last = settingsTabs.length - 1;
      let target: number;
      switch (event.key) {
        case "ArrowUp": target = index === 0 ? last : index - 1; break;
        case "ArrowDown": target = index === last ? 0 : index + 1; break;
        case "Home": target = 0; break;
        case "End": target = last; break;
        default: return;
      }
      event.preventDefault();
      activate(settingsTabs[target].id, true);
    });
    const panel = document.createElement("section");
    panel.className = tab.preference ? "settings-fields" : "settings-managed-panel";
    panel.id = `panel-${tab.id}`;
    panel.dataset.settingsTab = tab.id;
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", button.id);
    buttons.set(tab.id, button);
    panels.set(tab.id, panel);
    tablist.append(button);
    (tab.preference ? form : managed).append(panel);
  });

  form.addEventListener("invalid", event => {
    if (!(event.target instanceof HTMLElement)) return;
    const panel = event.target.closest<HTMLElement>("[data-settings-tab]");
    if (panel?.hidden) activate(panel.dataset.settingsTab as SettingsTab);
  }, true);
  activate("general");
  return { panel: (id: SettingsTab) => panels.get(id)!, activate };
}
