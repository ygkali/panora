#!/usr/bin/env bash
set -Eeuo pipefail

if [[ "${EUID}" -eq 0 ]]; then
  SUDO=()
else
  SUDO=(sudo)
fi

echo "Panora kaldırma işlemi başlayacak."
read -r -p "Panora paketini kaldırmak istediğinize emin misiniz? [y/N] " ANSWER
if [[ ! "$ANSWER" =~ ^[YyEe]$ ]]; then
  echo "İşlem iptal edildi."
  exit 0
fi

if command -v systemctl >/dev/null 2>&1 && systemctl --user >/dev/null 2>&1; then
  systemctl --user disable --now panod.service 2>/dev/null || true
fi

"${SUDO[@]}" apt-get remove -y panora

echo
echo "Panora programı kaldırıldı."
echo "Şifreli yerel geçmişiniz varsayılan olarak korunmuştur."
echo "Verileri de silmek istiyorsanız aşağıdaki yolu elle kaldırın:"
echo "  ~/.local/share/panora"
echo "  ~/.config/panora"
