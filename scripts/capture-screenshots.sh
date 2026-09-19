#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Renders the popup with its canned fixture data under Xvfb and saves the
# README / AppStream screenshots into docs/screenshots/. Needs Xvfb,
# ImageMagick (import) and the GTK development packages.
#
#   scripts/capture-screenshots.sh [out-dir]
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/docs/screenshots}"
cd "$ROOT_DIR"

for tool in Xvfb import xwininfo cargo; do
  command -v "$tool" >/dev/null 2>&1 || { echo "capture-screenshots: '$tool' not found" >&2; exit 2; }
done

cargo build -p panora-gui --features fixture --release
GUI="${CARGO_TARGET_DIR:-target}/release/panora-gui"
install -d "$OUT"
SCRATCH="$(mktemp -d -t panora-shots.XXXXXX)"
trap 'kill "${XVFB_PID:-}" "${GUI_PID:-}" 2>/dev/null; rm -rf "$SCRATCH"' EXIT

# The window opens at 420x660 in the top-left corner. The screen is wider
# than the window so the pointer, which Xvfb parks at the centre, hovers no
# row (hover reveals the row actions and a tooltip); the black remainder is
# trimmed off the shot afterwards.
DISPLAY_NO=":97"
Xvfb "$DISPLAY_NO" -screen 0 1000x700x24 -ac >/dev/null 2>&1 &
XVFB_PID=$!
sleep 1

shoot() {
  local theme="$1" language="$2" file="$3"
  install -d "$SCRATCH/config/panora"
  printf '[ui]\ntheme = "%s"\nlanguage = "%s"\n' "$theme" "$language" > "$SCRATCH/config/panora/config.toml"
  # GTK prefers Wayland whenever WAYLAND_DISPLAY is set (WSLg, a nested
  # session); the shot must land on the Xvfb screen, so force X11.
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 DISPLAY="$DISPLAY_NO" XDG_CONFIG_HOME="$SCRATCH/config" \
    GSK_RENDERER=cairo dbus-run-session -- "$GUI" >/dev/null 2>&1 &
  GUI_PID=$!
  # A cold GTK start on a slow machine (CI: icon and font caches) can take
  # well over ten seconds; wait for a top-level window, then let it paint.
  for _ in $(seq 1 120); do
    if DISPLAY="$DISPLAY_NO" xwininfo -root -children 2>/dev/null | grep -q "child"; then
      break
    fi
    sleep 0.25
  done
  sleep 2
  DISPLAY="$DISPLAY_NO" import -window root "$SCRATCH/root.png"
  convert "$SCRATCH/root.png" -trim +repage "$OUT/$file"
  kill "$GUI_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true
  echo "wrote $OUT/$file"
}

shoot light en popup-light.png
shoot dark en popup-dark.png
shoot light tr popup-light-tr.png
