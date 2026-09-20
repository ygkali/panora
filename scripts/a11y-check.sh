#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Accessibility smoke test: start the fixture popup under Xvfb with its own
# session bus (GTK finds the AT-SPI registry through it), dump the
# accessibility tree with scripts/a11y-tree.py and fail when the popup
# exposes no named controls. The dump lands in $1 (default: a11y-tree.txt).
#
# Needs: Xvfb, dbus-run-session, at-spi2-core, python3-pyatspi and a
# `cargo build -p panora-gui --features fixture --release`.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-a11y-tree.txt}"
GUI="${PANORA_GUI:-${CARGO_TARGET_DIR:-$ROOT_DIR/target}/release/panora-gui}"
DISPLAY_NO="${A11Y_DISPLAY:-:96}"

for tool in Xvfb dbus-run-session python3; do
  command -v "$tool" >/dev/null || { echo "a11y-check: $tool is missing" >&2; exit 2; }
done
python3 -c 'import pyatspi' 2>/dev/null || { echo "a11y-check: python3-pyatspi is missing" >&2; exit 2; }
[ -x "$GUI" ] || { echo "a11y-check: $GUI not built (cargo build -p panora-gui --features fixture --release)" >&2; exit 2; }

SCRATCH="$(mktemp -d -t panora-a11y.XXXXXX)"
trap 'kill "${GUI_PID:-}" "${XVFB_PID:-}" 2>/dev/null || true; rm -rf "$SCRATCH"' EXIT

install -d "$SCRATCH/config/panora"
printf '[ui]\nlanguage = "en"\n' > "$SCRATCH/config/panora/config.toml"
: > "$SCRATCH/config/panora/first-run"

Xvfb "$DISPLAY_NO" -screen 0 1000x700x24 -ac >/dev/null 2>&1 &
XVFB_PID=$!
sleep 1

# One session bus for both processes: the popup registers with the AT-SPI
# registry on it and the dumper reads the tree back from the same place.
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 DISPLAY="$DISPLAY_NO" XDG_CONFIG_HOME="$SCRATCH/config" \
  GSK_RENDERER=cairo GTK_A11Y=atspi \
  dbus-run-session -- bash -c '
    "$1" >/dev/null 2>&1 &
    gui=$!
    python3 "$2" > "$3"
    status=$?
    kill "$gui" 2>/dev/null || true
    exit "$status"
  ' _ "$GUI" "$ROOT_DIR/scripts/a11y-tree.py" "$OUT"
status=$?
tail -n 1 "$OUT"
if [ "$status" -ne 0 ]; then
  echo "a11y-check: FAIL (see $OUT)" >&2
  exit "$status"
fi
echo "a11y-check: PASS ($(wc -l < "$OUT") objects, $OUT)"
