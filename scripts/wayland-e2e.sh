#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# End-to-end test of the native Wayland backend without a desktop: a
# headless sway compositor (wlroots, ext/wlr-data-control), a throwaway
# gnome-keyring in a private session bus, panod, and wl-copy/wl-paste as the
# "applications". Everything lives in a scratch directory; the developer's
# own keyring, history and session are never touched.
#
#   scripts/wayland-e2e.sh            # builds panod/panora-cli in debug mode
#   PANORA_BIN_DIR=dist/bin scripts/wayland-e2e.sh   # use prebuilt binaries
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

for tool in sway wl-copy wl-paste dbus-run-session gnome-keyring-daemon; do
  command -v "$tool" >/dev/null 2>&1 || { echo "wayland-e2e: '$tool' not found" >&2; exit 2; }
done

if [[ -n "${PANORA_BIN_DIR:-}" ]]; then
  BIN="$(cd -- "$PANORA_BIN_DIR" && pwd)"
else
  command -v cargo >/dev/null 2>&1 || { echo "wayland-e2e: cargo not found" >&2; exit 2; }
  cargo build -p panod -p panora-cli
  cargo build -p panora-gui --features fixture
  BIN="${CARGO_TARGET_DIR:-target}/debug"
  BIN="$(cd -- "$BIN" && pwd)"
fi

SCRATCH="$(mktemp -d -t panora-wayland.XXXXXX)"
export XDG_RUNTIME_DIR="$SCRATCH/run"
export XDG_DATA_HOME="$SCRATCH/data"
export XDG_CONFIG_HOME="$SCRATCH/config"
install -d -m 0700 "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"
printf 'exec true\n' > "$SCRATCH/sway.conf"
export SCRATCH BIN
trap 'rm -rf "$SCRATCH"' EXIT

dbus-run-session -- bash -c '
set -uo pipefail
PASS=0; FAIL=0
pass() { PASS=$((PASS + 1)); printf "PASS  %s\n" "$1"; }
fail() { FAIL=$((FAIL + 1)); printf "FAIL  %s -- %s\n" "$1" "${2:-}"; }
cleanup() {
  kill "${PANOD_PID:-}" "${SWAY_PID:-}" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

# A throwaway password creates and unlocks the login collection without a
# prompter (gnome-keyring aliases it as "default").
eval "$(printf "panora-test" | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null | sed "s/^/export /")"
sleep 1

# Headless wlroots: no real input devices, software rendering.
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman
unset DISPLAY WAYLAND_DISPLAY
sway -c "$SCRATCH/sway.conf" > "$SCRATCH/sway.log" 2>&1 &
SWAY_PID=$!
for _ in $(seq 1 50); do
  SOCK="$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v "\.lock$" | head -n1 || true)"
  [[ -n "$SOCK" ]] && break
  sleep 0.2
done
if [[ -z "${SOCK:-}" ]]; then
  fail "sway headless" "no wayland socket appeared: $(tail -n 5 "$SCRATCH/sway.log" | tr "\n" " ")"
  exit 1
fi
export WAYLAND_DISPLAY="$(basename "$SOCK")"
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
pass "sway headless on $WAYLAND_DISPLAY"

RUST_LOG=debug "$BIN/panod" > "$SCRATCH/panod.log" 2>&1 &
PANOD_PID=$!
for _ in $(seq 1 100); do
  [[ -S "$XDG_RUNTIME_DIR/panora.sock" ]] && break
  if ! kill -0 "$PANOD_PID" 2>/dev/null; then break; fi
  sleep 0.1
done
CLI="$BIN/panora-cli"
if ! STATUS="$("$CLI" status 2>&1)"; then
  fail "panod start" "$STATUS; log: $(tail -n 8 "$SCRATCH/panod.log" | tr "\n" " ")"
  exit 1
fi
case "$STATUS" in
  *backend=wayland*) pass "backend=wayland ($STATUS)" ;;
  *) fail "backend" "expected wayland: $STATUS" ;;
esac

