#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the short feature tour (DOC-09): the popup runs with its canned
# fixture data under Xvfb, xdotool drives it through the keyboard, ffmpeg
# grabs the screen and a second pass burns a caption bar under each step.
# Nothing touches a real desktop, clipboard or daemon. Needs Xvfb, xdotool,
# xwininfo, bc, ffmpeg (with libvpx and drawtext) and the GTK development
# packages.
#
#   scripts/record-tour.sh [out-file]      # default: docs/book/src/media/tour.webm
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/docs/book/src/media/tour.webm}"
cd "$ROOT_DIR"

for tool in Xvfb xdotool xwininfo ffmpeg bc cargo; do
  command -v "$tool" >/dev/null 2>&1 || { echo "record-tour: '$tool' not found" >&2; exit 2; }
done
FONT="${TOUR_FONT:-/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf}"
[ -r "$FONT" ] || { echo "record-tour: font '$FONT' not found (set TOUR_FONT)" >&2; exit 2; }

cargo build -p panora-gui --features fixture --release
GUI="${CARGO_TARGET_DIR:-target}/release/panora-gui"
mkdir -p -- "$(dirname -- "$OUT")"
SCRATCH="$(mktemp -d -t panora-tour.XXXXXX)"
cleanup() {
  kill "${FFMPEG_PID:-}" "${GUI_PID:-}" "${XVFB_PID:-}" 2>/dev/null || true
  rm -rf "$SCRATCH"
}
trap cleanup EXIT

DISPLAY_NO=":96"
export DISPLAY="$DISPLAY_NO"
Xvfb "$DISPLAY_NO" -screen 0 1000x720x24 -ac >/dev/null 2>&1 &
XVFB_PID=$!
sleep 1

# A throwaway config: English UI, light style, welcome dialog already seen.
install -d "$SCRATCH/config/panora" "$SCRATCH/state" "$SCRATCH/data"
printf '[ui]\ntheme = "light"\nlanguage = "en"\n' > "$SCRATCH/config/panora/config.toml"
: > "$SCRATCH/config/panora/first-run"

env -u WAYLAND_DISPLAY GDK_BACKEND=x11 XDG_CONFIG_HOME="$SCRATCH/config" \
  XDG_STATE_HOME="$SCRATCH/state" XDG_DATA_HOME="$SCRATCH/data" \
  GSK_RENDERER=cairo dbus-run-session -- "$GUI" >/dev/null 2>&1 &
GUI_PID=$!

WIN=""
for _ in $(seq 1 120); do
  WIN="$(xdotool search --onlyvisible --name . 2>/dev/null | head -n1 || true)"
  [ -n "$WIN" ] && break
  sleep 0.25
done
[ -n "$WIN" ] || { echo "record-tour: the popup never mapped" >&2; exit 1; }
sleep 2

# Crop the recording to the popup; x264/vp9 want even dimensions.
geom="$(xwininfo -id "$WIN")"
X="$(awk '/Absolute upper-left X/ {print $NF}' <<<"$geom")"
Y="$(awk '/Absolute upper-left Y/ {print $NF}' <<<"$geom")"
W="$(awk '/Width:/ {print $NF}' <<<"$geom")"
H="$(awk '/Height:/ {print $NF}' <<<"$geom")"
W=$((W / 2 * 2))
H=$((H / 2 * 2))

# Park the pointer off the window: hover reveals row actions and tooltips.
xdotool mousemove 999 719
xdotool windowfocus --sync "$WIN" 2>/dev/null || xdotool windowfocus "$WIN"

ffmpeg -loglevel error -y -f x11grab -framerate 30 -video_size "${W}x${H}" \
  -i "${DISPLAY_NO}.0+${X},${Y}" -c:v libx264 -preset ultrafast -qp 0 \
  "$SCRATCH/raw.mkv" &
FFMPEG_PID=$!
START="$(date +%s.%N)"
sleep 1

# Each step appends "start<TAB>caption"; the caption stays until the next.
CAPTIONS="$SCRATCH/captions.tsv"
step() {
  printf '%s\t%s\n' "$(echo "$(date +%s.%N) - $START" | bc)" "$1" >> "$CAPTIONS"
}
key() { xdotool key --delay 120 "$@"; }
type_slowly() { xdotool type --delay 140 "$1"; }

step "Everything you copy, in one list"
sleep 3.5
step "Arrow keys walk the history"
key Down; sleep 0.8; key Down; sleep 0.8; key Down; sleep 0.8; key Up; sleep 0.8; key Up; sleep 1
step "Space opens the details of the selected entry"
key space; sleep 3.5; key Escape; sleep 1
step "Just start typing to search"
type_slowly "rapor"; sleep 3
key Escape; sleep 0.6
step "Filters: kind:, app:, pinned:, after:, re:"
type_slowly "kind:link"; sleep 2.5
key Escape; sleep 0.4
type_slowly "app:firefox"; sleep 2.5
key Escape; sleep 0.8
step "Ctrl+D pins an entry to the top"
key Down; sleep 0.8; key ctrl+d; sleep 2.5
step "Ctrl+Shift+P pauses capture (private mode)"
key ctrl+shift+p; sleep 3; key ctrl+shift+p; sleep 1
step "Ctrl+, opens the settings"
key ctrl+comma; sleep 4; key Escape; sleep 1
step "Enter pastes it back, Ctrl+1…9 picks a row"
sleep 3.5
step "Panora: a private clipboard history for Linux"
sleep 3

END="$(echo "$(date +%s.%N) - $START" | bc)"
kill -INT "$FFMPEG_PID"
wait "$FFMPEG_PID" 2>/dev/null || true
FFMPEG_PID=""

# Build the drawtext chain: one caption per step, shown until the next.
filter="pad=iw:ih+56:0:0:color=0x241f31"
mapfile -t lines < "$CAPTIONS"
for i in "${!lines[@]}"; do
  from="${lines[$i]%%$'\t'*}"
  text="${lines[$i]#*$'\t'}"
  if [ $((i + 1)) -lt "${#lines[@]}" ]; then
    to="${lines[$((i + 1))]%%$'\t'*}"
  else
    to="$END"
  fi
  printf '%s' "$text" > "$SCRATCH/caption-$i.txt"
  filter+=",drawtext=fontfile=${FONT}:textfile=${SCRATCH}/caption-$i.txt"
  filter+=":fontcolor=white:fontsize=15:x=(w-text_w)/2:y=h-38"
  filter+=":enable='between(t,${from},${to})'"
done

ffmpeg -loglevel error -y -i "$SCRATCH/raw.mkv" -vf "$filter" \
  -c:v libvpx-vp9 -b:v 0 -crf 40 -row-mt 1 -an "$OUT"
echo "wrote $OUT ($(du -h "$OUT" | cut -f1), ${END%.*}s)"
