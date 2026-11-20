import type { Routine, RoutineUpsert } from "../shared/settings";

const days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const weekdays = [1, 2, 3, 4, 5];

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

function label(text: string, control: HTMLElement) {
  const item = element("label");
  item.append(element("span", text), control);
  return item;
}

function scheduleSummary(schedule: Routine["schedule"]) {
  if (!schedule) return "On request";
  const selected = new Set(schedule.days);
  const description = selected.size === 7 ? "Every day"
    : selected.size === 5 && weekdays.every(day => selected.has(day)) ? "Weekdays"
      : days.filter((_, index) => selected.has(index)).map(day => day.slice(0, 3)).join(", ");
  return `${description} ${schedule.time}`;
}

function errorText(error: unknown, fallback: string) {
  if (!(error instanceof Error)) return fallback;
  return error.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, "") || fallback;
}

export function mountRoutines(root: HTMLElement) {
  const api = window.jarvis;
  const add = button("+ New routine");
  const header = element("header");
  header.className = "routines-header";
  header.append(element("h2", "Routines"), add);
  const status = element("p");
  status.className = "settings-status";
  status.role = "status";
  const editor = element("div");
  const list = element("div");
  list.className = "routine-list";
  root.append(header, status, editor, list);
  let editing = false;
  let revision = 0;
  let routines: Routine[] = [];
  const pending = new Set<number>();

  function report(text: string, error = false) {
    status.textContent = text;
    status.dataset.error = String(error);
  }

  async function reload() {
    const request = ++revision;
    try {
      const items = await api.invoke("routines:list");
      if (request !== revision) return;
      routines = items;
      renderList();
    } catch (error) {
      if (request === revision) report(errorText(error, "Could not load routines."), true);
    }
  }

  async function mutate(id: number, action: () => Promise<unknown>, success = "") {
    if (pending.has(id)) return;
    pending.add(id);
    renderList();
    report("");
    try {
      await action();
      report(success);
    } catch (error) {
      report(errorText(error, "Could not update the routine."), true);
    } finally {
      pending.delete(id);
      await reload();
      renderList();
    }
  }

  function renderList() {
    const rows = routines.map(routine => {
      const row = element("article");
      const heading = element("div");
      heading.append(element("h3", routine.name), element("p", scheduleSummary(routine.schedule)));
      const actions = element("div");
      actions.className = "actions";
      const enabled = element("input");
      enabled.type = "checkbox";
      enabled.checked = routine.enabled;
      enabled.disabled = editing || pending.has(routine.id);
      enabled.setAttribute("aria-label", `Enable ${routine.name}`);
      enabled.addEventListener("change", () => {
        const { id, name, triggers, instructions, schedule } = routine;
        void mutate(id, () => api.invoke("routines:upsert", { id, name, triggers, instructions, schedule, enabled: enabled.checked }));
      });
      const edit = button("Edit");
      const run = button("Run now");
      const remove = button("Delete");
      edit.disabled = run.disabled = remove.disabled = editing || pending.has(routine.id);
      edit.addEventListener("click", () => openEditor(routine));
      run.addEventListener("click", () => void mutate(routine.id, () => api.invoke("routines:run", routine.id), "Routine started."));
      remove.className = "danger";
      remove.addEventListener("click", () => {
        if (!window.confirm(`Delete “${routine.name}”?`)) return;
        void mutate(routine.id, () => api.invoke("routines:delete", routine.id), "Routine deleted.");
      });
      actions.append(label("Enabled", enabled), edit, run, remove);
      row.append(heading, actions);
      return row;
    });
    list.replaceChildren(...(rows.length ? rows : [element("p", "No routines yet.")]));
  }

  function openEditor(routine?: Routine) {
    if (editing) return;
    editing = add.disabled = true;
    renderList();
    report("");
    const form = element("form");
    form.className = "routine-edit";
    form.append(element("h3", routine ? "Edit routine" : "New routine"));
    const name = element("input");
    name.value = routine?.name ?? "";
    name.required = true;
    const triggers = element("textarea");
    triggers.value = routine?.triggers.join("\n") ?? "";
    triggers.placeholder = "How’s it looking this morning?";
    triggers.rows = 3;
    const instructions = element("textarea");
    instructions.value = routine?.instructions ?? "";
    instructions.required = true;
    instructions.rows = 4;
    const schedule = element("input");
    schedule.type = "checkbox";
    schedule.checked = Boolean(routine?.schedule);
    const timing = element("fieldset");
    timing.className = "routine-timing";
    timing.append(element("legend", "Schedule"));
    const time = element("input");
    time.type = "time";
    time.value = routine?.schedule?.time ?? "07:30";
    const choices = element("div");
    choices.className = "routine-days";
    const selectedDays = days.map((day, index) => {
      const checkbox = element("input");
      checkbox.type = "checkbox";
      checkbox.checked = routine?.schedule?.days.includes(index) ?? false;
      choices.append(label(day, checkbox));
      return checkbox;
    });
    const shortcuts = element("div");
    shortcuts.className = "actions";
    const week = button("Weekdays");
    const daily = button("Every day");
    week.addEventListener("click", () => selectedDays.forEach((box, index) => { box.checked = weekdays.includes(index); }));
    daily.addEventListener("click", () => selectedDays.forEach(box => { box.checked = true; }));
    shortcuts.append(week, daily);
    timing.append(label("Local time", time), choices, shortcuts);
    const showTiming = () => { timing.hidden = timing.disabled = !schedule.checked; time.required = schedule.checked; };
    schedule.addEventListener("change", showTiming);
    showTiming();
    const inline = element("p");
    inline.className = "settings-status";
    inline.dataset.error = "true";
    inline.role = "alert";
    const actions = element("div");
    actions.className = "actions";
    const save = button("Save routine");
    save.type = "submit";
    save.className = "primary";
    const cancel = button("Cancel");
    actions.append(save, cancel);
    form.append(label("Name", name), label("Trigger phrases (one per line)", triggers), label("Instructions", instructions),
      label("Run on a schedule", schedule), timing, inline, actions);

    function closeEditor() {
      editor.replaceChildren();
      editing = add.disabled = false;
      renderList();
    }
    cancel.addEventListener("click", closeEditor);
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (save.disabled) return;
      const selected = selectedDays.flatMap((box, day) => box.checked ? [day] : []);
      if (!name.value.trim() || !instructions.value.trim()) {
        inline.textContent = "Enter a name and instructions.";
        return;
      }
      if (schedule.checked && !selected.length) {
        inline.textContent = "Choose at least one day for the schedule.";
        return;
      }
      const value: RoutineUpsert = {
        id: routine?.id,
        name: name.value.trim(),
        triggers: triggers.value.split(/\r?\n/).map(text => text.trim()).filter(Boolean),
        instructions: instructions.value.trim(),
        schedule: schedule.checked ? { time: time.value, days: selected } : null,
        enabled: routine?.enabled ?? true,
      };
      inline.textContent = "";
      save.disabled = cancel.disabled = true;
      try {
        await api.invoke("routines:upsert", value);
        closeEditor();
        report("Routine saved.");
        await reload();
      } catch (error) {
        inline.textContent = errorText(error, "Could not save the routine.");
      } finally {
        save.disabled = cancel.disabled = false;
      }
    });
    editor.replaceChildren(form);
    name.focus();
  }

  add.addEventListener("click", () => openEditor());
  const unsubscribe = api.on(event => {
    if (event.type === "routines_changed") void reload();
  });
  window.addEventListener("pagehide", unsubscribe, { once: true });
  void reload();
}
