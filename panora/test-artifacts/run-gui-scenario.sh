#!/usr/bin/env bash
set -eu
set -o pipefail
PROJECT=/home/ubuntu/panora
HOME_TEST=/tmp/panora-gui-home
RUNTIME=/tmp/panora-gui-runtime
DISPLAY_NUM=:104
OUT="$PROJECT/test-artifacts"
rm -rf "$HOME_TEST" "$RUNTIME"
mkdir -p "$HOME_TEST" "$RUNTIME" "$OUT/logs" "$OUT/screenshots"
chmod 700 "$RUNTIME"

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac >"$OUT/logs/xvfb-gui.log" 2>&1 &
XVFB_PID=$!
cleanup() {
  kill "${GUI_PID:-}" 2>/dev/null || true
  kill "${PANOD_PID:-}" 2>/dev/null || true
  kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT
sleep 1

dbus-run-session -- bash -c '
  export HOME="'$HOME_TEST'" XDG_RUNTIME_DIR="'$RUNTIME'" XDG_CONFIG_HOME="'$HOME_TEST'/.config" XDG_DATA_HOME="'$HOME_TEST'/.local/share" DISPLAY="'$DISPLAY_NUM'" XDG_SESSION_TYPE=x11 GTK_A11Y=none RUST_LOG=info
  mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
  printf "\\n" | gnome-keyring-daemon --unlock --components=secrets 2>"'$OUT/logs'"/keyring-gui-unlock.log || true
  eval "$(gnome-keyring-daemon --start --components=secrets 2>"'$OUT/logs'"/keyring-gui.log)" || true
  export GNOME_KEYRING_CONTROL
  '$PROJECT'/target/release/panod >"'$OUT/logs'"/panod-gui.log 2>&1 &
  PANOD_PID=$!
  sleep 4
  printf "Merhaba Panora - GUI testi\n" | xclip -selection clipboard -in
  sleep 2
  printf "İkinci pano öğesi\n" | xclip -selection clipboard -in
  sleep 2
  '$PROJECT'/target/release/panora-gui >"'$OUT/logs'"/gui-history.log 2>&1 &
  GUI_PID=$!
  sleep 4
  export DISPLAY="'$DISPLAY_NUM'"
  python3 - <<"PY"
from PIL import ImageGrab
ImageGrab.grab().save("'$OUT'/screenshots/02-gui-history.png")
PY
  WIN=$(xdotool search --name Panora | tail -1)
  xdotool windowactivate "$WIN"
  xdotool mousemove --window "$WIN" 300 95 click 1
  xdotool key --window "$WIN" ctrl+a
  xdotool type --window "$WIN" --delay 20 "Merhaba"
  sleep 2
  python3 - <<"PY"
from PIL import ImageGrab
ImageGrab.grab().save("'$OUT'/screenshots/03-gui-search.png")
PY
  sleep 1
  xdotool mousemove --window "$WIN" 60 40 click 1
  sleep 2
  python3 - <<"PY"
from PIL import ImageGrab
ImageGrab.grab().save("'$OUT'/screenshots/04-gui-settings.png")
PY
  kill "$GUI_PID" 2>/dev/null || true
  kill "$PANOD_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true
  wait "$PANOD_PID" 2>/dev/null || true
'
