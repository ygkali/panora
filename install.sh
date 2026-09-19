#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Installs Panora on a Debian-based system. Uses dist/panora_*.deb when that
# package can actually run here, and otherwise builds one from source.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -f /etc/debian_version ]]; then
  echo "Hata: Bu otomatik kurulum Debian tabanlı sistemler içindir." >&2
  echo "      Başka bir dağıtımdaysanız INSTALL-local.md içindeki" >&2
  echo "      'Kaynak koddan derleme' bölümünü izleyin." >&2
  exit 1
fi

# ------------------------------------------------------------ release floor

# libadwaita >= 1.5 (GTK 4.12+) is the hard floor, which means Debian 13,
# Ubuntu 24.04 or Zorin OS 18 (Ubuntu 24.04 base). Older releases would fail
# halfway through apt with an unhelpful "libadwaita-1-0 (>= 1.5)" error, so
# refuse them here with a message that says what to upgrade.
os_field() {
  sed -n "s/^$1=//p" /etc/os-release 2>/dev/null | head -n1 | tr -d '"'
}
OS_ID="$(os_field ID || true)"
OS_PRETTY="$(os_field PRETTY_NAME || true)"
OS_VERSION_ID="$(os_field VERSION_ID || true)"
# Ubuntu derivatives (Zorin, Mint, Pop!_OS) carry the base release here; Zorin
# 17 ships UBUNTU_CODENAME=jammy, Zorin 18 UBUNTU_CODENAME=noble.
OS_UBUNTU_CODENAME="$(os_field UBUNTU_CODENAME || true)"
OS_UBUNTU_CODENAME="${OS_UBUNTU_CODENAME:-$(os_field VERSION_CODENAME || true)}"

too_old() {
  echo "Hata: ${OS_PRETTY:-Bu dağıtım} ($1) Panora tarafından desteklenmiyor." >&2
  echo "      Panora, libadwaita >= 1.5 ve GTK >= 4.12 gerektirir; bu kütüphaneler" >&2
  echo "      ancak Ubuntu 24.04, Debian 13 veya Zorin OS 18 ve sonrasında bulunur." >&2
  echo "      Zorin OS 17 (Ubuntu 22.04 tabanlı) libadwaita 1.1 ile gelir ve" >&2
  echo "      paket deposundan güncellenemez; sistemi Zorin OS 18'e yükseltin." >&2
  exit 1
}

# Ubuntu codenames older than noble (24.04). Unknown/newer codenames pass and
# apt has the final say.
OLD_UBUNTU_CODENAMES=" xenial bionic focal jammy kinetic lunar mantic "
case "$OS_ID" in
  zorin)
    if [[ -n "$OS_UBUNTU_CODENAME" ]]; then
      [[ "$OLD_UBUNTU_CODENAMES" == *" $OS_UBUNTU_CODENAME "* ]] &&
        too_old "Ubuntu $OS_UBUNTU_CODENAME tabanı"
    elif [[ "${OS_VERSION_ID%%.*}" =~ ^[0-9]+$ && "${OS_VERSION_ID%%.*}" -lt 18 ]]; then
      too_old "Zorin OS $OS_VERSION_ID"
    fi
    ;;
  ubuntu)
    if [[ -n "$OS_VERSION_ID" ]] && ! printf '%s\n%s\n' "24.04" "$OS_VERSION_ID" | sort -VC; then
      too_old "Ubuntu $OS_VERSION_ID"
    fi
    ;;
  debian)
    if [[ "${OS_VERSION_ID%%.*}" =~ ^[0-9]+$ && "${OS_VERSION_ID%%.*}" -lt 13 ]]; then
      too_old "Debian $OS_VERSION_ID"
    fi
    ;;
  *)
    if [[ -n "$OS_UBUNTU_CODENAME" && "$OLD_UBUNTU_CODENAMES" == *" $OS_UBUNTU_CODENAME "* ]]; then
      too_old "Ubuntu $OS_UBUNTU_CODENAME tabanı"
    fi
    ;;
esac

if [[ "${EUID}" -eq 0 ]]; then
  SUDO=()
