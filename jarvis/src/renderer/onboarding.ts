import type { Settings } from "../shared/settings";

export function offerOnboarding(root: HTMLElement, preferences: Settings, openKeys: () => void) {
  const api = window.jarvis;
  const dialog = document.createElement("dialog");
  dialog.className = "onboarding";
  const title = document.createElement("h2");
  title.id = "onboarding-title";
  dialog.setAttribute("aria-labelledby", title.id);
  const detail = document.createElement("p");
  const actions = document.createElement("div");
  actions.className = "actions";
  const feedback = document.createElement("p");
  feedback.setAttribute("role", "status");
  const meter = document.createElement("meter");
  meter.setAttribute("aria-label", "Microphone level");
  meter.max = 1;
  dialog.append(title, detail, meter, feedback, actions);
  root.append(dialog);
  let step = 0;
  let checking = false;
  let generation = 0;
  function button(label: string, click: () => void, primary = false) {
    const control = document.createElement("button");
    control.type = "button";
    control.textContent = label;
    control.classList.toggle("primary", primary);
    control.addEventListener("click", click);
    actions.append(control);
    return control;
  }
  function localClose() {
    generation++;
    checking = false;
    dialog.close();
    dialog.remove();
    resume.remove();
  }
  async function finish() {
    generation++;
    checking = false;
    for (const button of actions.querySelectorAll("button")) button.disabled = true;
    try {
      await api.invoke("settings:update", { onboarded: true });
      localClose();
    } catch {
      feedback.textContent = "Could not save your setup. Try again.";
      for (const button of actions.querySelectorAll("button")) button.disabled = false;
    }
  }
  function next() { step++; render(); }
  async function test(control: HTMLButtonElement) {
    if (checking) { generation++; checking = false; control.textContent = "Test microphone"; return; }
    const request = ++generation;
    checking = true;
    control.textContent = "Stop test";
    meter.value = 0;
    feedback.textContent = "Speak and watch the meter.";
    try {
      while (generation === request) {
        const level = await api.invoke("audio:test-input", preferences.audio.inputDevice, preferences.audio.inputGain ?? 100);
        if (generation === request) meter.value = level;
      }
    } catch {
      if (generation === request) {
        checking = false;
        control.textContent = "Test microphone";
        feedback.textContent = "Check microphone access and the selected device in Voice & audio.";
      }
    }
  }
  function render() {
    generation++;
    checking = false;
    actions.replaceChildren();
    feedback.textContent = "";
    meter.hidden = step !== 2;
    if (step === 0) {
      title.textContent = "Welcome to Jarvis";
      detail.textContent = "Set up your connection, test your microphone, and see the shortcuts.";
      button("Get started", next, true);
    } else if (step === 1) {
      title.textContent = "Choose how to use Jarvis";
      detail.textContent = "An API key enables full AI replies. Free mode covers built-in commands like time, timers, and volume.";
      button("Open API keys", () => { dialog.close(); resume.hidden = false; openKeys(); }, true);
      button("Continue with current setup", next);
    } else if (step === 2) {
      title.textContent = "Check your microphone";
      detail.textContent = "Speak while the test runs. If the meter moves, Jarvis is receiving audio.";
      const control = button("Test microphone", () => void test(control));
      button("Continue", next, true);
    } else if (step === 3) {
      title.textContent = "Try a voice command";
      detail.textContent = 'Say “Jarvis, what time is it?” You can also type the question using the talk shortcut.';
      button("Continue", next, true);
    } else {
      title.textContent = "Talk and mute shortcuts";
      detail.textContent = "Talk with " + preferences.hotkeys.talk + ". Mute or unmute with " + (preferences.hotkeys.mute ?? "Control+Alt+M") + ".";
      button("Finish", () => void finish(), true);
    }
    button("Skip tour", () => void finish());
  }
  dialog.addEventListener("cancel", event => { event.preventDefault(); void finish(); });
  const resume = document.createElement("button");
  resume.type = "button";
  resume.hidden = true;
  resume.textContent = "Continue setup";
  const keysPanel = root.querySelector("#panel-api-keys");
  keysPanel?.prepend(resume);
  resume.addEventListener("click", () => { resume.hidden = true; next(); dialog.showModal(); });
  render();
  dialog.showModal();
}
