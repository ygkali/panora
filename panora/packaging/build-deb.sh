#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the release binaries and assembles dist/panora_<version>_<arch>.deb.
# Uses plain dpkg-deb so the only build-time requirement beyond Rust is dpkg.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
if [[ -z "$VERSION" ]]; then
  echo "Hata: Cargo.toml içinden sürüm okunamadı." >&2
  exit 1
fi

for tool in cargo dpkg-deb dpkg; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Hata: '$tool' bulunamadı." >&2
    [[ "$tool" == "cargo" ]] && echo "Rust kurulumu: https://rustup.rs" >&2
    exit 1
  fi
done

ARCH="$(dpkg --print-architecture)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "[1/4] Release binaries derleniyor (panod, panora-gui, panora-cli)..."
cargo build --release --workspace

echo "[2/4] Paket ağacı hazırlanıyor: $STAGE"
install -d "$STAGE/DEBIAN"
install -d "$STAGE/usr/bin"
install -d "$STAGE/usr/lib/systemd/user"
install -d "$STAGE/usr/share/applications"
install -d "$STAGE/usr/share/dbus-1/services"
install -d "$STAGE/usr/share/doc/panora"

install -m 0755 target/release/panod "$STAGE/usr/bin/panod"
install -m 0755 target/release/panora-gui "$STAGE/usr/bin/panora-gui"
install -m 0755 target/release/panora-cli "$STAGE/usr/bin/panora-cli"
# Both README and the .desktop entry launch the popup as `panora`.
ln -s panora-gui "$STAGE/usr/bin/panora"

install -m 0644 packaging/panod.service "$STAGE/usr/lib/systemd/user/panod.service"
install -m 0644 packaging/io.panora.Panora.desktop "$STAGE/usr/share/applications/io.panora.Panora.desktop"
# D-Bus activation: panod and the GNOME extension toggle the popup by name.
install -m 0644 packaging/io.panora.Panora.service "$STAGE/usr/share/dbus-1/services/io.panora.Panora.service"
install -m 0644 LICENSE "$STAGE/usr/share/doc/panora/copyright"

# GNOME Shell bridge: Super+V, clipboard forwarding on GNOME < 48, paste helper.
EXT_UUID="panora@panora-clipboard.org"
EXT_DIR="$STAGE/usr/share/gnome-shell/extensions/$EXT_UUID"
install -d "$EXT_DIR/schemas"
install -m 0644 gnome-extension/metadata.json "$EXT_DIR/metadata.json"
install -m 0644 gnome-extension/extension.js "$EXT_DIR/extension.js"
install -m 0644 gnome-extension/schemas/*.gschema.xml "$EXT_DIR/schemas/"
if command -v glib-compile-schemas >/dev/null 2>&1; then
  glib-compile-schemas "$EXT_DIR/schemas"
else
  echo "Uyarı: glib-compile-schemas yok; Super+V kısayolu şema olmadan çalışmaz." >&2
  echo "        sudo apt install -y libglib2.0-dev-bin" >&2
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
  echo "      libc6 alt sınırı binary'lerden okundu: >= $LIBC_MIN"
else
  LIBC_DEP="libc6"
  echo "Uyarı: objdump/readelf yok; libc6 sürüm alt sınırı belirlenemedi." >&2
  echo "       Paket, kendisinden eski glibc'li bir sisteme kurulabilir ve" >&2
  echo "       çalışmayabilir. 'sudo apt install -y binutils' ile çözülür." >&2
fi

# Clipboard access is native (x11rb / wayland-client), so no helper binaries
# are required at runtime. gnome-keyring (or another Secret Service) stores
# the master key; wtype and ydotool are optional instant-paste helpers for
# non-GNOME Wayland compositors.
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: panora
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: $LIBC_DEP, libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.5), libglib2.0-0, adwaita-icon-theme, librsvg2-common
Recommends: gnome-keyring
Suggests: wtype, ydotool
Installed-Size: $INSTALLED_KB
Maintainer: Panora contributors <panora@panora-clipboard.org>
Homepage: https://github.com/panora-clipboard/panora
Description: Secure, lightweight clipboard manager for Debian desktops
 Panora keeps an encrypted clipboard history with a GTK4/libadwaita popup,
 FTS5 search, pinning and a private mode. Payloads are stored as
 XChaCha20-Poly1305 encrypted blobs and never leave the machine.
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

echo "[3/4] .deb paketleniyor..."
install -d dist
DEB_PATH="dist/panora_${VERSION}_${ARCH}.deb"
# root:root ownership regardless of who runs the build.
dpkg-deb --root-owner-group --build "$STAGE" "$DEB_PATH" >/dev/null

echo "[4/4] Doğrulama"
dpkg-deb --info "$DEB_PATH" | sed -n '1,12p'
sha256sum "$DEB_PATH"

echo
echo "Paket hazır: $DEB_PATH"
echo "Kurmak için: ./install.sh"
