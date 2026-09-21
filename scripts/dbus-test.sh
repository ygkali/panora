#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs the public D-Bus API integration tests (INT-05) against a throwaway
# session bus, so nothing touches the developer's own one. Needs
# dbus-run-session.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

command -v dbus-run-session >/dev/null 2>&1 || { echo "dbus-test: 'dbus-run-session' not found" >&2; exit 2; }

dbus-run-session -- bash -c '
  set -e
  cargo test -p panod --test dbus_api -- --ignored --test-threads=1 "$@"
' -- "$@"
