#!/usr/bin/env bash
# Installs Jarvis (Linux, X11) and everything it needs, from this source checkout:
#   - system packages (build tools, WebKitGTK, X11 tools, ImageMagick, media/audio helpers) via apt,
#   - Rust (rustup) if it is missing,
#   - the offline wake-word library (libvosk.so) and model,
#   - then it builds the app and installs it for the current user.
#
# Usage:  bash jarvis-rs/scripts/install-linux.sh            (asks for your password once, for apt)
#         PREFIX=/opt/jarvis bash jarvis-rs/scripts/install-linux.sh
#         SKIP_APT=1 bash ...                                (you already have the packages)
set -euo pipefail

VOSK_VERSION="${VOSK_VERSION:-0.3.45}"
MODEL="vosk-model-small-en-us-0.15"
PREFIX="${PREFIX:-$HOME/.local/share/jarvis}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RS="$(cd "$HERE/.." && pwd)"         # jarvis-rs
ROOT="$(cd "$RS/.." && pwd)"         # repository root
UI="$ROOT/jarvis"
TARGET="${CARGO_TARGET_DIR:-$RS/target}"   # where cargo puts the build

say() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
need() { command -v "$1" >/dev/null 2>&1; }

if [ "$(uname -s)" != "Linux" ]; then echo "This installer is for Linux. On Windows use scripts\\make-installer.ps1."; exit 1; fi
if [ "${XDG_SESSION_TYPE:-x11}" = "wayland" ]; then
  echo "Note: you are on Wayland. Jarvis controls the screen through X11 tools, so clicking, typing and"
  echo "screenshots only work in an X11 session (or XWayland apps). Talking and everything else works."
fi

# --- 1. System packages ---------------------------------------------------------------------------------
if [ -z "${SKIP_APT:-}" ]; then
  if ! need apt-get; then
    echo "apt-get not found. Install the equivalents of these yourself, then re-run with SKIP_APT=1:"
    echo "  build tools, pkg-config, WebKitGTK 4.1 dev, GTK3 dev, libayatana-appindicator3 dev, OpenSSL dev, ALSA dev,"
    echo "  xdotool, xclip, xprop/xrandr (x11-utils, x11-xserver-utils), ImageMagick, playerctl, xprintidle, Node.js, npm, curl, unzip"
    exit 1
  fi
  say "Installing system packages (needs your password)"
  SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo"
  $SUDO apt-get update
  $SUDO apt-get install -y build-essential pkg-config curl git unzip ca-certificates \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libasound2-dev libxdo-dev \
    xdotool xclip x11-utils x11-xserver-utils imagemagick playerctl xprintidle \
    nodejs npm
fi

# Everything below needs these; name any that are missing instead of failing halfway.
missing=""
for tool in curl unzip node npm xdotool xclip xprop xrandr import; do need "$tool" || missing="$missing $tool"; done
if [ -n "$missing" ]; then
  echo "Missing tools:$missing"
  echo "Install them (apt: curl unzip nodejs npm xdotool xclip x11-utils x11-xserver-utils imagemagick) and run this again."
  exit 1
fi

# --- 2. Rust ----------------------------------------------------------------------------------------------
if ! need cargo; then
  say "Installing Rust"
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# --- 3. Wake-word library and model -------------------------------------------------------------------------
mkdir -p "$PREFIX/bin" "$PREFIX/models" "$PREFIX/utilities"
if [ ! -f "$PREFIX/bin/libvosk.so" ]; then
  say "Downloading the wake-word library (Vosk $VOSK_VERSION)"
  TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
  curl -fL "https://github.com/alphacep/vosk-api/releases/download/v$VOSK_VERSION/vosk-linux-x86_64-$VOSK_VERSION.zip" -o "$TMP/vosk.zip"
  unzip -q -o "$TMP/vosk.zip" -d "$TMP"
  cp "$TMP"/vosk-linux-x86_64-*/libvosk.so "$PREFIX/bin/libvosk.so"
fi
if [ ! -f "$PREFIX/models/$MODEL/am/final.mdl" ]; then
  say "Downloading the wake-word model (about 40 MB)"
  TMP="${TMP:-$(mktemp -d)}"
  curl -fL "https://alphacephei.com/vosk/models/$MODEL.zip" -o "$TMP/model.zip"
  unzip -q -o "$TMP/model.zip" -d "$PREFIX/models"
fi

# --- 4. Build -----------------------------------------------------------------------------------------------
say "Building the web UI"
( cd "$UI" && npm install --no-audit --no-fund )
( cd "$RS" && node scripts/frontend.mjs && node scripts/build-utilities.mjs )
say "Building Jarvis (this takes a few minutes the first time)"
( cd "$RS" && cargo build --release -p jarvis-app )

# --- 5. Install -----------------------------------------------------------------------------------------------
say "Installing to $PREFIX"
install -m 755 "$TARGET/release/jarvis-app" "$PREFIX/jarvis"
install -m 644 "$RS/target/utilities.cjs" "$PREFIX/utilities/utilities.cjs"
rm -rf "$PREFIX/browser-extension" && cp -r "$ROOT/browser-extension" "$PREFIX/browser-extension"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
ln -sf "$PREFIX/jarvis" "$HOME/.local/bin/jarvis"
cat > "$HOME/.local/share/applications/jarvis.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Jarvis
Comment=Voice-controlled desktop assistant
Exec=$PREFIX/jarvis
Terminal=false
Categories=Utility;
EOF

say "Done"
echo "Start it from your applications menu, or run:  jarvis"
echo "Then right-click the orb, open Settings, and add your Anthropic key (and an ElevenLabs key for his voice) under API keys."
echo "Browser extension: load $PREFIX/browser-extension as an unpacked extension in Chrome or Edge."
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "(Add ~/.local/bin to your PATH to use the 'jarvis' command.)" ;; esac
