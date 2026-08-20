#!/usr/bin/env bash
set -Eeuo pipefail

echo "Panora lokal smoke test"
echo "========================"

if ! command -v panora-cli >/dev/null 2>&1; then
  echo "Hata: panora-cli bulunamadı. Önce ./install.sh çalıştırın." >&2
  exit 1
fi

if ! panora-cli status; then
  echo "Hata: panod daemon çalışmıyor. systemctl --user enable --now panod.service komutunu çalıştırın." >&2
  exit 1
fi

if [[ "${XDG_SESSION_TYPE:-}" == "wayland" ]] && command -v wl-copy >/dev/null 2>&1; then
  echo "Wayland clipboard testi..."
  printf 'Panora lokal Wayland smoke test\n' | wl-copy
elif [[ -n "${DISPLAY:-}" ]] && command -v xclip >/dev/null 2>&1; then
  echo "X11 clipboard testi..."
  printf 'Panora lokal X11 smoke test\n' | xclip -selection clipboard -in -t text/plain
else
  echo "Uyarı: X11 için DISPLAY+xclip veya Wayland için wl-copy bulunamadı."
  exit 2
fi

sleep 1

echo
echo "Pano geçmişi:"
panora-cli list

echo
echo "FTS5 arama testi:"
panora-cli search Panora || true

if [[ "${PANORA_NO_GUI:-0}" == "1" ]]; then
  echo
  echo "PANORA_NO_GUI=1 olduğu için GUI açılmadan smoke test tamamlandı."
  exit 0
fi

echo
echo "GUI başlatılıyor. Kapatmak için Esc veya pencere kapatma düğmesini kullanın."
if command -v panora >/dev/null 2>&1; then
  panora
else
  panora-gui
fi
