#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs the Secret Service integration tests (key load/store; rotate-key
# including a simulated mid-rotation crash, SEC-01; the lock password's
# keyring backup, SEC-02) against a throwaway gnome-keyring in a private
# session bus, so the developer's own keyring is never touched. Needs
# gnome-keyring and dbus-daemon installed.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

for tool in dbus-run-session gnome-keyring-daemon cargo; do
  command -v "$tool" >/dev/null 2>&1 || { echo "keyring-test: '$tool' not found" >&2; exit 2; }
done

SCRATCH="$(mktemp -d -t panora-keyring.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT

# XDG_DATA_HOME moves the keyring files (and XDG_RUNTIME_DIR its control
# socket) into the scratch directory.
export XDG_DATA_HOME="$SCRATCH/data"
export XDG_RUNTIME_DIR="$SCRATCH/run"
install -d -m 0700 "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"

dbus-run-session -- bash -c '
  set -e
  # A throwaway password creates and unlocks the login collection, which
  # gnome-keyring aliases as "default", without any prompter.
  eval "$(printf "panora-test" | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null | sed "s/^/export /")"
  sleep 1
  cargo test -p panod --test keyring -- --ignored "$@"
  cargo test -p panod --test rotate_key -- --ignored --test-threads=1 "$@"
  cargo test -p panod --test lock -- --ignored --test-threads=1 "$@"
' -- "$@"