else
  SUDO=(sudo)
fi

# systemctl --user and gnome-extensions must run as the desktop user. Under
# `sudo ./install.sh` that is SUDO_USER, not root: enabling the unit as root
# would start a daemon in a session that has no clipboard.
DESKTOP_USER="${SUDO_USER:-$(id -un)}"
DESKTOP_HOME="$(getent passwd "$DESKTOP_USER" 2>/dev/null | cut -d: -f6 || true)"
DESKTOP_HOME="${DESKTOP_HOME:-$HOME}"
run_as_desktop_user() {
  if [[ "$(id -un)" == "$DESKTOP_USER" ]]; then
    "$@"
    return
  fi
  local uid
  uid="$(id -u "$DESKTOP_USER")"
  # sudo's env_reset drops the display variables; hand them over explicitly
  # so the user unit learns about the session it must attach to.
  sudo -u "$DESKTOP_USER" \
    HOME="$DESKTOP_HOME" \
    PATH="$DESKTOP_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
    XDG_RUNTIME_DIR="/run/user/$uid" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
    DISPLAY="${DISPLAY:-}" \
    WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}" \
    XAUTHORITY="${XAUTHORITY:-}" \
    XDG_SESSION_TYPE="${XDG_SESSION_TYPE:-}" \
    XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-}" \
    "$@"
}

# ---------------------------------------------------------------- prebuilt?

# A prebuilt package is only a shortcut if it matches this machine. Architecture
# is obvious; the glibc floor is not -- a bare `libc6` dependency lets dpkg
# install a package built on a newer distro, and then every binary dies with
# "GLIBC_2.xx not found". Check before installing, not after.
deb_runs_here() {
  local deb="$1" deb_arch host_arch need have
  [[ -f "$deb" ]] || return 1

  deb_arch="$(dpkg-deb -f "$deb" Architecture 2>/dev/null || true)"
  host_arch="$(dpkg --print-architecture)"
  if [[ -n "$deb_arch" && "$deb_arch" != "all" && "$deb_arch" != "$host_arch" ]]; then
    echo "      hazır paket $deb_arch mimarisi için, bu makine $host_arch."
    return 1
  fi

  need="$(dpkg-deb -f "$deb" Depends 2>/dev/null |
    grep -oE 'libc6 \(>= [0-9]+\.[0-9]+\)' | grep -oE '[0-9]+\.[0-9]+' || true)"
  if [[ -n "$need" ]]; then
    have="$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+$' || true)"
    if [[ -n "$have" ]] && ! printf '%s\n%s\n' "$need" "$have" | sort -VC; then
      echo "      hazır paket glibc >= $need istiyor, bu makinede $have var."
      return 1
    fi
  fi
  return 0
}

DEB_FILE="$(find "$ROOT_DIR/dist" -maxdepth 1 -type f -name 'panora_*.deb' -print -quit 2>/dev/null || true)"
BUILD_FROM_SOURCE=0
if [[ -n "$DEB_FILE" ]] && deb_runs_here "$DEB_FILE"; then
  echo "Hazır paket kullanılacak: $DEB_FILE"
else
  if [[ -n "$DEB_FILE" ]]; then
    echo "Hazır paket bu makinede çalışmaz; kaynaktan derlenecek."
  else
    echo "dist/ içinde hazır paket yok; kaynaktan derlenecek."
  fi
  BUILD_FROM_SOURCE=1
  DEB_FILE=""
fi

# ------------------------------------------------------------ dependencies

echo "[1/5] Sistem bağımlılıkları kuruluyor..."
# A single broken third-party repo (Zorin's premium repo after an in-place
# upgrade is a known case) makes apt-get update exit non-zero even though the
# main archive refreshed fine; keep going with whatever lists we have.
if ! "${SUDO[@]}" apt-get update; then
  echo "      Uyarı: apt-get update bazı depolar için başarısız oldu; mevcut paket listeleriyle devam ediliyor."
fi

