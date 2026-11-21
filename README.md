# Jarvis

A voice-controlled desktop assistant for Windows 11 and Linux (X11), written in Rust. Say "Jarvis" and he listens,
answers out loud in a calm, dry-witted British voice, and can operate your PC: open apps, look at the screen, click
and type with his own virtual pointer, search the web, set timers and reminders, work out travel times, remember
things, send messages and more. He speaks English, Spanish, French, German and Cantonese.

He lives on screen as a small floating orb. Left-click it to type, right-click for Settings.

> **Heads up:** Jarvis can control your computer. He asks before risky actions, and there are three
> switchable safety modes in Settings. Read what he proposes before approving.

## Download

**Windows 11:** [**Download Jarvis-Setup.exe**](https://github.com/DreadBear1283/jarvis/releases/latest/download/Jarvis-Setup.exe)
(71 MB). Run it and you're done: it installs for your user only (no administrator rights), brings the wake-word
model, the helper tools and the browser extension, and installs the Microsoft Edge WebView2 runtime if your PC
doesn't have it. All releases are on the [Releases page](https://github.com/DreadBear1283/jarvis/releases).

> The installer isn't code-signed, so Windows SmartScreen may say "Windows protected your PC". Choose
> **More info → Run anyway**. You can also build it yourself from this source (below).

**Linux (X11):** from a checkout of this repository run

```bash
bash jarvis-rs/scripts/install-linux.sh
```

It installs the system packages with apt (it asks for your password once), installs Rust if you don't have it,
downloads the wake-word library and model, builds Jarvis and installs it for your user with a menu entry and a
`jarvis` command. On Wayland, talking and everything else works, but clicking, typing and screenshots need an X11
session.

After installing, start Jarvis, right-click the orb, open **Settings → API keys** and add your keys (below).

## What you need

Nothing, to start: with no keys Jarvis runs in **free mode** (below). For the full assistant:

- An AI: an **Anthropic API key** for Claude (the default, and the most capable), **or** a key for any other
  OpenAI-compatible service. You can add your own: OpenAI, Google Gemini, OpenRouter, Groq, or a model running on
  your own PC with Ollama (no key needed). Pick it in Settings → API keys → *Which AI answers*.
- An **ElevenLabs API key**, for the voice (speech recognition and spoken replies). Optional: turn voice off in
  Settings and use Jarvis as a text-only assistant.
- Optional: a free **Mapbox** public token (travel times and maps) and Xweather credentials (weather alerts).

## Free mode and other AIs

- **Free mode** (no AI key): type to him (click the orb or press `Control+Space`) and a handful of everyday
  commands work with preset text replies and no speech: the time and date, timers ("set a timer for 5 minutes",
  "cancel the timers"), volume ("volume up", "set volume to 30", "mute"), "open Notepad", "take a screenshot",
  "lock my PC", and "help". Nothing is sent anywhere. Add an AI key to unlock conversation, voice and everything else.
- **Other AI services:** any service that speaks the OpenAI chat format works. Choose *Another AI*, pick a preset
  (or Custom), enter the address, the model name and the service's key. All of Jarvis's tools work. What a
  non-Claude model can't do is direct click-and-type screen control or built-in web search, which are Claude features.

## Add your API keys

Start Jarvis, right-click the orb to open **Settings**, and go to the **API keys** tab. Paste your keys and
press Save. On Windows they are encrypted for your user (DPAPI, in `%APPDATA%\Jarvis\data\secrets.bin`); on Linux
they are in `~/.config/Jarvis/data/secrets.bin`, readable only by you. They are never shown again and never leave
your machine except in requests to the provider they belong to.

You can also use a `.env` file (copy `jarvis/.env.example` to the Jarvis settings folder as `.env`); keys found
there are imported on first start.

## Using Jarvis

- **Wake word:** say "Jarvis". **Hotkey:** `Control+Space` opens the text box. Both are changeable in Settings.
- **Language:** Settings → Personality → Language, or say "Jarvis, speak French". English, Spanish, French, German
  and Cantonese; the replies, the speech recognition and his voice all follow it.
- **Mute:** `Control+Alt+M` (or the tray menu) mutes and unmutes his spoken replies. Rebind it in Settings.
- **Cut him off:** left-click the orb while he's talking, or just start talking over him.
- **Voice typing:** say "start dictation" and what you say is typed into the focused window (with spoken
  "comma", "period", "new line", "scratch that"). Say "stop dictation" to finish.
- **Travel times and maps:** save your home address in Settings → Home (it stays private: encrypted, hidden, and
  never sent to the AI), add a free Mapbox token, then ask "how long to get to the airport?" or "show me the
  route". Maps appear next to the orb.
- **Installing software:** "install Python" uses winget on Windows or apt on Linux, with live progress, and asks
  first unless you've given him full access.
- **Photos of part of the screen:** ask for a screenshot of one window ("just my Discord window"), a part of the
  screen, or of Jarvis himself ("take a photo of yourself").
- **His own browser tab group:** tick *Set up the browser extension* in the installer, or open Settings → Browser
  (it's also a step in the first-run tour). Press *Set up in Edge* or *Set up in Chrome*: the extensions page and the
  extension folder open and the folder path is copied, so you just switch on Developer mode, choose Load unpacked,
  paste, and enter the pairing code shown in Settings. He then works in a collapsed "Jarvis" tab group when you ask
  him to use the browser.
- **Games:** "let's play chess" opens a little page in your browser, served only on your own PC, where you play
  against him from beginner to expert. There's also Connect Four, checkers, dots and boxes and sea battle. Click to
  move or just say it: "fire at B7", "c3 to d4", "knight f3", "A1 to B1", "column 4". Ask him how you're doing or for
  a hint and he looks at the position. **Twenty questions** is voice only: he thinks of something and you guess, or
  the other way round, and he keeps count. The chess rules come from [chess.js](https://github.com/jhlywa/chess.js) (BSD-2).
- **Finding and opening apps:** he checks everything installed (Start menu, Store apps, installed programs; on
  Linux, desktop entries, Flatpak and Snap) before saying something isn't there, so "do I have Steam?" or "what games
  do I have?" work. Say "open Discord in the background" and it starts minimised while your window stays in front.
- **Holograms inside the orb:** ask him to explain how things are connected, show a timeline, or show a picture, and
  the orb grows into a see-through 3D sphere of light: a web of nodes floating inside it, a timeline ring around
  its centre, or the picture projected inside. Drag to spin it (scroll turns the timeline), click a node to bring it
  to the front, and say "expand" (or press ⤢) to make it fill the screen; "shrink it" puts it back.
- **Looking back (off by default):** in Settings → Local history you can let him remember what you copied and
  take small screen snapshots (about one every five minutes, kept for a day), so "what was that link I copied?" or
  "what was I looking at at three?" work. Stored only on your PC, wiped when you switch them off.
- **Speaks up on his own:** a heads-up shortly before a calendar meeting, with the drive time added when the event
  has a place and your home address is saved ("time to leave"). Turn it off in Settings → Nudges.
- **Per-app rules:** in Settings, make him silent, brief, or careful (ask before acting) whenever a given app is in
  front, for example silent while a game is up.
- **Morning briefing:** Settings can create a scheduled routine: weather, calendar, travel and headlines.
- **Offline commands:** with no internet or no ElevenLabs key, voice still works for the free-mode commands (time,
  timers, volume, opening apps) using the offline recognizer that also handles the wake word.
- **Updates:** Jarvis checks GitHub for a newer release and offers to install it (Windows: it downloads the
  installer, checks its checksum, and restarts Jarvis). Turn the check off in Settings → General.
- **Modes:** the safety mode and personality are in Settings. **Ultron mode** (say "enable Ultron mode") turns him
  red with a different voice and a superior, condescending attitude, and he also answers to "Ultron" as a wake word.
  Tick *Ultron may insult me* in Settings → Personality if you want him to call you names as part of the act.
- **Screen shares and recordings (Windows):** by default Jarvis is visible in screenshots, recordings and screen
  shares. Turn that off in Settings → Display. He cannot appear over *exclusive*-fullscreen games, only borderless
  or windowed ones.
- **Voice off:** Settings → Voice → untick "Use voice" for text only (no microphone, no ElevenLabs).

## Layout

| Folder | What it is |
|---|---|
| [`jarvis-rs/`](jarvis-rs) | The app: Rust + Tauri 2. Crates for the brain (Claude), tools, voice, and desktop control for Windows and X11. Also the installer scripts. |
| [`browser-extension/`](browser-extension) | Chrome/Edge extension that gives Jarvis his own collapsed tab group. |
| [`jarvis/`](jarvis) | The web UI (floating orb HUD and Settings, TypeScript) that the app embeds, plus a few Node helper tools (calendar, Steam, YouTube). |

## Build from source

**Windows** (needs [Rust](https://rustup.rs) with the MSVC toolchain and [Node.js](https://nodejs.org) 24+):

```powershell
# Builds the app, downloads the wake-word model if needed, and produces the installer:
powershell -File jarvis-rs\scripts\make-installer.ps1
# -> jarvis-rs\target\installer\Jarvis-Setup-<version>.exe
```

(`make-installer.ps1` installs [Inno Setup](https://jrsoftware.org/isinfo.php) with winget the first time. To just
build the app: `cd jarvis && npm install`, then `cd ../jarvis-rs && node scripts/frontend.mjs` and
`cargo build --release -p jarvis-app`.)

**Linux:** `bash jarvis-rs/scripts/install-linux.sh` builds and installs it (see above). It needs these tools at run
time: `xdotool`, `xclip`, `x11-utils`, `x11-xserver-utils`, `imagemagick`, and optionally `playerctl` and
`xprintidle`; the script installs them.

## Known limitations

- Ultron mode ships with a voice ID from the original author's ElevenLabs account. Put a voice from *your*
  account in `jarvis/src/shared/ultron.json` (`voiceId`) before using it.
- Only the spoken replies, speech recognition and "never mind"-style dismissals follow the language setting. The
  Settings screen, the Ultron-mode phrases, the dictation commands and approval questions are English.
- Some antivirus products may flag PowerShell snippets that Jarvis runs for a few Windows tasks (volume,
  locking the screen, opening apps). If Windows Defender blocks one, allow-list the app.
- Linux support is newer and less tested than Windows: messaging-app profiles are Windows-only, and the Wayland
  limits above apply. On Linux, "who is in front" and window features need an X11 session.
- No macOS support.

## Development

- Rust tests: `cargo test --workspace` (from `jarvis-rs`). UI type check: `cd jarvis && npm run typecheck`.
- The chess and Connect Four engines have their own tests: `node --test jarvis-rs/crates/jarvis-brain/src/games/`.

## Third-party code

- [chess.js](https://github.com/jhlywa/chess.js) (BSD-2-Clause) provides the chess rules for the chess game; its
  licence is in `jarvis-rs/crates/jarvis-brain/src/games/LICENSE-chessjs.txt`.
- [Vosk](https://alphacephei.com/vosk/) (Apache-2.0) for offline wake word and offline commands.

## Licence

[MIT](LICENSE)
