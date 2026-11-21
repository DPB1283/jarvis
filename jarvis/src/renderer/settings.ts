import { offerOnboarding } from "./onboarding";
import { mountBrowserSettings } from "./browser-settings";
import { mountApiKeys } from "./api-keys";
import { DEFAULT_SETTINGS, LANGUAGES, type Language, type Settings, type SettingsPatch, type TimerInfo, type VoiceOption } from "../shared/settings";
import { mountMemory } from "./memory";
import { mountRoutines } from "./routines";
import { mountChatApps } from "./chat-apps";
import { mountSettingsTabs, type PreferenceTab, type SettingsTab } from "./settings-tabs";

function option(value: string, label: string) {
  return new Option(label, value);
}

export function mountSettings() {
  const api = window.jarvis;
  const root = document.querySelector<HTMLElement>("#settings")!;
  const form = document.querySelector<HTMLFormElement>("#settings-form")!;
  const status = document.querySelector<HTMLElement>("#settings-status")!;
  const save = document.querySelector<HTMLButtonElement>("#settings-save")!;
  const footer = document.querySelector<HTMLElement>("#settings-actions")!;
  const unsaved = document.querySelector<HTMLElement>("#settings-dirty")!;
  const report = (message: string, error = false) => {
    status.textContent = message;
    status.dataset.error = String(error);
  };
  let saved: Settings | undefined;
  let voices: VoiceOption[] = [];
  let dirty = false;
  let preference = true;
  let activeTab: SettingsTab = "general";
  const mounted = new Set<SettingsTab>();
  const sections = {
    routines: mountRoutines, timers: mountTimers, memory: mountMemory, messaging: mountChatApps, "api-keys": mountApiKeys, browser: mountBrowserSettings,
  };
  function updateFooter() {
    unsaved.hidden = !dirty;
    footer.hidden = !preference && !dirty;
  }
  const tabs = mountSettingsTabs(
    document.querySelector<HTMLElement>("#settings-tabs")!,
    form,
    document.querySelector<HTMLElement>("#settings-managed")!,
    (id, isPreference) => {
      activeTab = id;
      preference = isPreference;
      updateFooter();
      if (id === "voice" && saved && !mounted.has(id)) {
        mounted.add(id);
        void loadVoices();
        void loadDevices();
      }
      if (!isPreference && !mounted.has(id)) {
        mounted.add(id);
        sections[id as keyof typeof sections](tabs.panel(id));
      }
    },
  );

  function group(tab: PreferenceTab, title: string) {
    const section = document.createElement("fieldset");
    const heading = document.createElement("legend");
    heading.textContent = title;
    section.append(heading);
    tabs.panel(tab).append(section);
    return section;
  }

  function field(parent: HTMLElement, title: string, control: HTMLElement) {
    const label = document.createElement("label");
    label.className = "setting";
    const name = document.createElement("span");
    name.textContent = title;
    const input = control.matches("input, select") ? control : control.querySelector("input");
    input?.setAttribute("aria-label", title);
    label.append(name, control);
    parent.append(label);
  }

  function input(parent: HTMLElement, label: string, type = "text") {
    const control = document.createElement("input");
    control.type = type;
    field(parent, label, control);
    return control;
  }

  function range(parent: HTMLElement, label: string, min: number, max: number, step: number) {
    const control = document.createElement("input");
    control.type = "range";
    Object.assign(control, { min: String(min), max: String(max), step: String(step) });
    const entry = document.createElement("span");
    entry.className = "slider";
    const output = document.createElement("output");
    function update() { output.value = control.value; }
    control.addEventListener("input", update);
    entry.append(control, output);
    field(parent, label, entry);
    return (value?: number) => {
      if (value !== undefined) {
        control.value = String(value);
        update();
      }
      return control.valueAsNumber;
    };
  }

  function select(parent: HTMLElement, label: string, entries: Array<[string, string]>) {
    const control = document.createElement("select");
    control.append(...entries.map(([value, name]) => option(value, name)));
    field(parent, label, control);
    return control;
  }

  const home = group("general", "Home");
  const city = input(home, "Home city");
  city.placeholder = "City and region";
  const units = select(home, "Weather units", [["metric", "Metric (°C)"], ["imperial", "Imperial (°F)"]]);
  const newsRegion = input(home, "News country code");
  newsRegion.maxLength = 2;
  newsRegion.pattern = "[a-zA-Z]{2}";
  newsRegion.required = true;
  newsRegion.title = "Two-letter country code, such as US or GB";
  const newsSources = select(home, "National and world news", [
    ["neutral", "Wire services only"],
    ["all", "All outlets"],
  ]);
  const showMaps = input(home, "Let Jarvis show maps and routes on screen", "checkbox");

  // Home address: private. It is kept encrypted, hidden here until you press Show, never sent to the AI, and
  // only used by Jarvis to work out travel times from home. It saves with its own buttons, not with Save settings.
  const address = document.createElement("input");
  address.type = "password";
  address.autocomplete = "off";
  address.spellcheck = false;
  address.placeholder = "Not saved";
  const addressButtons = document.createElement("span");
  addressButtons.className = "actions";
  const addressButton = (label: string) => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    return b;
  };
  const addressShow = addressButton("Show");
  const addressSave = addressButton("Save address");
  const addressRemove = addressButton("Remove");
  const addressStatus = document.createElement("small");
  addressStatus.setAttribute("role", "status");
  addressButtons.append(addressShow, addressSave, addressRemove, addressStatus);
  const addressBox = document.createElement("span");
  addressBox.className = "address-box";
  addressBox.append(address, addressButtons);
  field(home, "Home address (private, for travel times)", addressBox);
  let addressSaved = false;
  function addressView(text: string) {
    address.type = "password";
    addressShow.textContent = "Show";
    addressStatus.textContent = text;
  }
  void api.invoke("address:get").then(value => {
    addressSaved = Boolean(value);
    address.value = value ?? "";
    addressView(addressSaved ? "Saved and hidden" : "Not saved");
  }).catch(() => addressView("Could not load"));
  addressShow.addEventListener("click", () => {
    const hidden = address.type === "password";
    address.type = hidden ? "text" : "password";
    addressShow.textContent = hidden ? "Hide" : "Show";
  });
  addressSave.addEventListener("click", async () => {
    const value = address.value.trim();
    if (!value) { addressStatus.textContent = "Type an address first"; return; }
    try {
      await api.invoke("address:set", value);
      addressSaved = true;
      addressView("Saved and hidden");
    } catch { addressStatus.textContent = "Could not save"; }
  });
  addressRemove.addEventListener("click", async () => {
    try {
      await api.invoke("address:clear");
      addressSaved = false;
      address.value = "";
      addressView("Removed");
    } catch { addressStatus.textContent = "Could not remove"; }
  });
  newsSources.title = "Wire services: Reuters, AP and AFP";

  const personality = group("general", "Personality");
  const title = input(personality, "Address me as");
  title.required = true;
  title.placeholder = "sir";
  const userName = input(personality, "Your name (used now and then)");
  userName.placeholder = "optional";
  userName.autocomplete = "given-name";
  const ultron = input(personality, "Ultron mode", "checkbox");
  const ultronInsults = input(personality, "Ultron may insult me (calls you stupid, insect and so on, in character)", "checkbox");
  const sarcasm = range(personality, "Sarcasm", 0, 10, 1);
  const language = select(personality, "Language (he speaks and understands it)", LANGUAGES.map(([code, name]): [string, string] => [code, name]));
  const reply = select(personality, "Reply length", [["brief", "Brief (one short sentence)"], ["middle", "Middle brief (one or two sentences)"], ["normal", "Normal"]]);

  const speech = group("voice", "Speech");
  const voiceOn = input(speech, "Use voice (ElevenLabs). Turn off for text only: no microphone, wake word or spoken replies", "checkbox");
  const voice = select(speech, "Voice", []);
  const voiceActions = document.createElement("div");
  voiceActions.className = "actions";
  const preview = document.createElement("button");
  preview.type = "button";
  preview.textContent = "Preview voice";
  const load = document.createElement("button");
  load.type = "button";
  load.textContent = "Load voices";
  voiceActions.append(load, preview);
  speech.append(voiceActions);
  const speed = range(speech, "Speed", 0.7, 1.2, 0.05);
  const stability = range(speech, "Stability", 0, 1, 0.05);
  const volume = range(speech, "Voice volume (%)", 0, 200, 5);
  const muted = input(speech, "Mute spoken replies", "checkbox");

  const audioGroup = group("voice", "Audio devices");
  const microphone = select(audioGroup, "Microphone (input)", [["", "Windows default"]]);
  const micGain = range(audioGroup, "Microphone gain (%)", 0, 500, 10);
  const speaker = select(audioGroup, "Speaker (output)", [["", "Windows default"]]);
  const audioActions = document.createElement("div");
  audioActions.className = "actions";
  const refreshDevices = document.createElement("button");
  refreshDevices.type = "button";
  refreshDevices.textContent = "Refresh devices";
  const testMic = document.createElement("button");
  testMic.type = "button";
  testMic.textContent = "Test microphone";
  audioActions.append(refreshDevices, testMic);
  audioGroup.append(audioActions);
  const micTest = document.createElement("div");
  micTest.hidden = true;
  const meterLabel = document.createElement("label");
  meterLabel.textContent = "Microphone level";
  const meter = document.createElement("meter");
  meter.min = 0;
  meter.max = 1;
  meter.value = 0;
  meter.setAttribute("aria-label", "Microphone level");
  const micStatus = document.createElement("p");
  micStatus.setAttribute("role", "status");
  micTest.append(meterLabel, meter, micStatus);
  audioGroup.append(micTest);
  let testGeneration = 0;
  let testStream: MediaStream | undefined;
  function stopMicTest() {
    testGeneration++;
    testStream?.getTracks().forEach(track => track.stop());
    testStream = undefined;
    testMic.textContent = "Test microphone";
    micTest.hidden = true;
    meter.value = 0;
  }
  testMic.addEventListener("click", async () => {
    if (!micTest.hidden) return stopMicTest();
    const generation = ++testGeneration;
    micTest.hidden = false;
    micStatus.textContent = "Speak into your microphone. The meter should move.";
    testMic.textContent = "Stop test";
    try {
      if ("__TAURI_INTERNALS__" in window) {
        while (generation === testGeneration) {
          const level = await api.invoke("audio:test-input", microphone.value, micGain());
          if (generation === testGeneration) meter.value = level;
        }
      } else {
        const devices = await navigator.mediaDevices.enumerateDevices();
        const selected = devices.find(device => device.kind === "audioinput" && device.label === microphone.value);
        const stream = await navigator.mediaDevices.getUserMedia({ audio: selected ? { deviceId: { exact: selected.deviceId } } : true });
        if (generation !== testGeneration) { stream.getTracks().forEach(track => track.stop()); return; }
        testStream = stream;
        const context = new AudioContext();
        const analyser = context.createAnalyser();
        context.createMediaStreamSource(stream).connect(analyser);
        const samples = new Float32Array(analyser.fftSize);
        while (generation === testGeneration) {
          analyser.getFloatTimeDomainData(samples);
          meter.value = Math.min(1, samples.reduce((peak, value) => Math.max(peak, Math.abs(value)), 0) * micGain() / 100);
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        await context.close();
      }
    } catch (error) {
      if (generation !== testGeneration) return;
      testStream?.getTracks().forEach(track => track.stop());
      testStream = undefined;
      testMic.textContent = "Close test";
      micStatus.textContent = `Could not access the microphone: ${String(error)}`;
    }
  });
  microphone.addEventListener("change", stopMicTest);
  window.addEventListener("pagehide", stopMicTest, { once: true });
  let micNames: string[] = [];
  let speakers: Array<{ id: string; label: string }> = [];

  function showDevices(selectedMic: string, selectedSpeakerId: string, selectedSpeakerLabel: string) {
    const mics = micNames.map(name => option(name, name));
    if (selectedMic && !micNames.includes(selectedMic)) mics.unshift(option(selectedMic, `${selectedMic} (not connected)`));
    microphone.replaceChildren(option("", "Windows default"), ...mics);
    microphone.value = selectedMic;

    const outs = speakers.map(s => option(s.id, s.label));
    const known = speakers.find(s => s.id === selectedSpeakerId) ?? speakers.find(s => s.label === selectedSpeakerLabel);
    if (selectedSpeakerLabel && !known) outs.unshift(option(selectedSpeakerId, `${selectedSpeakerLabel} (not connected)`));
    speaker.replaceChildren(option("", "Windows default"), ...outs);
    speaker.value = known?.id ?? selectedSpeakerId;
  }

  async function loadDevices() {
    refreshDevices.disabled = true;
    const prevMic = microphone.value;
    const prevSpeakerId = speaker.value;
    const prevSpeakerLabel = speakers.find(s => s.id === prevSpeakerId)?.label ?? saved?.audio?.outputDevice ?? "";
    try {
      micNames = await api.invoke("audio:inputs");
      speakers = "__TAURI_INTERNALS__" in window
        ? (await api.invoke("audio:outputs")).map(label => ({ id: label, label }))
        : (await navigator.mediaDevices.enumerateDevices())
          .filter(d => d.kind === "audiooutput" && d.deviceId !== "default" && d.deviceId !== "communications")
          .map((d, i) => ({ id: d.deviceId, label: d.label || `Speaker ${i + 1}` }));
      showDevices(prevMic, prevSpeakerId, prevSpeakerLabel);
    } catch {
      report("Could not list audio devices.", true);
    } finally {
      refreshDevices.disabled = false;
    }
  }
  refreshDevices.addEventListener("click", () => void loadDevices());
  navigator.mediaDevices?.addEventListener("devicechange", () => void loadDevices());

  const display = group("display", "Display");
  const captions = input(display, "Show captions", "checkbox");
  const position = select(display, "Caption position", [["below_orb", "By the orb"], ["bottom_center", "Bottom center"]]);
  const captionSize = range(display, "Caption size (px)", 10, 28, 1);
  const corner = select(display, "Orb corner", [
    ["top_right", "Top right"], ["top_left", "Top left"],
    ["bottom_right", "Bottom right"], ["bottom_left", "Bottom left"],
  ]);
  const showInCaptures = input(display, "Show Jarvis in screen shares, recordings and screenshots", "checkbox");
  const orbSize = range(display, "Orb size (px)", 48, 180, 1);
  const opacity = range(display, "Orb opacity", 0.2, 1, 0.05);

  const system = group("controls", "Controls");
  const safety = select(system, "Approval mode", [
    ["ask_all", "Ask every time"], ["ask_unsafe", "Ask before risky actions"], ["full_access", "Full access"],
  ]);
  const wake = input(system, "Enable “Jarvis” wake word", "checkbox");
  const sensitivity = range(system, "Wake word sensitivity", 0, 1, 0.05);
  const followUp = range(system, "Keep listening after a reply (seconds, 0 = off)", 0, 180, 5);
  const endOfSpeech = range(system, "Wait after I stop talking (seconds)", 0.4, 3, 0.1);
  const interruptBySpeaking = input(system, "Cut Jarvis off when I start talking over him", "checkbox");
  const talk = input(system, "Talk hotkey");
  const settingsKey = input(system, "Settings hotkey");
  const muteKey = input(system, "Mute / unmute voice hotkey");
  const startup = input(system, "Launch at startup", "checkbox");

  const gaming = group("controls", "Gaming and PC health");
  const gameMode = select(gaming, "Game mode", [
    ["auto", "Auto-detect fullscreen games"], ["on", "Always on"], ["off", "Off"],
  ]);
  const health = input(gaming, "Warn me about PC health", "checkbox");
  const clipApp = select(gaming, "Clip app", [
    ["auto", "Auto-detect"], ["medal", "Medal"], ["nvidia", "NVIDIA"], ["xbox", "Xbox Game Bar"],
  ]);
  const clipHotkey = input(gaming, "Clip hotkey");
  clipHotkey.placeholder = "Default";

  const native = "__TAURI_INTERNALS__" in window;
  const updates = group("general", "Updates");
  updates.hidden = !native;
  const autoUpdate = input(updates, "Automatically check for updates", "checkbox");
  const updateStatus = document.createElement("p");
  updateStatus.setAttribute("role", "status");
  const updateNotes = document.createElement("p");
  updateNotes.className = "update-notes";
  updateNotes.hidden = true;
  const updateActions = document.createElement("div");
  updateActions.className = "actions";
  const checkUpdates = document.createElement("button");
  checkUpdates.type = "button";
  checkUpdates.textContent = "Check for updates";
  const installUpdate = document.createElement("button");
  installUpdate.type = "button";
  installUpdate.textContent = "Install update";
  installUpdate.hidden = true;
  updateActions.append(checkUpdates, installUpdate);
  updates.append(updateStatus, updateActions, updateNotes);
  async function getUpdate(check: boolean) {
    checkUpdates.disabled = true;
    if (check) updateStatus.textContent = "Checking for updates…";
    try {
      const result = await api.invoke(check ? "update:check" : "update:status");
      const current = "Current version " + result.current;
      updateStatus.textContent = result.available ? current + " · Update " + result.latest + " available" : current + (check ? " · You're up to date" : "");
      updateNotes.textContent = result.notes;
      updateNotes.hidden = !result.notes;
      installUpdate.hidden = !result.installable || !result.available;
    } catch {
      updateStatus.textContent = "Could not check for updates.";
    } finally {
      checkUpdates.disabled = false;
    }
  }
  checkUpdates.addEventListener("click", () => void getUpdate(true));
  installUpdate.addEventListener("click", async () => {
    installUpdate.disabled = true;
    try { await api.invoke("update:install"); }
    catch { updateStatus.textContent = "Could not install the update."; }
    finally { installUpdate.disabled = false; }
  });
  if (native) void getUpdate(false);

  const history = group("controls", "Local history");
  history.hidden = !native;
  const clipboardHistory = input(history, "Keep clipboard history", "checkbox");
  const screenHistory = input(history, "Keep recent screen snapshots", "checkbox");
  const historyNote = document.createElement("p");
  historyNote.className = "settings-help";
  historyNote.textContent = "Both are off by default. History is stored only on this PC. Small screen snapshots are kept for about one day.";
  history.append(historyNote);

  const appRules = group("controls", "App rules");
  appRules.hidden = !native;
  const ruleList = document.createElement("div");
  const addRule = document.createElement("button");
  addRule.type = "button";
  addRule.textContent = "Add app rule";
  type AppRule = NonNullable<Settings["appRules"]>[number];
  const rules: Array<() => AppRule> = [];
  function addRuleRow(value: AppRule = { app: "", silent: false, replyLength: "", askFirst: false }) {
    const row = document.createElement("div");
    row.className = "app-rule";
    const app = input(row, "App or window title");
    app.value = value.app;
    const silent = input(row, "Silent", "checkbox");
    silent.checked = value.silent;
    const length = select(row, "Reply length", [["", "Default"], ["brief", "Brief"], ["middle", "Medium"], ["normal", "Normal"]]);
    length.value = value.replyLength;
    const askFirst = input(row, "Ask first", "checkbox");
    askFirst.checked = value.askFirst;
    const read = (): AppRule => ({ app: app.value.trim(), silent: silent.checked, replyLength: length.value as AppRule["replyLength"], askFirst: askFirst.checked });
    rules.push(read);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remove";
    remove.setAttribute("aria-label", "Remove app rule");
    remove.addEventListener("click", () => {
      rules.splice(rules.indexOf(read), 1);
      row.remove();
      addRule.focus();
      form.dispatchEvent(new Event("input"));
    });
    row.append(remove);
    ruleList.append(row);
    return app;
  }
  addRule.addEventListener("click", () => addRuleRow().focus());
  appRules.append(ruleList, addRule);

  const calendar = group("general", "Calendar");
  const calendarUrl = input(calendar, "Google Calendar secret iCal URL", "password");
  calendarUrl.autocomplete = "off";
  const calendarHelp = document.createElement("p");
  calendarHelp.className = "settings-help";
  calendarHelp.id = "calendar-help";
  calendarHelp.textContent = "In Google Calendar, select a calendar in Settings. Under Integrate calendar, copy the Secret address in iCal format.";
  calendarUrl.setAttribute("aria-describedby", calendarHelp.id);
  calendar.append(calendarHelp);
  const nudge = input(calendar, "Remind me about upcoming events", "checkbox");
  const lead = input(calendar, "Remind me this many minutes before", "number");
  lead.min = "1";
  lead.max = "240";
  lead.step = "1";
  lead.required = native;
  nudge.closest("label")!.hidden = lead.closest("label")!.hidden = !native;
  const briefing = group("general", "Morning briefing");
  briefing.hidden = !native;
  const briefingTime = input(briefing, "Time", "time");
  briefingTime.value = "07:30";
  const briefingDays = document.createElement("div");
  briefingDays.className = "briefing-days";
  const dayChoices = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day, index) => {
    const checkbox = input(briefingDays, day, "checkbox");
    checkbox.checked = index >= 1 && index <= 5;
    return checkbox;
  });
  briefing.append(briefingDays);
  // These fields belong to the routine button, independent of Save settings.
  briefing.addEventListener("input", event => event.stopPropagation());
  const addBriefing = document.createElement("button");
  addBriefing.type = "button";
  addBriefing.textContent = "Create briefing routine";
  const briefingStatus = document.createElement("p");
  briefingStatus.setAttribute("role", "status");
  addBriefing.addEventListener("click", async () => {
    const days = dayChoices.flatMap((choice, day) => choice.checked ? [day] : []);
    if (!briefingTime.value || !days.length) {
      briefingStatus.textContent = "Choose a time and at least one day.";
      return;
    }
    addBriefing.disabled = true;
    try {
      await api.invoke("routines:add-briefing", briefingTime.value, days);
      briefingStatus.textContent = "Morning briefing added to Routines.";
    } catch { briefingStatus.textContent = "Could not create the briefing routine."; }
    finally { addBriefing.disabled = false; }
  });
  briefing.append(addBriefing, briefingStatus);

  function showVoices(selected: string) {
    const options = voices.map(item => option(item.voiceId, item.description ? `${item.name} · ${item.description}` : item.name));
    if (selected && !voices.some(item => item.voiceId === selected)) {
      options.unshift(option(selected, `Configured voice (${selected})`));
    }
    voice.replaceChildren(option("", "Default ElevenLabs voice"), ...options);
    voice.value = selected;
  }

  function render(value: Settings) {
    saved = value;
    const location = value.location ?? DEFAULT_SETTINGS.location;
    city.value = location.city;
    units.value = location.units;
    newsRegion.value = location.newsRegion;
    newsSources.value = location.newsSources ?? "neutral";
    showMaps.checked = location.showMaps !== false;
    ultron.checked = value.ultronMode;
    ultronInsults.checked = value.ultronInsults === true;
    language.value = value.language ?? "en";
    title.value = value.userTitle;
    userName.value = value.userName ?? "";
    sarcasm(value.personality.sarcasm);
    reply.value = value.personality.replyLength;
    showVoices(value.voice.voiceId);
    speed(value.voice.speed);
    stability(value.voice.stability);
    volume(value.voice.volume ?? 100);
    muted.checked = value.voice.muted;
    voiceOn.checked = value.voice.enabled !== false;
    micGain(value.audio?.inputGain ?? 100);
    showDevices(value.audio?.inputDevice ?? "", value.audio?.outputDeviceId ?? "", value.audio?.outputDevice ?? "");
    captions.checked = value.captions.enabled;
    position.value = value.captions.position;
    captionSize(value.captions.size);
    corner.value = value.orb.corner;
    showInCaptures.checked = value.orb.showInCaptures ?? true;
    orbSize(value.orb.size);
    opacity(value.orb.opacity);
    safety.value = value.safetyMode;
    wake.checked = value.wakeWord.enabled;
    sensitivity(value.wakeWord.sensitivity);
    followUp(value.conversation?.followUpSeconds ?? 60);
    endOfSpeech(value.conversation?.endOfSpeechSeconds ?? 1.2);
    interruptBySpeaking.checked = value.conversation?.interruptBySpeaking !== false;
    talk.value = value.hotkeys.talk;
    settingsKey.value = value.hotkeys.settings;
    muteKey.value = value.hotkeys.mute ?? "Control+Alt+M";
    startup.checked = value.launchAtStartup;
    gameMode.value = value.gameMode?.mode ?? DEFAULT_SETTINGS.gameMode.mode;
    health.checked = value.health?.warnings ?? DEFAULT_SETTINGS.health.warnings;
    clipApp.value = value.clips?.app ?? DEFAULT_SETTINGS.clips.app;
    clipHotkey.value = value.clips?.hotkey ?? "";
    calendarUrl.value = value.calendar?.icsUrl ?? "";
    autoUpdate.checked = value.updates?.check ?? true;
    clipboardHistory.checked = value.history?.clipboard ?? false;
    screenHistory.checked = value.history?.screens ?? false;
    nudge.checked = value.nudges?.enabled ?? true;
    lead.value = String(value.nudges?.leadMinutes ?? 10);
    ruleList.replaceChildren();
    rules.length = 0;
    for (const rule of value.appRules ?? []) addRuleRow(rule);
  }

  function changedFields<T extends object>(baseline: T, values: T): Partial<T> {
    const patch: Partial<T> = {};
    for (const key of Object.keys(values) as Array<keyof T>) {
      if (baseline[key] !== values[key]) patch[key] = values[key];
    }
    return patch;
  }

  async function loadVoices() {
    load.disabled = true;
    try {
      voices = await api.invoke("voices:list");
      showVoices(voice.value);
      report(voices.length ? "" : "No ElevenLabs voices are available.");
    } catch {
      report("Could not load ElevenLabs voices.", true);
    } finally {
      load.disabled = false;
    }
  }

  load.addEventListener("click", () => void loadVoices());
  preview.addEventListener("click", async () => {
    if (!voice.value) {
      report("Pick a voice from the list first.", true);
      return;
    }
    preview.disabled = true;
    report("");
    try {
      await api.invoke("voice:preview", voice.value);
    } catch {
      report("Could not preview this voice.", true);
    } finally {
      preview.disabled = false;
    }
  });
  form.addEventListener("input", () => {
    if (!dirty) {
      dirty = true;
      report("");
      updateFooter();
    }
  });
  form.addEventListener("submit", async event => {
    event.preventDefault();
    if (!saved || save.disabled) return;
    const userTitle = title.value.trim();
    if (!userTitle) {
      report("Enter the name Jarvis should use.", true);
      title.focus();
      return;
    }
    const currentRules = rules.map(read => read()).filter(rule => rule.app);
    const patch: SettingsPatch = {
      ...(native ? {
        ...((saved.updates?.check ?? true) === autoUpdate.checked ? {} : { updates: { check: autoUpdate.checked } }),
        ...((saved.history?.clipboard ?? false) === clipboardHistory.checked && (saved.history?.screens ?? false) === screenHistory.checked ? {} : { history: { clipboard: clipboardHistory.checked, screens: screenHistory.checked } }),
        ...((saved.nudges?.enabled ?? true) === nudge.checked && (saved.nudges?.leadMinutes ?? 10) === Number(lead.value) ? {} : { nudges: { enabled: nudge.checked, leadMinutes: Number(lead.value) || 10 } }),
        ...(JSON.stringify(currentRules) === JSON.stringify(saved.appRules ?? []) ? {} : { appRules: currentRules }),
      } : {}),
      ...changedFields(
        { ultronMode: saved.ultronMode, ultronInsults: saved.ultronInsults === true, language: saved.language ?? "en", userTitle: saved.userTitle, userName: saved.userName ?? "", safetyMode: saved.safetyMode, launchAtStartup: saved.launchAtStartup },
        { ultronMode: ultron.checked, ultronInsults: ultronInsults.checked, language: language.value as Language, userTitle, userName: userName.value.trim(), safetyMode: safety.value as Settings["safetyMode"], launchAtStartup: startup.checked },
      ),
      location: changedFields(saved.location ?? DEFAULT_SETTINGS.location, {
        city: city.value.trim(),
        units: units.value === "imperial" ? "imperial" : "metric",
        newsRegion: newsRegion.value.toUpperCase(),
        newsSources: newsSources.value === "all" ? "all" : "neutral",
        showMaps: showMaps.checked,
      }),
      captions: changedFields(saved.captions, { enabled: captions.checked, position: position.value as Settings["captions"]["position"], size: captionSize() }),
      voice: changedFields(saved.voice, { voiceId: voice.value, speed: speed(), stability: stability(), volume: volume(), muted: muted.checked, enabled: voiceOn.checked }),
      audio: changedFields(saved.audio ?? { inputDevice: "", inputGain: 100, outputDevice: "", outputDeviceId: "" }, {
        inputDevice: microphone.value,
        inputGain: micGain(),
        outputDeviceId: speaker.value,
        outputDevice: speaker.value
          ? speakers.find(s => s.id === speaker.value)?.label ?? saved.audio?.outputDevice ?? ""
          : "",
      }),
      personality: changedFields(saved.personality, { sarcasm: sarcasm(), replyLength: reply.value as Settings["personality"]["replyLength"] }),
      wakeWord: changedFields(saved.wakeWord, { enabled: wake.checked, sensitivity: sensitivity() }),
      conversation: changedFields(saved.conversation ?? { followUpSeconds: 60, endOfSpeechSeconds: 1.2 }, {
        followUpSeconds: followUp(),
        endOfSpeechSeconds: Math.round(endOfSpeech() * 10) / 10,
        interruptBySpeaking: interruptBySpeaking.checked,
      }),
      hotkeys: changedFields(saved.hotkeys, { talk: talk.value.trim(), settings: settingsKey.value.trim(), mute: muteKey.value.trim() }),
      orb: changedFields(saved.orb, { corner: corner.value as Settings["orb"]["corner"], position: saved.orb.position, size: orbSize(), opacity: opacity(), showInCaptures: showInCaptures.checked }),
      gameMode: changedFields(saved.gameMode ?? DEFAULT_SETTINGS.gameMode, { mode: gameMode.value as Settings["gameMode"]["mode"] }),
      health: changedFields(saved.health ?? DEFAULT_SETTINGS.health, { warnings: health.checked }),
      clips: changedFields(saved.clips ?? DEFAULT_SETTINGS.clips, { app: clipApp.value as Settings["clips"]["app"], hotkey: clipHotkey.value.trim() }),
      calendar: changedFields(saved.calendar ?? DEFAULT_SETTINGS.calendar, { icsUrl: calendarUrl.value.trim() }),
    };
    save.disabled = true;
    for (const section of form.querySelectorAll("fieldset")) section.disabled = true;
    try {
      render(await api.invoke("settings:update", patch));
      dirty = false;
      report("Settings saved.");
      updateFooter();
    } catch {
      report("Could not save settings. Check the values and try again.", true);
    } finally {
      for (const section of form.querySelectorAll("fieldset")) section.disabled = false;
      save.disabled = false;
    }
  });
  const unsubscribe = api.on(event => {
    if (event.type === "settings_changed" && !dirty) render(event.settings);
    if (event.type === "update_available" && native) void getUpdate(false);
  });
  window.addEventListener("pagehide", unsubscribe, { once: true });
  save.disabled = true;
  void api.invoke("settings:get").then(value => {
    if (!saved) render(value);
    if ("__TAURI_INTERNALS__" in window && value.onboarded === false) {
      offerOnboarding(root, value, () => tabs.activate("api-keys", true));
    }
    save.disabled = false;
    if (activeTab === "voice" && !mounted.has("voice")) {
      mounted.add("voice");
      void loadVoices();
      void loadDevices();
    }
  }).catch(() => report("Could not load settings.", true));
  root.hidden = false;
}

