#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# End-to-end functional test for an installed Panora, run on the real desktop
# after ./install.sh. Every feature prints PASS / FAIL / SKIP and the script
# exits non-zero when anything FAILs.
#
# The test writes real clipboard content with whatever the session offers
# (xclip on X11 and, through XWayland, on the GNOME bridge backend; wl-copy
# on native Wayland), waits for panod to record it by polling
# `panora-cli list` (never a fixed sleep), and cleans up after itself: test
# entries are deleted, private mode and config.toml are restored.
#
# Two steps are destructive by nature and only run after confirmation (or
# with --yes): `clear` removes every unpinned entry, and the config-reload
# step temporarily sets max_entries = 3, which prunes unpinned history to
# three entries. --safe skips both.
set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Kullanım: scripts/e2e-test.sh [seçenekler]

  --install-helpers   xclip ve wl-clipboard eksikse apt ile kur (sudo ister)
  --safe              geçmişi silen adımları atla (clear, max_entries)
  --yes               onay sormadan devam et
  --no-gui            popup aç/kapa testini atla
  --keep              test kayıtlarını sonunda silme (inceleme için)
  -h, --help          bu yardım

Ortam: PANORA_E2E_TIMEOUT (varsayılan 3) bir kaydın görünmesi için
beklenecek saniye.
USAGE
}

INSTALL_HELPERS=0
SAFE=0
YES=0
NO_GUI=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --install-helpers) INSTALL_HELPERS=1 ;;
    --safe) SAFE=1 ;;
    --yes|-y) YES=1 ;;
    --no-gui) NO_GUI=1 ;;
    --keep) KEEP=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "e2e-test: bilinmeyen seçenek: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

WAIT_SECS="${PANORA_E2E_TIMEOUT:-3}"
RUN_ID="$(date +%s)$RANDOM"
TMP="$(mktemp -d -t panora-e2e.XXXXXX)"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/panora/config.toml"
EXT_UUID="panora@panora-clipboard.org"

PASS_N=0
FAIL_N=0
SKIP_N=0
CREATED_IDS=()
CONFIG_BACKUP=""
CONFIG_EXISTED=0
CONFIG_TOUCHED=0
PRIVATE_BEFORE=""
HELPER_PIDS=()

has() { command -v "$1" >/dev/null 2>&1; }

# Bounded wrapper so a hung daemon never hangs the test.
cli() {
  if has timeout; then
    timeout 10 panora-cli "$@"
  else
    panora-cli "$@"
  fi
}

pass() { PASS_N=$((PASS_N + 1)); printf 'PASS  %s\n' "$1"; }
fail() { FAIL_N=$((FAIL_N + 1)); printf 'FAIL  %s -- %s\n' "$1" "${2:-}"; }
skip() { SKIP_N=$((SKIP_N + 1)); printf 'SKIP  %s -- %s\n' "$1" "${2:-}"; }
note() { printf '      %s\n' "$1"; }

