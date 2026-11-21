import type { BusEvents } from "../shared/events";
import { createHolo, type Holo, type Vec3 } from "./holo";
import { holoTimeline } from "./holo-timeline";

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string, className?: string) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}
const string = (value: unknown) => typeof value === "string" ? value : "";
const list = (value: unknown) => Array.isArray(value) ? value.map(record) : [];
const IDLE_SPIN = .12;

type Layout = "normal" | "large" | "closed";

/** Lay the nodes out in 3D inside the sphere: linked nodes pull together, all nodes push apart. */
function layout3d(count: number, links: [number, number][], centerIndex: number) {
  const points: Vec3[] = [];
  for (let i = 0; i < count; i++) {
    const y = 1 - (i + .5) * 2 / count;
    const ring = Math.sqrt(1 - y * y);
    const angle = i * 2.399963229728653;
    points.push({ x: Math.cos(angle) * ring * .65, y: y * .65, z: Math.sin(angle) * ring * .65 });
  }
  if (centerIndex >= 0) points[centerIndex] = { x: 0, y: 0, z: 0 };
  const rest = Math.min(.62, Math.max(.36, 1.25 / Math.sqrt(count)));
  for (let round = 0; round < 320; round++) {
    const heat = 1 - round / 320;
    const move = points.map(() => ({ x: 0, y: 0, z: 0 }));
    for (let a = 0; a < count; a++) {
      for (let b = a + 1; b < count; b++) {
        const dx = points[a].x - points[b].x, dy = points[a].y - points[b].y, dz = points[a].z - points[b].z;
        const distance = Math.max(.05, Math.hypot(dx, dy, dz));
        const push = Math.min(.06, .016 / (distance * distance));
        move[a].x += dx / distance * push; move[a].y += dy / distance * push; move[a].z += dz / distance * push;
        move[b].x -= dx / distance * push; move[b].y -= dy / distance * push; move[b].z -= dz / distance * push;
      }
    }
    for (const [a, b] of links) {
      const dx = points[b].x - points[a].x, dy = points[b].y - points[a].y, dz = points[b].z - points[a].z;
      const distance = Math.max(.01, Math.hypot(dx, dy, dz));
      const pull = (distance - rest) * .09;
      move[a].x += dx / distance * pull; move[a].y += dy / distance * pull; move[a].z += dz / distance * pull;
      move[b].x -= dx / distance * pull; move[b].y -= dy / distance * pull; move[b].z -= dz / distance * pull;
    }
    for (let i = 0; i < count; i++) {
      if (i === centerIndex) continue;
      const p = points[i];
      p.x += move[i].x * (.4 + heat); p.y += move[i].y * (.4 + heat); p.z += move[i].z * (.4 + heat);
      const r = Math.hypot(p.x, p.y, p.z);
      if (r > .8) { p.x *= .8 / r; p.y *= .8 / r; p.z *= .8 / r; }
    }
    if (centerIndex < 0) {
      const mean = points.reduce((sum, p) => ({ x: sum.x + p.x / count, y: sum.y + p.y / count, z: sum.z + p.z / count }), { x: 0, y: 0, z: 0 });
      for (const p of points) { p.x -= mean.x; p.y -= mean.y; p.z -= mean.z; }
    }
  }
  return points;
}

/** The rotation that brings a point to face the viewer, nearest to the current one. */
function facing(p: Vec3, yaw: number) {
  const wanted = Math.atan2(-p.x, p.z);
  const turn = Math.atan2(Math.sin(wanted - yaw), Math.cos(wanted - yaw));
  return { yaw: yaw + turn, pitch: Math.atan2(-p.y, Math.hypot(p.x, p.z)) };
}

