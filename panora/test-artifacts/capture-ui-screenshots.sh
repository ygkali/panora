#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Captures screenshots of the current GUI under Xvfb, in an isolated HOME so
# the real clipboard history is never touched. Run on a Debian/Ubuntu box:
#
#   sudo apt install -y xvfb dbus-x11 xdotool xclip imagemagick gnome-keyring \
#     libgtk-4-1 libadwaita-1-0 adwaita-icon-theme librsvg2-common
#   ./test-artifacts/capture-ui-screenshots.sh
#
# Output: test-artifacts/screenshots/*.png
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT_DIR/test-artifacts"
SHOTS="$OUT/screenshots"
LOGS="$OUT/logs"
DISPLAY_NUM="${PANORA_DISPLAY:-:104}"
SCREEN="${PANORA_SCREEN:-1280x900x24}"
HOME_TEST=/tmp/panora-shot-home
RUNTIME=/tmp/panora-shot-runtime

missing=()
for tool in Xvfb dbus-run-session xdotool xclip import; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if (( ${#missing[@]} )); then
  echo "Eksik araçlar: ${missing[*]}" >&2
  echo "Kurmak için: sudo apt install -y xvfb dbus-x11 xdotool xclip imagemagick gnome-keyring" >&2
  exit 1
fi

# Adwaita 48's symbolic icons ship only as SVG. Without gdk-pixbuf's SVG loader
# every icon that GTK does not carry in its own gresource renders as
# "image-missing" — screenshots would look broken for a reason that has nothing
# to do with the app.
if ! find /usr/lib /usr/lib64 -name 'libpixbufloader?svg.so' -print -quit 2>/dev/null | grep -q .; then
  echo "SVG pixbuf loader yok; ikonlar 'image-missing' olarak çizilir." >&2
  echo "Kurmak için: sudo apt install -y librsvg2-common adwaita-icon-theme" >&2
  exit 1
fi

PANOD="$ROOT_DIR/target/release/panod"
GUI="$ROOT_DIR/target/release/panora-gui"
if [[ ! -x "$PANOD" || ! -x "$GUI" ]]; then
  echo "Release binaries yok. Önce derleyin:" >&2
  echo "  cargo build --release --workspace" >&2
  exit 1
fi

rm -rf "$HOME_TEST" "$RUNTIME"
mkdir -p "$HOME_TEST" "$RUNTIME" "$SHOTS" "$LOGS"
chmod 700 "$RUNTIME"

# -nolisten tcp keeps this throwaway display off the network. It still has no
# access control, so any local user could read it while the run is in progress;
# do not run this on a shared machine with real clipboard data.
Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN" -ac -nolisten tcp >"$LOGS/xvfb-shots.log" 2>&1 &
XVFB_PID=$!
cleanup() { kill "$XVFB_PID" 2>/dev/null || true; }
trap cleanup EXIT
sleep 1

# One dbus session hosts the keyring, the daemon and both GUI runs.
HOME_TEST="$HOME_TEST" RUNTIME="$RUNTIME" DISPLAY_NUM="$DISPLAY_NUM" \
GUI="$GUI" PANOD="$PANOD" SHOTS="$SHOTS" LOGS="$LOGS" \
dbus-run-session -- bash -euo pipefail -c '
  export HOME="$HOME_TEST" XDG_RUNTIME_DIR="$RUNTIME" \
         XDG_CONFIG_HOME="$HOME_TEST/.config" XDG_DATA_HOME="$HOME_TEST/.local/share" \
         DISPLAY="$DISPLAY_NUM" XDG_SESSION_TYPE=x11 GTK_A11Y=none RUST_LOG=info
  mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"

  printf "\n" | gnome-keyring-daemon --unlock --components=secrets \
    >"$LOGS/keyring-unlock.log" 2>&1 || true
  eval "$(gnome-keyring-daemon --start --components=secrets 2>"$LOGS/keyring.log")" || true
  export GNOME_KEYRING_CONTROL

  "$PANOD" >"$LOGS/panod-shots.log" 2>&1 &
  PANOD_PID=$!
  sleep 4

  if ! kill -0 "$PANOD_PID" 2>/dev/null; then
    echo "panod başlamadı; $LOGS/panod-shots.log dosyasına bakın." >&2
    exit 1
  fi

  # --- seed a mix of content kinds so every card style is exercised ---
  feed() { printf "%s" "$1" | xclip -selection clipboard -in; sleep 1.2; }
  feed "Merhaba Panora - pano gecmisi testi"
  feed "https://github.com/panora-clipboard/panora"
  feed "#36c2ff"
  feed "SELECT id, preview FROM entries WHERE pinned = 1 ORDER BY last_seen_at DESC;"
  feed "İkinci metin öğesi - Türkçe karakter testi: ğüşiöç ĞÜŞİÖÇ"

  shoot() { sleep 2; import -window root "$SHOTS/$1"; echo "  -> $1"; }
  win() { xdotool search --name Panora | tail -1; }

  echo "[1/5] Geçmiş görünümü (açık tema)"
  "$GUI" >"$LOGS/gui-light.log" 2>&1 &
  GUI_PID=$!
  sleep 4
  W=$(win); xdotool windowactivate "$W" || true
  shoot 10-history-light.png

  echo "[2/5] Arama"
  xdotool key --window "$W" --clearmodifiers ctrl+f
  sleep 0.5
  xdotool type --window "$W" --delay 40 "Panora"
  shoot 11-search.png

  echo "[3/5] Tür filtresi (Bağlantı)"
  xdotool key --window "$W" --clearmodifiers ctrl+f
  sleep 0.3
  xdotool key --window "$W" --clearmodifiers ctrl+a Delete
  sleep 1
  # Chip row sits under the search box; Tab-walk to it instead of guessing pixels.
  xdotool key --window "$W" --clearmodifiers Tab Tab Tab Right Right Right
  sleep 0.5
  xdotool key --window "$W" --clearmodifiers space
  shoot 12-filter-link.png

  echo "[4/5] Klavye seçimi"
  xdotool key --window "$W" --clearmodifiers Escape
  sleep 0.5
  xdotool key --window "$W" --clearmodifiers Down Down
  shoot 13-keyboard-selection.png

  kill "$GUI_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true

  echo "[5/5] Koyu tema"
  ADW_DEBUG_COLOR_SCHEME=prefer-dark "$GUI" >"$LOGS/gui-dark.log" 2>&1 &
  GUI_PID=$!
  sleep 4
  W=$(win); xdotool windowactivate "$W" || true
  shoot 14-history-dark.png
  kill "$GUI_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true

  kill "$PANOD_PID" 2>/dev/null || true
  wait "$PANOD_PID" 2>/dev/null || true
'

echo
echo "Ekran görüntüleri: $SHOTS"
ls -1 "$SHOTS"/1*.png 2>/dev/null || true