# ------------------------------------------------------------------ cleanup

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  for pid in "${HELPER_PIDS[@]:-}"; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null
  done
  if [[ "$CONFIG_TOUCHED" -eq 1 ]]; then
    if [[ "$CONFIG_EXISTED" -eq 1 && -f "$CONFIG_BACKUP" ]]; then
      cp -p -- "$CONFIG_BACKUP" "$CONFIG"
    else
      rm -f -- "$CONFIG"
    fi
    cli reload >/dev/null 2>&1
    note "config.toml geri yüklendi."
  fi
  if [[ -n "$PRIVATE_BEFORE" ]]; then
    if [[ "$PRIVATE_BEFORE" == "true" ]]; then
      cli private on >/dev/null 2>&1
    else
      cli private off >/dev/null 2>&1
    fi
  fi
  if [[ "$KEEP" -eq 0 && ${#CREATED_IDS[@]} -gt 0 ]]; then
    local id
    for id in "${CREATED_IDS[@]}"; do
      cli unpin "$id" >/dev/null 2>&1
      cli delete "$id" >/dev/null 2>&1
    done
    note "Test kayıtları silindi (${#CREATED_IDS[@]} adet)."
  fi
  rm -rf -- "$TMP"
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ----------------------------------------------------------- list parsing

# `panora-cli list` prints: {pin} {id:>5} [{kind}] {preview}
# where pin is "*" or " ". Newlines in previews are shown as " ⏎ ".
LIST_RE='^([* ]) +([0-9]+) \[([a-z]+)\] (.*)$'

list_all() { cli list --limit 200 2>/dev/null || true; }

# ids of every listed entry, one per line.
list_ids() {
  local line
  while IFS= read -r line; do
    [[ "$line" =~ $LIST_RE ]] && printf '%s\n' "${BASH_REMATCH[2]}"
  done < <(list_all)
}

# Snapshot of ids taken before a capture, so a new entry can be told apart
# from an old one that merely moved.
BEFORE_IDS=""
snapshot() { BEFORE_IDS="$(list_ids | sort -n)"; }

is_new_id() {
  [[ -n "$1" ]] || return 1
  ! grep -qx -- "$1" <<<"$BEFORE_IDS"
}

# wait_new REGEX [SECONDS] -> prints the id of the first (newest) unpinned
# entry whose "[kind] preview" text matches REGEX and that did not exist at
# the last snapshot. Polls `panora-cli list` every 200 ms.
wait_new() {
  local re="$1" secs="${2:-$WAIT_SECS}" deadline line id rest
  deadline=$(( $(date +%s%N) / 1000000 + secs * 1000 ))
  while :; do
    while IFS= read -r line; do
      [[ "$line" =~ $LIST_RE ]] || continue
      id="${BASH_REMATCH[2]}"
      rest="[${BASH_REMATCH[3]}] ${BASH_REMATCH[4]}"
      if [[ "$rest" =~ $re ]] && is_new_id "$id"; then
        printf '%s\n' "$id"
        return 0
      fi
    done < <(list_all)
    (( $(date +%s%N) / 1000000 >= deadline )) && return 1
    sleep 0.2
  done
}

# wait_until CMD... -> polls CMD every 200 ms until it succeeds or WAIT_SECS pass.
wait_until() {
  local deadline
  deadline=$(( $(date +%s%N) / 1000000 + WAIT_SECS * 1000 ))
  while ! "$@"; do
    (( $(date +%s%N) / 1000000 >= deadline )) && return 1
    sleep 0.2
  done
}

# Full line for one id, empty when absent.
entry_line() {
  list_all | grep -E "^[* ] +$1 \[" || true
}

count_matching() { list_all | grep -cE "$1" || true; }

# --------------------------------------------------------- clipboard write

BACKEND=""
WRITER=""        # xclip | wl-copy
READER=""        # xclip | wl-paste | ""
GTK_HELPER=0     # python3 + GTK multi-target offer available

# Note on the GNOME bridge backend (GNOME <= 47 on Wayland): the Shell
# extension's io.panora.GnomeShell1.SetClipboard is deliberately NOT used as
# a writer. It exists for recalls, and the extension mutes its own
# owner-changed echo for 1.5 s after the call, so nothing would ever reach
# panod. X11 clients through XWayland (xclip) are bridged into Mutter's
# selection and captured like any other app; wl-copy falls back to a
# transient focus surface on compositors without data-control.

# set_clip MIME FILE -> put one payload on the CLIPBOARD selection.
set_clip() {
  local mime="$1" file="$2"
  case "$WRITER" in
    xclip) xclip -selection clipboard -t "$mime" -i "$file" ;;
    wl-copy) wl-copy --type "$mime" < "$file" ;;
    *) return 1 ;;
  esac
}

set_clip_text() {
  printf '%s' "$1" > "$TMP/text.bin"
  set_clip "text/plain" "$TMP/text.bin"
}

# Multi-target offer through GTK (python3-gi). Needed where a single tool
# cannot advertise two formats at once (xclip/wl-copy offer exactly one).
# GTK4's GdkContentProvider union is introspectable; GTK3's set_with_data
# is tried second because some PyGObject builds do not expose it. Runs in
# the background and keeps the selection for HOLD seconds. Whether it
# actually reached panod is verified by the capture that follows, so a
# compositor that ignores an unfocused client's selection degrades to SKIP.
GTK_HELPER_PY="$TMP/offer_targets.py"
cat > "$GTK_HELPER_PY" <<'PY'
import sys
import gi
from gi.repository import GLib

hold = float(sys.argv[1])
pairs = [a.split('=', 1) for a in sys.argv[2:]]
data = {}
for mime, path in pairs:
    with open(path, 'rb') as fh:
        data[mime] = fh.read()
mimes = list(data)


def with_gtk4():
    gi.require_version('Gtk', '4.0')
    gi.require_version('Gdk', '4.0')
    from gi.repository import Gtk, Gdk
    if not Gtk.init_check():
        raise RuntimeError('no display')
    clipboard = Gdk.Display.get_default().get_clipboard()
    providers = [Gdk.ContentProvider.new_for_bytes(m, GLib.Bytes.new(data[m])) for m in mimes]
    if not clipboard.set_content(Gdk.ContentProvider.new_union(providers)):
        raise RuntimeError('set_content refused')
    loop = GLib.MainLoop()
    GLib.timeout_add(int(hold * 1000), loop.quit)
    loop.run()


def with_gtk3():
    gi.require_version('Gtk', '3.0')
    gi.require_version('Gdk', '3.0')
    from gi.repository import Gtk, Gdk

    def get_func(_clipboard, selection, info, _user):
        mime = mimes[info]
        selection.set(Gdk.Atom.intern(mime, False), 8, data[mime])

    def clear_func(_clipboard, _user):
        pass

    clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
    targets = [Gtk.TargetEntry.new(m, 0, i) for i, m in enumerate(mimes)]
    if not clipboard.set_with_data(targets, get_func, clear_func, None):
        raise RuntimeError('set_with_data refused')
    GLib.timeout_add(int(hold * 1000), Gtk.main_quit)
    Gtk.main()


