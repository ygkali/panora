#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DEB_FILE="$(find "$ROOT_DIR/dist" -maxdepth 1 -type f -name 'panora_*.deb' -print -quit 2>/dev/null || true)"

if [[ ! -f /etc/debian_version ]]; then
  echo "Hata: Bu otomatik kurulum Debian tabanlı sistemler içindir." >&2
  exit 1
fi

if [[ -z "$DEB_FILE" || ! -f "$DEB_FILE" ]]; then
  echo "Hata: dist/ içinde panora_*.deb bulunamadı." >&2
  exit 1
fi

if [[ "${EUID}" -eq 0 ]]; then
  SUDO=()
else
  SUDO=(sudo)
fi

echo "[1/4] Sistem bağımlılıkları kuruluyor..."
"${SUDO[@]}" apt-get update
"${SUDO[@]}" apt-get install -y \
  libgtk-4-1 libadwaita-1-0 libsqlite3-0 \
  xclip wl-clipboard gnome-keyring

echo "[2/4] Panora Debian paketi kuruluyor: $DEB_FILE"
if ! "${SUDO[@]}" dpkg -i "$DEB_FILE"; then
  echo "Paket bağımlılıkları tamamlanıyor..."
  "${SUDO[@]}" apt-get -f install -y
  "${SUDO[@]}" dpkg -i "$DEB_FILE"
fi

echo "[3/4] Kullanıcı daemon servisi etkinleştiriliyor..."
if command -v systemctl >/dev/null 2>&1 && systemctl --user >/dev/null 2>&1; then
  systemctl --user daemon-reload
  systemctl --user enable --now panod.service || \
    echo "Uyarı: panod.service otomatik başlatılamadı; README içindeki manuel komutu kullanın."
else
  echo "Uyarı: systemd --user bulunamadı; panod'u elle başlatmanız gerekir."
fi

echo "[4/4] Kurulum doğrulanıyor..."
if command -v panora-cli >/dev/null 2>&1; then
  panora-cli status || true
fi

echo
echo "Panora kurulumu tamamlandı."
echo "GUI:       panora"
echo "Durum:     panora-cli status"
echo "Test:      ./test-local.sh"
echo "Kaldırma:  ./uninstall.sh"
