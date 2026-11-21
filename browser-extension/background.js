let socket;
let connecting = false;
let serial = Promise.resolve();
const alarm = 'jarvis-reconnect';

function allowed(url) {
  const parsed = new URL(url);
  if (!['https:', 'http:'].includes(parsed.protocol)) throw new Error('Only HTTP and HTTPS pages are supported.');
  return parsed.href;
}

async function connect() {
  if (connecting || socket?.readyState === WebSocket.OPEN || socket?.readyState === WebSocket.CONNECTING) return;
  const { port, code } = await chrome.storage.local.get(['port', 'code']);
  if (!port || !code) return;
  connecting = true;
  try {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/`);
    socket = ws;
    ws.onopen = () => { connecting = false; ws.send(JSON.stringify({ code })); };
    ws.onclose = () => { connecting = false; if (socket === ws) socket = null; };
    ws.onerror = () => { connecting = false; };
    ws.onmessage = event => {
      let request;
      try { request = JSON.parse(event.data); } catch { return; }
      if (!request.id) return;
      serial = serial.then(async () => {
        try {
          const result = await execute(request.action, request.input || {});
          if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ id: request.id, result }));
        } catch (error) {
          if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ id: request.id, error: String(error?.message || error) }));
        }
      });
    };
  } catch { connecting = false; }
}

chrome.alarms.create(alarm, { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(a => { if (a.name === alarm) void connect(); });
chrome.runtime.onStartup.addListener(() => void connect());
chrome.runtime.onInstalled.addListener(() => void connect());
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message.type === 'pair') {
    (async () => {
      await chrome.storage.local.set({ port: message.port, code: message.code });
      socket?.close(); socket = null; connecting = false;
      await connect();
      respond({ ok: true });
    })().catch(() => respond({ ok: false }));
    return true;
  }
});
void connect();

async function owned() {
  const { groupId, tabIds = [] } = await chrome.storage.session.get(['groupId', 'tabIds']);
  if (!Number.isInteger(groupId)) return { groupId: null, tabs: [] };
  let group;
  try { group = await chrome.tabGroups.get(groupId); } catch { return { groupId: null, tabs: [] }; }
  if (group.title !== 'Jarvis') return { groupId: null, tabs: [] };
  const actual = await chrome.tabs.query({ groupId });
  return { groupId, tabs: actual.filter(tab => tabIds.includes(tab.id)) };
}

async function getOwned(id) {
  const { tabs } = await owned();
  const tab = tabs.find(t => t.id === Number(id));
  if (!tab) throw new Error('That tab is not owned by Jarvis.');
  return tab;
}

async function open(url) {
  const { groupId, tabs } = await owned();
  const tab = await chrome.tabs.create({ url: allowed(url), active: false, ...(tabs[0]?.windowId ? { windowId: tabs[0].windowId } : {}) });
  try {
    const target = groupId == null ? await chrome.tabs.group({ tabIds: [tab.id] }) : await chrome.tabs.group({ tabIds: [tab.id], groupId });
    await chrome.tabGroups.update(target, { title: 'Jarvis', color: 'cyan', collapsed: true });
    await chrome.storage.session.set({ groupId: target, tabIds: [...tabs.map(t => t.id), tab.id] });
    return { id: tab.id, title: tab.title, url: tab.url };
  } catch (error) { await chrome.tabs.remove(tab.id).catch(() => {}); throw error; }
}

async function execute(action, input) {
  if (action === 'open') return open(input.url);
  if (action === 'list_tabs') return (await owned()).tabs.map(({ id, title, url, status }) => ({ id, title, url, status }));
  if (action === 'expand_group') {
    const { groupId } = await owned();
    if (groupId == null) throw new Error('No Jarvis tab group exists.');
    await chrome.tabGroups.update(groupId, { collapsed: false });
    return 'Jarvis tab group expanded.';
  }
  const tab = await getOwned(input.tab_id);
  if (action === 'close_tab') {
    await chrome.tabs.remove(tab.id);
    const { tabIds = [] } = await chrome.storage.session.get('tabIds');
    await chrome.storage.session.set({ tabIds: tabIds.filter(id => id !== tab.id) });
    return 'Tab closed.';
  }
  if (action === 'navigate') {
    await chrome.tabs.update(tab.id, { url: allowed(input.url) });
    return 'Navigation started.';
  }
  if (action === 'screenshot') {
    const target = { tabId: tab.id };
    await chrome.debugger.attach(target, '1.3');
    try {
      const result = await chrome.debugger.sendCommand(target, 'Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      return { data: result.data };
    } finally { await chrome.debugger.detach(target).catch(() => {}); }
  }
  if (action === 'press_key') {
    const keys = { Enter: 13, Tab: 9, Escape: 27, Backspace: 8, Delete: 46, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Home: 36, End: 35, ' ': 32 };
    const key = String(input.key || '');
    const virtualKey = keys[key] ?? (key.length === 1 && /^[a-z0-9]$/i.test(key) ? key.toUpperCase().charCodeAt(0) : null);
    if (virtualKey == null) throw new Error('Unsupported key.');
    if (input.selector) {
      await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: selector => {
        const element = document.querySelector(selector);
        if (!element) throw new Error('Element not found.');
        element.focus();
      }, args: [input.selector] });
    }
    await getOwned(tab.id);
    const target = { tabId: tab.id };
    await chrome.debugger.attach(target, '1.3');
    try {
      const event = { key, windowsVirtualKeyCode: virtualKey, nativeVirtualKeyCode: virtualKey };
      await chrome.debugger.sendCommand(target, 'Input.dispatchKeyEvent', { ...event, type: 'rawKeyDown' });
      await chrome.debugger.sendCommand(target, 'Input.dispatchKeyEvent', { ...event, type: 'keyUp' });
    } finally { await chrome.debugger.detach(target).catch(() => {}); }
    return 'Key pressed.';
  }
  if (!['read_page', 'click', 'type', 'scroll'].includes(action)) throw new Error('Unknown browser action.');
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: (action, input) => {
      if (action === 'read_page') {
        const text = document.body?.innerText?.slice(0, 25000) || '';
        const links = Array.from(document.querySelectorAll('a[href]')).filter(a => a.getClientRects().length).slice(0, 150).map(a => ({ text: (a.innerText || a.getAttribute('aria-label') || '').trim().slice(0, 200), url: a.href }));
        return { title: document.title, url: location.href, text, links };
      }
      if (action === 'scroll') {
        const n = Math.max(1, Math.min(3000, Number(input.amount) || 600));
        const delta = { up: [0, -n], down: [0, n], left: [-n, 0], right: [n, 0] }[input.direction || 'down'];
        if (!delta) throw new Error('Unknown scroll direction');
        scrollBy(...delta);
        return 'Scrolled.';
      }
      let element = input.selector ? document.querySelector(input.selector) : null;
      if (!element && input.text && action === 'click') {
        const wanted = input.text.toLowerCase();
        element = [...document.querySelectorAll('button, a, [role=button], input[type=submit]')].find(el => (el.innerText || el.getAttribute('aria-label') || el.value || '').trim().toLowerCase().includes(wanted) && el.getClientRects().length);
      }
      if (!element) throw new Error('Specify a matching element selector or button text.');
      if (action === 'click') { element.click(); return 'Clicked.'; }
      if (action === 'type') {
        if (!('value' in element) && !element.isContentEditable) throw new Error('Element is not editable.');
        if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
          const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
          Object.getOwnPropertyDescriptor(prototype, 'value').set.call(element, String(input.text || ''));
        } else element.textContent = String(input.text || '');
        element.dispatchEvent(new Event('input', { bubbles: true }));
        element.dispatchEvent(new Event('change', { bubbles: true }));
        return 'Entered text.';
      }
    },
    args: [action, input]
  });
  return result.result;
}
