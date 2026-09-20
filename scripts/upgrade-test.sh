#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Upgrade path test (PKG-10): the history a released Panora wrote must still
# open, read and migrate under the build in this working tree.
#
# The old daemon records a few entries in a scratch data directory, it is
# stopped, and the new daemon is started on the very same directory. The
# entries have to come back byte for byte, the schema has to end at the
# version this build targets, and a migration has to leave the pre-migration
# backup next to the file.
#
# The old build comes from a released `.deb` (fast) or from a git tag (slow;
# a worktree and a full dependency build). With neither -- no release yet --
# there is nothing to upgrade from and the script says so and exits 0.
#
#   scripts/upgrade-test.sh                       # auto: newest v* tag
#   scripts/upgrade-test.sh --from v1.3.0
#   scripts/upgrade-test.sh --deb dist/panora_1.3.0_amd64.deb
#
# Needs Xvfb, dbus-run-session, gnome-keyring-daemon, python3 and cargo.
# Nothing outside the scratch directory is touched: the keyring, the session
# bus, the X server and both data directories are throwaway.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: scripts/upgrade-test.sh [--from REF] [--deb FILE]

  --from REF   build the old side from this git ref (default: newest v* tag)
  --deb FILE   take the old binaries out of this .deb instead of building
  -h, --help   this help

Exits 0 with a message when there is no previous release to upgrade from.
USAGE
}

FROM="${PANORA_UPGRADE_FROM:-}"
DEB="${PANORA_UPGRADE_DEB:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --from) FROM="${2:-}"; shift 2 ;;
    --deb) DEB="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "upgrade-test: unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -z "$DEB" && -z "$FROM" ]]; then
  FROM="$(git tag -l 'v*' --sort=-v:refname | head -n1 || true)"
fi
if [[ -z "$DEB" && -z "$FROM" ]]; then
  echo "upgrade-test: no previous release to upgrade from (no .deb given and no v* tag); nothing to check"
  exit 0
fi

for tool in Xvfb dbus-run-session gnome-keyring-daemon python3 cargo; do
  command -v "$tool" >/dev/null 2>&1 || { echo "upgrade-test: '$tool' not found" >&2; exit 2; }
done
if [[ -n "$DEB" ]]; then
  command -v dpkg-deb >/dev/null 2>&1 || { echo "upgrade-test: 'dpkg-deb' not found" >&2; exit 2; }
  [[ -f "$DEB" ]] || { echo "upgrade-test: no such file: $DEB" >&2; exit 2; }
  DEB="$(cd -- "$(dirname -- "$DEB")" && pwd)/$(basename -- "$DEB")"
fi

# The schema this working tree targets; the whole point is that the old file
# arrives here.
TARGET_SCHEMA="$(sed -n 's/^pub const SCHEMA_VERSION: i64 = \([0-9]*\);.*/\1/p' \
  crates/panora-core/src/storage/db.rs)"
[[ -n "$TARGET_SCHEMA" ]] || { echo "upgrade-test: cannot read SCHEMA_VERSION" >&2; exit 2; }

SCRATCH="$(mktemp -d -t panora-upgrade.XXXXXX)"
WORKTREE=""
cleanup() {
  [[ -n "$WORKTREE" ]] && git worktree remove --force "$WORKTREE" >/dev/null 2>&1
  rm -rf "$SCRATCH"
}
trap cleanup EXIT

echo "== new side =="
cargo build -q -p panod -p panora-cli
NEW_BIN="$(cd -- "${CARGO_TARGET_DIR:-target}/debug" && pwd)"
echo "new binaries: $NEW_BIN"

echo "== old side =="
if [[ -n "$DEB" ]]; then
  install -d "$SCRATCH/old-root"
  dpkg-deb -x "$DEB" "$SCRATCH/old-root"
  OLD_BIN="$SCRATCH/old-bin"
  install -d "$OLD_BIN"
  for prog in panod panora-cli; do
    found="$(find "$SCRATCH/old-root" -type f -name "$prog" -perm -u+x | head -n1)"
    [[ -n "$found" ]] || { echo "upgrade-test: $prog not in $DEB" >&2; exit 1; }
    cp "$found" "$OLD_BIN/$prog"
  done
  OLD_LABEL="$(basename -- "$DEB")"
else
  git rev-parse --verify "$FROM^{commit}" >/dev/null 2>&1 \
    || { echo "upgrade-test: no such ref: $FROM" >&2; exit 2; }
  WORKTREE="$SCRATCH/old-src"
  git worktree add --detach --quiet "$WORKTREE" "$FROM"
  ( cd "$WORKTREE" && CARGO_TARGET_DIR="$SCRATCH/old-target" cargo build -q -p panod -p panora-cli )
  OLD_BIN="$SCRATCH/old-target/debug"
  OLD_LABEL="$FROM"
fi
echo "old binaries: $OLD_BIN ($OLD_LABEL)"

export SCRATCH NEW_BIN OLD_BIN OLD_LABEL TARGET_SCHEMA
export XDG_RUNTIME_DIR="$SCRATCH/run"
export XDG_DATA_HOME="$SCRATCH/data"
export XDG_CONFIG_HOME="$SCRATCH/config"
install -d -m 0700 "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"

dbus-run-session -- bash -c '
set -uo pipefail
PASS=0; FAIL=0
pass() { PASS=$((PASS + 1)); printf "PASS  %s\n" "$1"; }
fail() { FAIL=$((FAIL + 1)); printf "FAIL  %s -- %s\n" "$1" "${2:-}"; }
DB="$XDG_DATA_HOME/panora/history.db"

