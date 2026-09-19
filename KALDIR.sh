#!/usr/bin/env bash
# Turkish-named convenience wrapper; the real script is uninstall.sh.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec bash "$ROOT_DIR/uninstall.sh" "$@"
