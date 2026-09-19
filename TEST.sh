#!/usr/bin/env bash
# Turkish-named convenience wrapper; the real script is test-local.sh.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec bash "$ROOT_DIR/test-local.sh" "$@"