function network(surface: HTMLElement, holo: Holo, data: Record<string, unknown>, reduced: boolean) {
  const layer = element("div", undefined, "visual-web");
  const canvas = element("canvas");
  canvas.setAttribute("aria-hidden", "true");
  const tip = element("span", undefined, "visual-link-label");
  tip.hidden = true;
  layer.append(canvas);
  surface.append(layer);
  const context = canvas.getContext("2d");
  const raw = list(data.nodes).filter(node => string(node.id)).slice(0, 40);
  const nodes = raw.map(node => {
    const button = element("button", string(node.label) || string(node.id), "visual-node");
    button.type = "button";
    button.setAttribute("aria-pressed", "false");
    if (typeof node.group === "string" && node.group) button.title = node.group;
    layer.append(button);
    return { id: string(node.id), center: node.id === data.center, button, home: { x: 0, y: 0, z: 0 } as Vec3, width: 0, height: 0 };
  });
  layer.append(tip);
  const index = new Map(nodes.map((node, i) => [node.id, i]));
  const links = list(data.links).slice(0, 120).flatMap(link => {
    const from = index.get(string(link.from)), to = index.get(string(link.to));
    return from !== undefined && to !== undefined && from !== to ? [{ from, to, label: string(link.label) }] : [];
  });
  const points = layout3d(nodes.length, links.map(link => [link.from, link.to]), nodes.findIndex(node => node.center));
  nodes.forEach((node, i) => {
    node.home = points[i];
    node.button.classList.toggle("center", node.center);
    node.width = node.button.offsetWidth;
    node.height = node.button.offsetHeight;
  });
  let selected: number | undefined;
  let hovering = false;
  const born = performance.now();
  const color = getComputedStyle(surface).getPropertyValue("--aqua").trim() || "#5ff0e6";

  function updateSpin() { holo.spin(hovering || selected !== undefined ? 0 : IDLE_SPIN); }
  function connected(i: number) {
    return selected === undefined || i === selected || links.some(link =>
      (link.from === selected && link.to === i) || (link.to === selected && link.from === i));
  }

  let projected: ReturnType<Holo["project"]>[] = [];
  function draw(time: number) {
    const size = holo.size;
    // Grow out of the centre when it opens.
    const t = reduced ? 1 : Math.min(1, (time - born) / 750);
    const bloom = 1 - Math.pow(1 - t, 3);
    projected = nodes.map(node => holo.project({ x: node.home.x * bloom, y: node.home.y * bloom, z: node.home.z * bloom }));
    if (context) {
      const pixels = Math.round(size * Math.min(2, devicePixelRatio || 1));
      if (canvas.width !== pixels) canvas.width = canvas.height = pixels;
      context.setTransform(pixels / size, 0, 0, pixels / size, 0, 0);
      context.clearRect(0, 0, size, size);
      context.globalCompositeOperation = "lighter";
      context.strokeStyle = context.fillStyle = color;
      for (const link of links) {
        const a = projected[link.from], b = projected[link.to];
        const near = ((a.depth + b.depth) / 2 + 1) / 2;
        const lit = selected !== undefined && (link.from === selected || link.to === selected);
        context.globalAlpha = selected !== undefined && !lit ? .05 : lit ? .95 : .12 + near * .45;
        context.lineWidth = (lit ? 2 : 1.1) * (a.scale + b.scale) / 2;
        context.beginPath();
        context.moveTo(a.x, a.y);
        context.lineTo(b.x, b.y);
        context.stroke();
      }
      nodes.forEach((node, i) => {
        const p = projected[i];
        const near = (p.depth + 1) / 2;
        const dim = connected(i) ? 1 : .2;
        const radius = (node.center ? 5.5 : 3.4) * p.scale * Math.max(.9, size / 560);
        context.globalAlpha = (.12 + near * .2) * dim;
        context.beginPath();
        context.arc(p.x, p.y, radius * 2.6, 0, Math.PI * 2);
        context.fill();
        context.globalAlpha = (.45 + near * .55) * dim;
        context.beginPath();
        context.arc(p.x, p.y, radius, 0, Math.PI * 2);
        context.fill();
      });
      context.globalAlpha = 1;
      context.globalCompositeOperation = "source-over";
    }
    // Labels sit just above their dot. Front labels win: one that would cover a nearer label fades right back.
    const order = nodes.map((_, i) => i).sort((a, b) => projected[b].depth - projected[a].depth);
    const placed: { left: number; top: number; right: number; bottom: number }[] = [];
    for (const i of order) {
      const node = nodes[i], p = projected[i];
      const scale = Math.max(.72, Math.min(1.25, p.scale * Math.max(.95, size / 560)));
      const width = node.width * scale, height = node.height * scale;
      const box = { left: p.x - width / 2, right: p.x + width / 2, top: p.y - 7 * scale - height, bottom: p.y - 7 * scale };
      const covered = placed.some(other => box.left < other.right - 4 && box.right > other.left + 4 && box.top < other.bottom - 3 && box.bottom > other.top + 3);
      const important = i === selected || node.center;
      const hidden = covered && !important;
      if (!hidden) placed.push(box);
      node.button.style.left = p.x + "px";
      node.button.style.top = p.y + "px";
      node.button.style.setProperty("--s", scale.toFixed(3));
      node.button.style.zIndex = String(Math.round(20 + p.depth * 10) + (important ? 20 : 0));
      const near = (p.depth + 1) / 2;
      node.button.style.opacity = String(hidden ? .08 : (connected(i) ? .35 + near * .65 : .15));
      node.button.tabIndex = hidden ? -1 : 0;
      node.button.setAttribute("aria-pressed", String(i === selected));
    }
  }

  nodes.forEach((node, i) => node.button.addEventListener("click", () => {
    selected = selected === i ? undefined : i;
    if (selected !== undefined && !nodes[i].center) {
      const turn = facing(nodes[i].home, holo.yaw);
      holo.turnTo(turn.yaw, turn.pitch);
    }
    updateSpin();
    draw(performance.now());
  }));
  const enter = () => { hovering = true; updateSpin(); };
  const leave = () => { hovering = false; tip.hidden = true; updateSpin(); };
  const move = (event: PointerEvent) => {
    if (!projected.length || surface.classList.contains("holo-dragging")) { tip.hidden = true; return; }
    const bounds = surface.getBoundingClientRect();
    const x = event.clientX - bounds.left, y = event.clientY - bounds.top;
    const link = links.find(({ from, to, label }) => {
      if (!label) return false;
      const a = projected[from], b = projected[to];
      const dx = b.x - a.x, dy = b.y - a.y;
      const t = ((x - a.x) * dx + (y - a.y) * dy) / Math.max(1, dx * dx + dy * dy);
      return t > .15 && t < .85 && Math.hypot(x - a.x - t * dx, y - a.y - t * dy) < 7;
    });
    tip.hidden = !link;
    if (link) {
      tip.textContent = link.label;
      tip.style.left = x + "px";
      tip.style.top = y + "px";
    }
  };
  surface.addEventListener("pointerenter", enter);
  surface.addEventListener("pointerleave", leave);
  surface.addEventListener("pointermove", move);
  const unsubscribe = holo.onFrame(draw);
  updateSpin();
  return () => {
    unsubscribe();
    surface.removeEventListener("pointerenter", enter);
    surface.removeEventListener("pointerleave", leave);
    surface.removeEventListener("pointermove", move);
    layer.remove();
  };
}