for attempt in (with_gtk4, with_gtk3):
    try:
        attempt()
        sys.exit(0)
    except SystemExit:
        raise
    except Exception as error:  # any failure means "try the next toolkit"
        sys.stderr.write('%s: %s\n' % (attempt.__name__, error))
sys.exit(3)
PY

# offer_targets HOLD mime=file [mime=file...]
offer_targets() {
  python3 "$GTK_HELPER_PY" "$@" >/dev/null 2>&1 &
  HELPER_PIDS+=("$!")
}

# -------------------------------------------------------- clipboard read

read_clip_types() {
  case "$READER" in
    wl-paste) wl-paste --list-types 2>/dev/null ;;
    xclip) xclip -selection clipboard -o -t TARGETS 2>/dev/null ;;
    *) return 1 ;;
  esac
}

read_clip_text() {
  case "$READER" in
    wl-paste) wl-paste --no-newline 2>/dev/null ;;
    xclip) xclip -selection clipboard -o 2>/dev/null ;;
    *) return 1 ;;
  esac
}

types_have_text() { grep -qiE '^(text/plain|UTF8_STRING|STRING|TEXT)' <<<"$1"; }
types_have_html() { grep -qi '^text/html' <<<"$1"; }

clip_text_is() { [[ "$(read_clip_text || true)" == "$1" ]]; }

# --------------------------------------------------------------- D-Bus

name_has_owner() {
  if has gdbus; then
    gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner "$1" 2>/dev/null | grep -q true
  elif has busctl; then
    busctl --user list --acquired --no-legend 2>/dev/null | awk '{print $1}' | grep -qx -- "$1"
  else
    return 2
  fi
}
name_absent() { ! name_has_owner "$1"; }

# ================================================================== start

echo "Panora uçtan uca test  (çalıştırma kimliği $RUN_ID)"
echo "============================================================"

if ! has panora-cli; then
  echo "HATA: panora-cli bulunamadı. Önce ./install.sh çalıştırın." >&2
  exit 1
fi

# 1. status --------------------------------------------------------------
if STATUS="$(cli status 2>&1)"; then
  BACKEND="$(sed -n 's/.*backend=\([^ ]*\).*/\1/p' <<<"$STATUS")"
  PRIVATE_BEFORE="$(sed -n 's/.*private=\([^ ]*\).*/\1/p' <<<"$STATUS")"
  pass "status ($STATUS)"
else
  fail "status" "$STATUS"
  echo "Daemon yanıt vermiyor; devam edilemez. Önce panora-doctor çalıştırın." >&2
  exit 1
fi

if [[ "$PRIVATE_BEFORE" == "true" ]]; then
  note "Özel mod açıktı; test süresince kapatılıyor, sonunda geri açılacak."
  cli private off >/dev/null
fi

# 2. tools ---------------------------------------------------------------
SESSION="${XDG_SESSION_TYPE:-}"
IS_GNOME=0
if [[ "${XDG_CURRENT_DESKTOP:-}" == *[Gg][Nn][Oo][Mm][Ee]* ]] || name_has_owner org.gnome.Shell 2>/dev/null; then
  IS_GNOME=1
fi

if [[ "$INSTALL_HELPERS" -eq 1 ]]; then
  PKGS=()
  has xclip || PKGS+=(xclip)
  has wl-copy || PKGS+=(wl-clipboard)
  if [[ ${#PKGS[@]} -gt 0 ]]; then
    echo "Yardımcı araçlar kuruluyor: ${PKGS[*]}"
    sudo apt-get install -y "${PKGS[@]}"
  fi
fi

# Writer choice per backend: what actually reaches panod on this session.
case "$BACKEND" in
  gnome-bridge)
    if [[ -n "${DISPLAY:-}" ]] && has xclip; then WRITER=xclip
    elif has wl-copy; then WRITER=wl-copy
    fi ;;
  wayland)
    if has wl-copy; then WRITER=wl-copy
    elif [[ -n "${DISPLAY:-}" ]] && has xclip; then WRITER=xclip
    fi ;;
  *)
    if [[ -n "${DISPLAY:-}" ]] && has xclip; then WRITER=xclip
    elif has wl-copy; then WRITER=wl-copy
    fi ;;
esac

# Reader: wl-paste only on a native Wayland backend (needs data-control);
# xclip reads through XWayland on every GNOME session.
if [[ "$BACKEND" == "wayland" ]] && has wl-paste; then
  READER=wl-paste
elif [[ -n "${DISPLAY:-}" ]] && has xclip; then
  READER=xclip
elif [[ -n "${WAYLAND_DISPLAY:-}" ]] && has wl-paste && wl-paste --list-types >/dev/null 2>&1; then
  READER=wl-paste
fi

