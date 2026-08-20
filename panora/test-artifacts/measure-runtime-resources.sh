#!/usr/bin/env bash
set -u

HOME_TEST=/tmp/panora-resource-home
RUNTIME=/tmp/panora-resource-runtime
DISPLAY_NUM=:131
OUT=/home/ubuntu/panora/test-artifacts/resource-retest.log
PANOD_PID=""
GUI_PID=""
XVFB_PID=""

rm -rf "$HOME_TEST" "$RUNTIME"
mkdir -p "$HOME_TEST" "$RUNTIME"
chmod 700 "$RUNTIME"
Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac >/tmp/panora-resource-xvfb.log 2>&1 &
XVFB_PID=$!
sleep 1
trap 'kill "$GUI_PID" "$PANOD_PID" "$XVFB_PID" 2>/dev/null || true' EXIT

export HOME_TEST RUNTIME DISPLAY_NUM OUT
dbus-run-session -- bash -c '
set +e
export HOME="$HOME_TEST"
export XDG_RUNTIME_DIR="$RUNTIME"
export XDG_CONFIG_HOME="$HOME_TEST/.config"
export XDG_DATA_HOME="$HOME_TEST/.local/share"
export DISPLAY="$DISPLAY_NUM"
export XDG_SESSION_TYPE=x11
export GTK_A11Y=none
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
printf "\\n" | gnome-keyring-daemon --unlock --components=secrets >/tmp/panora-resource-keyring-unlock.log 2>&1 || true
eval "$(gnome-keyring-daemon --start --components=secrets 2>/tmp/panora-resource-keyring.log)" || true
export GNOME_KEYRING_CONTROL
/usr/bin/panod >/tmp/panora-resource-panod.log 2>&1 &
P=$!
sleep 4
/usr/bin/panora-gui >/tmp/panora-resource-gui.log 2>&1 &
G=$!
sleep 4
{
  echo "resource_snapshot_t4=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "pid_rss_vsz_cpu_command"
  ps -o pid=,rss=,vsz=,pcpu=,comm= -p "$P","$G"
  echo "panod_status_t4"
  /usr/bin/panora-cli status
  sleep 12
  echo "resource_snapshot_t16=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "pid_rss_vsz_cpu_command"
  ps -o pid=,rss=,vsz=,pcpu=,comm= -p "$P","$G"
  echo "panod_status_t16"
  /usr/bin/panora-cli status
} > "$OUT"
kill "$G" "$P" 2>/dev/null || true
wait "$G" "$P" 2>/dev/null || true
' 
cat "$OUT"
