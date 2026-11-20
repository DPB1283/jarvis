import type { JarvisApi } from "../shared/events";

const keys = [
  { id: "ANTHROPIC_API_KEY", title: "Anthropic API key", help: "Required for Jarvis to answer. Create a key in the Anthropic Console." },
  { id: "ELEVENLABS_API_KEY", title: "ElevenLabs API key", help: "Required for voice input and speech. Create a key in your ElevenLabs account." },
  { id: "XWEATHER_CLIENT_ID", title: "Xweather client ID", help: "Optional. Find it in your Xweather account." },
  { id: "XWEATHER_CLIENT_SECRET", title: "Xweather client secret", help: "Optional. Find it in your Xweather account." },
] as const;

export function mountApiKeys(root: HTMLElement) {
  const api: JarvisApi = window.jarvis;
  const heading = document.createElement("h2");
  heading.textContent = "API keys";
  const introduction = document.createElement("p");
  introduction.textContent = "Keys are stored securely on this PC. Saved values cannot be displayed again.";
  const notice = document.createElement("p");
  notice.className = "settings-status";
  notice.setAttribute("role", "status");
  const list = document.createElement("div");
  list.className = "api-key-list";
  const chips = new Map<string, HTMLElement>();
  let revision = 0;

  async function refresh() {
    const current = ++revision;
    try {
      const status = await api.invoke("secrets:status");
      if (current !== revision) return;
      for (const [id, chip] of chips) {
        chip.textContent = status[id] ? "Saved" : "Not set";
        chip.dataset.saved = String(!!status[id]);
      }
    } catch {
      if (current === revision) notice.textContent = "Could not check API keys.";
    }
  }

  for (const key of keys) {
    const card = document.createElement("section");
    card.className = "api-key-card";
    const title = document.createElement("h3");
    title.textContent = key.title;
    const chip = document.createElement("span");
    chip.className = "api-key-status";
    chip.textContent = "Checking…";
    chips.set(key.id, chip);
    const help = document.createElement("p");
    help.textContent = key.help;
    const controls = document.createElement("div");
    controls.className = "api-key-controls";
    const field = document.createElement("input");
    field.type = "password";
    field.autocomplete = "off";
    field.spellcheck = false;
    field.setAttribute("aria-label", key.title);
    field.placeholder = "Enter a new value";
    const reveal = document.createElement("button");
    reveal.type = "button";
    reveal.textContent = "Show";
    reveal.setAttribute("aria-label", `Show ${key.title}`);
    reveal.addEventListener("click", () => {
      field.type = field.type === "password" ? "text" : "password";
      reveal.textContent = field.type === "password" ? "Show" : "Hide";
      reveal.setAttribute("aria-label", `${reveal.textContent} ${key.title}`);
    });
    const save = document.createElement("button");
    save.type = "button";
    save.textContent = "Save";
    save.addEventListener("click", async () => {
      if (!field.value.trim()) {
        notice.textContent = `Enter a value for ${key.title}.`;
        field.focus();
        return;
      }
      save.disabled = clear.disabled = true;
      notice.textContent = "";
      try {
        await api.invoke("secrets:set", key.id, field.value.trim());
        field.value = "";
        field.type = "password";
        reveal.textContent = "Show";
        reveal.setAttribute("aria-label", `Show ${key.title}`);
        notice.textContent = `${key.title} saved.`;
        await refresh();
      } catch {
        notice.textContent = `Could not save ${key.title}.`;
      } finally {
        save.disabled = clear.disabled = false;
      }
    });
    const clear = document.createElement("button");
    clear.type = "button";
    clear.textContent = "Clear";
    clear.addEventListener("click", async () => {
      save.disabled = clear.disabled = true;
      notice.textContent = "";
      try {
        await api.invoke("secrets:clear", key.id);
        field.value = "";
        notice.textContent = `${key.title} cleared.`;
        await refresh();
      } catch {
        notice.textContent = `Could not clear ${key.title}.`;
      } finally {
        save.disabled = clear.disabled = false;
      }
    });
    const row = document.createElement("div");
    row.className = "api-key-heading";
    row.append(title, chip);
    controls.append(field, reveal, save, clear);
    card.append(row, help, controls);
    list.append(card);
  }
  root.append(heading, introduction, notice, list);
  void refresh();
}