if [[ -z "$WRITER" ]]; then
  fail "pano yazma aracı" "xclip / wl-copy yok (GNOME köprüsünde eklentinin SetClipboard'ı yakalanmaz). --install-helpers ile kurun."
  echo "Pano yazılamadığı için kalan testler çalıştırılamaz." >&2
  exit 1
fi
note "backend=$BACKEND oturum=${SESSION:-?} yazıcı=$WRITER okuyucu=${READER:-yok}"

# GTK helper availability (python3-gi + gir1.2-gtk-3.0); verified live below.
if has python3 && python3 - >/dev/null 2>&1 <<'PY'
import gi
for version in ('4.0', '3.0'):
    try:
        gi.require_version('Gtk', version)
        from gi.repository import Gtk  # noqa: F401
        raise SystemExit(0)
    except (ValueError, ImportError):
        continue
raise SystemExit(1)
PY
then
  GTK_HELPER=1
fi

# Destructive steps need consent.
if [[ "$SAFE" -eq 0 && "$YES" -eq 0 ]]; then
  echo
  echo "UYARI: 'clear' ve 'max_entries' adımları sabitlenmemiş TÜM pano geçmişini siler."
  echo "       Atlamak için --safe, sormadan devam için --yes kullanın."
  if [[ -t 0 ]]; then
    read -r -p "Geçmiş silinerek devam edilsin mi? [e/H] " ANSWER
    [[ "$ANSWER" =~ ^[EeYy]$ ]] || SAFE=1
  else
    echo "       Etkileşimli değil: yıkıcı adımlar atlanıyor (--yes ile açın)."
    SAFE=1
  fi
fi
echo

# 3. text capture --------------------------------------------------------
MARKER="panora-e2e-$RUN_ID zqx$RUN_ID"
snapshot
set_clip_text "$MARKER"
if TEXT_ID="$(wait_new "^\[text\] .*zqx$RUN_ID")"; then
  CREATED_IDS+=("$TEXT_ID")
  pass "metin yakalama (id $TEXT_ID)"
else
  fail "metin yakalama" "$WAIT_SECS sn içinde listede görünmedi (backend=$BACKEND, yazıcı=$WRITER)"
  echo "Temel yakalama çalışmadığı için kalan testler anlamsız; panora-doctor çalıştırın." >&2
  exit 1
fi

# 4. FTS prefix search ---------------------------------------------------
if cli search "zqx" 2>/dev/null | grep -qE "^[* ] +$TEXT_ID \["; then
  pass "FTS önek araması (zqx -> id $TEXT_ID)"
else
  fail "FTS önek araması" "'panora-cli search zqx' kaydı döndürmedi"
fi

# 5. HTML capture --------------------------------------------------------
HTML_TEXT="panora-e2e-html-$RUN_ID"
printf '<b>%s</b>' "$HTML_TEXT" > "$TMP/html.bin"
printf '%s' "$HTML_TEXT" > "$TMP/html-plain.bin"
HTML_MULTI=0
HTML_ID=""
snapshot
if [[ "$GTK_HELPER" -eq 1 ]]; then
  offer_targets 4 "text/html=$TMP/html.bin" "text/plain=$TMP/html-plain.bin"
  if HTML_ID="$(wait_new "^\[richtext\] .*$HTML_TEXT")"; then
    HTML_MULTI=1
  else
    note "GTK çok biçimli yardımcı bu oturumda çalışmadı; tek biçimli araca dönülüyor."
    GTK_HELPER=0
  fi
fi
if [[ -z "$HTML_ID" ]]; then
  snapshot
  set_clip "text/html" "$TMP/html.bin"
  HTML_ID="$(wait_new "^\[richtext\] .*$HTML_TEXT" || true)"
fi
if [[ -n "$HTML_ID" ]]; then
  CREATED_IDS+=("$HTML_ID")
  pass "HTML yakalama -> richtext (id $HTML_ID)"
  PREVIEW="$(cli preview "$HTML_ID" 2>/dev/null || true)"
  if grep -q '^--- text/html' <<<"$PREVIEW"; then
    if grep -qE '^--- (text/plain|UTF8_STRING|STRING|TEXT)' <<<"$PREVIEW"; then
      pass "richtext önizleme text/html + düz metin sunuyor"
    elif [[ "$HTML_MULTI" -eq 0 ]]; then
      skip "richtext önizleme düz metin" "$WRITER tek biçim sunar; iki biçim için python3-gi (GTK) gerekir"
    else
      fail "richtext önizleme düz metin" "text/plain payload'ı kaydedilmedi"
    fi
  else
    fail "richtext önizleme" "text/html payload'ı yok: $(head -n3 <<<"$PREVIEW")"
  fi
else
  fail "HTML yakalama" "richtext kaydı görünmedi"
fi