function mountTimers(root: HTMLElement) {
  const api = window.jarvis;
  const heading = document.createElement("h2");
  heading.textContent = "Running timers";
  const list = document.createElement("div");
  list.className = "routine-list";
  const status = document.createElement("p");
  status.className = "settings-status";
  status.dataset.error = "true";
  status.role = "status";
  root.append(heading, status, list);
  let revision = 0;
  const pending = new Set<string>();
  const date = new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "medium" });

  function render(timers: TimerInfo[]) {
    const rows = [...timers].sort((a, b) => a.endsAt - b.endsAt).map(timer => {
      const row = document.createElement("article");
      const info = document.createElement("div");
      const name = document.createElement("h3");
      name.textContent = timer.label;
      const ends = document.createElement("p");
      ends.textContent = `${timer.kind === "alarm" ? "Alarm" : "Timer"} · ${date.format(timer.endsAt)}`;
      info.append(name, ends);
      const cancel = document.createElement("button");
      cancel.type = "button";
      cancel.textContent = "Cancel";
      cancel.disabled = pending.has(timer.id);
      cancel.setAttribute("aria-label", `Cancel ${timer.label}`);
      cancel.addEventListener("click", async () => {
        if (pending.has(timer.id)) return;
        pending.add(timer.id);
        cancel.disabled = true;
        status.textContent = "";
        try {
          await api.invoke("timers:cancel", timer.id);
        } catch {
          status.textContent = "Could not cancel the timer.";
        } finally {
          pending.delete(timer.id);
          cancel.disabled = false;
          void reload();
        }
      });
      row.append(info, cancel);
      return row;
    });
    const empty = document.createElement("p");
    empty.textContent = "No timers or alarms running.";
    list.replaceChildren(...(rows.length ? rows : [empty]));
  }

  async function reload() {
    const request = ++revision;
    try {
      const items = await api.invoke("timers:list");
      if (request === revision) render(items);
    } catch {
      if (request === revision) status.textContent = "Could not load timers.";
    }
  }
  const unsubscribe = api.on(event => {
    if (event.type === "timers") {
      revision++;
      render(event.timers);
    }
  });
  window.addEventListener("pagehide", unsubscribe, { once: true });
  void reload();
}

