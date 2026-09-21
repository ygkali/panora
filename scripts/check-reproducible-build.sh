#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# SEC-08: builds panod/panora-gui/panora-cli twice, from a clean target
# directory each time, with SOURCE_DATE_EPOCH pinned and build-machine
# paths stripped from the binaries, then compares the SHA256 of each
# binary. Matching hashes mean the build is reproducible: the same source
# at the same commit produces byte-identical binaries no matter when or
# where it is built. Needs cargo-auditable (`cargo install cargo-auditable
# --locked`) and the same system deps as packaging/build-deb.sh.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

for tool in cargo sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || { echo "check-reproducible-build: '$tool' not found" >&2; exit 2; }
done
if ! cargo auditable --version >/dev/null 2>&1; then
  echo "check-reproducible-build: cargo-auditable not found; cargo install cargo-auditable --locked" >&2
  exit 2
fi

# A fixed, arbitrary but stable epoch: what matters is that both builds use
# the *same* one, not which one. The release workflow instead uses the
# tagged commit's own timestamp.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1700000000}"
# Absolute paths (this checkout's own location) would otherwise leak into
# debug info and panic messages and differ between two checkouts of the
# same commit.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT_DIR=/build"

BINARIES=(panod panora-gui panora-cli)
SCRATCH="$(mktemp -d -t panora-repro.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT

build_once() {
  local target_dir="$1"
  CARGO_TARGET_DIR="$target_dir" cargo auditable build --release --workspace >/dev/null
}

echo "[1/3] First build ($SCRATCH/a)..."
build_once "$SCRATCH/a"
echo "[2/3] Second build ($SCRATCH/b)..."
build_once "$SCRATCH/b"

echo "[3/3] Comparing..."
status=0
for bin in "${BINARIES[@]}"; do
  hash_a="$(sha256sum "$SCRATCH/a/release/$bin" | cut -d' ' -f1)"
  hash_b="$(sha256sum "$SCRATCH/b/release/$bin" | cut -d' ' -f1)"
  if [[ "$hash_a" == "$hash_b" ]]; then
    echo "  ok    $bin  $hash_a"
  else
    echo "  DIFFERS  $bin"
    echo "    build a: $hash_a"
    echo "    build b: $hash_b"
    status=1
  fi
done
exit "$status"
