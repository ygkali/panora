#!/usr/bin/env bash
set -u

PROJECT=/home/ubuntu/panora
TEST_LOG="$PROJECT/test-artifacts/package-runtime-retest.log"
SCREEN="$PROJECT/test-artifacts/package-runtime-gui.png"
TEST_HOME=/tmp/panora-package-home
TEST_RUNTIME=/tmp/panora-package-runtime
DISPLAY_NUM=:130
PANOD_PID=""
GUI_PID=""
XVFB_PID=""

rm -f "$TEST_LOG" "$SCREEN"
rm -rf "$TEST_HOME" "$TEST_RUNTIME"
mkdir -p "$TEST_HOME" "$TEST_RUNTIME"
chmod 700 "$TEST_RUNTIME"

cleanup() {
  [ -n "$GUI_PID" ] && kill "$GUI_PID" 2>/dev/null || true
  [ -n "$PANOD_PID" ] && kill "$PANOD_PID" 2>/dev/null || true
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac > /tmp/panora-package-xvfb.log 2>&1 &
XVFB_PID=$!
sleep 1

export PROJECT TEST_LOG SCREEN TEST_HOME TEST_RUNTIME DISPLAY_NUM

dbus-run-session -- bash -c '
set +e
export HOME="$TEST_HOME"
export XDG_RUNTIME_DIR="$TEST_RUNTIME"
export XDG_CONFIG_HOME="$TEST_HOME/.config"
export XDG_DATA_HOME="$TEST_HOME/.local/share"
export DISPLAY="$DISPLAY_NUM"
export XDG_SESSION_TYPE=x11
export GTK_A11Y=none
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
printf "\\n" | gnome-keyring-daemon --unlock --components=secrets 2>/tmp/panora-package-keyring-unlock.log >/dev/null 2>&1 || true
eval "$(gnome-keyring-daemon --start --components=secrets 2>/tmp/panora-package-keyring.log 2>/dev/null)" || true
export GNOME_KEYRING_CONTROL

run_test() {
  label="$1"
  shift
  printf "\\n>>> %s\\n" "$label" >> "$TEST_LOG"
  "$@" >> "$TEST_LOG" 2>&1
  printf "status=%s\\n" "$?" >> "$TEST_LOG"
}

printf "package=%s\\n" "$(dpkg-query -W -f="\${Status} \${Version}\\n" panora 2>/dev/null)" > "$TEST_LOG"
printf "\\n>>> panod başlatma\\n" >> "$TEST_LOG"
/usr/bin/panod >> /tmp/panora-package-panod.log 2>&1 &
PANOD_PID=$!
printf "pid=%s\\n" "$PANOD_PID" >> "$TEST_LOG"
sleep 4
run_test "CLI status" /usr/bin/panora-cli status
printf "Paket runtime ilk metin\\n" | xclip -selection clipboard -in
sleep 2
printf "Paket runtime ikinci metin\\n" | xclip -selection clipboard -in
sleep 2
run_test "CLI list" /usr/bin/panora-cli list
run_test "FTS5 search" /usr/bin/panora-cli search ikinci
run_test "CLI copy" /usr/bin/panora-cli copy 2
printf "clipboard=%s\\n" "$(xclip -selection clipboard -out 2>/dev/null)" >> "$TEST_LOG"
run_test "CLI pin" /usr/bin/panora-cli pin 2
run_test "CLI list pinned" /usr/bin/panora-cli list
run_test "CLI unpin" /usr/bin/panora-cli unpin 2
run_test "private on" /usr/bin/panora-cli private on
printf "Private mode kaydedilmemeli\\n" | xclip -selection clipboard -in
sleep 2
run_test "private status" /usr/bin/panora-cli status
run_test "private list" /usr/bin/panora-cli list
run_test "private off" /usr/bin/panora-cli private off
run_test "CLI delete" /usr/bin/panora-cli delete 1
run_test "CLI clear" /usr/bin/panora-cli clear
run_test "CLI list after clear" /usr/bin/panora-cli list
run_test "oversize IPC frame" env XDG_RUNTIME_DIR="$TEST_RUNTIME" python3 "$PROJECT/test-artifacts/oversize-ipc-test.py"

printf "\\n>>> GTK4/libadwaita GUI\\n" >> "$TEST_LOG"
/usr/bin/panora-gui > /tmp/panora-package-gui.log 2>&1 &
GUI_PID=$!
sleep 4
PANORA_SCREENSHOT="$SCREEN" DISPLAY="$DISPLAY_NUM" python3 "$PROJECT/test-artifacts/capture-x11-screenshot.py" >> "$TEST_LOG" 2>&1
printf "gui_pid=%s\\n" "$GUI_PID" >> "$TEST_LOG"

kill "$GUI_PID" 2>/dev/null || true
kill "$PANOD_PID" 2>/dev/null || true
wait "$GUI_PID" 2>/dev/null || true
wait "$PANOD_PID" 2>/dev/null || true
printf "runtime_test_finished=1\\n" >> "$TEST_LOG"
' 

cat "$TEST_LOG"
