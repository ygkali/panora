#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the distributable install kit: the source tree, a prebuilt .deb and
# the one-command wrappers, in one tar.gz.
#
# Run this on Linux. Packing on Windows drops the executable bits and users
# get "permission denied" from every script.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
KIT="panora-${VERSION}-install-kit"
OUT_DIR="${1:-$ROOT_DIR/dist}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# PANORA_DEB=path reuses a package built earlier (the release workflow builds
# it in its own step); otherwise one is built here.
if [[ -n "${PANORA_DEB:-}" && -f "${PANORA_DEB}" ]]; then
  echo "[1/4] Using the prebuilt package $PANORA_DEB"
  DEB="$PANORA_DEB"
else
  echo "[1/4] Building the Debian package..."
  ./packaging/build-deb.sh >/dev/null
  DEB="$(find dist -maxdepth 1 -name 'panora_*.deb' -print -quit)"
  [[ -n "$DEB" ]] || { echo "Error: no .deb was produced." >&2; exit 1; }
fi

echo "[2/4] Staging the kit tree..."
install -d "$STAGE/$KIT"
# Everything except build output and git metadata: sources, docs, scripts.
tar -C "$ROOT_DIR" --exclude=./target --exclude=./.git --exclude=./dist -cf - . \
  | tar -C "$STAGE/$KIT" -xf -
install -d "$STAGE/$KIT/dist"
install -m 0644 "$DEB" "$STAGE/$KIT/dist/"

chmod 0755 "$STAGE/$KIT/install.sh" "$STAGE/$KIT/uninstall.sh" "$STAGE/$KIT/test-local.sh" \
           "$STAGE/$KIT/KUR.sh" "$STAGE/$KIT/TEST.sh" "$STAGE/$KIT/KALDIR.sh" \
           "$STAGE/$KIT/packaging/"*.sh "$STAGE/$KIT/scripts/"*.sh "$STAGE/$KIT/scripts/panora-doctor"

echo "[3/4] Writing the kit notes..."
cat > "$STAGE/$KIT/README-KIT.md" <<'READMEKIT'
# Panora install kit

## Install (one command)

In the directory where you unpacked the archive:

```sh
./install.sh
```

You will be asked for your sudo password. The script installs the
dependencies, installs the Panora package from `dist/` (or builds it from
source when the prebuilt one does not match your system) and starts the
background service. Messages are in English, or in Turkish when your locale
is Turkish. `KUR.sh`, `TEST.sh` and `KALDIR.sh` are Turkish-named aliases of
`install.sh`, `test-local.sh` and `uninstall.sh`.

## Use

Open the panel with `panora`, or **Super+V** on GNOME after enabling the
extension and logging in again:

```sh
gnome-extensions enable panora@ygkali.github.io
```

Check the installation with `panora-doctor` and try the daemon with
`panora-cli status` / `panora-cli list`.

## Remove

```sh
./uninstall.sh              # keeps your encrypted history
./uninstall.sh --purge-data # deletes it too
```

## Türkçe

`./KUR.sh` kurar, `./TEST.sh` sınar, `./KALDIR.sh` kaldırır. GNOME'da
kısayol **Super+V**; eklentiyi `gnome-extensions enable panora@ygkali.github.io`
ile etkinleştirip oturumu yeniden açın. Sorun giderme için `panora-doctor`.
READMEKIT

echo "[4/4] Packing the archive..."
install -d "$OUT_DIR"
ARCHIVE="$OUT_DIR/$KIT.tar.gz"
rm -f "$ARCHIVE"
tar -C "$STAGE" -czf "$ARCHIVE" "$KIT"

echo
echo "Kit ready: $ARCHIVE"
du -h "$ARCHIVE" | cut -f1 | sed 's/^/Size: /'
sha256sum "$ARCHIVE"
