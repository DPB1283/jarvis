// Flat (2D) pictures inside the hologram sphere. The shell still shimmers and turns around them, but the content
// faces the viewer: an orbit diagram (things circling a centre) and a bar chart (comparing amounts).
import type { Holo } from "./holo";

export interface OrbitBody { label: string; detail?: string; period?: number; size?: number; distance?: number }
export interface OrbitData { center: string; centerDetail?: string; bodies: OrbitBody[] }
export interface ChartBar { label: string; value: number; detail?: string }
export interface ChartData { unit?: string; bars: ChartBar[] }

const FILL = .43; // must match holo.ts: the sphere's radius as a fraction of the surface
const GOLDEN = 2.399963229728653;

function make<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
function tone(surface: HTMLElement) {
  return getComputedStyle(surface).getPropertyValue("--aqua").trim() || "#5ff0e6";
}

/** True when every body has a real distance: the diagram is then drawn by true relative distance and period. */
export function isScaledOrbit(data: OrbitData) {
  const bodies = data.bodies.slice(0, 12);
  return bodies.length > 0 && bodies.every(body => body.distance !== undefined && body.distance > 0);
}

/**
 * Bodies going round a glowing centre. With real distances ("scaled") the rings sit at the square root of the true
 * distance (a straight scale would pile the inner planets on the Sun) and the speeds keep their true ratios, with
 * Kepler's third law (period = distance^1.5, in AU and years) filling in a missing period. Without distances it is a
 * schematic: evenly spaced rings and squashed speeds. Sizes are never to scale.
 */
export function holoOrbit(surface: HTMLElement, holo: Holo, data: OrbitData, reduced: boolean): () => void {
  const layer = make("div", "visual-flat");
  const canvas = make("canvas");
  canvas.setAttribute("aria-hidden", "true");
  const detail = make("p", "visual-flat-detail");
  detail.setAttribute("aria-live", "polite");
  detail.hidden = true;
  layer.append(canvas);
  surface.append(layer);
  const context = canvas.getContext("2d");
  const color = tone(surface);

  function label(text: string, center = false) {
    const button = make("button", center ? "visual-node center" : "visual-node", text);
    button.type = "button";
    button.setAttribute("aria-pressed", "false");
    layer.append(button);
    return button;
  }
  const centre = { label: data.center, detail: data.centerDetail, button: label(data.center, true), width: 0, height: 0 };
  const raw = data.bodies.slice(0, 12);
  // Real periods span a huge range (Mercury laps Neptune ~680 times), so speeds are squashed to stay watchable.
  const scaled = isScaledOrbit(data);
  const farthest = Math.max(...raw.map(body => body.distance ?? 0));
  const periods = raw.map((body, i) => body.period && body.period > 0 ? body.period : Math.pow(scaled ? body.distance! : i + 1, 1.5));
  const fastest = Math.min(...periods);
  const bodies = raw.map((body, i) => ({
    ...body,
    button: label(body.label),
    start: i * GOLDEN + .7,
    // Scaled: the fastest body takes 20 s a lap and the rest keep their true ratios, so outer ones crawl.
    speed: scaled ? Math.PI * 2 / 20 * (fastest / periods[i]) : .6 * Math.pow(fastest / periods[i], .55),
    width: 0,
    height: 0,
  }));
  layer.append(detail);
  for (const item of [centre, ...bodies]) { item.width = item.button.offsetWidth; item.height = item.button.offsetHeight; }

  let selected: number | undefined; // -1 is the centre
  const born = performance.now();
  holo.lockPitch(true);
  holo.spin(.05);

  function choose(index: number) {
    selected = selected === index ? undefined : index;
    const item = selected === undefined ? undefined : selected < 0 ? centre : bodies[selected];
    detail.textContent = item ? (item.detail ? `${item.label}: ${item.detail}` : item.label) : "";
    detail.hidden = !item;
    draw(performance.now());
  }
  centre.button.addEventListener("click", () => choose(-1));
  bodies.forEach((body, i) => body.button.addEventListener("click", () => choose(i)));

  function draw(time: number) {
    const size = holo.size;
    if (!context) return;
    const pixels = Math.round(size * Math.min(2, devicePixelRatio || 1));
    if (canvas.width !== pixels) canvas.width = canvas.height = pixels;
    context.setTransform(pixels / size, 0, 0, pixels / size, 0, 0);
    context.clearRect(0, 0, size, size);
    const half = size / 2, R = size * FILL * .92, grow = Math.max(.9, size / 560);
    const opened = reduced ? 1 : Math.min(1, (time - born) / 900);
    const bloom = 1 - Math.pow(1 - opened, 3);
    const seconds = reduced ? 0 : (time - born) / 1000;
    context.globalCompositeOperation = "lighter";
    context.strokeStyle = context.fillStyle = color;

    // Rings
    const radii = bodies.map((body, i) => R * (scaled ? .1 + .88 * Math.sqrt(body.distance! / farthest) : .24 + .72 * (i + .5) / bodies.length) * bloom);
    context.setLineDash([2 * grow, 5 * grow]);
    radii.forEach((r, i) => {
      context.globalAlpha = selected === i ? .8 : .32;
      context.lineWidth = selected === i ? 1.6 : 1;
      context.beginPath();
      context.arc(half, half, r, 0, Math.PI * 2);
      context.stroke();
    });
    context.setLineDash([]);

    // The centre: a soft glow with a bright core.
    const core = R * .085 * (.4 + .6 * bloom);
    const glow = context.createRadialGradient(half, half, 0, half, half, core * 2.6);
    glow.addColorStop(0, color);
    glow.addColorStop(1, "transparent");
    context.globalAlpha = .55;
    context.fillStyle = glow;
    context.fillRect(half - core * 3, half - core * 3, core * 6, core * 6);
    context.fillStyle = color;
    context.globalAlpha = .9;
    context.beginPath();
    context.arc(half, half, core, 0, Math.PI * 2);
    context.fill();

    // Bodies
    const spots = bodies.map((body, i) => {
      const angle = body.start + seconds * body.speed;
      const x = half + Math.cos(angle) * radii[i], y = half + Math.sin(angle) * radii[i];
      const radius = (2 + Math.max(1, Math.min(5, body.size ?? 2)) * 1.5) * grow;
      const dim = selected === undefined || selected === i || selected < 0 ? 1 : .35;
      context.globalAlpha = .22 * dim;
      context.beginPath();
      context.arc(x, y, radius * 2.4, 0, Math.PI * 2);
      context.fill();
      context.globalAlpha = .95 * dim;
      context.beginPath();
      context.arc(x, y, radius, 0, Math.PI * 2);
      context.fill();
      return { x, y, radius };
    });
    context.globalAlpha = 1;
    context.globalCompositeOperation = "source-over";

    // Labels sit just above their dot; one that would cover an already placed label fades back.
    const scale = Math.max(.8, Math.min(1.2, size / 560));
    const placed: { left: number; right: number; top: number; bottom: number }[] = [];
    const place = (item: { button: HTMLButtonElement; width: number; height: number }, x: number, y: number, important: boolean) => {
      const width = item.width * scale, height = item.height * scale;
      const box = { left: x - width / 2, right: x + width / 2, top: y - height, bottom: y };
      const covered = placed.some(o => box.left < o.right - 3 && box.right > o.left + 3 && box.top < o.bottom - 2 && box.bottom > o.top + 2);
      const hidden = covered && !important;
      if (!hidden) placed.push(box);
      item.button.style.left = x + "px";
      item.button.style.top = (y + 7) + "px"; // .visual-node lifts itself 7px above this point
      item.button.style.setProperty("--s", scale.toFixed(3));
      item.button.style.opacity = String(hidden ? .1 : bloom);
      item.button.style.zIndex = important ? "40" : "20";
      item.button.tabIndex = hidden ? -1 : 0;
    };
    place(centre, half, half - core - 4, true);
    placed.push({ left: half - core, right: half + core, top: half - core, bottom: half + core }); // other labels keep off the centre
    centre.button.setAttribute("aria-pressed", String(selected === -1));
    // The chosen body first, then from the outside in (outer rings have more room).
    const order = bodies.map((_, i) => i).sort((a, b) => (b === selected ? 1 : 0) - (a === selected ? 1 : 0) || b - a);
    for (const i of order) {
      place(bodies[i], spots[i].x, spots[i].y - spots[i].radius - 3, i === selected);
      bodies[i].button.setAttribute("aria-pressed", String(i === selected));
    }
  }

  const unsubscribe = holo.onFrame(draw);
  return () => { unsubscribe(); layer.remove(); };
}