# wait_listed REGEX -> polls panora-cli list for up to 5 s.
wait_listed() {
  for _ in $(seq 1 50); do
    if "$CLI" list --limit 50 2>/dev/null | grep -qE "$1"; then return 0; fi
    sleep 0.1
  done
  return 1
}
id_of() { "$CLI" list --limit 50 | grep -E "$1" | head -n1 | sed -E "s/^[* ] +([0-9]+) .*/\1/"; }

# 1. capture through data-control
MARK="wayland-e2e-$$-zqx"
printf "%s" "$MARK" | wl-copy
if wait_listed "\[text\] $MARK"; then pass "text capture"; else fail "text capture" "$("$CLI" list 2>&1 | head -n 3)"; fi
TEXT_ID="$(id_of "\[text\] $MARK" || true)"

# 2. recall: panod becomes the data source, wl-paste reads it back
OTHER="wayland-e2e-$$-other"
printf "%s" "$OTHER" | wl-copy
wait_listed "\[text\] $OTHER" || true
if [[ -n "$TEXT_ID" ]] && "$CLI" copy "$TEXT_ID" >/dev/null 2>&1; then
  GOT="$(wl-paste --no-newline 2>/dev/null || true)"
  if [[ "$GOT" == "$MARK" ]]; then pass "recall served by panod"; else fail "recall" "wl-paste read \"$GOT\""; fi
else
  fail "recall" "panora-cli copy failed"
fi

# 3. HTML + text captured together (wl-copy offers one type; the daemon
#    still records the single format under the right kind)
printf "<b>%s</b>" "$MARK-html" | wl-copy --type text/html
if wait_listed "\[richtext\]"; then pass "html capture -> richtext"; else fail "html capture" ""; fi

# 4. private mode
"$CLI" private on >/dev/null
printf "%s" "wayland-e2e-$$-private" | wl-copy
sleep 1
if "$CLI" list --limit 50 | grep -q "wayland-e2e-$$-private"; then fail "private mode" "recorded"; else pass "private mode blocks capture"; fi
"$CLI" private off >/dev/null

# 5. persistence: the source client goes away, the content stays
KEEP="wayland-e2e-$$-keep"
printf "%s" "$KEEP" | wl-copy --foreground &
COPY_PID=$!
wait_listed "\[text\] $KEEP" || true
kill "$COPY_PID" 2>/dev/null || true
wait "$COPY_PID" 2>/dev/null || true
sleep 1
GOT="$(wl-paste --no-newline 2>/dev/null || true)"
if [[ "$GOT" == "$KEEP" ]]; then
  pass "clipboard survives the source exiting (panod re-offered it)"
else
  fail "persistence after source exit" "wl-paste read \"$GOT\""
fi
# ... but an entry that was never recorded is not brought back either.
"$CLI" private on >/dev/null
printf "%s" "wayland-e2e-$$-secret" | wl-copy --foreground &
COPY_PID=$!
sleep 1
kill "$COPY_PID" 2>/dev/null || true
wait "$COPY_PID" 2>/dev/null || true
sleep 1
"$CLI" private off >/dev/null
GOT="$(wl-paste --no-newline 2>/dev/null || true)"
if [[ "$GOT" == *"-secret" ]]; then
  fail "no re-offer of unrecorded content" "wl-paste read \"$GOT\""
else
  pass "unrecorded content is not re-offered"
fi

# 6. source application from wlr-foreign-toplevel: a GTK window (the
#    fixture popup) is the activated toplevel while wl-copy runs.
if [[ -x "$BIN/panora-gui" ]]; then
  GSK_RENDERER=cairo "$BIN/panora-gui" > "$SCRATCH/gui.log" 2>&1 &
  GUI_PID=$!
  sleep 3
  APPMARK="wayland-e2e-$$-fromgui"
  printf "%s" "$APPMARK" | wl-copy
  if wait_listed "\[text\] $APPMARK"; then
    if "$CLI" --json list --limit 5 | grep -Eq "\"source_app\": *\"io.github.ygkali.Panora\""; then
      pass "source application from the activated toplevel"
    else
      fail "source application" "$("$CLI" --json list --limit 1 2>&1 | grep -oE "\"source_app\": *[^,}]*" | head -n1)"
    fi
  else
    fail "capture while a window is active" ""
  fi
  kill "$GUI_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true