# 6. PNG capture + byte-identical round trip -----------------------------
# Known 1x1 PNG without its IEND chunk, plus a tEXt chunk carrying the run
# id (so each run stores unique bytes), then IEND. CRC32 comes from gzip's
# trailer, which avoids a CRC table in bash.
PNG_BASE='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=='
PNG="$TMP/e2e.png"
PNG_ID=""
if has base64 && has gzip; then
  # write_bytes B1 B2 B3 B4 -> the four decimal byte values, raw, on stdout.
  write_bytes() {
    printf "\\x$(printf %02x "$1")\\x$(printf %02x "$2")\\x$(printf %02x "$3")\\x$(printf %02x "$4")"
  }
  base64 -d <<<"$PNG_BASE" | head -c -12 > "$PNG"
  printf 'tEXtpanora\0%s' "$RUN_ID" > "$TMP/chunk.bin"
  CHUNK_LEN=$(( $(stat -c %s "$TMP/chunk.bin") - 4 ))
  write_bytes $((CHUNK_LEN >> 24 & 255)) $((CHUNK_LEN >> 16 & 255)) $((CHUNK_LEN >> 8 & 255)) $((CHUNK_LEN & 255)) >> "$PNG"
  cat "$TMP/chunk.bin" >> "$PNG"
  # gzip trailer: CRC32 (little-endian) then ISIZE; PNG wants big-endian.
  # (gawk's %c would emit UTF-8 for values >= 128, so bytes go through printf.)
  read -r b1 b2 b3 b4 < <(gzip -c -n "$TMP/chunk.bin" | tail -c 8 | head -c 4 | od -An -v -tu1)
  write_bytes "$b4" "$b3" "$b2" "$b1" >> "$PNG"
  printf '\x00\x00\x00\x00IEND\xaeB`\x82' >> "$PNG"
  snapshot
  set_clip "image/png" "$PNG"
  if PNG_ID="$(wait_new '^\[image\]')"; then
    CREATED_IDS+=("$PNG_ID")
    pass "PNG yakalama -> image (id $PNG_ID)"
    if cli preview "$PNG_ID" --mime image/png --out "$TMP/back.png" >/dev/null 2>&1 && cmp -s "$PNG" "$TMP/back.png"; then
      pass "PNG önizleme dışa aktarma bayt bayt aynı ($(stat -c %s "$PNG") bayt)"
    else
      fail "PNG önizleme dışa aktarma" "preview --mime image/png --out çıktısı özgün dosyayla aynı değil"
    fi
  else
    fail "PNG yakalama" "image kaydı görünmedi"
  fi
else
  skip "PNG yakalama" "base64/gzip yok"
fi

# 7. file URI list -> files ---------------------------------------------
FILE_URI="file:///tmp/panora-e2e-$RUN_ID.txt"
printf '%s\r\n' "$FILE_URI" > "$TMP/uri.bin"
printf 'copy\n%s' "$FILE_URI" > "$TMP/gnome-files.bin"
FILES_ID=""
snapshot
if [[ "$GTK_HELPER" -eq 1 ]]; then
  offer_targets 4 "x-special/gnome-copied-files=$TMP/gnome-files.bin" "text/uri-list=$TMP/uri.bin"
  FILES_ID="$(wait_new '^\[files\]' || true)"
fi
if [[ -z "$FILES_ID" && "$BACKEND" != "gnome-bridge" ]]; then
  snapshot
  set_clip "x-special/gnome-copied-files" "$TMP/gnome-files.bin"
  FILES_ID="$(wait_new '^\[files\]' || true)"
fi
if [[ -n "$FILES_ID" ]]; then
  CREATED_IDS+=("$FILES_ID")
  pass "dosya listesi yakalama -> files (id $FILES_ID)"
elif [[ "$BACKEND" == "gnome-bridge" ]]; then
  skip "dosya listesi yakalama" "GNOME köprüsünde eklenti yalnızca text/uri-list iletir; dosya yöneticisinden elle kopyalayıp 'files' türünü doğrulayın"
else
  fail "dosya listesi yakalama" "files kaydı görünmedi"
fi

# 8. color ---------------------------------------------------------------
COLOR="$(printf '#ff88%02x' $((RANDOM % 256)))"
snapshot
set_clip_text "$COLOR"
if COLOR_ID="$(wait_new "^\[color\] $COLOR")"; then
  CREATED_IDS+=("$COLOR_ID")
  pass "renk yakalama $COLOR -> color (id $COLOR_ID)"
else
  fail "renk yakalama" "$COLOR için color kaydı görünmedi"
fi

# 9. link ----------------------------------------------------------------
LINK="https://example.org/panora-e2e-$RUN_ID"
snapshot
set_clip_text "$LINK"
if LINK_ID="$(wait_new "^\[link\] $LINK")"; then
  CREATED_IDS+=("$LINK_ID")
  pass "bağlantı yakalama -> link (id $LINK_ID)"
else
  fail "bağlantı yakalama" "link kaydı görünmedi"
  LINK_ID=""
fi

# 10. pin / unpin --------------------------------------------------------
if cli pin "$TEXT_ID" >/dev/null 2>&1 && cli list --pinned 2>/dev/null | grep -qE "^\* +$TEXT_ID \["; then
  pass "pin -> list --pinned"
