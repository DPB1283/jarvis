import type { JarvisState } from "../shared/events";
import { PALETTE, type TimerInfo } from "../shared/settings";

const nodeCount = 42;
const goldenAngle = Math.PI * (3 - Math.sqrt(5));

// Space the nodes evenly on a sphere. Only nearby nodes share a connection.
const nodes = Array.from({ length: nodeCount }, (_, index) => {
  const y = 1 - (2 * (index + 0.5)) / nodeCount;
  const ring = Math.sqrt(1 - y * y);
  const longitude = index * goldenAngle;
  return { x: Math.cos(longitude) * ring, y, z: Math.sin(longitude) * ring };
});
const connections: Array<[number, number]> = [];
for (const [index, node] of nodes.entries()) {
  for (let other = index + 1; other < nodes.length; other++) {
    const peer = nodes[other];
    if ((node.x - peer.x) ** 2 + (node.y - peer.y) ** 2 + (node.z - peer.z) ** 2 < 0.46) {
      connections.push([index, other]);
    }
  }
}

const looks = {
  idle: { spin: 0.18, brightness: 0.76, radius: 38.5, front: PALETTE.aqua, rear: PALETTE.blue },
  listening: { spin: 0.32, brightness: 1, radius: 42, front: PALETTE.glow, rear: PALETTE.blue },
  thinking: { spin: 0.9, brightness: 0.87, radius: 40, front: PALETTE.aqua, rear: PALETTE.blue },
  speaking: { spin: 0.3, brightness: 0.84, radius: 36, front: PALETTE.glow, rear: PALETTE.blue },
  error: { spin: 0, brightness: 0.92, radius: 39, front: PALETTE.warn, rear: PALETTE.danger },
} satisfies Record<JarvisState, { spin: number; brightness: number; radius: number; front: string; rear: string }>;

