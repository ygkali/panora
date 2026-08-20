#!/usr/bin/env bash
set -u
PROJECT=/home/ubuntu/panora
BIN="$PROJECT/target/release"
LOG="$PROJECT/test-artifacts/release-ui-runtime-retest.log"
SCREEN="$PROJECT/test-artifacts/release-ui-runtime-gui.png"
HOME_DIR=/tmp/panora-release-ui-home
RUNTIME_DIR=/tmp/panora-release-ui-runtime
DISPLAY_NUM=:131
XVFB_PID=""

rm -f "$LOG" "$SCREEN"
rm -rf "$HOME_DIR" "$RUNTIME_DIR"
mkdir -p "$HOME_DIR" "$RUNTIME_DIR"
chmod 700 "$RUNTIME_DIR"

cleanup() {
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac > /tmp/panora-release-ui-xvfb.log 2>&1 &
XVFB_PID=$!
sleep 1

export PROJECT BIN LOG SCREEN HOME_DIR RUNTIME_DIR DISPLAY_NUM

dbus-run-session -- bash -c '
set +e
export HOME="$HOME_DIR"
export XDG_RUNTIME_DIR="$RUNTIME_DIR"
export XDG_CONFIG_HOME="$HOME_DIR/.config"
export XDG_DATA_HOME="$HOME_DIR/.local/share"
export DISPLAY="$DISPLAY_NUM"
export XDG_SESSION_TYPE=x11
export GTK_A11Y=none
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
printf "\\n" | gnome-keyring-daemon --unlock --components=secrets 2>/tmp/panora-release-ui-keyring-unlock.log >/dev/null 2>&1 || true
eval "$(gnome-keyring-daemon --start --components=secrets 2>/tmp/panora-release-ui-keyring.log 2>/dev/null)" || true
export GNOME_KEYRING_CONTROL

run_test() {
  label="$1"
  shift
  printf "\\n>>> %s\\n" "$label" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  printf "status=%s\\n" "$?" >> "$LOG"
}

printf "release_bins=%s\\n" "$BIN" > "$LOG"
printf "environment=Ubuntu sandbox Xvfb 1280x800 X11 D-Bus GNOME Keyring\\n" >> "$LOG"

printf "\\n>>> panod start\\n" >> "$LOG"
"$BIN/panod" >> /tmp/panora-release-ui-panod.log 2>&1 &
PANOD_PID=$!
printf "pid=%s\\n" "$PANOD_PID" >> "$LOG"
for _ in $(seq 1 60); do
  [ -S "$RUNTIME_DIR/panora.sock" ] && break
  sleep 0.25
done
run_test "release CLI status" "$BIN/panora-cli" status

printf "Win+V metin kartı\\n" | xclip -selection clipboard -in -t text/plain
sleep 1
run_test "text list" "$BIN/panora-cli" list
run_test "text search" "$BIN/panora-cli" search Win+V

printf "<b>Panora rich text</b>\\n" | xclip -selection clipboard -in -t text/html
sleep 1
run_test "rich text list" "$BIN/panora-cli" list

xclip -selection clipboard -in -t image/png < "$PROJECT/test-artifacts/panora-runtime-photo.png"
sleep 1
run_test "photo list" "$BIN/panora-cli" list
run_test "photo status" "$BIN/panora-cli" status

printf "file:///tmp/panora-photo.png\\n" | xclip -selection clipboard -in -t text/uri-list
sleep 1
run_test "URI list" "$BIN/panora-cli" list

run_test "private on" "$BIN/panora-cli" private on
printf "Bu private kart kaydedilmemeli\\n" | xclip -selection clipboard -in -t text/plain
sleep 1
run_test "private status" "$BIN/panora-cli" status
run_test "private off" "$BIN/panora-cli" private off
run_test "oversize IPC frame" env XDG_RUNTIME_DIR="$RUNTIME_DIR" python3 "$PROJECT/test-artifacts/oversize-ipc-test.py"

printf "\\n>>> release GTK4/libadwaita GUI\\n" >> "$LOG"
"$BIN/panora-gui" > /tmp/panora-release-ui-gui.log 2>&1 &
GUI_PID=$!
sleep 5
PANORA_SCREENSHOT="$SCREEN" DISPLAY="$DISPLAY_NUM" python3 "$PROJECT/test-artifacts/capture-x11-screenshot.py" >> "$LOG" 2>&1
printf "gui_pid=%s\\n" "$GUI_PID" >> "$LOG"
kill "$GUI_PID" 2>/dev/null || true
kill "$PANOD_PID" 2>/dev/null || true
wait "$GUI_PID" 2>/dev/null || true
wait "$PANOD_PID" 2>/dev/null || true
printf "runtime_test_finished=1\\n" >> "$LOG"
'

cat "$LOG"