# Ubuntu 24.04 and Debian 13 renamed the GLib runtime to libglib2.0-0t64 for
# the time64 transition (the old name survives only as a versioned Provides);
# GTK 4 and libadwaita kept their names on both. Ask apt which spelling has a
# real candidate here instead of hard-coding one distro's.
apt_first_available() {
  local pkg
  for pkg in "$@"; do
    if apt-cache policy "$pkg" 2>/dev/null | grep -qE '^\s*Candidate: [0-9]'; then
      echo "$pkg"
      return 0
    fi
  done
  echo "$1"
}
GLIB_RUNTIME="$(apt_first_available libglib2.0-0t64 libglib2.0-0)"
GTK_RUNTIME="$(apt_first_available libgtk-4-1 libgtk-4-1t64)"
ADW_RUNTIME="$(apt_first_available libadwaita-1-0 libadwaita-1-0t64)"

# librsvg2-common is not optional: Adwaita 48 ships symbolic icons as SVG only,
# and without gdk-pixbuf's SVG loader half the icons render as "image-missing".
# Both libgtk-4-1 and adwaita-icon-theme list it as Recommends, so a machine
# installed with --no-install-recommends does not have it. gnome-keyring is
# the Secret Service that keeps the encryption key; clipboard access itself is
# native and needs no xclip/wl-clipboard.
"${SUDO[@]}" apt-get install -y \
  "$GLIB_RUNTIME" "$GTK_RUNTIME" "$ADW_RUNTIME" \
  adwaita-icon-theme librsvg2-common \
  gnome-keyring

# The workspace declares rust-version 1.85 and Cargo.lock is v4 (cargo 1.78+).
# Ubuntu 24.04's apt cargo is 1.75, so "some cargo on PATH" is not enough.
MIN_RUST="1.85"
cargo_version() {
  run_as_desktop_user bash -c 'command -v cargo >/dev/null 2>&1 && cargo --version' 2>/dev/null |
    grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -n1 || true
}
cargo_ok() {
  local have
  have="$(cargo_version)"
  [[ -n "$have" ]] && printf '%s\n%s\n' "$MIN_RUST" "$have" | sort -VC
}

if [[ "$BUILD_FROM_SOURCE" -eq 1 ]]; then
  echo "[2/5] Derleme bağımlılıkları kuruluyor..."
  # binutils lets build-deb.sh read the real glibc floor out of the binaries;
  # libglib2.0-bin provides glib-compile-schemas for the Super+V shortcut
  # (it lives there on both Debian 13 and Ubuntu 24.04, not in -dev-bin).
  "${SUDO[@]}" apt-get install -y \
    build-essential pkg-config curl \
    libgtk-4-dev libadwaita-1-dev \
    binutils libglib2.0-bin

  # Prefer a rustup toolchain installed for the desktop user over apt's cargo:
  # ~/.cargo/env only prepends when the directory is missing from PATH, so
  # force it to the front. run_as_desktop_user does the same under sudo.
  # shellcheck disable=SC1091
  [[ -f "$DESKTOP_HOME/.cargo/env" ]] && source "$DESKTOP_HOME/.cargo/env"
  [[ -d "$DESKTOP_HOME/.cargo/bin" ]] && export PATH="$DESKTOP_HOME/.cargo/bin:$PATH"

  if ! cargo_ok; then
    HAVE_RUST="$(cargo_version)"
    if [[ -n "$HAVE_RUST" ]]; then
      echo "      Sistemdeki cargo $HAVE_RUST çok eski (en az $MIN_RUST gerekiyor)."
    else
      echo "      'cargo' bulunamadı."
    fi
    if [[ "$(id -un)" != "$DESKTOP_USER" ]]; then
      echo "Hata: Rust araç zinciri (rustup) masaüstü kullanıcısı olarak kurulmalı; sudo olmadan çalıştırın." >&2
      exit 1
    fi
    if [[ -x "$HOME/.cargo/bin/rustup" ]]; then
      echo "      rustup mevcut; stable araç zinciri güncelleniyor..."
      "$HOME/.cargo/bin/rustup" update stable
      "$HOME/.cargo/bin/rustup" default stable
    else
      echo "      Rust araç zinciri rustup ile kuruluyor (kullanıcı dizinine)..."
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
    fi
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  if ! cargo_ok; then
    echo "Hata: Rust >= $MIN_RUST kurulamadı (bulunan: $(cargo_version))." >&2
    echo "      https://rustup.rs adresinden kurup tekrar deneyin." >&2
    exit 1
  fi
  echo "      $(run_as_desktop_user cargo --version)"

  echo "      Panora derleniyor (ilk derleme birkaç dakika sürebilir)..."
  # Built as the desktop user so target/ and dist/ do not end up root-owned
  # when the script itself runs under sudo.
  run_as_desktop_user bash "$ROOT_DIR/packaging/build-deb.sh"
  DEB_FILE="$(find "$ROOT_DIR/dist" -maxdepth 1 -type f -name 'panora_*.deb' -print -quit 2>/dev/null || true)"
  if [[ -z "$DEB_FILE" ]]; then
    echo "Hata: paket üretilemedi." >&2
    exit 1
  fi
