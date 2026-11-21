import type { JarvisApi } from "../shared/events";

// Ready-made settings for popular OpenAI-compatible services. All of them can be edited, and "Custom" is blank.
const presets: Array<{ id: string; name: string; baseUrl: string; model: string }> = [
  { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  { id: "gemini", name: "Google Gemini", baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.0-flash" },
  { id: "openrouter", name: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini" },
  { id: "groq", name: "Groq", baseUrl: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile" },
  { id: "ollama", name: "Ollama (on this PC, no key)", baseUrl: "http://localhost:11434/v1", model: "llama3.1" },
  { id: "custom", name: "Custom", baseUrl: "", model: "" },
];

const keys = [
  { id: "ANTHROPIC_API_KEY", title: "Anthropic API key (Claude)", help: "For Claude, the default AI. Create a key in the Anthropic Console." },
  { id: "AI_API_KEY", title: "Other AI service key", help: "For the other AI service chosen above (OpenAI, Gemini, OpenRouter, Groq...). A local server such as Ollama needs no key." },
  { id: "ELEVENLABS_API_KEY", title: "ElevenLabs API key", help: "Required for voice input and speech. Create a key in your ElevenLabs account." },
  { id: "MAPBOX_ACCESS_TOKEN", title: "Mapbox access token", help: "Optional. Lets Jarvis work out travel times and show maps. A free Mapbox account is enough; create a public access token (starts with pk.) on your Mapbox account page." },
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
  // --- Which AI answers --------------------------------------------------------------------------
  const provider = document.createElement("section");
  provider.className = "api-key-card";
  const providerTitle = document.createElement("h3");
  providerTitle.textContent = "Which AI answers";
  const providerHelp = document.createElement("p");
  providerHelp.textContent =
    "Claude is the default and does the most (it can also click and type for you). Any other AI that speaks the OpenAI format works too: add its address, a model name and its key below. With no AI at all, Jarvis runs in free mode: a few commands such as the time, timers and volume, answered in text only.";
  const field = (label: string, control: HTMLElement) => {
    const wrap = document.createElement("label");
    wrap.className = "setting";
    const name = document.createElement("span");
    name.textContent = label;
    control.setAttribute("aria-label", label);
    wrap.append(name, control);
    return wrap;
  };
  const kind = document.createElement("select");
  for (const [value, name] of [["anthropic", "Claude (Anthropic)"], ["openai", "Another AI (OpenAI-compatible)"]]) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = name;
    kind.append(o);
  }
  const preset = document.createElement("select");
  for (const p of presets) {
    const o = document.createElement("option");
    o.value = p.id;
    o.textContent = p.name;
    preset.append(o);
  }
  const baseUrl = document.createElement("input");
  baseUrl.placeholder = "https://api.example.com/v1";
  baseUrl.spellcheck = false;
  const model = document.createElement("input");
  model.placeholder = "Model name, e.g. gpt-4o-mini";
  model.spellcheck = false;
  const others = document.createElement("div");
  others.className = "provider-others";
  others.append(field("Service", preset), field("Address (base URL)", baseUrl), field("Model", model));
  const saveProvider = document.createElement("button");
  saveProvider.type = "button";
  saveProvider.textContent = "Save AI choice";
  const providerNotice = document.createElement("small");
  providerNotice.setAttribute("role", "status");
  const sync = () => { others.hidden = kind.value !== "openai"; };
  kind.addEventListener("change", sync);
  preset.addEventListener("change", () => {
    const chosen = presets.find(p => p.id === preset.value);
    if (chosen && chosen.id !== "custom") {
      baseUrl.value = chosen.baseUrl;
      model.value = chosen.model;
    }
  });
  saveProvider.addEventListener("click", async () => {
    const choice = { kind: kind.value as "anthropic" | "openai", baseUrl: baseUrl.value.trim(), model: model.value.trim() };
    if (choice.kind === "openai" && (!choice.baseUrl || !choice.model)) {
      providerNotice.textContent = "Fill in the address and the model name.";
      return;
    }
    try {
      await api.invoke("settings:update", { provider: choice });
      providerNotice.textContent = choice.kind === "openai" ? "Saved. Add that service's key below if it needs one." : "Saved. Jarvis will use Claude.";
    } catch {
      providerNotice.textContent = "Could not save.";
    }
  });
  provider.append(providerTitle, providerHelp, field("AI", kind), others, saveProvider, providerNotice);
  void api.invoke("settings:get").then(settings => {
    const current = settings.provider;
    kind.value = current?.kind ?? "anthropic";
    baseUrl.value = current?.baseUrl ?? "";
    model.value = current?.model ?? "";
    const match = presets.find(p => p.baseUrl && p.baseUrl === current?.baseUrl);
    preset.value = match?.id ?? "custom";
    sync();
  }).catch(() => {});
  sync();

  root.append(heading, introduction, notice, provider, list);
  void refresh();
}
