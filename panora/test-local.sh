#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Local smoke test for an installed Panora: panora-doctor diagnostics first,
# then daemon status, a real clipboard copy, list + FTS search, recall, and
# (unless PANORA_NO_GUI=1) the popup. PANORA_E2E=1 also runs the full
# scripts/e2e-test.sh in its safe mode.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

echo "Panora lokal smoke test"
echo "========================"

# Diagnostics first: the doctor explains a broken install far better than a
# failing copy below. Packaged as /usr/bin/panora-doctor; the source tree
# copy covers runs before the package is installed.
DOCTOR=""
if command -v panora-doctor >/dev/null 2>&1; then
  DOCTOR="$(command -v panora-doctor)"
elif [[ -x "$ROOT_DIR/scripts/panora-doctor" ]]; then
  DOCTOR="$ROOT_DIR/scripts/panora-doctor"
fi
if [[ -n "$DOCTOR" ]]; then
  echo
  echo "[1/3] Tanı (panora-doctor)"
  echo "--------------------------"
  if ! "$DOCTOR"; then
    echo
    echo "Hata: panora-doctor daemon'un kullanılamaz olduğunu bildirdi; yukarıdaki HATA satırlarını düzeltip tekrar deneyin." >&2
    exit 1
  fi
  echo
fi

echo "[2/3] Hızlı pano testi"
echo "----------------------"

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

echo
echo "[3/3] Uçtan uca test (isteğe bağlı)"
echo "-----------------------------------"
echo "Tüm özellikleri (HTML/PNG/dosya/renk/bağlantı yakalama, sabitleme, geri"
echo "çağırma, tekilleştirme, özel mod, clear, config reload, popup) sınamak için:"
echo "  $ROOT_DIR/scripts/e2e-test.sh            # sorar: clear ve max_entries geçmişi siler"
echo "  $ROOT_DIR/scripts/e2e-test.sh --safe     # geçmişi silen adımları atla"
echo "  $ROOT_DIR/scripts/e2e-test.sh --install-helpers   # xclip / wl-clipboard eksikse kur"
if [[ "${PANORA_E2E:-0}" == "1" ]]; then
  echo
  "$ROOT_DIR/scripts/e2e-test.sh" --safe --no-gui
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
