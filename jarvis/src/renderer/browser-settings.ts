import type { InvokeApi } from "../shared/events";

const browserNames = { edge: "Edge", chrome: "Chrome" } as const;
type SetupTarget = Parameters<InvokeApi["browser:setup"]>[0];

function button(text: string) {
  const control = document.createElement("button");
  control.type = "button";
  control.textContent = text;
  return control;
}

function readOnlyField(label: string, type = "text") {
  const field = document.createElement("label");
  field.className = "browser-setup-field";
  field.textContent = label;
  const input = document.createElement("input");
  input.type = type;
  input.readOnly = true;
  input.autocomplete = "off";
  input.spellcheck = false;
  field.append(input);
  return { field, input };
}

/** Share the install and pairing steps between the guided tour and Browser settings. */
export function mountBrowserSetup(root: HTMLElement, visible: () => boolean) {
  const api = window.jarvis;
  const setup = document.createElement("div");
  setup.className = "browser-setup";
  const actions = document.createElement("div");
  actions.className = "actions";
  const feedback = document.createElement("p");
  feedback.className = "settings-status";
  feedback.setAttribute("role", "status");
  const instructions = document.createElement("ol");
  instructions.className = "browser-setup-steps";
  for (const line of [
    "Choose a browser above to open its Extensions page. If you use Show folder, open the Extensions page yourself.",
    "Turn on Developer mode.",
    "Choose Load unpacked and paste the extension folder path. Jarvis copies it for you.",
    "Open the Jarvis extension from the browser toolbar and enter the port and pairing code below.",
  ]) {
    const item = document.createElement("li");
    item.textContent = line;
    instructions.append(item);
  }

  const status = document.createElement("p");
  status.className = "browser-setup-status";
  status.setAttribute("role", "status");
  status.textContent = "Checking connection…";
  const { field: portField, input: port } = readOnlyField("Port");
  const { field: codeField, input: code } = readOnlyField("Pairing code", "password");
  const credentials = document.createElement("div");
  credentials.className = "browser-setup-credentials";
  const show = button("Show pairing code");
  show.setAttribute("aria-pressed", "false");
  show.disabled = true;
  function showCode(reveal: boolean) {
    code.type = reveal ? "text" : "password";
    show.setAttribute("aria-pressed", String(reveal));
    show.textContent = reveal ? "Hide pairing code" : "Show pairing code";
  }
  show.addEventListener("click", () => showCode(code.type === "password"));
  credentials.append(portField, codeField, show);

  let stopped = false;
  let pending = false;
  let opening = false;
  let loaded = false;
  function setupButton(label: string, target: SetupTarget) {
    const control = button(label);
    control.disabled = opening;
    control.addEventListener("click", async () => {
      if (opening) return;
      opening = true;
      for (const item of actions.querySelectorAll("button")) item.disabled = true;
      feedback.textContent = "";
      try {
        await api.invoke("browser:setup", target);
        if (!stopped) feedback.textContent = "Extension folder path copied.";
      } catch {
        if (!stopped) feedback.textContent = target === "folder" ? "Could not open the extension folder." : "Could not open the browser. Try Show folder.";
      } finally {
        opening = false;
        for (const item of actions.querySelectorAll("button")) item.disabled = false;
      }
    });
    return control;
  }
  const folder = setupButton("Show folder", "folder");
  actions.append(folder);
  async function loadBrowsers() {
    try {
      const info = await api.invoke("browser:setup-info");
      if (stopped) return;
      if (info.folder) folder.title = info.folder;
      for (const browser of new Set(info.browsers)) {
        if (Object.hasOwn(browserNames, browser)) {
          folder.before(setupButton("Set up in " + browserNames[browser], browser));
        }
      }
    } catch {
      if (!stopped) feedback.textContent = "Could not check which browsers are installed.";
    }
  }
  async function update() {
    if (stopped) return;
    if (!visible()) {
      if (code.type === "text") showCode(false);
      return;
    }
    if (!loaded) {
      loaded = true;
      void loadBrowsers();
    }
    if (pending) return;
    pending = true;
    try {
      const result = await api.invoke("browser:status");
      if (stopped || !visible()) return;
      const portNumber = String(result.port);
      if (port.value !== portNumber) port.value = portNumber;
      if (code.value !== result.pairingCode) code.value = result.pairingCode;
      show.disabled = !code.value;
      const message = result.connected ? "Connected ✓" : "Not connected";
      status.dataset.connected = String(result.connected);
      if (status.textContent !== message) status.textContent = message;
    } catch {
      if (!stopped && visible()) {
        delete status.dataset.connected;
        status.textContent = "Browser connection status unavailable.";
      }
    } finally {
      pending = false;
    }
  }
  setup.append(actions, instructions, credentials, status, feedback);
  root.append(setup);
  const timer = setInterval(() => void update(), 2000);
  function dispose() {
    stopped = true;
    clearInterval(timer);
    code.value = "";
    setup.remove();
    window.removeEventListener("pagehide", dispose);
  }
  window.addEventListener("pagehide", dispose, { once: true });
  void update();
  return { dispose };
}

export function mountBrowserSettings(root: HTMLElement) {
  const heading = document.createElement("h2");
  heading.textContent = "Browser";
  const help = document.createElement("p");
  help.textContent = "Give Jarvis his own collapsed tab group for browser tasks.";
  root.append(heading, help);
  mountBrowserSetup(root, () => !root.hidden);
}
