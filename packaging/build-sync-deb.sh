#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Assembles dist/panora-sync_<version>_<arch>.deb: the optional device-sync
# service (SYNC-04, ADR 0004). Separate from the main package on purpose:
# without it installed, Panora has no network code at all. Reuses the
# release binary build-deb.sh built; builds it when it is missing.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

MAINTAINER="ygkali <kompansebuyucu@proton.me>"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
ARCH="$(dpkg --print-architecture)"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
BIN="$TARGET_DIR/release/panora-sync"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

if [[ ! -x "$BIN" ]]; then
  echo "[1/4] Building the release binary..."
  RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT_DIR=/build" \
    cargo auditable build --release -p panora-sync
else
  echo "[1/4] Using $BIN"
fi

echo "[2/4] Staging the package tree: $STAGE"
DOC="$STAGE/usr/share/doc/panora-sync"
install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/lib/systemd/user" \
  "$STAGE/usr/share/man/man1" "$DOC" "$STAGE/usr/share/lintian/overrides"
install -m 0755 "$BIN" "$STAGE/usr/bin/panora-sync"
install -m 0644 packaging/panora-sync.service "$STAGE/usr/lib/systemd/user/panora-sync.service"
install -m 0644 packaging/copyright "$DOC/copyright"
install -m 0644 docs/SYNC.md "$DOC/SYNC.md"
cat > "$DOC/changelog" <<CHANGELOG
panora-sync ($VERSION) stable; urgency=medium

  * Release $VERSION. Full release notes: /usr/share/doc/panora/CHANGELOG.md.gz
    and https://github.com/ygkali/panora/blob/main/CHANGELOG.md

 -- $MAINTAINER  $(date -R -u -d "@${SOURCE_DATE_EPOCH:-$(date +%s)}")
CHANGELOG
gzip -9n "$DOC/changelog"
"$BIN" man "$STAGE/usr/share/man/man1"
gzip -9n "$STAGE/usr/share/man/man1/panora-sync.1"

LIBC_DEP="libc6"
if command -v objdump >/dev/null 2>&1; then
  floor="$(objdump -T "$STAGE/usr/bin/panora-sync" 2>/dev/null |
    grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sed 's/GLIBC_//' | sort -uV | tail -1 || true)"
  [[ -n "$floor" ]] && LIBC_DEP="libc6 (>= $floor)"
fi
INSTALLED_KB="$(du -sk "$STAGE" | cut -f1)"

cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: panora-sync
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: $LIBC_DEP, panora (= $VERSION)
Installed-Size: $INSTALLED_KB
Maintainer: $MAINTAINER
Homepage: https://github.com/ygkali/panora
Bugs: https://github.com/ygkali/panora/issues
Description: Panora clipboard history sync between your own devices (experimental)
 Keeps the Panora clipboard history of your own computers in step over the
 local network: QUIC with mDNS discovery, devices paired with an invitation
 link / QR code or by comparing a six-digit code, records encrypted with a
 group key that changes whenever a device is removed. Nothing is sent to any
 server, and only local-network addresses are ever contacted.
 .
 Off until enabled per user: systemctl --user enable --now panora-sync
CONTROL

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "upgrade" ]; then
    # Stop the service for every logged-in user before the binary goes away.
    if command -v loginctl >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; then
        for uid in $(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $2}' | sort -u); do
            systemctl --user --machine="${uid}@.host" stop panora-sync.service >/dev/null 2>&1 || true
        done
    fi
fi
exit 0
PRERM
chmod 0755 "$STAGE/DEBIAN/prerm"

cat > "$STAGE/usr/share/lintian/overrides/panora-sync" <<'OVERRIDES'
# panora-sync.service is a systemd *user* unit; deb-systemd-helper only
# handles system units, so prerm stops it for logged-in users directly.
panora-sync: maintainer-script-calls-systemctl
OVERRIDES
chmod 0644 "$STAGE/usr/share/lintian/overrides/panora-sync"

(cd "$STAGE" && find . -type f ! -path './DEBIAN/*' -exec md5sum {} + | sed 's| \./| |' > DEBIAN/md5sums)
chmod 0644 "$STAGE/DEBIAN/md5sums"

echo "[3/4] Building the .deb..."
install -d dist
DEB_PATH="dist/panora-sync_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB_PATH" >/dev/null

echo "[4/4] Done: $DEB_PATH"
dpkg-deb --info "$DEB_PATH" | sed -n '1,12p'
sha256sum "$DEB_PATH"
