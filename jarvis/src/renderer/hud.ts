import { DEFAULT_SETTINGS, type Settings } from "../shared/settings";
import type { BusEvents } from "../shared/events";
import { mountOrb } from "./orb";

export function mountHud() {
  const api = window.jarvis;
  const hud = document.querySelector<HTMLElement>("#hud")!;
  const orb = document.querySelector<HTMLButtonElement>("#orb")!;
  const orbVisual = mountOrb(orb);
  const captions = document.querySelector<HTMLElement>("#captions")!;
  const user = document.querySelector<HTMLElement>("#user-caption")!;
  const assistant = document.querySelector<HTMLElement>("#assistant-caption")!;
  const dictationStatus = document.querySelector<HTMLElement>("#dictation-status")!;
  const activity = document.querySelector<HTMLElement>("#activity")!;
  const activityText = document.querySelector<HTMLElement>("#activity-text")!;
  const activityPercent = document.querySelector<HTMLElement>("#activity-percent")!;
  const activityFill = document.querySelector<HTMLElement>("#activity-fill")!;
  const mapCard = document.querySelector<HTMLElement>("#map-card")!;
  const mapImage = document.querySelector<HTMLImageElement>("#map-image")!;
  const mapTitle = document.querySelector<HTMLElement>("#map-title")!;
  const mapCaption = document.querySelector<HTMLElement>("#map-caption")!;
  const mapClose = document.querySelector<HTMLButtonElement>("#map-close")!;
  let mapTimer: ReturnType<typeof setTimeout> | undefined;
  const fullAccess = document.querySelector<HTMLElement>("#full-access")!;
  const approvals = document.querySelector<HTMLElement>("#approvals")!;
  const form = document.querySelector<HTMLFormElement>("#message-form")!;
  const closeMessage = document.querySelector<HTMLButtonElement>("#close-message")!;
  const message = document.querySelector<HTMLInputElement>("#message")!;
  const toast = document.querySelector<HTMLElement>("#toast")!;
  const cards = new Map<string, HTMLElement>();
  let settings = DEFAULT_SETTINGS;
  let streaming = false;
  let followUpUntil = 0;
  let jarvisState: string = "idle";
  let gameMode = false;
  let receivedTimers = false;
  let ignoring: boolean | undefined;
  let toastTimer: ReturnType<typeof setTimeout>;
  let activityDelay: ReturnType<typeof setTimeout> | undefined;
  let activityStale: ReturnType<typeof setTimeout> | undefined;

  // A map (route or place) next to the orb. It closes itself after a while, or with the x.
  function hideMap() {
    clearTimeout(mapTimer);
    mapCard.hidden = true;
    mapImage.removeAttribute("src");
  }
  function showMap(value: BusEvents["map"]) {
    if (gameMode) return;
    clearTimeout(mapTimer);
    mapImage.src = `data:image/png;base64,${value.image}`;
    mapImage.alt = value.title;
    mapTitle.textContent = value.title;
    mapCaption.textContent = value.caption;
    mapCaption.hidden = !value.caption;
    mapCard.hidden = false;
    mapTimer = setTimeout(hideMap, 45_000);
  }
  mapClose.addEventListener("click", hideMap);

  // The "he is working on it" chip. It waits a moment before appearing so quick tools do not flicker, and
  // hides itself if the brain somehow never says it finished.
  function setActivity(value: BusEvents["activity"]) {
    clearTimeout(activityStale);
    if (!value.active || gameMode) {
      clearTimeout(activityDelay);
      activityDelay = undefined;
      activity.hidden = true;
      return;
    }
    activityText.textContent = value.text;
    const known = typeof value.progress === "number" && Number.isFinite(value.progress);
    const fraction = known ? Math.min(1, Math.max(0, value.progress as number)) : 0;
    activity.toggleAttribute("data-indeterminate", !known);
    activityFill.style.width = known ? `${Math.round(fraction * 100)}%` : "";
    activityPercent.hidden = !known;
    activityPercent.textContent = known ? `${Math.round(fraction * 100)}%` : "";
    if (activity.hidden && activityDelay === undefined) {
      activityDelay = setTimeout(() => {
        activityDelay = undefined;
        if (!gameMode) activity.hidden = false;
      }, 300);
    }
    activityStale = setTimeout(() => setActivity({ text: "", active: false }), 20 * 60_000);
  }

  function setIgnore(ignore: boolean) {
    if (ignoring === ignore) return;
    ignoring = ignore;
    void api.invoke("window:set-ignore-mouse", ignore).catch(() => {
      if (ignoring === ignore) ignoring = undefined;
    });
  }

  function updateCaptions() {
    user.hidden = !user.textContent;
    assistant.hidden = !assistant.textContent;
    captions.hidden = gameMode || !settings.captions.enabled || (user.hidden && assistant.hidden);
    if (!captions.hidden) captions.scrollTop = captions.scrollHeight;
  }

  function applySettings(value: Settings) {
    settings = value;
    const name = value.ultronMode ? "Ultron" : "Jarvis";
    assistant.dataset.name = name;
    message.placeholder = `Type to ${name}…`;
    message.setAttribute("aria-label", `Message to ${name}`);
    orb.setAttribute("aria-label", `Talk to ${name}`);
    hud.dataset.ultron = String(value.ultronMode);
    orb.dataset.ultron = hud.dataset.ultron;
    hud.style.setProperty("--aqua", value.ultronMode ? "#ff5759" : "");
    hud.style.setProperty("--glow", value.ultronMode ? "#ffa8a8" : "");
    hud.style.setProperty("--blue", value.ultronMode ? "#891d2b" : "");
    hud.dataset.corner = value.orb.corner;
    captions.dataset.position = value.captions.position;
    captions.style.fontSize = `${value.captions.size}px`;
    orb.style.setProperty("--orb-size", `${value.orb.size}px`);
    orb.style.opacity = String(value.orb.opacity);
    // The user found the red "Full access" badge ugly; the mode still shows in the tray tooltip and menu.
    fullAccess.hidden = true;
    updateCaptions();
  }

  function showToast(text: string, level: string) {
    if (gameMode) return;
    clearTimeout(toastTimer);
    toast.textContent = text;
    toast.dataset.level = level;
    toast.hidden = false;
    toastTimer = setTimeout(() => { toast.hidden = true; }, 6000);
  }

  function closeApproval(id: string) {
    cards.get(id)?.remove();
    cards.delete(id);
  }

  function showApproval(event: BusEvents["approval_request"]) {
    closeApproval(event.id);
    const card = document.createElement("article");
    card.className = "approval";
    card.dataset.hit = "";
    const title = document.createElement("strong");
    title.textContent = `Approve ${event.tool}?`;
    const summary = document.createElement("p");
    summary.textContent = event.summary;
    const actions = document.createElement("div");
    actions.className = "actions";
    for (const [label, ok] of [["✓ Allow", true], ["✗ Deny", false]] as const) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.addEventListener("click", () => {
        api.send({ type: "approval_response", id: event.id, ok });
        closeApproval(event.id);
        setIgnore(true);
      });
      actions.append(button);
    }
    card.append(title, summary, actions);
    cards.set(event.id, card);
    approvals.append(card);
  }

  function showInput(focus = false) {
    if (gameMode) return;
    form.hidden = false;
    if (focus) {
      setIgnore(false);
      message.focus();
    }
  }

  function hideInput() {
    form.hidden = true;
    message.blur();
    setIgnore(true);
  }

  applySettings(settings);
  hud.hidden = false;
  setIgnore(true);
  let receivedSettings = false;
  const unsubscribe = api.on(event => {
    switch (event.type) {
      case "wake":
        if (event.source === "hotkey") showInput(true);
        break;
      case "followup":
        followUpUntil = event.active ? event.until : 0;
        orbVisual.setFollowUp(followUpUntil);
        break;
      case "game_mode":
        // Hide only when the orb shares the game's screen (main sends hideHud); on another monitor he stays.
        gameMode = event.hideHud ?? event.active;
        orb.hidden = gameMode;
        if (gameMode) {
          setActivity({ text: "", active: false });
          hideMap();
        }
        if (gameMode) {
          hideInput();
          toast.hidden = true;
        }
        updateCaptions();
        break;
      case "dictation":
        dictationStatus.hidden = !event.active;
        break;
      case "activity":
        setActivity(event);
        break;
      case "map":
        showMap(event);
        break;
      case "timers":
        receivedTimers = true;
        orbVisual.setTimers(event.timers);
        break;
      case "timer_done":
        orbVisual.flashTimer();
        break;
      case "state":
        jarvisState = event.state;
        orbVisual.setState(event.state);
        if (event.state === "listening") {
          user.textContent = "";
          if (followUpUntil <= Date.now()) assistant.textContent = "";
          streaming = false;
          updateCaptions();
          // Voice wakes don't open the text box; only the talk hotkey does (see the "wake" case).
        } else if (event.state === "thinking" && !message.value.trim()) {
          hideInput();
        }
        break;
      case "audio_level":
        orbVisual.setLevel(event.source, event.level);
        break;
      case "user_said":
        user.textContent = event.text;
        updateCaptions();
        break;
      case "assistant_delta":
        if (!streaming) assistant.textContent = "";
        assistant.textContent += event.text;
        streaming = true;
        updateCaptions();
        break;
      case "assistant_done":
        assistant.textContent = event.text;
        streaming = false;
        updateCaptions();
        break;
      case "approval_request":
        showApproval(event);
        break;
      case "approval_response":
        closeApproval(event.id);
        break;
      case "settings_changed":
        receivedSettings = true;
        applySettings(event.settings);
        break;
      case "toast":
        showToast(event.text, event.level);
        break;
      case "reminder_due":
        showToast(event.text, "info");
        break;
    }
  });
  void api.invoke("settings:get").then(value => {
    if (!receivedSettings) applySettings(value);
  }).catch(() => showToast("Could not load settings.", "error"));
  if ("__TAURI_INTERNALS__" in window) {
    void Promise.all([api.invoke("secrets:status"), api.invoke("settings:get")]).then(([keys, config]) => {
      const missing = [!keys.ANTHROPIC_API_KEY && "Anthropic", config.voice.enabled && !keys.ELEVENLABS_API_KEY && "ElevenLabs"].filter(Boolean);
      if (missing.length) showToast(`Add your ${missing.join(" and ")} API key in Settings → API keys.`, "warn");
    }).catch(() => {});
  }
  void api.invoke("timers:list").then(timers => {
    if (!receivedTimers) orbVisual.setTimers(timers);
  }).catch(() => {});
  window.addEventListener("pagehide", () => {
    unsubscribe();
    orbVisual.dispose();
  }, { once: true });

  // Drag the orb to move Jarvis. A press that moves > 5 px is a drag; otherwise it's a normal click.
  // Main moves the window with the cursor between "start" and "end" and remembers the spot.
  let press: { x: number; y: number } | null = null;
  let dragging = false;
  let suppressClick = false;
  orb.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    press = { x: event.screenX, y: event.screenY };
    orb.setPointerCapture(event.pointerId);
  });
  orb.addEventListener("pointermove", event => {
    if (!press || dragging) return;
    if (Math.hypot(event.screenX - press.x, event.screenY - press.y) < 5) return;
    dragging = true;
    hud.dataset.dragging = "";
    void api.invoke("window:drag", "start");
  });
  const endPress = () => {
    press = null;
    if (!dragging) return;
    dragging = false;
    suppressClick = true;
    delete hud.dataset.dragging;
    ignoring = undefined; // main ignored click-through changes during the drag; resync on next move
    void api.invoke("window:drag", "end");
  };
  orb.addEventListener("pointerup", endPress);
  orb.addEventListener("pointercancel", endPress);
  orb.addEventListener("lostpointercapture", endPress);

  // Electron forwards pointer movement while the window is click-through.
  document.addEventListener("mousemove", event => {
    if (dragging) return;
    const target = event.target;
    setIgnore(!(target instanceof Element && target.closest("[data-hit]")));
  });
  document.documentElement.addEventListener("mouseleave", () => {
    if (!dragging) setIgnore(true);
  });
  orb.addEventListener("click", () => {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    // Clicking the orb while he talks (or works on a reply) cuts him off instead of opening the text box.
    if (jarvisState === "speaking" || jarvisState === "thinking") {
      api.send({ type: "interrupt", reason: "user" });
      return;
    }
    showInput(true);
  });
  orb.addEventListener("contextmenu", event => {
    event.preventDefault();
    void api.invoke("window:open-settings").catch(() => showToast("Could not open settings.", "error"));
  });
  closeMessage.addEventListener("click", () => {
    hideInput();
    orb.focus({ preventScroll: true });
  });
  form.addEventListener("submit", event => {
    event.preventDefault();
    const text = message.value.trim();
    if (!text) return;
    api.send({ type: "user_text", text });
    user.textContent = text;
    assistant.textContent = "";
    streaming = false;
    updateCaptions();
    message.value = "";
    hideInput();
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") {
      hideInput();
      api.send({ type: "interrupt", reason: "user" });
    }
  });
}

