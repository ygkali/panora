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

declare -A TR=(
  ["Panora local smoke test"]="Panora lokal smoke test"
  ["[1/3] Diagnostics (panora-doctor)"]="[1/3] Tanı (panora-doctor)"
  ["Error: panora-doctor reports that the daemon is unusable; fix the ERROR lines above and retry."]="Hata: panora-doctor daemon'un kullanılamaz olduğunu bildirdi; yukarıdaki ERROR satırlarını düzeltip tekrar deneyin."
  ["[2/3] Quick clipboard test"]="[2/3] Hızlı pano testi"
  ["Error: panora-cli not found. Run ./install.sh first."]="Hata: panora-cli bulunamadı. Önce ./install.sh çalıştırın."
  ["Error: the panod daemon is not running. Run: systemctl --user enable --now panod.service"]="Hata: panod daemon çalışmıyor. Çalıştırın: systemctl --user enable --now panod.service"
  ["Wayland clipboard test (wl-copy)..."]="Wayland pano testi (wl-copy)..."
  ["X11 clipboard test (xclip)..."]="X11 pano testi (xclip)..."
  ["Clipboard test through the GNOME Shell helper service..."]="GNOME Shell yardımcı servisi üzerinden pano testi..."
  ["Warning: no tool can write the clipboard (wl-copy, xclip or the GNOME extension)."]="Uyarı: panoya yazacak bir araç yok (wl-copy, xclip veya GNOME eklentisi)."
  ["         Copy some text from any application and check it with 'panora-cli list'."]="         Bir uygulamadan elle bir metin kopyalayıp 'panora-cli list' ile doğrulayın."
  ["Clipboard history:"]="Pano geçmişi:"
  ["FTS5 search test:"]="FTS5 arama testi:"
  ["  search: OK"]="  arama: OK"
  ["  search: the copied text was not found"]="  arama: kopyalanan metin bulunamadı"
  ["Recall test (id %s):"]="Geri çağırma testi (id %s):"
  ["[3/3] End-to-end test (optional)"]="[3/3] Uçtan uca test (isteğe bağlı)"
  ["To exercise every feature (HTML/PNG/file/colour/link capture, pinning, recall,"]="Tüm özellikleri (HTML/PNG/dosya/renk/bağlantı yakalama, sabitleme, geri"
  ["deduplication, private mode, clear, config reload, popup):"]="çağırma, tekilleştirme, özel mod, clear, config reload, popup) sınamak için:"
  ["  %s            # asks first: clear and max_entries delete history"]="  %s            # sorar: clear ve max_entries geçmişi siler"
  ["  %s --safe     # skip the steps that delete history"]="  %s --safe     # geçmişi silen adımları atla"
  ["  %s --install-helpers   # install xclip / wl-clipboard when missing"]="  %s --install-helpers   # xclip / wl-clipboard eksikse kur"
  ["PANORA_NO_GUI=1 is set; the smoke test finished without opening the popup."]="PANORA_NO_GUI=1 olduğu için GUI açılmadan smoke test tamamlandı."
  ["Opening the popup. Close it with Esc or the window close button."]="GUI başlatılıyor. Kapatmak için Esc veya pencere kapatma düğmesini kullanın."
)
ui_lang() {
  local l="${PANORA_LANG:-${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}}"
  case "${l,,}" in tr*) echo tr ;; *) echo en ;; esac
}
UI_LANG="$(ui_lang)"
t() {
  local fmt="$1"
  shift
  if [[ "$UI_LANG" == tr && -n "${TR[$fmt]+x}" ]]; then
    fmt="${TR[$fmt]}"
  fi
  # shellcheck disable=SC2059
  printf -- "$fmt" "$@"
}
say() { t "$@"; printf '\n'; }
warn() { say "$@" >&2; }

