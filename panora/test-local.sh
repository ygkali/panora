#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Local smoke test for an installed Panora: daemon status, a real clipboard
# copy, list + FTS search, recall, and (unless PANORA_NO_GUI=1) the popup.
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

MARKER="Panora smoke test $(date +%s)"
COPIED=0
if [[ "${XDG_SESSION_TYPE:-}" == "wayland" ]] && command -v wl-copy >/dev/null 2>&1; then
  echo "Wayland clipboard testi (wl-copy)..."
  printf '%s\n' "$MARKER" | wl-copy
  COPIED=1
elif [[ -n "${DISPLAY:-}" ]] && command -v xclip >/dev/null 2>&1; then
  echo "X11 clipboard testi (xclip)..."
  printf '%s\n' "$MARKER" | xclip -selection clipboard -in -t text/plain
  COPIED=1
elif command -v gdbus >/dev/null 2>&1 && gdbus introspect --session --dest io.panora.GnomeShell1 --object-path /io/panora/GnomeShell1 >/dev/null 2>&1; then
  echo "GNOME Shell yardımcı servisi üzerinden clipboard testi..."
  gdbus call --session --dest io.panora.GnomeShell1 --object-path /io/panora/GnomeShell1 \
    --method io.panora.GnomeShell1.SetClipboard "text/plain" "[$(printf '%s' "$MARKER" | od -An -tu1 | tr -s ' \n' ',,' | sed 's/^,//; s/,$//')]" >/dev/null
  COPIED=1
else
  echo "Uyarı: panoya yazacak bir araç yok (wl-copy, xclip veya GNOME eklentisi)."
  echo "       Bir uygulamadan elle bir metin kopyalayıp 'panora-cli list' ile doğrulayın."
fi

sleep 1

echo
echo "Pano geçmişi:"
panora-cli list --limit 10

if [[ "$COPIED" -eq 1 ]]; then
  echo
  echo "FTS5 arama testi:"
  if panora-cli search "smoke" | grep -q "Panora smoke test"; then
    echo "  arama: OK"
  else
    echo "  arama: kopyalanan metin bulunamadı" >&2
    exit 1
  fi
  ID="$(panora-cli search "smoke" | head -n1 | grep -oE '^[* ] *[0-9]+' | tr -dc '0-9')"
  if [[ -n "$ID" ]]; then
    echo "Geri çağırma testi (id $ID):"
    panora-cli copy "$ID"
  fi
fi

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
