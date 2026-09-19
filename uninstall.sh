#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Removes the Panora package and its user service. The encrypted history and
# the configuration are kept unless --purge-data is given.
#
#   ./uninstall.sh [--yes] [--purge-data]
set -Eeuo pipefail

declare -A TR=(
  ["Usage: uninstall.sh [--yes] [--purge-data]"]="Kullanım: uninstall.sh [--yes] [--purge-data]"
  ["  --yes         do not ask for confirmation"]="  --yes         onay sorma"
  ["  --purge-data  also delete the encrypted history and the configuration"]="  --purge-data  şifreli geçmişi ve ayarları da sil"
  ["uninstall.sh: unknown option: %s"]="uninstall.sh: bilinmeyen seçenek: %s"
  ["This removes the Panora package and stops its user service."]="Bu işlem Panora paketini kaldırır ve kullanıcı servisini durdurur."
  ["The encrypted history and the configuration will also be deleted."]="Şifreli geçmiş ve ayarlar da silinecek."
  ["Continue? [y/N] "]="Devam edilsin mi? [e/H] "
  ["Cancelled."]="İşlem iptal edildi."
  ["Panora has been removed."]="Panora programı kaldırıldı."
  ["The encrypted history and the configuration were deleted."]="Şifreli geçmiş ve ayarlar silindi."
  ["Your encrypted history was kept. To delete it too:"]="Şifreli yerel geçmişiniz korundu. Verileri de silmek isterseniz:"
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

YES=0
PURGE=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) YES=1 ;;
    --purge-data) PURGE=1 ;;
    -h|--help)
      say "Usage: uninstall.sh [--yes] [--purge-data]"
      say "  --yes         do not ask for confirmation"
      say "  --purge-data  also delete the encrypted history and the configuration"
      exit 0 ;;
    *) say "uninstall.sh: unknown option: %s" "$arg" >&2; exit 2 ;;
  esac
done

if [[ "${EUID}" -eq 0 ]]; then
  SUDO=()
else
  SUDO=(sudo)
fi

say "This removes the Panora package and stops its user service."
[[ "$PURGE" -eq 1 ]] && say "The encrypted history and the configuration will also be deleted."
if [[ "$YES" -eq 0 ]]; then
  if [[ ! -t 0 ]]; then
    say "Cancelled."
    exit 1
  fi
  read -r -p "$(t "Continue? [y/N] ")" ANSWER
  if [[ ! "$ANSWER" =~ ^[YyEe]$ ]]; then
    say "Cancelled."
    exit 0
  fi
fi

if command -v systemctl >/dev/null 2>&1 && systemctl --user >/dev/null 2>&1; then
  systemctl --user disable --now panod.service 2>/dev/null || true
fi
if command -v gnome-extensions >/dev/null 2>&1; then
  gnome-extensions disable panora@ygkali.github.io 2>/dev/null || true
fi

"${SUDO[@]}" apt-get remove -y panora

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/panora"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/panora"
echo
say "Panora has been removed."
if [[ "$PURGE" -eq 1 ]]; then
  rm -rf -- "$DATA_DIR" "$CONFIG_DIR"
  say "The encrypted history and the configuration were deleted."
else
  say "Your encrypted history was kept. To delete it too:"
  echo "  rm -rf \"$DATA_DIR\" \"$CONFIG_DIR\""
fi
