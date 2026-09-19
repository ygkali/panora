#!/usr/bin/env bash
# Turkish-named convenience wrapper; the real script is install.sh.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec bash "$ROOT_DIR/install.sh" "$@"