function formatValue(value: number, unit?: string) {
  const size = Math.abs(value);
  const text = new Intl.NumberFormat(undefined, size >= 1e6
    ? { notation: "compact", maximumFractionDigits: 1 }
    : { maximumFractionDigits: size < 10 ? 2 : size < 1000 ? 1 : 0 }).format(value);
  if (!unit) return text;
  return unit === "%" ? text + "%" : `${text} ${unit}`;
}

/** Horizontal bars, biggest highlighted, growing in one after another. */
export function holoChart(surface: HTMLElement, holo: Holo, data: ChartData, reduced: boolean): () => void {
  const layer = make("div", "visual-chart");
  const bars = make("ol", "visual-chart-bars");
  bars.setAttribute("aria-label", "Bar chart");
  const detail = make("p", "visual-chart-detail");
  detail.setAttribute("aria-live", "polite");
  layer.append(bars, detail);
  surface.append(layer);
  holo.lockPitch(true);
  holo.spin(.05);

  const items = data.bars.slice(0, 12);
  const max = Math.max(0, ...items.map(bar => bar.value)) || 1;
  const fills: [HTMLElement, string][] = [];
  layer.style.setProperty("--rows", String(Math.max(3, items.length)));
  items.forEach((bar, i) => {
    const value = formatValue(bar.value, data.unit);
    const row = make("li");
    const button = make("button", bar.value === max ? "visual-chart-row top" : "visual-chart-row");
    button.type = "button";
    button.setAttribute("aria-label", `${bar.label}: ${value}`);
    const track = make("span", "visual-chart-track");
    const fill = make("span", "visual-chart-fill");
    fill.style.transitionDelay = reduced ? "0s" : `${i * 70}ms`;
    track.append(fill);
    button.append(make("span", "visual-chart-label", bar.label), track, make("span", "visual-chart-value", value));
    const say = () => { detail.textContent = bar.detail ? `${bar.label}: ${value}. ${bar.detail}` : `${bar.label}: ${value}`; };
    button.addEventListener("pointerenter", say);
    button.addEventListener("focus", say);
    button.addEventListener("click", say);
    row.append(button);
    bars.append(row);
    fills.push([fill, `${Math.max(.8, bar.value / max * 100)}%`]);
  });
  const grow = () => { for (const [fill, width] of fills) fill.style.width = width; };
  let frame = 0;
  if (reduced) grow();
  else frame = requestAnimationFrame(() => { frame = requestAnimationFrame(grow); });
  return () => { cancelAnimationFrame(frame); layer.remove(); };
}
