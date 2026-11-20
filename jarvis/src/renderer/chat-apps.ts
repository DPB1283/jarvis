import type { ChatAppProfile } from "../shared/settings";

const comboFields = [
  ["searchKeys", "Search or quick switcher", "Escape; ctrl+k"],
  ["pickKeys", "Select the conversation", "Return"],
  ["newChatKeys", "Start a new AI chat", "ctrl+shift+o"],
  ["focusMessageKeys", "Focus the composer", "shift+Escape"],
  ["sendKeys", "Send the message", "Return"],
] as const;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string) {
  const item = document.createElement(tag);
  if (text) item.textContent = text;
  return item;
}

function button(text: string) {
  const item = element("button", text);
  item.type = "button";
  return item;
}

function errorText(error: unknown, fallback: string) {
  return error instanceof Error
    ? error.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, "") || fallback
    : fallback;
}

export function mountChatApps(root: HTMLElement) {
  const api = window.jarvis;
  const header = element("header");
  header.className = "routines-header";
  const add = button("Add profile");
  header.append(element("h2", "Messaging apps"), add);
  const status = element("p");
  status.className = "settings-status";
  status.role = "status";
  const editor = element("div");
  const list = element("div");
  list.className = "chat-app-list";
  root.append(header, status, editor, list);
  let profiles: { builtin: ChatAppProfile[]; custom: ChatAppProfile[] } = { builtin: [], custom: [] };
  let busy = false;

  function report(text: string, error = false) {
    status.textContent = text;
    status.dataset.error = String(error);
  }

  function rowFor(profile: ChatAppProfile, custom: boolean) {
    const row = element("details");
    row.className = "chat-app-profile";
    const summary = element("summary");
    summary.append(element("strong", profile.name), element("small", profile.kind === "assistant" ? "AI assistant" : "People"));
    const data = element("dl");
    const items: Array<[string, string]> = [
      ["Open with", profile.launch],
      ["Processes", profile.processNames.join(", ")],
      ["Conversation check", profile.verify === "title" ? "Window title" : "Screenshot"],
      ...comboFields.map(([key, name]): [string, string] => [name, profile[key]?.join(" → ") ?? ""]),
    ];
    for (const [name, value] of items) {
      if (value) data.append(element("dt", name), element("dd", value));
    }
    const actions = element("div");
    actions.className = "actions";
    const override = profiles.custom.find(item => item.id === profile.id);
    const edit = button(custom ? "Edit" : override ? "Edit customization" : "Customize");
    edit.disabled = busy;
    edit.addEventListener("click", () => openEditor(override ?? profile));
    actions.append(edit);
    if (custom) {
      const remove = button("Delete");
      remove.className = "danger";
      remove.disabled = busy;
      remove.addEventListener("click", async () => {
        if (busy || !window.confirm(`Delete the custom profile for “${profile.name}”?`)) return;
        busy = true;
        remove.disabled = add.disabled = true;
        try {
          await api.invoke("chatapps:delete", profile.id);
          report("Custom profile deleted.");
        } catch (error) {
          report(errorText(error, "Could not delete the profile."), true);
        } finally {
          busy = add.disabled = false;
          await reload();
        }
      });
      actions.append(remove);
    }
    row.append(summary, data);
    if (profile.notes) row.append(element("p", profile.notes));
    row.append(actions);
    return row;
  }

  function renderList() {
    const custom = element("section");
    custom.append(element("h3", "Custom profiles"), ...(profiles.custom.length
      ? profiles.custom.map(profile => rowFor(profile, true))
      : [element("p", "No custom profiles yet.")]));
    const builtin = element("section");
    builtin.append(element("h3", "Built-in profiles"), ...profiles.builtin.map(profile => rowFor(profile, false)));
    list.replaceChildren(custom, builtin);
  }

  async function reload() {
    try {
      profiles = await api.invoke("chatapps:list");
      renderList();
    } catch (error) {
      report(errorText(error, "Could not load messaging profiles."), true);
    }
  }

  function openEditor(profile?: ChatAppProfile) {
    if (busy) return;
    busy = add.disabled = true;
    renderList();
    report("");
    const form = element("form");
    form.className = "chat-app-edit";
    form.append(element("h3", profile ? `Edit ${profile.name}` : "New messaging profile"));

    function field(title: string, control: HTMLElement) {
      const label = element("label");
      label.append(element("span", title), control);
      control.setAttribute("aria-label", title);
      form.append(label);
    }

    function input(title: string, value: string, placeholder = "", required = false) {
      const control = element("input");
      Object.assign(control, { type: "text", value, placeholder, required });
      field(title, control);
      return control;
    }

    function select(title: string, value: string, options: Array<[string, string]>) {
      const control = element("select");
      control.append(...options.map(([key, label]) => new Option(label, key)));
      control.value = value;
      field(title, control);
      return control;
    }

    const name = input("Name", profile?.name ?? "", "Example Chat", true);
    const kind = select("App type", profile?.kind ?? "people", [["people", "People"], ["assistant", "AI assistant"]]);
    const aliases = input("Aliases", profile?.aliases.join(", ") ?? "", "example chat, example");
    const processes = input("Process names", profile?.processNames.join(", ") ?? "", "ExampleChat, chrome", true);
    const launch = input("Start menu name or URL", profile?.launch ?? "", "Example Chat", true);
    const verify = select("Verify the conversation with", profile?.verify ?? "screenshot", [["screenshot", "Screenshot"], ["title", "Window title"]]);
    form.append(element("p", "Separate successive keyboard shortcuts with a semicolon. Leave steps blank if the shortcut is unavailable."));
    const shortcuts = comboFields.map(([key, label, placeholder]) => input(label, profile?.[key]?.join("; ") ?? "", placeholder));
    const notes = input("Notes", profile?.notes ?? "");
    const inline = element("p");
    inline.className = "settings-status";
    inline.dataset.error = "true";
    inline.role = "alert";
    const actions = element("div");
    actions.className = "actions";
    const save = button("Save profile");
    save.type = "submit";
    save.className = "primary";
    const cancel = button("Cancel");
    actions.append(save, cancel);
    form.append(inline, actions);

    function close() {
      busy = add.disabled = false;
      editor.replaceChildren();
      renderList();
    }

    cancel.addEventListener("click", close);
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (save.disabled) return;
      const split = (value: string, separator: string) => value.split(separator).map(item => item.trim()).filter(Boolean);
      const id = profile?.id ?? name.value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
      if (!profile && [...profiles.builtin, ...profiles.custom].some(item => item.id === id)) {
        inline.textContent = "A profile with that name already exists. Open it to edit or customize it.";
        return;
      }
      const [searchKeys, pickKeys, newChatKeys, focusMessageKeys, sendKeys] = shortcuts.map(control => split(control.value, ";"));
      const value: ChatAppProfile = {
        id, name: name.value.trim(), kind: kind.value === "assistant" ? "assistant" : "people",
        aliases: split(aliases.value, ","), processNames: split(processes.value, ","), launch: launch.value.trim(),
        launchWaitMs: profile?.launchWaitMs, searchWaitMs: profile?.searchWaitMs,
        searchKeys, pickKeys, newChatKeys, focusMessageKeys, sendKeys,
        verify: verify.value === "title" ? "title" : "screenshot", notes: notes.value.trim(),
      };
      inline.textContent = "";
      save.disabled = cancel.disabled = true;
      try {
        await api.invoke("chatapps:upsert", value);
        close();
        report("Profile saved.");
        await reload();
      } catch (error) {
        inline.textContent = errorText(error, "Could not save the profile.");
      } finally {
        save.disabled = cancel.disabled = false;
      }
    });
    editor.replaceChildren(form);
    name.focus();
  }
  add.addEventListener("click", () => openEditor());
  void reload();
}
