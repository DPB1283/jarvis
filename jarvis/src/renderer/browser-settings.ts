import type { JarvisApi } from "../shared/events";

export function mountBrowserSettings(root: HTMLElement) {
  const api: JarvisApi = window.jarvis;
  const heading = document.createElement("h2");
  heading.textContent = "Browser";
  const help = document.createElement("p");
  help.textContent = "Pair the Jarvis browser extension to give Jarvis a separate tab group.";
  const status = document.createElement("p");
  status.setAttribute("role", "status");
  const port = document.createElement("input");
  port.readOnly = true;
  port.setAttribute("aria-label", "Browser bridge port");
  const code = document.createElement("input");
  code.type = "password";
  code.readOnly = true;
  code.setAttribute("aria-label", "Browser pairing code");
  const show = document.createElement("button");
  show.type = "button";
  show.textContent = "Show pairing code";
  show.addEventListener("click", () => {
    code.type = code.type === "password" ? "text" : "password";
    show.textContent = code.type === "password" ? "Show pairing code" : "Hide pairing code";
  });
  const labelPort = document.createElement("label");
  labelPort.textContent = "Port";
  labelPort.append(port);
  const labelCode = document.createElement("label");
  labelCode.textContent = "Pairing code";
  labelCode.append(code);
  const instructions = document.createElement("p");
  instructions.textContent = "In Chrome or Edge, open Extensions, enable Developer mode, and load the browser-extension folder unpacked. Open the extension and enter this port and pairing code.";
  const refresh = document.createElement("button");
  refresh.type = "button";
  refresh.textContent = "Refresh connection";
  let active = true;
  async function update() {
    if (root.hidden) return;
    try {
      const result = await api.invoke("browser:status");
      if (!active) return;
      port.value = String(result.port);
      code.value = result.pairingCode;
      status.textContent = result.connected ? "Connected" : "Not connected";
    } catch {
      if (active) status.textContent = "Browser bridge unavailable.";
    }
  }
  refresh.addEventListener("click", () => void update());
  const timer = setInterval(() => void update(), 3000);
  window.addEventListener("pagehide", () => { active = false; clearInterval(timer); }, { once: true });
  root.append(heading, help, status, labelPort, labelCode, show, instructions, refresh);
  void update();
}
