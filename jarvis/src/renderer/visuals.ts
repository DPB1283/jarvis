import type { BusEvents } from "../shared/events";

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  return node;
}
function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}
const string = (value: unknown) => typeof value === "string" ? value : "";
const list = (value: unknown) => Array.isArray(value) ? value.map(record) : [];

function network(surface: HTMLElement, data: Record<string, unknown>, reduced: boolean) {
  const canvas = element("canvas");
  canvas.width = canvas.height = 480;
  canvas.setAttribute("aria-hidden", "true");
  surface.append(canvas);
  const tip = element("span");
  tip.className = "visual-link-label";
  tip.hidden = true;
  surface.append(tip);
  const context = canvas.getContext("2d");
  const nodes = list(data.nodes).filter(node => string(node.id)).slice(0, 40).map((node, i) => {
    const angle = i * 2.39996;
    const radius = 24 + 136 * Math.sqrt(i / Math.max(1, Math.min(40, list(data.nodes).length)));
    const center = node.id === data.center;
    const button = element("button", string(node.label) || string(node.id));
    button.type = "button";
    button.className = "visual-node";
    button.setAttribute("aria-pressed", "false");
    if (typeof node.group === "string") button.title = node.group;
    surface.append(button);
    return { id: node.id, center, button, x: center ? 240 : 240 + radius * Math.cos(angle), y: center ? 240 : 240 + radius * Math.sin(angle) };
  });
  const byId = new Map(nodes.map(node => [node.id, node]));
  const links = list(data.links).slice(0, 100).flatMap(link => {
    const from = byId.get(link.from), to = byId.get(link.to);
    return from && to ? [{ from, to, label: string(link.label) }] : [];
  });
  let selected: typeof nodes[number] | undefined;
  let frame = 0;
  function draw() {
    if (context) {
      context.clearRect(0, 0, 480, 480);
      context.lineWidth = 1.4;
      for (const { from, to } of links) {
        context.strokeStyle = !selected || selected === from || selected === to ? "rgba(190,205,224,.64)" : "rgba(190,205,224,.14)";
        context.beginPath();
        context.moveTo(from.x, from.y);
        context.lineTo(to.x, to.y);
        context.stroke();
      }
    }
    for (const node of nodes) {
      node.button.style.left = (node.x / 4.8) + "%";
      node.button.style.top = (node.y / 4.8) + "%";
      const connected = !selected || node === selected || links.some(link =>
        (link.from === selected && link.to === node) || (link.to === selected && link.from === node));
      node.button.classList.toggle("dim", !connected);
      node.button.setAttribute("aria-pressed", String(node === selected));
    }
  }
  for (const node of nodes) node.button.addEventListener("click", () => {
    selected = selected === node ? undefined : node;
    draw();
  });
  surface.onpointermove = event => {
    const bounds = surface.getBoundingClientRect();
    const x = (event.clientX - bounds.left) / bounds.width * 480;
    const y = (event.clientY - bounds.top) / bounds.height * 480;
    const link = links.find(({ from, to, label }) => {
      if (!label) return false;
      const dx = to.x - from.x, dy = to.y - from.y;
      const t = ((x - from.x) * dx + (y - from.y) * dy) / (dx * dx + dy * dy);
      return t > .15 && t < .85 && Math.hypot(x - from.x - t * dx, y - from.y - t * dy) < 8;
    });
    tip.hidden = !link;
    if (link) {
      tip.textContent = link.label;
      tip.style.left = x / 4.8 + "%";
      tip.style.top = y / 4.8 + "%";
    }
  };
  surface.onpointerleave = () => { tip.hidden = true; };
  function layout() {
    for (const a of nodes) {
      if (a.center) continue;
      for (const b of nodes) {
        if (a === b) continue;
        const dx = a.x - b.x, dy = a.y - b.y;
        const distance = Math.max(2, Math.hypot(dx, dy));
        const push = Math.max(0, (65 - distance) * .025);
        a.x += dx / distance * push;
        a.y += dy / distance * push;
      }
      for (const link of links) {
        const other = link.from === a ? link.to : link.to === a ? link.from : undefined;
        if (!other) continue;
        const dx = other.x - a.x, dy = other.y - a.y;
        const distance = Math.max(2, Math.hypot(dx, dy));
        const pull = (distance - 107) * .008;
        a.x += dx / distance * pull;
        a.y += dy / distance * pull;
      }
      const dx = a.x - 240, dy = a.y - 240;
      const limit = Math.min(1, 174 / Math.max(1, Math.hypot(dx, dy)));
      a.x = 240 + dx * limit;
      a.y = 240 + dy * limit;
    }
  }
  let animation = 0;
  function tick() {
    layout();
    draw();
    if (++frame < 95) animation = requestAnimationFrame(tick);
  }
  if (reduced) {
    for (let i = 0; i < 95; i++) layout();
    draw();
  } else tick();
  return () => {
    cancelAnimationFrame(animation);
    surface.onpointermove = surface.onpointerleave = null;
  };
}

