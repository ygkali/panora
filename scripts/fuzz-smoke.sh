#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Short smoke run of every SEC-06 fuzz target (default 15s each): builds
# and runs each one briefly to catch an immediate crash, without the long
# unattended run a real fuzzing campaign needs. Needs nightly Rust,
# cargo-fuzz and clang.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR/crates/panora-core"

SECONDS_EACH="${1:-15}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/panora-target-fuzz}"

for target in fuzz/fuzz_targets/*.rs; do
  name="$(basename "$target" .rs)"
  echo "=== $name (${SECONDS_EACH}s) ==="
  cargo +nightly fuzz run "$name" -- "-max_total_time=$SECONDS_EACH"
done
