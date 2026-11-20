import type { MemoryItem, MemoryKind } from "../shared/settings";

const kinds: Array<[MemoryKind, string]> = [
  ["fact", "Fact"], ["note", "Note"], ["todo", "To-do"], ["reminder", "Reminder"],
];

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string) {
  const result = document.createElement(tag);
  if (text) result.textContent = text;
  return result;
}

function label(text: string, control: HTMLElement) {
  const result = element("label");
  control.setAttribute("aria-label", text);
  result.append(element("span", text), control);
  return result;
}

function localDate(iso: string | null) {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}

export function mountMemory(root: HTMLElement) {
  const api = window.jarvis;
  const filter = element("select");
  filter.append(new Option("All types", ""), ...kinds.map(([value, name]) => new Option(name, value)));
  const query = element("input");
  query.type = "search";
  query.placeholder = "Search memory…";
  const controls = element("div");
  controls.className = "memory-controls";
  const refresh = element("button", "Refresh");
  const add = element("button", "Add item");
  const clear = element("button", "Forget everything");
  clear.className = "danger";
  controls.append(label("Search", query), label("Type", filter), refresh, add, clear);
  const status = element("p");
  status.role = "status";
  status.className = "settings-status";
  const editor = element("div");
  const list = element("div");
  list.className = "memory-list";
  root.append(element("h2", "Memory"), controls, status, editor, list);
  let revision = 0;
  let timer: ReturnType<typeof setTimeout>;
  let editing = false;

  function report(message: string, error = false) {
    status.textContent = message;
    status.dataset.error = String(error);
  }

  async function reload() {
    const request = ++revision;
    try {
      const items = await api.invoke("memory:list", {
        kind: filter.value ? filter.value as MemoryKind : undefined,
        query: query.value.trim() || undefined,
      });
      if (request !== revision) return;
      const rows = items.map(item => {
        const article = element("article");
        const type = kinds.find(([kind]) => kind === item.kind)?.[1] ?? item.kind;
        const meta = element("small", [type, item.done && item.kind === "todo" ? "Done" : "",
          item.dueAt && item.kind === "reminder" ? new Date(item.dueAt).toLocaleString() : ""].filter(Boolean).join(" · "));
        const actions = element("div");
        actions.className = "actions";
        const edit = element("button", "Edit");
        const remove = element("button", "Delete");
        edit.addEventListener("click", () => {
          if (!editing) openEditor(article, item);
        });
        remove.addEventListener("click", async () => {
          if (editing) return;
          remove.disabled = true;
          try {
            await api.invoke("memory:delete", item.id);
            await reload();
          } catch {
            report("Could not delete this item.", true);
          } finally {
            remove.disabled = false;
          }
        });
        if (item.imagePath) {
          const open = element("button", "Open screenshot");
          open.type = "button";
          open.addEventListener("click", async () => {
            open.disabled = true;
            report("");
            try {
              if (!(await api.invoke("memory:open-image", item.id))) {
                report("Could not open the screenshot. It may have been moved or deleted.", true);
              }
            } catch {
              report("Could not open the screenshot.", true);
            } finally {
              open.disabled = false;
            }
          });
          actions.append(open);
        }
        actions.append(edit, remove);
        article.append(meta, element("p", item.text), actions);
        return article;
      });
      if (!editing) list.replaceChildren(...(rows.length ? rows : [element("p", "No matching items.")]));
    } catch {
      if (request === revision) report("Could not load memory.", true);
    }
  }

  function openEditor(container: HTMLElement, item?: MemoryItem) {
    editing = true;
    report("");
    const form = element("form");
    form.className = "memory-edit";
    const kind = element("select");
    kind.append(...kinds.map(([value, name]) => new Option(name, value)));
    kind.value = item?.kind ?? (filter.value || "fact");
    const text = element("textarea");
    text.value = item?.text ?? "";
    text.required = true;
    text.rows = 3;
    const done = element("input");
    done.type = "checkbox";
    done.checked = item?.done ?? false;
    const doneField = label("Done", done);
    const due = element("input");
    due.type = "datetime-local";
    due.value = localDate(item?.dueAt ?? null);
    const dueField = label("Due", due);
    const actions = element("div");
    actions.className = "actions";
    const save = element("button", "Save item");
    save.className = "primary";
    save.type = "submit";
    const cancel = element("button", "Cancel");
    cancel.type = "button";
    actions.append(save, cancel);
    form.append(label("Type", kind), label("Text", text), doneField, dueField, actions);
    function toggleFields() {
      doneField.hidden = kind.value !== "todo";
      dueField.hidden = kind.value !== "reminder";
      due.required = !dueField.hidden;
    }
    function close() {
      editing = false;
      form.remove();
      void reload();
    }
    toggleFields();
    kind.addEventListener("change", toggleFields);
    cancel.addEventListener("click", close);
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (!text.value.trim()) {
        report("Enter text for this item.", true);
        return;
      }
      save.disabled = cancel.disabled = true;
      try {
        await api.invoke("memory:upsert", {
          id: item?.id,
          kind: kind.value as MemoryKind,
          text: text.value.trim(),
          done: kind.value === "todo" && done.checked,
          dueAt: kind.value === "reminder" ? new Date(due.value).toISOString() : null,
        });
        close();
      } catch {
        report("Could not save this memory item.", true);
      } finally {
        save.disabled = cancel.disabled = false;
      }
    });
    container.replaceChildren(form);
    text.focus();
  }

  refresh.addEventListener("click", () => void reload());
  add.addEventListener("click", () => { if (!editing) openEditor(editor); });
  filter.addEventListener("change", () => void reload());
  query.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(() => void reload(), 175);
  });
  clear.addEventListener("click", async () => {
    if (editing || !window.confirm("Delete all facts, notes, to-dos, and reminders?")) return;
    clear.disabled = true;
    try {
      const removed = await api.invoke("memory:clear");
      report(`Deleted ${removed} ${removed === 1 ? "item" : "items"}.`);
      await reload();
    } catch {
      report("Could not clear memory.", true);
    } finally {
      clear.disabled = false;
    }
  });
  window.addEventListener("focus", () => { if (!editing) void reload(); });
  void reload();
}
