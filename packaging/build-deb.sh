#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the release binaries and assembles dist/panora_<version>_<arch>.deb.
# Uses plain dpkg-deb so the only build-time requirement beyond Rust is dpkg.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_ID="io.github.ygkali.Panora"
EXT_UUID="panora@ygkali.github.io"
MAINTAINER="ygkali <kompansebuyucu@proton.me>"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
if [[ -z "$VERSION" ]]; then
  echo "Error: could not read the version from Cargo.toml." >&2
  exit 1
fi

for tool in cargo dpkg-deb dpkg gzip; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Error: '$tool' not found." >&2
    [[ "$tool" == "cargo" ]] && echo "Install Rust from https://rustup.rs" >&2
    exit 1
  fi
done

ARCH="$(dpkg --print-architecture)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

echo "[1/5] Building release binaries (panod, panora-gui, panora-cli)..."
cargo build --release --workspace

echo "[2/5] Staging the package tree: $STAGE"
install -d "$STAGE/DEBIAN"
install -d "$STAGE/usr/bin"
install -d "$STAGE/usr/lib/systemd/user"
install -d "$STAGE/usr/share/applications"
install -d "$STAGE/usr/share/dbus-1/services"
install -d "$STAGE/usr/share/doc/panora"
install -d "$STAGE/usr/share/icons/hicolor/scalable/apps"
install -d "$STAGE/usr/share/icons/hicolor/symbolic/apps"
install -d "$STAGE/usr/share/metainfo"
install -d "$STAGE/usr/share/man/man1"
install -d "$STAGE/usr/share/bash-completion/completions"
install -d "$STAGE/usr/share/zsh/vendor-completions"
install -d "$STAGE/usr/share/fish/vendor_completions.d"

install -m 0755 "$TARGET_DIR/release/panod" "$STAGE/usr/bin/panod"
install -m 0755 "$TARGET_DIR/release/panora-gui" "$STAGE/usr/bin/panora-gui"
install -m 0755 "$TARGET_DIR/release/panora-cli" "$STAGE/usr/bin/panora-cli"
install -m 0755 scripts/panora-doctor "$STAGE/usr/bin/panora-doctor"
# Both README and the .desktop entry launch the popup as `panora`.
ln -s panora-gui "$STAGE/usr/bin/panora"

install -m 0644 packaging/panod.service "$STAGE/usr/lib/systemd/user/panod.service"
install -m 0644 "packaging/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
# D-Bus activation: panod and the GNOME extension toggle the popup by name.
install -m 0644 "packaging/$APP_ID.service" "$STAGE/usr/share/dbus-1/services/$APP_ID.service"
install -m 0644 "packaging/$APP_ID.metainfo.xml" "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
install -m 0644 "packaging/icons/$APP_ID.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -m 0644 "packaging/icons/$APP_ID-symbolic.svg" "$STAGE/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

# Documentation: DEP-5 copyright, a Debian-format changelog entry pointing at
# the full notes, and the upstream README/CHANGELOG.
install -m 0644 packaging/copyright "$STAGE/usr/share/doc/panora/copyright"
install -m 0644 README.md "$STAGE/usr/share/doc/panora/README.md"
gzip -9n -c CHANGELOG.md > "$STAGE/usr/share/doc/panora/CHANGELOG.md.gz"
gzip -9n -c THIRD_PARTY_LICENSES.md > "$STAGE/usr/share/doc/panora/THIRD_PARTY_LICENSES.md.gz"
cat > "$STAGE/usr/share/doc/panora/changelog" <<CHANGELOG
panora ($VERSION) stable; urgency=medium

  * Release $VERSION. Full release notes: /usr/share/doc/panora/CHANGELOG.md.gz
    and https://github.com/ygkali/panora/blob/main/CHANGELOG.md

 -- $MAINTAINER  $(date -R -u -d "@${SOURCE_DATE_EPOCH:-$(date +%s)}")
CHANGELOG
gzip -9n "$STAGE/usr/share/doc/panora/changelog"