else
  fail "pin" "id $TEXT_ID 'list --pinned' çıktısında '*' ile görünmedi"
fi
if cli unpin "$TEXT_ID" >/dev/null 2>&1 && ! cli list --pinned 2>/dev/null | grep -qE "^[* ] +$TEXT_ID \["; then
  pass "unpin -> list --pinned dışında"
else
  fail "unpin" "id $TEXT_ID hâlâ sabitli görünüyor"
fi

# 11. recall -------------------------------------------------------------
if cli copy "$TEXT_ID" >/dev/null 2>&1; then
  if [[ -n "$READER" ]]; then
    if wait_until clip_text_is "$MARKER"; then
      pass "geri çağırma (copy $TEXT_ID) panoya yazdı ($READER ile doğrulandı)"
    else
      fail "geri çağırma" "$READER panoda beklenen metni okumadı: '$(read_clip_text | head -c 60 || true)'"
    fi
  else
    top_is_text() { [[ "$(list_all | grep -E '^  +[0-9]+ \[' | head -n1)" =~ $LIST_RE && "${BASH_REMATCH[2]}" == "$TEXT_ID" ]]; }
    if wait_until top_is_text; then
      pass "geri çağırma (copy $TEXT_ID) kaydı listenin başına taşıdı (okuyucu yok)"
    else
      fail "geri çağırma" "kayıt listenin başına gelmedi"
    fi
  fi
else
  fail "geri çağırma" "panora-cli copy $TEXT_ID başarısız"
fi

# 12. recall of HTML entry offers both formats ---------------------------
if [[ -z "$HTML_ID" ]]; then
  skip "HTML geri çağırma (iki biçim)" "HTML kaydı yok"
elif [[ "$BACKEND" == "gnome-bridge" ]]; then
  skip "HTML geri çağırma (iki biçim)" "GNOME köprüsü (St.Clipboard) tek biçim sunar; tasarım gereği"
elif [[ -z "$READER" ]]; then
  skip "HTML geri çağırma (iki biçim)" "pano okuyucu (xclip/wl-paste) yok"
elif [[ "$HTML_MULTI" -eq 0 ]]; then
  skip "HTML geri çağırma (iki biçim)" "kayıt tek biçimle yakalandı (yazıcı $WRITER)"
else
  cli copy "$HTML_ID" >/dev/null 2>&1 || true
  both_offered() { local t; t="$(read_clip_types || true)"; types_have_html "$t" && types_have_text "$t"; }
  if wait_until both_offered; then
    pass "HTML geri çağırma text/html + düz metin sunuyor"
  else
    fail "HTML geri çağırma (iki biçim)" "sunulan biçimler: $(read_clip_types | tr '\n' ' ' || true)"
  fi
fi

# 13. copy --mime text/plain offers only plain text ----------------------
if [[ -z "$HTML_ID" ]]; then
  skip "copy --mime text/plain" "HTML kaydı yok"
elif [[ -z "$READER" ]]; then
  skip "copy --mime text/plain" "pano okuyucu yok"
elif [[ "$HTML_MULTI" -eq 0 ]]; then
  skip "copy --mime text/plain" "kayıtta düz metin payload'ı yok (tek biçimli yazıcı); text/html'e düşmesi tasarım gereği"
else
  if cli copy "$HTML_ID" --mime text/plain >/dev/null 2>&1; then
    only_text() { local t; t="$(read_clip_types || true)"; types_have_text "$t" && ! types_have_html "$t"; }
    if wait_until only_text; then
      pass "copy --mime text/plain yalnızca düz metin sunuyor"
    else
      fail "copy --mime text/plain" "sunulan biçimler: $(read_clip_types | tr '\n' ' ' || true)"
    fi
  else
    fail "copy --mime text/plain" "panora-cli copy $HTML_ID --mime text/plain başarısız"
  fi
fi

# 14. delete -------------------------------------------------------------
if [[ -n "$LINK_ID" ]]; then
  gone() { [[ -z "$(entry_line "$LINK_ID")" ]]; }
  if cli delete "$LINK_ID" >/dev/null 2>&1 && wait_until gone; then
    pass "delete (id $LINK_ID)"
  else
    fail "delete" "id $LINK_ID hâlâ listede"
  fi
else
  skip "delete" "silinecek link kaydı yok"
fi