export function mountVisuals(hud: HTMLElement, orb: HTMLElement, onOpen: (open: boolean) => void) {
  const overlay = element("dialog");
  overlay.className = "orb-visual";
  const header = element("header");
  const title = element("h2");
  title.id = "orb-visual-title";
  overlay.setAttribute("aria-labelledby", title.id);
  const closeButton = element("button", "×");
  closeButton.type = "button";
  closeButton.setAttribute("aria-label", "Close visual");
  header.append(title, closeButton);
  const surface = element("div");
  surface.className = "visual-surface";
  const caption = element("p");
  caption.className = "visual-caption";
  overlay.append(header, surface, caption);
  hud.append(overlay);
  const motion = matchMedia("(prefers-reduced-motion: reduce)");
  let expiry: ReturnType<typeof setTimeout> | undefined;
  let closing: ReturnType<typeof setTimeout> | undefined;
  let cleanup: (() => void) | undefined;
  function close(immediate = false) {
    clearTimeout(expiry);
    clearTimeout(closing);
    cleanup?.();
    cleanup = undefined;
    overlay.classList.remove("expanded");
    if (!overlay.open) return;
    function finish() {
      overlay.close();
      surface.replaceChildren();
      hud.removeAttribute("data-visual-open");
      onOpen(false);
    }
    if (immediate || motion.matches) finish();
    else closing = setTimeout(finish, 220);
  }
  closeButton.addEventListener("click", () => close());
  overlay.addEventListener("cancel", event => { event.preventDefault(); close(); });
  overlay.addEventListener("click", event => { if (event.target === overlay) close(); });

  function show(value: BusEvents["visual"]) {
    clearTimeout(expiry);
    clearTimeout(closing);
    cleanup?.();
    cleanup = undefined;
    surface.replaceChildren();
    const data = record(value.data);
    title.textContent = value.title;
    caption.textContent = "";
    surface.dataset.kind = value.kind;
    if (value.kind === "web") {
      cleanup = network(surface, data, motion.matches);
    } else if (value.kind === "timeline") {
      const timeline = element("ol");
      timeline.className = "visual-timeline";
      timeline.tabIndex = 0;
      timeline.setAttribute("aria-label", value.title + " timeline");
      for (const event of list(data.events).slice(0, 100)) {
        const item = element("li");
        const when = element("time", string(event.when));
        const label = element("strong", string(event.title));
        const detail = string(event.detail);
        if (detail) { item.title = detail; item.tabIndex = 0; item.setAttribute("aria-description", detail); }
        item.append(when, label);
        timeline.append(item);
      }
      surface.append(timeline);
    } else {
      const src = string(data.src);
      if (/^data:image\/(?:png|jpeg|gif|webp|avif);base64,[\w+/=]+$/i.test(src)) {
        const image = element("img");
        image.src = src;
        image.alt = string(data.caption) || value.title;
        surface.append(image);
      }
      caption.textContent = string(data.caption);
    }
    caption.hidden = !caption.textContent;
    if (!overlay.open) {
      const rect = orb.getBoundingClientRect();
      const size = Math.min(400, innerWidth - 60);
      overlay.style.setProperty("--visual-origin-x", (rect.x + rect.width / 2 - innerWidth / 2) + "px");
      overlay.style.setProperty("--visual-origin-y", (rect.y + rect.height / 2 - innerHeight / 2) + "px");
      overlay.style.setProperty("--visual-start", String(Math.max(.12, rect.width / size)));
      overlay.showModal();
      hud.setAttribute("data-visual-open", "");
      onOpen(true);
      overlay.getBoundingClientRect();
    }
    overlay.classList.add("expanded");
    if (typeof value.ttlSeconds === "number" && value.ttlSeconds > 0) {
      expiry = setTimeout(close, Math.min(600, value.ttlSeconds) * 1000);
    }
  }
  return { show, close, isOpen: () => overlay.open };
}
