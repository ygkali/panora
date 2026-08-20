#!/usr/bin/env bash
set -eu
set -o pipefail
PROJECT=/home/ubuntu/panora
TEST_HOME=/tmp/panora-integration-home
RUNTIME=/tmp/panora-integration-runtime
DISPLAY_NUM=:103
OUT="$PROJECT/test-artifacts"
rm -rf "$TEST_HOME" "$RUNTIME"
mkdir -p "$TEST_HOME" "$RUNTIME" "$OUT/logs" "$OUT/screenshots"
chmod 700 "$RUNTIME"
export HOME="$TEST_HOME"
export XDG_RUNTIME_DIR="$RUNTIME"
export XDG_CONFIG_HOME="$TEST_HOME/.config"
export XDG_DATA_HOME="$TEST_HOME/.local/share"
export DISPLAY="$DISPLAY_NUM"
export XDG_SESSION_TYPE=x11
export GTK_A11Y=none
export RUST_LOG=debug

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac >"$OUT/logs/xvfb-integration.log" 2>&1 &
XVFB_PID=$!
cleanup() {
  kill "${PANOD_PID:-}" 2>/dev/null || true
  kill "${XVFB_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT
sleep 1

# Start a user D-Bus and GNOME Keyring Secret Service in the same process
# namespace. No real user password or host account is used.
dbus-run-session -- bash -c '
  export HOME="'$TEST_HOME'" XDG_RUNTIME_DIR="'$RUNTIME'" XDG_CONFIG_HOME="'$TEST_HOME'/.config" XDG_DATA_HOME="'$TEST_HOME'/.local/share" DISPLAY="'$DISPLAY_NUM'" XDG_SESSION_TYPE=x11 GTK_A11Y=none
  mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
  printf "\\n" | gnome-keyring-daemon --unlock --components=secrets 2>"'$OUT/logs'"/keyring-unlock.log || true
  eval "$(gnome-keyring-daemon --start --components=secrets 2>"'$OUT/logs'"/keyring.log)" || true
  export GNOME_KEYRING_CONTROL
  '$PROJECT'/target/release/panod >"'$OUT/logs'"/panod.log 2>&1 &
  PANOD_PID=$!
  echo "$PANOD_PID" >"'$OUT'"/panod.pid
  sleep 4
  '$PROJECT'/target/release/panora-cli status >"'$OUT/logs'"/cli-status.log 2>&1 || true
  python3 - <<"PY" >"'$OUT/logs'"/ipc-oversize.log 2>&1
import socket
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect("'$RUNTIME'/panora.sock")
sock.settimeout(3)
frame = b"{\\\"method\\\":\\\"Status\\\",\\\"params\\\":{" + b"x" * 70000 + b"}\n"
sock.sendall(frame)
print(sock.recv(4096).decode("utf-8", "replace"))
sock.close()
PY
  printf "Merhaba Panora\n" | xclip -selection clipboard -in
  sleep 3
  printf "İkinci örnek pano\n" | xclip -selection clipboard -in
  sleep 3
  '$PROJECT'/target/release/panora-cli list >"'$OUT/logs'"/cli-list.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli search "Merhaba" >"'$OUT/logs'"/cli-search.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli copy 2 >"'$OUT/logs'"/cli-copy.log 2>&1 || true
  xclip -selection clipboard -out >"'$OUT/logs'"/clipboard-after-copy.txt 2>&1 || true
  '$PROJECT'/target/release/panora-cli pin 2 >"'$OUT/logs'"/cli-pin.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli list >"'$OUT/logs'"/cli-list-pinned.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli unpin 2 >"'$OUT/logs'"/cli-unpin.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli private on >"'$OUT/logs'"/cli-private-on.log 2>&1 || true
  sleep 2
  '$PROJECT'/target/release/panora-cli list >"'$OUT/logs'"/cli-list-before-private-clipboard.log 2>&1 || true
  printf "Bu kaydedilmemeli\n" | xclip -selection clipboard -in
  sleep 3
  '$PROJECT'/target/release/panora-cli status >"'$OUT/logs'"/cli-status-private.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli list >"'$OUT/logs'"/cli-list-private.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli private off >"'$OUT/logs'"/cli-private-off.log 2>&1 || true
  '$PROJECT'/target/release/panora-cli list >"'$OUT/logs'"/cli-list-after-private.log 2>&1 || true
  kill "$PANOD_PID" 2>/dev/null || true
  wait "$PANOD_PID" 2>/dev/null || true
'
STATUS=$?
# Copy the inner process status and logs to a stable report input.
printf 'integration_shell_status=%s\n' "$STATUS" >"$OUT/logs/integration-status.txt"
exit 0
