import "./tauri-bridge";
import { PALETTE } from "../shared/settings";
import { mountHud } from "./hud";
import { mountPlayer } from "./player";
import { mountSettings } from "./settings";

for (const [name, color] of Object.entries(PALETTE)) {
  document.documentElement.style.setProperty(`--${name}`, color);
}

if (location.hash === "#pointer") {
  const pointer = document.querySelector<HTMLElement>("#virtual-pointer")!;
  const name = pointer.querySelector<HTMLElement>(".pointer-name")!;
  pointer.hidden = false;
  const update = (ultron: boolean) => { pointer.dataset.ultron = String(ultron); name.textContent = ultron ? "Ultron" : "Jarvis"; };
  void window.jarvis.invoke("settings:get").then(settings => update(settings.ultronMode));
  window.jarvis.on(event => { if (event.type === "settings_changed") update(event.settings.ultronMode); });
} else if (location.hash === "#player") {
  mountPlayer(); // hidden audio window, no preload / window.jarvis here
} else if (location.hash === "#settings") {
  mountSettings();
} else {
  mountHud();
}
