// The hologram the orb turns into when it shows something: a see-through sphere of glowing points with the content
// floating inside it in 3D. Two canvases make the shell (the far half behind the content, the near half in front of
// it), and project() lets content place DOM or canvas drawings at 3D positions.
//
// Coordinates: the sphere has radius 1, y is up, +z points at the viewer before rotation.
// Rotation: yaw turns about the vertical axis (x' = x·cos(yaw) + z·sin(yaw), z' = z·cos(yaw) − x·sin(yaw)), so a
// larger yaw moves the front of the sphere to the right. It does not wrap. Pitch then tilts about the horizontal
// axis; a negative pitch looks down on the sphere from above (the front dips, the back rises).

export interface Vec3 { x: number; y: number; z: number }
export interface Projected {
  /** CSS pixels inside the surface. */
  x: number;
  y: number;
  /** Perspective size factor: 1 at the sphere's middle depth, larger in front. */
  scale: number;
  /** −1 at the back of the sphere to 1 at the front, after rotation. */
  depth: number;
  visible: boolean;
}
export interface Holo {
  readonly size: number;
  project(p: Vec3): Projected;
  /** Runs every animation frame after the rotation update (once per change when motion is reduced). */
  onFrame(callback: (timeMs: number) => void): () => void;
  readonly yaw: number;
  readonly pitch: number;
  /** Idle auto-rotation about the vertical axis. */
  spin(radiansPerSecond: number): void;
  /** Eased turn to that rotation; cancels drag inertia. */
  turnTo(yaw: number, pitch?: number): void;
  /** When on, dragging only turns left and right. */
  lockPitch(on: boolean): void;
  dispose(): void;
}

const CAMERA = 3.4; // distance from the viewer to the sphere's centre, in sphere radii
const FILL = .43; // the sphere's middle (z = 0) circle spans this fraction of the surface on each side
const PITCH_LIMIT = 1.1;

function fibonacci(count: number): Vec3[] {
  const points: Vec3[] = [];
  for (let i = 0; i < count; i++) {
    const y = 1 - (i + .5) * 2 / count;
    const ring = Math.sqrt(1 - y * y);
    const angle = i * 2.399963229728653;
    points.push({ x: Math.cos(angle) * ring, y, z: Math.sin(angle) * ring });
  }
  return points;
}