# 15. dedup: A, B, A -> one entry, A on top -------------------------------
DEDUP_A="panora-e2e-dedup-$RUN_ID"
DEDUP_B="panora-e2e-other-$RUN_ID"
snapshot
set_clip_text "$DEDUP_A"
A_ID="$(wait_new "^\[text\] $DEDUP_A\$" || true)"
snapshot
set_clip_text "$DEDUP_B"
B_ID="$(wait_new "^\[text\] $DEDUP_B\$" || true)"
[[ -n "$A_ID" ]] && CREATED_IDS+=("$A_ID")
[[ -n "$B_ID" ]] && CREATED_IDS+=("$B_ID")
if [[ -n "$A_ID" && -n "$B_ID" ]]; then
  set_clip_text "$DEDUP_A"
  a_on_top() { [[ "$(list_all | grep -E '^  +[0-9]+ \[' | head -n1)" =~ $LIST_RE && "${BASH_REMATCH[2]}" == "$A_ID" ]]; }
  if wait_until a_on_top && [[ "$(count_matching "\[text\] $DEDUP_A\$")" == "1" ]]; then
    pass "tekilleştirme: aynı metin ikinci kez -> tek kayıt, üste taşındı"
  else
    fail "tekilleştirme" "kayıt sayısı $(count_matching "\[text\] $DEDUP_A\$"), üstteki: $(list_all | grep -E '^  +[0-9]+ \[' | head -n1)"
  fi
else
  fail "tekilleştirme" "hazırlık kayıtları yakalanamadı"
fi

# 16. private mode -------------------------------------------------------
PRIV_TEXT="panora-e2e-private-$RUN_ID"
if cli private on >/dev/null 2>&1 && cli status 2>/dev/null | grep -q 'private=true'; then
  snapshot
  set_clip_text "$PRIV_TEXT"
  if PRIV_ID="$(wait_new "$PRIV_TEXT" 2)"; then
    fail "özel mod" "özel mod açıkken kopyalanan metin kaydedildi (id $PRIV_ID)"
    CREATED_IDS+=("$PRIV_ID")
  else
    pass "özel mod açık -> kopya kaydedilmedi"
  fi
  if cli private off >/dev/null 2>&1 && cli status 2>/dev/null | grep -q 'private=false'; then
    pass "özel mod kapatıldı"
  else
    fail "özel mod kapatma" "status private=false göstermiyor"
  fi
else
  fail "özel mod" "panora-cli private on başarısız"
fi

# 17. clear keeps pinned (destructive) -----------------------------------
if [[ "$SAFE" -eq 1 ]]; then
  skip "clear sabitliyi korur" "--safe (geçmiş silinmedi)"
else
  cli pin "$TEXT_ID" >/dev/null 2>&1 || true
  if cli clear >/dev/null 2>&1; then
    UNPINNED_LEFT="$(list_all | grep -cE '^  +[0-9]+ \[' || true)"
    if [[ -n "$(entry_line "$TEXT_ID")" && "$UNPINNED_LEFT" == "0" ]]; then
      pass "clear: sabitli kayıt korundu, sabitsizler silindi"
    else
      fail "clear" "sabitli id $TEXT_ID: '$(entry_line "$TEXT_ID")', kalan sabitsiz: $UNPINNED_LEFT"
    fi
  else
    fail "clear" "panora-cli clear başarısız"
  fi
  cli unpin "$TEXT_ID" >/dev/null 2>&1 || true
fi

# 18. config reload: max_entries = 3 (destructive) -----------------------
if [[ "$SAFE" -eq 1 ]]; then
  skip "config reload (max_entries=3)" "--safe (geçmiş silinmedi)"
