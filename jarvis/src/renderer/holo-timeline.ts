import type { Holo, Vec3 } from "./holo";

interface TimelineEvent {
  when: string;
  title: string;
  detail?: string;
}

const radius = .86;
const viewingPitch = -.32;
const fullTurn = Math.PI * 2;

function make<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function holoTimeline(surface: HTMLElement, holo: Holo, events: TimelineEvent[], reduced: boolean): () => void {
  const layer = make("section", "holo-timeline");
  layer.tabIndex = 0;
  layer.setAttribute("aria-label", "Timeline. Use the arrow keys to move through events, or Home and End to jump.");
  const spine = make("canvas", "holo-timeline-spine");
  spine.setAttribute("aria-hidden", "true");
  const context = spine.getContext("2d");
  const detail = make("div", "holo-timeline-detail");
  detail.setAttribute("aria-live", "polite");
  const count = make("span", "holo-timeline-count");
  const description = make("p", "holo-timeline-description");
  detail.append(description, count);
  layer.append(spine, detail);
  surface.append(layer);

  const items = events.slice(0, 100).map((event, index) => {
    const card = make("button", "holo-timeline-card");
    card.type = "button";
    card.hidden = true;
    card.tabIndex = -1;
    card.setAttribute("aria-label", [event.when, event.title].filter(Boolean).join(", "));
    card.append(make("time", "holo-timeline-date", event.when), make("strong", "holo-timeline-heading", event.title));
    if (event.detail) card.title = event.detail;
    card.addEventListener("click", () => select(index));
    layer.append(card);
    return { event, card };
  });
  if (!items.length) {
    description.textContent = "No events to display.";
    return () => layer.remove();
  }

  holo.lockPitch(true);
  holo.spin(0);
  holo.turnTo(holo.yaw, viewingPitch);
  const slots = Math.max(8, Math.min(11, items.length));
  const spacing = fullTurn / slots;
  let position = 0;
  let target: number | undefined;
  let previousYaw = holo.yaw;
  let leader = -1;
  let visible = new Set<number>();
  let frame = 0;
  let settle: ReturnType<typeof setTimeout> | undefined;
  const clamp = (index: number) => Math.max(0, Math.min(items.length - 1, index));

  function trackYaw() {
    const delta = holo.yaw - previousYaw;
    previousYaw = holo.yaw;
    position = clamp(position - Math.atan2(Math.sin(delta), Math.cos(delta)) / spacing);
    if (target !== undefined && Math.abs(target - position) < .015) target = undefined;
  }

  function offsetFor(index: number) {
    const offset = index - position;
    // A short timeline wraps around the rim. A long one pages through a local set of events.
    return items.length <= slots ? offset - Math.round(offset / slots) * slots : offset;
  }

  function point(offset: number): Vec3 {
    const turn = offset * spacing;
    const angle = turn - holo.yaw;
    return { x: radius * Math.sin(angle), y: .10 - .063 * Math.sin(turn), z: radius * Math.cos(angle) };
  }

  function project(offset: number, size: number) {
    const p = holo.project(point(offset));
    // A width transition can change the surface before the engine's resize observer runs.
    const ratio = size / holo.size;
    return { ...p, x: p.x * ratio, y: p.y * ratio };
  }

  function drawSpine(indices: number[], size: number) {
    if (!context || size <= 0) return;
    const dpi = Math.min(2, window.devicePixelRatio || 1);
    const pixels = Math.round(size * dpi);
    if (spine.width !== pixels || spine.height !== pixels) {
      spine.width = spine.height = pixels;
    }
    context.setTransform(pixels / size, 0, 0, pixels / size, 0, 0);
    context.clearRect(0, 0, size, size);
    const tone = getComputedStyle(layer).getPropertyValue("--aqua").trim() || "#64dce5";
    context.strokeStyle = context.fillStyle = context.shadowColor = tone;
    context.lineCap = "round";
    let previous = project(0, size);
    for (let segment = 1; segment <= 96; segment++) {
      const next = project(segment * slots / 96, size);
      if (previous.visible && next.visible) {
        const depth = (previous.depth + next.depth + 2) / 4;
        context.globalAlpha = .12 + .61 * depth;
        context.shadowBlur = 2 + 5 * depth;
        context.lineWidth = 1 + depth;
        context.beginPath();
        context.moveTo(previous.x, previous.y);
        context.lineTo(next.x, next.y);
        context.stroke();
      }
      previous = next;
    }
    for (const index of indices) {
      const p = project(offsetFor(index), size);
      if (!p.visible) continue;
      context.globalAlpha = .3 + Math.max(0, p.depth) * .65;
      context.shadowBlur = 6;
      context.beginPath();
      context.arc(p.x, p.y, 2 + Math.max(0, p.depth) * 2, 0, Math.PI * 2);
      context.fill();
    }
    context.globalAlpha = 1;
  }

  function draw() {
    trackYaw();
    const nearest = Math.round(position);
    const size = layer.clientWidth || holo.size;
    const begin = items.length <= slots ? 0 : clamp(nearest - 5);
    const end = items.length <= slots ? items.length - 1 : clamp(nearest + 5);
    const indices = Array.from({ length: end - begin + 1 }, (_, index) => begin + index);
    drawSpine(indices, size);
    for (const index of visible) if (index < begin || index > end) items[index].card.hidden = true;
    visible = new Set(indices);
    const projectedEvents = indices.map(index => ({ index, p: project(offsetFor(index), size) }));
    projectedEvents.sort((a, b) => Number(b.index === nearest) - Number(a.index === nearest) || b.p.depth - a.p.depth);
    const boxes: Array<{ left: number; right: number; top: number; bottom: number }> = [];
    for (const { index, p } of projectedEvents) {
      const { card } = items[index];
      const front = index === nearest;
      card.classList.toggle("holo-timeline-card-front", front);
      if (front) card.setAttribute("aria-current", "step");
      else card.removeAttribute("aria-current");
      if (!p.visible) { card.hidden = true; continue; }
      const scale = front ? p.scale : p.scale * .81;
      const width = (front ? Math.min(220, Math.max(105, size * .21)) : Math.min(165, Math.max(73, size * .16))) * scale;
      const font = front ? Math.min(18, Math.max(11, size / 48)) : Math.min(15, Math.max(10, size / 56));
      const height = ((front ? 3.65 : 2.45) * font + 12) * scale;
      const gap = Math.max(5, size * .011);
      const left = Math.max(size * .12, Math.min(size * .88 - width, p.x - width / 2));
      const lift = front ? size * .026 : gap;
      const base = p.y - lift - height;
      const offsets = [0, -(height + gap), height + gap, -2 * (height + gap)];
      const top = offsets.map(offset => base + offset).find(y =>
        y >= size * .18 && y + height <= size * .71 &&
        boxes.every(other => left + width + gap < other.left || other.right + gap < left ||
          y + height + gap < other.top || other.bottom + gap < y));
      card.hidden = top === undefined;
      if (top === undefined) continue;
      boxes.push({ left, right: left + width, top, bottom: top + height });
      card.style.left = (left + width / 2) + "px";
      card.style.top = top + "px";
      card.style.zIndex = String(Math.round(10 + p.depth * 9));
      card.style.opacity = String(front ? 1 : .26 + (p.depth + 1) * .29);
      card.style.setProperty("--holo-timeline-scale", String(scale));
    }
    if (nearest !== leader) {
      leader = nearest;
      description.textContent = items[nearest].event.detail ?? "";
      description.hidden = !description.textContent;
      count.textContent = (nearest + 1) + " / " + items.length;
    }
  }

  function requestDraw() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(draw);
  }

  function select(index: number) {
    clearTimeout(settle);
    trackYaw();
    const destination = clamp(index);
    // A jump across many events puts the viewer near the destination before turning to it.
    if (Math.abs(destination - position) > 3) position = destination;
    target = destination;
    holo.turnTo(holo.yaw - (destination - position) * spacing, viewingPitch);
    draw();
  }

  function snap() {
    trackYaw();
    select(Math.round(position));
  }
  function scheduleSnap(delay: number) {
    target = undefined;
    clearTimeout(settle);
    settle = setTimeout(snap, reduced ? 0 : delay);
    if (reduced) requestDraw();
  }
  const release = () => scheduleSnap(30);
  const wheel = () => scheduleSnap(105);
  const startDrag = () => { clearTimeout(settle); target = undefined; };
  const refresh = () => { if (reduced) requestDraw(); };
  function keyboard(event: KeyboardEvent) {
    const current = target ?? Math.round(position);
    let next: number;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowUp": next = current - 1; break;
      case "ArrowRight":
      case "ArrowDown": next = current + 1; break;
      case "Home": next = 0; break;
      case "End": next = items.length - 1; break;
      default: return;
    }
    event.preventDefault();
    event.stopPropagation();
    select(next);
  }
  surface.addEventListener("pointerdown", startDrag);
  surface.addEventListener("pointerup", release);
  surface.addEventListener("pointercancel", release);
  surface.addEventListener("pointermove", refresh);
  surface.addEventListener("wheel", wheel, { passive: true });
  const keyboardScope = surface.closest("dialog") ?? surface;
  keyboardScope.addEventListener("keydown", keyboard, true);
  const unsubscribe = holo.onFrame(draw);
  const resize = new ResizeObserver(draw);
  resize.observe(surface);
  draw();
  return () => {
    clearTimeout(settle);
    cancelAnimationFrame(frame);
    unsubscribe();
    resize.disconnect();
    surface.removeEventListener("pointerdown", startDrag);
    surface.removeEventListener("pointerup", release);
    surface.removeEventListener("pointercancel", release);
    surface.removeEventListener("pointermove", refresh);
    surface.removeEventListener("wheel", wheel);
    keyboardScope.removeEventListener("keydown", keyboard, true);
    holo.lockPitch(false);
    layer.remove();
  };
}
