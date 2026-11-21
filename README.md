# Jarvis

A voice-controlled desktop assistant for Windows 11, written in Rust. Say "Jarvis" and he listens, answers out
loud in a calm, dry-witted British voice, and can operate your PC: open apps, look at the screen, click and type
with his own virtual pointer, search the web, set timers and reminders, remember things, send messages and more.

He lives on screen as a small floating orb. Left-click it to type, right-click for Settings.

> **Heads up:** Jarvis can control your computer. He asks before risky actions, and there are three
> switchable safety modes in Settings. Read what he proposes before approving.

## Layout

| Folder | What it is |
|---|---|
| [`jarvis-rs/`](jarvis-rs) | The app: Rust + Tauri 2. Crates for the brain (Claude), tools, voice, and Windows desktop control. |
| [`browser-extension/`](browser-extension) | Chrome/Edge extension that gives Jarvis his own collapsed tab group (load unpacked, pair in Settings → Browser). |
| [`jarvis/`](jarvis) | The web UI (floating orb HUD and Settings, TypeScript) that the app embeds, plus a few Node helper tools (calendar, Steam, YouTube). |

## What you need

- Windows 11
- [Rust](https://rustup.rs) (stable, MSVC toolchain) and [Node.js](https://nodejs.org) 24+
- An **Anthropic API key**, for the brain (Claude).
- An **ElevenLabs API key**, for the voice (speech recognition and spoken replies). Optional: turn voice off in
  Settings and use Jarvis as a text-only assistant.
- Optional: Xweather credentials (weather alerts and air quality).

## Build and run

```powershell
# 1. UI dependencies
cd jarvis
npm install

# 2. Offline wake-word model (about 40 MB, not stored in git)
#    Download vosk-model-small-en-us-0.15 from https://alphacephei.com/vosk/models
#    and unzip it to:  jarvis/models/vosk-model-small-en-us-0.15

# 3. Build the UI and the app
cd ../jarvis-rs
node scripts/frontend.mjs
cargo build --release -p jarvis-app
# -> jarvis-rs/target/release/jarvis-app.exe

# 4. Make a portable folder + zip with the wake-word runtime and model next to the exe
node scripts/build-utilities.mjs
powershell -File scripts/package-windows.ps1
```

Run the packaged `Jarvis.exe`. The wake-word runtime (`bin/libvosk.dll`) and `models/` must sit beside it.

## Add your API keys

Start Jarvis, right-click the orb to open **Settings**, and go to the **API keys** tab. Paste your keys and
press Save. They are stored encrypted for your Windows user (DPAPI, in `%APPDATA%\Jarvis\data\secrets.bin`),
are never shown again, and never leave your machine except in requests to the provider they belong to.

You can also use a `.env` file (copy `jarvis/.env.example` to `%APPDATA%\Jarvis\.env`); keys found there are
imported on first start.

## Using Jarvis

- **Wake word:** say "Jarvis". **Hotkey:** `Control+Space` opens the text box. Both are changeable in Settings.
- **Mute:** `Control+Alt+M` (or the tray menu) mutes and unmutes his spoken replies. Rebind it in Settings.
- **Cut him off:** left-click the orb while he's talking, or just start talking over him.
- **Voice typing:** say "start dictation" and what you say is typed into the focused window (with spoken
  "comma", "period", "new line", "scratch that"). Say "stop dictation" to finish.
- **Photos of part of the screen:** ask for a screenshot of one window ("just my Discord window"), a part of the
  screen, or of Jarvis himself ("take a photo of yourself").
- **Modes:** the safety mode and personality (including "Ultron mode": say "enable Ultron mode") are in Settings.
- **Screen shares and recordings:** by default Jarvis is visible in screenshots, recordings and screen shares.
  Turn that off in Settings → Display. He cannot appear over *exclusive*-fullscreen games, only borderless or
  windowed ones.
- **Voice off:** Settings → Voice → untick "Use voice" for text only (no microphone, no ElevenLabs).

## Known limitations

- Ultron mode ships with a voice ID from the original author's ElevenLabs account. Put a voice from *your*
  account in `jarvis/src/shared/ultron.json` (`voiceId`) before using it.
- Some antivirus products may flag PowerShell snippets that Jarvis runs for a few Windows tasks (volume,
  locking the screen, opening apps). If Windows Defender blocks one, allow-list the app.
- Windows only.

## Licence

[MIT](LICENSE)
