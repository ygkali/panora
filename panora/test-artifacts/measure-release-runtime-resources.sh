#!/usr/bin/env bash
set -u
PROJECT=/home/ubuntu/panora
BIN="$PROJECT/target/release"
HOME_TEST=/tmp/panora-release-resource-home
RUNTIME=/tmp/panora-release-resource-runtime
DISPLAY_NUM=:132
OUT="$PROJECT/test-artifacts/release-resource-retest.log"
P=""
G=""
X=""
rm -rf "$HOME_TEST" "$RUNTIME" "$OUT"
mkdir -p "$HOME_TEST" "$RUNTIME"
chmod 700 "$RUNTIME"
Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac >/tmp/panora-release-resource-xvfb.log 2>&1 &
X=$!
sleep 1
trap 'kill "$G" "$P" "$X" 2>/dev/null || true' EXIT
export PROJECT BIN HOME_TEST RUNTIME DISPLAY_NUM OUT
dbus-run-session -- bash -c '
set +e
export HOME="$HOME_TEST"
export XDG_RUNTIME_DIR="$RUNTIME"
export XDG_CONFIG_HOME="$HOME_TEST/.config"
export XDG_DATA_HOME="$HOME_TEST/.local/share"
export DISPLAY="$DISPLAY_NUM"
export XDG_SESSION_TYPE=x11
export GTK_A11Y=none
export GSK_RENDERER=cairo
export LIBGL_ALWAYS_SOFTWARE=1
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
printf "\\n" | gnome-keyring-daemon --unlock --components=secrets >/tmp/panora-release-resource-keyring-unlock.log 2>&1 || true
eval "$(gnome-keyring-daemon --start --components=secrets 2>/tmp/panora-release-resource-keyring.log)" || true
export GNOME_KEYRING_CONTROL
"$BIN/panod" >/tmp/panora-release-resource-panod.log 2>&1 &
P=$!
for _ in $(seq 1 40); do [ -S "$RUNTIME/panora.sock" ] && break; sleep 0.25; done
"$BIN/panora-gui" >/tmp/panora-release-resource-gui.log 2>&1 &
G=$!
sleep 4
{
  echo "snapshot_t4=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "pid_rss_kib_vsz_kib_cpu_command"
  ps -o pid=,rss=,vsz=,pcpu=,comm= -p "$P","$G"
  for pid in "$P" "$G"; do
    printf "pid=%s " "$pid"
    grep -E "^(Pss|Private_Clean|Private_Dirty):" "/proc/$pid/smaps_rollup" 2>/dev/null | tr "\\n" " "
    printf "\\n"
  done
  echo "status_t4"
  "$BIN/panora-cli" status
  sleep 10
  echo "snapshot_t14=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "pid_rss_kib_vsz_kib_cpu_command"
  ps -o pid=,rss=,vsz=,pcpu=,comm= -p "$P","$G"
  for pid in "$P" "$G"; do
    printf "pid=%s " "$pid"
    grep -E "^(Pss|Private_Clean|Private_Dirty):" "/proc/$pid/smaps_rollup" 2>/dev/null | tr "\\n" " "
    printf "\\n"
  done
  echo "status_t14"
  "$BIN/panora-cli" status
} > "$OUT"
kill "$G" "$P" 2>/dev/null || true
wait "$G" "$P" 2>/dev/null || true
'
cat "$OUT"