cleanup() {
  kill "${PANOD_PID:-}" "${XVFB_PID:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

# A throwaway password unlocks the login collection without a prompter, so
# both daemons find the same master key.
eval "$(printf "panora-test" | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null | sed "s/^/export /")"
sleep 1

DISPLAY_NO="${PANORA_UPGRADE_DISPLAY:-:95}"
unset WAYLAND_DISPLAY
Xvfb "$DISPLAY_NO" -screen 0 800x600x24 -ac >"$SCRATCH/xvfb.log" 2>&1 &
XVFB_PID=$!
export DISPLAY="$DISPLAY_NO"
for _ in $(seq 1 50); do
  [[ -e "/tmp/.X11-unix/X${DISPLAY_NO#:}" ]] && break
  sleep 0.2
done

start_panod() {
  # $1 = bin dir, $2 = label
  "$1/panod" > "$SCRATCH/panod-$2.log" 2>&1 &
  PANOD_PID=$!
  for _ in $(seq 1 100); do
    [[ -S "$XDG_RUNTIME_DIR/panora.sock" ]] && return 0
    kill -0 "$PANOD_PID" 2>/dev/null || break
    sleep 0.1
  done
  return 1
}

stop_panod() {
  kill "$PANOD_PID" 2>/dev/null
  for _ in $(seq 1 100); do
    kill -0 "$PANOD_PID" 2>/dev/null || break
    sleep 0.1
  done
  kill -9 "$PANOD_PID" 2>/dev/null
  wait "$PANOD_PID" 2>/dev/null
  PANOD_PID=""
}

schema_version() {
  python3 - "$DB" <<"PY"
import sqlite3, sys
try:
    con = sqlite3.connect("file:%s?mode=ro" % sys.argv[1], uri=True)
    row = con.execute("SELECT value FROM meta WHERE key = ?", ("schema_version",)).fetchone()
    print(row[0] if row else 0)
except Exception:
    print(-1)
PY
}

if ! start_panod "$OLD_BIN" old; then
  fail "old panod starts" "$(tail -n 8 "$SCRATCH/panod-old.log" | tr "\n" " ")"
  exit 1
fi
pass "old panod ($OLD_LABEL) started: $("$OLD_BIN/panora-cli" status 2>&1)"

MARKERS=("upgrade-marker-one" "upgrade-marker-two ĞÜŞİÖÇ" "upgrade-marker-three")
IDS=()
for text in "${MARKERS[@]}"; do
  if ! printf "%s" "$text" | "$OLD_BIN/panora-cli" store --no-copy >/dev/null 2>&1; then
    fail "old panod records" "store failed for: $text"
    exit 1
  fi
done
# `list` is newest first; the ids come back in the order they were stored.
mapfile -t IDS < <("$OLD_BIN/panora-cli" pick --format "{id}" 2>/dev/null | head -n "${#MARKERS[@]}" | tac)
if [[ "${#IDS[@]}" -eq "${#MARKERS[@]}" ]]; then
  pass "old panod recorded ${#IDS[@]} entries (ids ${IDS[*]})"
else
  fail "old panod records" "expected ${#MARKERS[@]} ids, got ${IDS[*]-none}"
  exit 1
fi

OLD_SCHEMA="$(schema_version)"
pass "history written at schema version $OLD_SCHEMA"

stop_panod
if [[ -S "$XDG_RUNTIME_DIR/panora.sock" ]] && "$OLD_BIN/panora-cli" status >/dev/null 2>&1; then
  fail "old panod stops" "the socket still answers"
else
  pass "old panod stopped"
fi

if ! start_panod "$NEW_BIN" new; then
  fail "new panod opens the old history" "$(tail -n 12 "$SCRATCH/panod-new.log" | tr "\n" " ")"
  exit 1
fi
pass "new panod started on the old data directory: $("$NEW_BIN/panora-cli" status 2>&1)"

ok=1
for i in "${!MARKERS[@]}"; do
  want="${MARKERS[$i]}"
  got="$("$NEW_BIN/panora-cli" preview "${IDS[$i]}" --mime "text/plain;charset=utf-8" 2>&1)"
  if [[ "$got" != "$want" ]]; then
    fail "entry ${IDS[$i]} survives" "want [$want], got [$got]"
    ok=0
  fi
done
[[ "$ok" -eq 1 ]] && pass "all ${#MARKERS[@]} entries read back unchanged under the new daemon"

if "$NEW_BIN/panora-cli" list "upgrade-marker" | grep -q "upgrade-marker"; then
  pass "the search index still finds the old entries"
else
  fail "search index" "list upgrade-marker returned nothing"
fi

NEW_SCHEMA="$(schema_version)"
if [[ "$NEW_SCHEMA" == "$TARGET_SCHEMA" ]]; then
  pass "schema migrated $OLD_SCHEMA -> $NEW_SCHEMA (this build targets $TARGET_SCHEMA)"
else
  fail "schema version" "expected $TARGET_SCHEMA, found $NEW_SCHEMA"
fi

if [[ "$OLD_SCHEMA" -lt "$TARGET_SCHEMA" ]]; then
  BACKUP="${DB%.db}.db.bak-v$OLD_SCHEMA"
  if [[ -f "$BACKUP" ]]; then
    pass "pre-migration backup kept: $(basename "$BACKUP") ($(stat -c %s "$BACKUP") bytes)"
  else
    fail "pre-migration backup" "no $(basename "$BACKUP") next to the history"
  fi
else
  pass "no migration needed; the release already wrote schema $OLD_SCHEMA"
fi

printf "\n%d passed, %d failed\n" "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
'