export function mountOrb(button: HTMLButtonElement) {
  const canvas = button.querySelector("canvas")!;
  const context = canvas.getContext("2d")!;
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let state: JarvisState = "idle";
  let angle = 0.46;
  let micLevel = 0;
  let micPulse = 0;
  let speechLevel = 0;
  let speechPulse = 0;
  let speechSampleAt = -Infinity;
  let followUpUntil = 0;
  let followUpDuration = 0;
  let timers: TimerInfo[] = [];
  let flashUntil = 0;
  let previousFrame = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let resizeFrame = 0;

  function draw() {
    clearTimeout(timer);
    const size = Math.round(canvas.clientWidth * Math.min(2, window.devicePixelRatio));
    if (!size || document.hidden) return;
    if (canvas.width !== size || canvas.height !== size) {
      canvas.width = canvas.height = size;
    }

    const now = performance.now();
    const elapsed = previousFrame ? Math.min((now - previousFrame) / 1000, 0.1) : 1 / 30;
    previousFrame = now;
    // Speech is metered in short PCM chunks. Let stale levels fall away during
    // pauses, and make quieter syllables visible with a faster attack than release.
    const level = now - speechSampleAt <= 85 ? speechLevel : 0;
    const target = Math.sqrt(Math.max(0, (level - 0.015) / 0.985));
    const responseTime = target > speechPulse ? 0.019 : 0.072;
    speechPulse += (target - speechPulse) * (1 - Math.exp(-elapsed / responseTime));
    micPulse += (micLevel - micPulse) * 0.38;
    // The brain can resume thinking between tools while speech is still playing.
    const visualState = state === "thinking" && speechPulse > 0.035 ? "speaking" : state;
    if (button.dataset.state !== visualState) button.dataset.state = visualState;
    const normal = looks[visualState];
    const ultron = button.dataset.ultron === "true";
    const look = ultron ? { ...normal, rear: "#981d2d", front: visualState === "speaking" ? "#ff9d92" : "#ff484c" } : normal;
    const motion = !reducedMotion.matches;
    const animate = motion && look.spin > 0;
    const idle = visualState === "idle";
    const clock = Date.now();
    const remaining = followUpUntil - clock;
    const followUpReady = idle && remaining > 0;
    const running = timers.find(timer => timer.endsAt > clock);
    const flashing = flashUntil > clock;
    const description = [
      followUpReady ? `Listening for a follow-up, ${Math.ceil(remaining / 1000)} seconds remaining` : "",
      running ? `${running.kind === "alarm" ? "Alarm" : "Timer"} running: ${running.label}` : "",
    ].filter(Boolean).join(". ");
    if (description && button.getAttribute("aria-description") !== description) button.setAttribute("aria-description", description);
    else if (!description && button.hasAttribute("aria-description")) button.removeAttribute("aria-description");
    const breath = idle && motion ? (1 - Math.cos(now * 2 * Math.PI / 5000)) / 2 : 0;
    angle = (angle + (animate ? look.spin * elapsed : 0)) % (Math.PI * 2);
    const talking = motion && visualState === "speaking" ? speechPulse : 0;
    const listening = motion && visualState === "listening" ? micPulse : 0;
    const radius = look.radius + breath * 2.8 + talking * 10 + listening * 3;
    const brightness = look.brightness + breath * 0.19 + talking * 0.16;
    const yawCos = Math.cos(angle);
    const yawSin = Math.sin(angle);
    const tilt = 0.4 + breath * 0.08;
    const tiltCos = Math.cos(tilt);
    const tiltSin = Math.sin(tilt);
    const points = nodes.map(({ x, y, z }) => {
      const horizontal = x * yawCos + z * yawSin;
      const rotatedZ = -x * yawSin + z * yawCos;
      const depth = y * tiltSin + rotatedZ * tiltCos;
      const projection = radius / (1 - depth * 0.16);
      return {
        x: 50 + horizontal * projection,
        y: 50 - (y * tiltCos - rotatedZ * tiltSin) * projection,
        depth,
      };
    });

    context.setTransform(size / 100, 0, 0, size / 100, 0, 0);
    context.clearRect(0, 0, 100, 100);
    if (idle) {
      // Light the edge of the sphere while keeping its center transparent.
      const halo = context.createRadialGradient(50, 50, radius * 0.68, 50, 50, radius + 10);
      halo.addColorStop(0, "transparent");
      halo.addColorStop(0.54, `${look.rear}55`);
      halo.addColorStop(1, "transparent");
      context.globalAlpha = 0.2 + breath * 0.36;
      context.fillStyle = halo;
      context.fillRect(0, 0, 100, 100);
    }
    // Layers soften the rear hemisphere so the foreground reads as a shell.
    const paths = Array.from({ length: 4 }, () => new Path2D());
    for (const [from, to] of connections) {
      const a = points[from];
      const b = points[to];
      const layer = Math.min(3, Math.floor((a.depth + b.depth + 2)));
      paths[layer].moveTo(a.x, a.y);
      paths[layer].lineTo(b.x, b.y);
    }
    for (const [index, path] of paths.entries()) {
      context.strokeStyle = index < 2 ? look.rear : look.front;
      context.globalAlpha = [0.11, 0.23, 0.41, 0.55][index] * brightness;
      context.lineWidth = index < 2 ? 0.55 : 0.72 + talking * 0.33;
      context.stroke(path);
    }

    for (const point of points.sort((a, b) => a.depth - b.depth)) {
      const depth = (point.depth + 1) * 0.5;
      const dot = (0.8 + depth * 0.85) * (1 + talking * depth * 0.38);
      context.fillStyle = point.depth < 0 ? look.rear : look.front;
      if (point.depth >= 0) {
        context.globalAlpha = (0.13 + breath * 0.14 + talking * 0.24 + listening * 0.09) * depth;
        context.beginPath();
        context.arc(point.x, point.y, dot * 2.3, 0, Math.PI * 2);
        context.fill();
      }
      context.globalAlpha = (0.32 + depth * 0.68) * brightness;
      context.beginPath();
      context.arc(point.x, point.y, dot, 0, Math.PI * 2);
      context.fill();
    }
    if (followUpReady) {
      const start = -Math.PI / 2;
      const progress = Math.min(1, remaining / followUpDuration);
      context.strokeStyle = ultron ? look.front : PALETTE.aqua;
      context.lineWidth = 1;
      context.globalAlpha = 0.22;
      context.beginPath();
      context.arc(50, 50, 43.4, 0, Math.PI * 2);
      context.stroke();
      context.globalAlpha = 0.65;
      context.beginPath();
      context.arc(50, 50, 45.8, start, start + progress * Math.PI * 2);
      context.stroke();
      context.globalAlpha = 0.9;
      context.fillStyle = ultron ? look.front : PALETTE.aqua;
      context.font = "600 7px system-ui, sans-serif";
      context.textAlign = "center";
      context.textBaseline = "middle";
      context.fillText(String(Math.ceil(remaining / 1000)), 50, 88);
    }
    if (running || flashing) {
      const timeLeft = running ? running.endsAt - clock : 0;
      const progress = running ? Math.min(1, timeLeft / Math.max(1, running.durationMs)) : 0;
      const color = progress <= 0.1 ? PALETTE.danger : progress <= 0.25 ? "#FFA347" : progress <= 0.5 ? "#F5D66F" : ultron ? look.front : PALETTE.aqua;
      if (running) {
        drawTimerArc(color, 2.25, 0.18, 1);
        const opacity = motion && timeLeft <= 10_000 ? 0.70 + 0.25 * Math.cos(now / 210) : 0.94;
        drawTimerArc(color, 1.55, opacity, progress);
      }
      if (flashing) {
        const fraction = (flashUntil - clock) / 1400;
        const glow = motion ? (0.45 + 0.55 * Math.sin(fraction * Math.PI * 2) ** 2) * fraction : 0.85;
        drawTimerArc(PALETTE.warn, 1.9, glow, 1);
      }
    }
    context.globalAlpha = 1;

    // Idle drift needs fewer updates than the animation driven by speech.
    if (animate || (motion && flashing)) timer = setTimeout(draw, 1000 / (idle ? 15 : 30));
    else if (running || flashing || followUpReady) {
      timer = setTimeout(draw, Math.min(1000, running ? running.endsAt - clock : Infinity,
        flashing ? flashUntil - clock : Infinity, followUpReady ? remaining : Infinity));
    }
  }

  function drawTimerArc(color: string, width: number, opacity: number, fraction: number) {
    const start = -Math.PI / 2;
    context.strokeStyle = color;
    context.lineWidth = width;
    context.globalAlpha = opacity;
    context.beginPath();
    context.arc(50, 50, 48.8, start, start + fraction * Math.PI * 2);
    context.stroke();
  }

  const resize = new ResizeObserver(() => {
    if (resizeFrame) return;
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = 0;
      draw();
    });
  });
  resize.observe(canvas);
  reducedMotion.addEventListener("change", draw);
  document.addEventListener("visibilitychange", draw);

  return {
    setState(value: JarvisState) {
      if (state === value) return;
      state = value;
      if (value !== "listening") micLevel = micPulse = 0;
      if (value !== "thinking" && value !== "speaking") {
        speechLevel = speechPulse = 0;
        speechSampleAt = -Infinity;
      }
      draw();
    },
    setLevel(source: "mic" | "tts", value: number) {
      const level = Number.isFinite(value) ? Math.max(0, Math.min(value, 1)) : 0;
      if (source === "mic" && state === "listening") micLevel = level;
      if (source === "tts" && (state === "thinking" || state === "speaking")) {
        speechLevel = level;
        speechSampleAt = performance.now();
      }
    },
    setFollowUp(until: number) {
      followUpUntil = until;
      followUpDuration = Math.max(1, until - Date.now());
      draw();
    },
    setTimers(value: TimerInfo[]) {
      timers = [...value].sort((a, b) => a.endsAt - b.endsAt);
      draw();
    },
    flashTimer() {
      flashUntil = Date.now() + 1400;
      draw();
    },
    dispose() {
      clearTimeout(timer);
      cancelAnimationFrame(resizeFrame);
      resize.disconnect();
      reducedMotion.removeEventListener("change", draw);
      document.removeEventListener("visibilitychange", draw);
    },
  };
}