else
  CONFIG_BACKUP="$TMP/config.toml.bak"
  if [[ -f "$CONFIG" ]]; then
    CONFIG_EXISTED=1
    cp -p -- "$CONFIG" "$CONFIG_BACKUP"
  else
    mkdir -p -- "$(dirname "$CONFIG")"
    chmod 700 -- "$(dirname "$CONFIG")" 2>/dev/null || true
  fi
  CONFIG_TOUCHED=1
  # Insert max_entries = 3 into [history], replacing an existing key, or
  # append the section when the file has none.
  (
    umask 077
    if [[ "$CONFIG_EXISTED" -eq 1 ]]; then
      awk -v n=3 '
        /^[[:space:]]*\[history\][[:space:]]*$/ { print; print "max_entries = " n; inhist = 1; done = 1; next }
        /^[[:space:]]*\[/ { inhist = 0 }
        inhist && /^[[:space:]]*max_entries[[:space:]]*=/ { next }
        { print }
        END { if (!done) { print ""; print "[history]"; print "max_entries = " n } }
      ' "$CONFIG_BACKUP" > "$CONFIG.e2e.tmp"
    else
      printf '[history]\nmax_entries = 3\n' > "$CONFIG.e2e.tmp"
    fi
    mv -f -- "$CONFIG.e2e.tmp" "$CONFIG"
  )
  if cli reload >/dev/null 2>&1; then
    OK_ALL=1
    for i in 1 2 3 4 5; do
      snapshot
      set_clip_text "panora-e2e-limit-$RUN_ID-$i"
      if LID="$(wait_new "^\[text\] panora-e2e-limit-$RUN_ID-$i\$")"; then
        CREATED_IDS+=("$LID")
      else
        OK_ALL=0
      fi
    done
    UNPINNED="$(list_all | grep -cE '^  +[0-9]+ \[' || true)"
    if [[ "$OK_ALL" -eq 1 && "$UNPINNED" -le 3 && "$UNPINNED" -ge 1 ]]; then
      pass "config reload: max_entries=3 -> 5 kopyadan sonra $UNPINNED sabitsiz kayıt"
    else
      fail "config reload" "sabitsiz kayıt sayısı $UNPINNED (beklenen <= 3), tüm kopyalar yakalandı: $OK_ALL"
    fi
  else
    fail "config reload" "panora-cli reload başarısız (yazılan dosya: $CONFIG)"
  fi
  # Restore right away so the rest of the run and the user get the original.
  if [[ "$CONFIG_EXISTED" -eq 1 ]]; then
    cp -p -- "$CONFIG_BACKUP" "$CONFIG"
  else
    rm -f -- "$CONFIG"
  fi
  CONFIG_TOUCHED=0
  if cli reload >/dev/null 2>&1; then
    pass "config.toml geri yüklendi ve yeniden okundu"
  else
    fail "config geri yükleme" "panora-cli reload başarısız; $CONFIG dosyasını kontrol edin"
  fi
fi

# 19. password-manager hint is never recorded ----------------------------
if [[ "$GTK_HELPER" -eq 1 ]]; then
  SECRET_TEXT="panora-e2e-secret-$RUN_ID"
  printf '%s' "$SECRET_TEXT" > "$TMP/secret.bin"
  printf 'ignore' > "$TMP/hint.bin"
  snapshot
  offer_targets 3 "text/plain=$TMP/secret.bin" "x-kde-passwordManagerHint=$TMP/hint.bin"
  if SECRET_ID="$(wait_new "$SECRET_TEXT" 2)"; then
    fail "parola yöneticisi bayrağı" "x-kde-passwordManagerHint sunulan içerik kaydedildi (id $SECRET_ID)"
    CREATED_IDS+=("$SECRET_ID")
  else
    pass "parola yöneticisi bayrağı (x-kde-passwordManagerHint) -> kaydedilmedi"
  fi
else
  skip "parola yöneticisi bayrağı" "iki biçimi birlikte sunmak için python3-gi (GTK3) gerekir; elle: KeePassXC'den kopyalayın, listede görünmemeli"
fi

# 20. GUI toggle via D-Bus name ------------------------------------------
if [[ "$NO_GUI" -eq 1 ]]; then
  skip "GUI aç/kapa" "--no-gui"
elif [[ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  skip "GUI aç/kapa" "görüntü sunucusu yok"
elif ! has gdbus && ! has busctl; then
  skip "GUI aç/kapa" "gdbus/busctl yok"
else
  if name_has_owner io.panora.Panora; then
    note "Popup zaten açık; önce kapatılıyor."
    cli toggle >/dev/null 2>&1 || true
    wait_until name_absent io.panora.Panora || true
  fi
  WAIT_SAVE="$WAIT_SECS"; WAIT_SECS=6
  if cli toggle >/dev/null 2>&1 && wait_until name_has_owner io.panora.Panora; then
    pass "GUI aç: io.panora.Panora veriyolunda"
    if cli toggle >/dev/null 2>&1 && wait_until name_absent io.panora.Panora; then
      pass "GUI kapa: io.panora.Panora veriyolundan ayrıldı"
    else
      fail "GUI kapa" "ikinci toggle sonrası io.panora.Panora hâlâ veriyolunda"
    fi
  else
    fail "GUI aç" "toggle sonrası io.panora.Panora veriyolunda görünmedi (journalctl --user -u panod.service)"
  fi
  WAIT_SECS="$WAIT_SAVE"
fi

# 21. GNOME extension ----------------------------------------------------
if [[ "$IS_GNOME" -eq 1 ]]; then
  if has gnome-extensions; then
    EXT_STATE="$(gnome-extensions info "$EXT_UUID" 2>/dev/null | sed -n 's/^ *State: *//p' | head -n1)"
    if [[ "$EXT_STATE" == "ACTIVE" ]]; then
      pass "GNOME eklentisi ACTIVE"
    elif [[ "$BACKEND" == "gnome-bridge" ]]; then
      fail "GNOME eklentisi" "durum '${EXT_STATE:-yok}'; köprü backend'i eklentisiz yakalayamaz: gnome-extensions enable $EXT_UUID"
    else
      skip "GNOME eklentisi" "durum '${EXT_STATE:-yok}'; Super+V için: gnome-extensions enable $EXT_UUID"
    fi
  else
    skip "GNOME eklentisi" "gnome-extensions aracı yok"
  fi
else
  skip "GNOME eklentisi" "GNOME dışı masaüstü (bilgi)"
fi

# ================================================================ summary
echo
echo "------------------------------------------------------------"
echo "Özet: $PASS_N PASS, $FAIL_N FAIL, $SKIP_N SKIP"
if [[ "$FAIL_N" -gt 0 ]]; then
  echo "Sonuç: BAŞARISIZ. Ayrıntı için panora-doctor ve journalctl --user -u panod.service -n 50"
  exit 1
fi
echo "Sonuç: BAŞARILI."
exit 0