export function createHolo(surface: HTMLElement, options: { reduced: boolean }): Holo {
  const reduced = options.reduced;
  const back = document.createElement("canvas");
  const front = document.createElement("canvas");
  back.className = "holo-shell holo-shell-back";
  front.className = "holo-shell holo-shell-front";
  back.setAttribute("aria-hidden", "true");
  front.setAttribute("aria-hidden", "true");
  surface.prepend(back);
  surface.append(front);
  const backContext = back.getContext("2d");
  const frontContext = front.getContext("2d");
  // Each point gets a little random brightness and twinkle phase so the shell shimmers like light, not a grid.
  // Bigger holograms get more points so the shell stays dense.
  let shell: (Vec3 & { glow: number; phase: number })[] = [];
  function populate(count: number) {
    shell = fibonacci(count).map(p => ({ ...p, glow: .6 + Math.random() * .4, phase: Math.random() * Math.PI * 2 }));
  }

  let size = surface.clientWidth || 440;
  let yaw = 0, pitch = -.18;
  let spinRate = reduced ? 0 : .12;
  let yawVelocity = 0, pitchVelocity = 0;
  let target: { yaw: number; pitch: number } | undefined;
  let pitchLocked = false;
  let dragging: { id: number; x: number; y: number; moved: boolean; time: number } | undefined;
  const listeners = new Set<(time: number) => void>();
  let frame = 0, last = performance.now(), disposed = false;

  function tone() {
    return getComputedStyle(surface).getPropertyValue("--holo").trim() || getComputedStyle(surface).getPropertyValue("--aqua").trim() || "#5ff0e6";
  }
  let color = tone();

  function resize() {
    size = Math.max(1, surface.clientWidth);
    // Big holograms use fewer canvas pixels per CSS pixel: the points are soft glows, so it doesn't show.
    const pixels = Math.round(size * Math.min(size > 760 ? 1.25 : 2, devicePixelRatio || 1));
    for (const canvas of [back, front]) {
      if (canvas.width !== pixels) canvas.width = canvas.height = pixels;
    }
    color = tone();
    const count = Math.round(Math.min(3400, Math.max(1500, size * 3.2)));
    if (Math.abs(count - shell.length) > 200) populate(count);
  }

  function rotate(p: Vec3): Vec3 {
    const cy = Math.cos(yaw), sy = Math.sin(yaw);
    const x = p.x * cy + p.z * sy;
    const z1 = p.z * cy - p.x * sy;
    const cp = Math.cos(pitch), sp = Math.sin(pitch);
    return { x, y: p.y * cp + z1 * sp, z: z1 * cp - p.y * sp };
  }

  function project(p: Vec3): Projected {
    const r = rotate(p);
    const perspective = CAMERA / (CAMERA - r.z);
    const half = size / 2;
    return {
      x: half + r.x * perspective * size * FILL,
      y: half - r.y * perspective * size * FILL,
      scale: perspective,
      depth: Math.max(-1, Math.min(1, r.z)),
      visible: r.z < CAMERA - .2,
    };
  }

  function drawShell(time: number) {
    if (!backContext || !frontContext) return;
    const pixels = back.width;
    const k = pixels / size;
    for (const context of [backContext, frontContext]) {
      context.setTransform(k, 0, 0, k, 0, 0);
      context.clearRect(0, 0, size, size);
      context.globalCompositeOperation = "lighter";
      context.fillStyle = color;
    }
    const twinkle = reduced ? 0 : time / 900;
    const grow = Math.max(.85, size / 560);
    // Points are grouped into a few brightness bands so each band is one path (thousands of points per frame).
    const BANDS = 10;
    const paths = [backContext, frontContext].map(() => Array.from({ length: BANDS }, () => new Path2D()));
    for (const point of shell) {
      const p = project(point);
      // Facing (|z| near 1) points are see-through; edge-on points near the rim glow, like a hologram.
      const rim = 1 - Math.abs(p.depth);
      const shimmer = reduced ? 1 : .7 + .3 * Math.sin(twinkle + point.phase);
      const near = p.depth >= 0;
      const alpha = Math.min(1, (near ? .1 : .07) + Math.pow(rim, 1.6) * (near ? .95 : .7) * point.glow * shimmer);
      const radius = (.6 + rim * .95) * p.scale * grow;
      const path = paths[near ? 1 : 0][Math.min(BANDS - 1, Math.floor(alpha * BANDS))];
      path.moveTo(p.x + radius, p.y);
      path.arc(p.x, p.y, radius, 0, Math.PI * 2);
    }
    [backContext, frontContext].forEach((context, side) => paths[side].forEach((path, band) => {
      context.globalAlpha = (band + .5) / BANDS;
      context.fill(path);
    }));
    // A soft halo just outside the rim and a faint core glow give the sphere body.
    const half = size / 2, rim = size * FILL * 1.04;
    // The halo must fade out before the canvas edge, or the edge clips it into a hard ring.
    backContext.globalAlpha = .11;
    const halo = backContext.createRadialGradient(half, half, rim * .86, half, half, Math.min(rim * 1.08, half * .985));
    halo.addColorStop(0, "transparent");
    halo.addColorStop(.62, color);
    halo.addColorStop(1, "transparent");
    backContext.fillStyle = halo;
    backContext.fillRect(0, 0, size, size);
    backContext.globalAlpha = .1;
    const glow = backContext.createRadialGradient(half, half, 0, half, half, rim);
    glow.addColorStop(0, color);
    glow.addColorStop(1, "transparent");
    backContext.fillStyle = glow;
    backContext.fillRect(0, 0, size, size);
    for (const context of [backContext, frontContext]) {
      context.globalAlpha = 1;
      context.globalCompositeOperation = "source-over";
    }
  }

  function step(time: number) {
    const dt = Math.min(.05, (time - last) / 1000);
    last = time;
    if (target) {
      const ease = reduced ? 1 : 1 - Math.pow(.0015, dt);
      yaw += (target.yaw - yaw) * ease;
      pitch += (target.pitch - pitch) * ease;
      if (Math.abs(target.yaw - yaw) < .0008 && Math.abs(target.pitch - pitch) < .0008) {
        yaw = target.yaw;
        pitch = target.pitch;
        target = undefined;
      }
    } else if (!dragging) {
      yaw += (yawVelocity + spinRate) * dt;
      pitch = Math.max(-PITCH_LIMIT, Math.min(PITCH_LIMIT, pitch + pitchVelocity * dt));
      const friction = Math.pow(.04, dt);
      yawVelocity *= friction;
      pitchVelocity *= friction;
      if (Math.abs(yawVelocity) < .002) yawVelocity = 0;
      if (Math.abs(pitchVelocity) < .002) pitchVelocity = 0;
    }
    drawShell(time);
    for (const listener of listeners) listener(time);
  }

  function loop(time: number) {
    if (disposed) return;
    if (!document.hidden) step(time);
    else last = time;
    frame = requestAnimationFrame(loop);
  }
  function redraw() {
    if (reduced) {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(time => { last = time; step(time); });
    }
  }

  const DRAG = .0085; // radians per pixel at the 440 px size
  function pointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    dragging = { id: event.pointerId, x: event.clientX, y: event.clientY, moved: false, time: performance.now() };
    target = undefined;
    yawVelocity = pitchVelocity = 0;
  }
  function pointerMove(event: PointerEvent) {
    if (!dragging || event.pointerId !== dragging.id) return;
    const dx = event.clientX - dragging.x, dy = event.clientY - dragging.y;
    if (!dragging.moved && Math.hypot(dx, dy) < 4) return;
    if (!dragging.moved) {
      dragging.moved = true;
      surface.setPointerCapture(event.pointerId);
      surface.classList.add("holo-dragging");
    }
    const now = performance.now();
    const dt = Math.max(1, now - dragging.time) / 1000;
    const scale = DRAG * 440 / size;
    const turnYaw = dx * scale, turnPitch = pitchLocked ? 0 : -dy * scale;
    yaw += turnYaw;
    pitch = Math.max(-PITCH_LIMIT, Math.min(PITCH_LIMIT, pitch + turnPitch));
    if (!reduced) {
      yawVelocity = yawVelocity * .5 + (turnYaw / dt) * .5;
      pitchVelocity = pitchVelocity * .5 + (turnPitch / dt) * .5;
    }
    dragging.x = event.clientX;
    dragging.y = event.clientY;
    dragging.time = now;
    redraw();
  }
  function pointerUp(event: PointerEvent) {
    if (!dragging || event.pointerId !== dragging.id) return;
    const moved = dragging.moved;
    if (moved) {
      // A drag must not also count as a click on the node it started on.
      surface.addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => surface.removeEventListener("click", swallow, true), 0);
    }
    if (performance.now() - dragging.time > 90) yawVelocity = pitchVelocity = 0; // held still before letting go
    dragging = undefined;
    surface.classList.remove("holo-dragging");
    redraw();
  }
  function swallow(event: Event) {
    event.stopPropagation();
    event.preventDefault();
  }
  function wheel(event: WheelEvent) {
    event.preventDefault();
    target = undefined;
    const amount = (Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY) * (event.deltaMode === 1 ? 16 : 1);
    yaw -= amount * .0035;
    yawVelocity = 0;
    redraw();
  }
  surface.addEventListener("pointerdown", pointerDown);
  surface.addEventListener("pointermove", pointerMove);
  surface.addEventListener("pointerup", pointerUp);
  surface.addEventListener("pointercancel", pointerUp);
  surface.addEventListener("wheel", wheel, { passive: false });
  const observer = new ResizeObserver(() => { resize(); redraw(); });
  observer.observe(surface);
  resize();

  if (reduced) redraw();
  else frame = requestAnimationFrame(time => { last = time; loop(time); });

  return {
    get size() { return size; },
    get yaw() { return yaw; },
    get pitch() { return pitch; },
    project,
    onFrame(callback) {
      listeners.add(callback);
      redraw();
      return () => listeners.delete(callback);
    },
    spin(rate) { spinRate = reduced ? 0 : rate; },
    turnTo(nextYaw, nextPitch = pitch) {
      yawVelocity = pitchVelocity = 0;
      target = { yaw: nextYaw, pitch: Math.max(-PITCH_LIMIT, Math.min(PITCH_LIMIT, nextPitch)) };
      if (reduced) { yaw = target.yaw; pitch = target.pitch; target = undefined; }
      redraw();
    },
    lockPitch(on) { pitchLocked = on; if (on) pitchVelocity = 0; },
    dispose() {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      surface.removeEventListener("pointerdown", pointerDown);
      surface.removeEventListener("pointermove", pointerMove);
      surface.removeEventListener("pointerup", pointerUp);
      surface.removeEventListener("pointercancel", pointerUp);
      surface.removeEventListener("wheel", wheel);
      surface.classList.remove("holo-dragging");
      back.remove();
      front.remove();
      listeners.clear();
    },
  };
}