else
  echo "SKIP  source application (build panora-gui --features fixture for this case)"
fi

# 7. store / restore through the CLI
printf "stored-%s" "$MARK" | "$CLI" store --app e2e >/dev/null
if wait_listed "\[text\] stored-$MARK"; then pass "cli store"; else fail "cli store" ""; fi
SID="$(id_of "\[text\] stored-$MARK" || true)"
if [[ -n "$SID" ]] && "$CLI" delete "$SID" >/dev/null && "$CLI" restore "$SID" >/dev/null && "$CLI" list | grep -q "stored-$MARK"; then
  pass "delete + restore"
else
  fail "delete + restore" ""
fi


# 8. CAP-07: restart panod with clear_clipboard_after_seconds set, confirm
# a recall clears itself off the live clipboard once the delay passes.
kill "$PANOD_PID" 2>/dev/null || true
wait "$PANOD_PID" 2>/dev/null || true
install -d "$XDG_CONFIG_HOME/panora"
printf "[privacy]\nclear_clipboard_after_seconds = 2\n" > "$XDG_CONFIG_HOME/panora/config.toml"
RUST_LOG=debug "$BIN/panod" >> "$SCRATCH/panod.log" 2>&1 &
PANOD_PID=$!
for _ in $(seq 1 100); do
  [[ -S "$XDG_RUNTIME_DIR/panora.sock" ]] && break
  if ! kill -0 "$PANOD_PID" 2>/dev/null; then break; fi
  sleep 0.1
done
"$CLI" status >/dev/null 2>&1 || { fail "panod restart (CAP-07 config)" ""; }
CLEARME="wayland-e2e-$$-clearme"
printf "%s" "$CLEARME" | wl-copy
wait_listed "\[text\] $CLEARME" || true
CLEAR_ID="$(id_of "\[text\] $CLEARME" || true)"
if [[ -n "$CLEAR_ID" ]] && "$CLI" copy "$CLEAR_ID" >/dev/null 2>&1; then
  GOT="$(wl-paste --no-newline 2>/dev/null || true)"
  if [[ "$GOT" == "$CLEARME" ]]; then pass "CAP-07 recall still there before the delay"; else fail "CAP-07 pre-delay" "wl-paste read \"$GOT\""; fi
  sleep 3
  GOT="$(wl-paste --no-newline 2>/dev/null || true)"
  if [[ -z "$GOT" ]]; then
    pass "CAP-07 clipboard clears itself after clear_clipboard_after_seconds"
  else
    fail "CAP-07 auto-clear" "wl-paste still read \"$GOT\""
  fi
else
  fail "CAP-07 auto-clear" "panora-cli copy failed"
fi


# 9. CAP-10: recall to PRIMARY (middle-click paste) leaves CLIPBOARD alone.
printf "%s" "wayland-e2e-$$-onclipboard" | wl-copy
wait_listed "\[text\] wayland-e2e-\$\$-onclipboard" || true
if [[ -n "$CLEAR_ID" ]] && "$CLI" copy "$CLEAR_ID" --primary >/dev/null 2>&1; then
  GOT_PRIMARY="$(wl-paste --primary --no-newline 2>/dev/null || true)"
  GOT_CLIPBOARD="$(wl-paste --no-newline 2>/dev/null || true)"
  if [[ "$GOT_PRIMARY" == "$CLEARME" && "$GOT_CLIPBOARD" == "wayland-e2e-$$-onclipboard" ]]; then
    pass "CAP-10 recall to PRIMARY does not touch CLIPBOARD"
  else
    fail "CAP-10 primary recall" "primary=\"$GOT_PRIMARY\" clipboard=\"$GOT_CLIPBOARD\""
  fi
else
  fail "CAP-10 primary recall" "panora-cli copy --primary failed"
fi

echo "------------------------------------------------------------"
echo "Summary: $PASS PASS, $FAIL FAIL"
grep -iE "panic|error" "$SCRATCH/panod.log" | head -n 5 || true
[[ "$FAIL" -eq 0 ]]
'