# Man pages: the CLI renders its own from clap; the others are hand-written.
"$TARGET_DIR/release/panora-cli" man "$STAGE/usr/share/man/man1"
install -m 0644 packaging/man/panod.1 packaging/man/panora-gui.1 packaging/man/panora-doctor.1 \
  "$STAGE/usr/share/man/man1/"
gzip -9n "$STAGE"/usr/share/man/man1/*.1
ln -s panora-gui.1.gz "$STAGE/usr/share/man/man1/panora.1.gz"

# Shell completions, rendered by clap.
"$TARGET_DIR/release/panora-cli" completions bash > "$STAGE/usr/share/bash-completion/completions/panora-cli"
"$TARGET_DIR/release/panora-cli" completions zsh > "$STAGE/usr/share/zsh/vendor-completions/_panora-cli"
"$TARGET_DIR/release/panora-cli" completions fish > "$STAGE/usr/share/fish/vendor_completions.d/panora-cli.fish"
chmod 0644 "$STAGE/usr/share/bash-completion/completions/panora-cli" \
  "$STAGE/usr/share/zsh/vendor-completions/_panora-cli" \
  "$STAGE/usr/share/fish/vendor_completions.d/panora-cli.fish"

# GNOME Shell bridge: Super+V, clipboard forwarding on GNOME < 48, paste helper.
EXT_DIR="$STAGE/usr/share/gnome-shell/extensions/$EXT_UUID"
install -d "$EXT_DIR/schemas"
install -m 0644 gnome-extension/metadata.json "$EXT_DIR/metadata.json"
install -m 0644 gnome-extension/extension.js "$EXT_DIR/extension.js"
install -m 0644 gnome-extension/prefs.js "$EXT_DIR/prefs.js"
install -m 0644 gnome-extension/schemas/*.gschema.xml "$EXT_DIR/schemas/"
if command -v glib-compile-schemas >/dev/null 2>&1; then
  glib-compile-schemas "$EXT_DIR/schemas"
else
  echo "Warning: glib-compile-schemas not found; the Super+V shortcut will not work without the compiled schema." >&2
  echo "         sudo apt install -y libglib2.0-bin" >&2
fi

INSTALLED_KB="$(du -sk "$STAGE" | cut -f1)"

# Derive the real libc6 floor from the binaries instead of guessing. Building on
# a newer distro than the target is the one mismatch dpkg cannot catch on its
# own: a bare `libc6` dependency installs happily and then every binary dies
# with "GLIBC_2.xx not found". GTK/libadwaita minimums stay hand-written on
# purpose -- dpkg-shlibdeps would pin them to the build machine's versions and
# refuse installs on distros where the package actually runs fine.
libc_floor() {
  local reader=""
  if command -v objdump >/dev/null 2>&1; then
    reader="objdump -T"
  elif command -v readelf >/dev/null 2>&1; then
    reader="readelf -W --dyn-syms"
  else
    return 1
  fi
  # shellcheck disable=SC2086
  $reader "$STAGE"/usr/bin/panod "$STAGE"/usr/bin/panora-gui "$STAGE"/usr/bin/panora-cli 2>/dev/null |
    grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sed 's/GLIBC_//' | sort -uV | tail -1
}

if LIBC_MIN="$(libc_floor)" && [[ -n "$LIBC_MIN" ]]; then
  LIBC_DEP="libc6 (>= $LIBC_MIN)"
  echo "      libc6 floor read from the binaries: >= $LIBC_MIN"
else
  LIBC_DEP="libc6"
  echo "Warning: objdump/readelf not found; the libc6 floor could not be determined." >&2
  echo "         The package may install on a system with an older glibc and then" >&2
  echo "         fail to run. 'sudo apt install -y binutils' fixes this." >&2
fi

# Clipboard access is native (x11rb / wayland-client), so no helper binaries
# are required at runtime. gnome-keyring (or another Secret Service) stores
# the master key; wtype and ydotool are optional instant-paste helpers for
# non-GNOME Wayland compositors; xclip and wl-clipboard only serve the
# end-to-end test script.
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: panora
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: $LIBC_DEP, libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.5), libglib2.0-0t64 (>= 2.76) | libglib2.0-0 (>= 2.76), adwaita-icon-theme, librsvg2-common
Recommends: gnome-keyring | kwalletmanager
Suggests: wtype, ydotool, xclip, wl-clipboard
Installed-Size: $INSTALLED_KB
Maintainer: $MAINTAINER
Homepage: https://github.com/ygkali/panora
Bugs: https://github.com/ygkali/panora/issues
Description: Secure, lightweight clipboard manager for Debian desktops
 Panora keeps an encrypted clipboard history with a GTK4/libadwaita popup,
 FTS5 search, pinning and a private mode. Payloads are stored as
 XChaCha20-Poly1305 encrypted blobs and never leave the machine.
 .
 It speaks the X11 and Wayland clipboard protocols natively and ships a
 GNOME Shell extension for Super+V and for GNOME releases without a
 data-control protocol.
CONTROL

cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    # The daemon runs per user, so enabling it is left to install.sh / the user.
    if command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload >/dev/null 2>&1 || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q /usr/share/applications || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
    fi
    # prerm stopped the daemon for every logged-in user before the upgrade;
    # bring it back for users who had it enabled so capture does not stay
    # off until the next login.
    if command -v loginctl >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; then
        for uid in $(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $2}' | sort -u); do
            systemctl --user --machine="${uid}@.host" daemon-reload >/dev/null 2>&1 || true
            if systemctl --user --machine="${uid}@.host" is-enabled panod.service >/dev/null 2>&1; then
                systemctl --user --machine="${uid}@.host" start panod.service >/dev/null 2>&1 || true
            fi
        done
    fi
fi
exit 0
POSTINST
chmod 0755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "upgrade" ]; then
    # Stop the daemon for every logged-in user before the binary disappears.
    if command -v loginctl >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; then
        for uid in $(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $2}' | sort -u); do
            systemctl --user --machine="${uid}@.host" stop panod.service >/dev/null 2>&1 || true
        done
    fi
fi
exit 0
PRERM
chmod 0755 "$STAGE/DEBIAN/prerm"

cat > "$STAGE/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q /usr/share/applications || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
    fi
fi
# User data (~/.local/share/panora, ~/.config/panora) is never touched here:
# it belongs to the user, not to the package.
exit 0
POSTRM
chmod 0755 "$STAGE/DEBIAN/postrm"

# Lintian: the daemon is a per-user unit and dpkg has no helper for those,
# so the maintainer scripts drive systemctl --user for logged-in users
# themselves (every call is guarded and non-fatal).
install -d "$STAGE/usr/share/lintian/overrides"
cat > "$STAGE/usr/share/lintian/overrides/panora" <<'OVERRIDES'
# panod.service is a systemd *user* unit; deb-systemd-helper only handles
# system units, so postinst/prerm restart it for logged-in users directly.
panora: maintainer-script-calls-systemctl
OVERRIDES
chmod 0644 "$STAGE/usr/share/lintian/overrides/panora"

# Every shipped file gets a checksum so dpkg -V and lintian are happy.
(cd "$STAGE" && find . -type f ! -path './DEBIAN/*' -exec md5sum {} + | sed 's| \./| |' > DEBIAN/md5sums)
chmod 0644 "$STAGE/DEBIAN/md5sums"

echo "[3/5] Building the .deb..."
install -d dist
DEB_PATH="dist/panora_${VERSION}_${ARCH}.deb"
# root:root ownership regardless of who runs the build.
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB_PATH" >/dev/null

echo "[4/5] Verifying"
dpkg-deb --info "$DEB_PATH" | sed -n '1,12p'
sha256sum "$DEB_PATH"

echo "[5/5] Done"
echo "Package: $DEB_PATH"
echo "Install with: ./install.sh"