say "Panora local smoke test"
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
  say "[1/3] Diagnostics (panora-doctor)"
  echo "--------------------------"
  if ! bash "$DOCTOR"; then
    echo
    warn "Error: panora-doctor reports that the daemon is unusable; fix the ERROR lines above and retry."
    exit 1
  fi
  echo
fi

say "[2/3] Quick clipboard test"
echo "----------------------"

if ! command -v panora-cli >/dev/null 2>&1; then
  warn "Error: panora-cli not found. Run ./install.sh first."
  exit 1
fi

if ! panora-cli status; then
  warn "Error: the panod daemon is not running. Run: systemctl --user enable --now panod.service"
  exit 1
fi

HELPER=io.github.ygkali.Panora.GnomeShell1
HELPER_PATH=/io/github/ygkali/Panora/GnomeShell1
MARKER="Panora smoke test $(date +%s)"
COPIED=0
if [[ "${XDG_SESSION_TYPE:-}" == "wayland" ]] && command -v wl-copy >/dev/null 2>&1; then
  say "Wayland clipboard test (wl-copy)..."
  printf '%s\n' "$MARKER" | wl-copy
  COPIED=1
elif [[ -n "${DISPLAY:-}" ]] && command -v xclip >/dev/null 2>&1; then
  say "X11 clipboard test (xclip)..."
  printf '%s\n' "$MARKER" | xclip -selection clipboard -in -t text/plain
  COPIED=1
elif command -v gdbus >/dev/null 2>&1 && gdbus introspect --session --dest "$HELPER" --object-path "$HELPER_PATH" >/dev/null 2>&1; then
  say "Clipboard test through the GNOME Shell helper service..."
  gdbus call --session --dest "$HELPER" --object-path "$HELPER_PATH" \
    --method "$HELPER.SetClipboard" "text/plain" "[$(printf '%s' "$MARKER" | od -An -tu1 | tr -s ' \n' ',,' | sed 's/^,//; s/,$//')]" >/dev/null
  COPIED=1
else
  say "Warning: no tool can write the clipboard (wl-copy, xclip or the GNOME extension)."
  say "         Copy some text from any application and check it with 'panora-cli list'."
fi

sleep 1

echo
say "Clipboard history:"
panora-cli list --limit 10

if [[ "$COPIED" -eq 1 ]]; then
  echo
  say "FTS5 search test:"
  if panora-cli search "smoke" | grep -q "Panora smoke test"; then
    say "  search: OK"
  else
    warn "  search: the copied text was not found"
    exit 1
  fi
  ID="$(panora-cli search "smoke" | head -n1 | grep -oE '^[* ] *[0-9]+' | tr -dc '0-9')"
  if [[ -n "$ID" ]]; then
    say "Recall test (id %s):" "$ID"
    panora-cli copy "$ID"
  fi
fi

echo
say "[3/3] End-to-end test (optional)"
echo "-----------------------------------"
say "To exercise every feature (HTML/PNG/file/colour/link capture, pinning, recall,"
say "deduplication, private mode, clear, config reload, popup):"
say "  %s            # asks first: clear and max_entries delete history" "$ROOT_DIR/scripts/e2e-test.sh"
say "  %s --safe     # skip the steps that delete history" "$ROOT_DIR/scripts/e2e-test.sh"
say "  %s --install-helpers   # install xclip / wl-clipboard when missing" "$ROOT_DIR/scripts/e2e-test.sh"
if [[ "${PANORA_E2E:-0}" == "1" ]]; then
  echo
  bash "$ROOT_DIR/scripts/e2e-test.sh" --safe --no-gui
fi

if [[ "${PANORA_NO_GUI:-0}" == "1" ]]; then
  echo
  say "PANORA_NO_GUI=1 is set; the smoke test finished without opening the popup."
  exit 0
fi

echo
say "Opening the popup. Close it with Esc or the window close button."
if command -v panora >/dev/null 2>&1; then
  panora
else
  panora-gui
fi