/** A picture floating inside the sphere: tinted, scan-lined, and tilting a little as the hologram turns. */
function picture(surface: HTMLElement, holo: Holo, src: string, alt: string) {
  const frame = element("div", undefined, "holo-image");
  const image = element("img");
  image.src = src;
  image.alt = alt;
  frame.append(image, element("div", undefined, "holo-image-tint"), element("div", undefined, "holo-image-scan"));
  surface.append(frame);
  holo.spin(.3);
  const unsubscribe = holo.onFrame(() => {
    const tiltY = Math.sin(holo.yaw) * 18;
    const tiltX = (holo.pitch + .18) * -22;
    frame.style.transform = `translate(-50%, -50%) perspective(900px) rotateY(${tiltY.toFixed(2)}deg) rotateX(${tiltX.toFixed(2)}deg)`;
  });
  return () => { unsubscribe(); frame.remove(); };
}

export function mountVisuals(
  hud: HTMLElement,
  orb: HTMLElement,
  onOpen: (open: boolean) => void,
  layout: (mode: Layout) => Promise<unknown> = async () => {},
) {
  const overlay = element("dialog", undefined, "orb-visual");
  const header = element("header");
  const title = element("h2");
  title.id = "orb-visual-title";
  overlay.setAttribute("aria-labelledby", title.id);
  const sizeButton = element("button", "⤢", "visual-size");
  sizeButton.type = "button";
  const closeButton = element("button", "×");
  closeButton.type = "button";
  closeButton.setAttribute("aria-label", "Close visual");
  const buttons = element("div", undefined, "visual-buttons");
  buttons.append(sizeButton, closeButton);
  header.append(title, buttons);
  const surface = element("div", undefined, "visual-surface");
  const caption = element("p", undefined, "visual-caption");
  overlay.append(header, surface, caption);
  hud.append(overlay);
  const motion = matchMedia("(prefers-reduced-motion: reduce)");
  let expiry: ReturnType<typeof setTimeout> | undefined;
  let closing: ReturnType<typeof setTimeout> | undefined;
  let cleanup: (() => void) | undefined;
  let holo: Holo | undefined;
  let large = false;
  let opening = false;
  let generation = 0;

  function setLarge(value: boolean) {
    large = value;
    overlay.classList.toggle("large", large);
    sizeButton.setAttribute("aria-label", large ? "Make it smaller" : "Expand");
    sizeButton.textContent = large ? "⤡" : "⤢";
  }
  setLarge(false);

  function teardown() {
    cleanup?.();
    cleanup = undefined;
    holo?.dispose();
    holo = undefined;
  }
  function close(immediate = false) {
    generation++;
    if (opening) { opening = false; setLarge(false); void layout("closed"); }
    clearTimeout(expiry);
    clearTimeout(closing);
    overlay.classList.remove("expanded");
    if (!overlay.open) { teardown(); return; }
    function finish() {
      teardown();
      overlay.close();
      surface.replaceChildren();
      setLarge(false);
      hud.removeAttribute("data-visual-open");
      void layout("closed");
      onOpen(false);
    }
    if (immediate || motion.matches) finish();
    else closing = setTimeout(finish, 220);
  }
  closeButton.addEventListener("click", () => close());
  sizeButton.addEventListener("click", () => resize(!large));
  overlay.addEventListener("cancel", event => { event.preventDefault(); close(); });
  overlay.addEventListener("click", event => { if (event.target === overlay) close(); });

  async function show(value: BusEvents["visual"]) {
    const mine = ++generation;
    clearTimeout(expiry);
    clearTimeout(closing);
    if (!overlay.open) {
      // Make room first so the hologram never draws squashed into the small orb window. An "expand" that arrives
      // meanwhile (show_visual with large) is remembered and applied once the window has room.
      const asked = large;
      opening = true;
      try { await layout(asked ? "large" : "normal"); } finally { opening = false; }
      if (mine !== generation) return;
      if (large !== asked) void layout(large ? "large" : "normal");
    }
    teardown();
    surface.replaceChildren();
    const data = record(value.data);
    title.textContent = value.title;
    caption.textContent = "";
    surface.dataset.kind = value.kind;
    if (!overlay.open) {
      overlay.showModal();
      hud.setAttribute("data-visual-open", "");
      onOpen(true);
      overlay.getBoundingClientRect();
    }
    holo = createHolo(surface, { reduced: motion.matches });
    if (value.kind === "web") {
      cleanup = network(surface, holo, data, motion.matches);
    } else if (value.kind === "timeline") {
      const events = list(data.events).map(event => ({ when: string(event.when), title: string(event.title), detail: string(event.detail) || undefined }));
      cleanup = holoTimeline(surface, holo, events, motion.matches);
    } else {
      const src = string(data.src);
      if (/^data:image\/(?:png|jpeg|gif|webp|avif);base64,[\w+/=]+$/i.test(src)) {
        cleanup = picture(surface, holo, src, string(data.caption) || value.title);
      }
      caption.textContent = string(data.caption);
    }
    caption.hidden = !caption.textContent;
    overlay.classList.add("expanded");
    if (typeof value.ttlSeconds === "number" && value.ttlSeconds > 0) {
      expiry = setTimeout(close, Math.min(600, value.ttlSeconds) * 1000);
    }
  }

  function resize(value: boolean) {
    if ((!overlay.open && !opening) || value === large) return;
    setLarge(value);
    if (overlay.open) void layout(value ? "large" : "normal");
  }
  void orb; // kept for the call signature; the hologram now opens from the middle of its (resized) window
  return { show: (value: BusEvents["visual"]) => { void show(value); }, close, resize, isOpen: () => overlay.open };
}