else
  echo "[2/5] Derleme atlandı (hazır paket kullanılıyor)."
fi

# --------------------------------------------------------------- install

echo "[3/5] Panora paketi kuruluyor: $DEB_FILE"
if ! "${SUDO[@]}" dpkg -i "$DEB_FILE"; then
  echo "      Bağımlılıklar tamamlanıyor..."
  "${SUDO[@]}" apt-get -f install -y
  "${SUDO[@]}" dpkg -i "$DEB_FILE"
fi

# ------------------------------------------------------------ user session

echo "[4/5] Kullanıcı servisi etkinleştiriliyor ($DESKTOP_USER)..."
if command -v systemctl >/dev/null 2>&1; then
  # Without these the unit starts with no DISPLAY/WAYLAND_DISPLAY and the
  # backend fails to connect. GNOME imports them itself; other sessions do not.
  run_as_desktop_user systemctl --user import-environment \
    DISPLAY WAYLAND_DISPLAY XAUTHORITY XDG_SESSION_TYPE XDG_CURRENT_DESKTOP 2>/dev/null || true
  if run_as_desktop_user systemctl --user daemon-reload 2>/dev/null &&
    run_as_desktop_user systemctl --user enable --now panod.service 2>/dev/null; then
    echo "      panod.service çalışıyor."
  else
    echo "      Uyarı: panod.service otomatik başlatılamadı."
    echo "      Masaüstü oturumunuzda şunu çalıştırın:"
    echo "        systemctl --user enable --now panod.service"
  fi
else
  echo "      Uyarı: systemd bulunamadı; panod'u elle başlatmanız gerekir."
fi

if command -v gnome-extensions >/dev/null 2>&1; then
  if run_as_desktop_user gnome-extensions enable panora@ygkali.github.io 2>/dev/null; then
    echo "      GNOME eklentisi etkinleştirildi (Super+V)."
  else
    # gnome-shell only discovers a newly installed system extension after it
    # restarts, so this is expected on a first install.
    echo "      GNOME eklentisi henüz etkinleştirilemedi (normal)."
    echo "      Oturumu kapatıp açtıktan sonra:"
    echo "        gnome-extensions enable panora@ygkali.github.io"
  fi
fi

# ---------------------------------------------------------------- verify

echo "[5/5] Kurulum doğrulanıyor..."
for binary in panod panora-gui panora-cli panora; do
  if [[ ! -e "/usr/bin/$binary" ]]; then
    echo "Hata: /usr/bin/$binary kurulmamış." >&2
    exit 1
  fi
done
if ! /usr/bin/panora-cli --help >/dev/null 2>&1; then
  echo "      Uyarı: panora-cli çalıştırılamadı." >&2
fi
run_as_desktop_user panora-cli status || \
  echo "      (daemon henüz yanıt vermiyor; masaüstü oturumunda tekrar deneyin)"

cat <<'DONE'

Panora kurulumu tamamlandı.

  Popup:     panora            (GNOME'da Super+V, oturumu yeniden açtıktan sonra)
  Durum:     panora-cli status
  Test:      ./test-local.sh
  Kaldırma:  ./uninstall.sh
DONE
