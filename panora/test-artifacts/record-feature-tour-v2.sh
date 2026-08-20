#!/usr/bin/env bash
set -u

export DISPLAY=:120
export XDG_SESSION_TYPE=x11
export HOME=/tmp/panora-video-home
export XDG_RUNTIME_DIR=/tmp/panora-video-runtime
export XDG_CONFIG_HOME=/tmp/panora-video-config
export XDG_DATA_HOME=/tmp/panora-video-data
export DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-session-bus-socket
export GTK_A11Y=none

OUT=/home/ubuntu/panora/test-artifacts
LOG=/tmp/panora-video-logs
mkdir -p "$OUT" "$LOG"
VIDEO="$OUT/panora-feature-tour.mp4"
RAW="$OUT/panora-feature-tour.raw.mp4"
rm -f "$VIDEO" "$RAW"

stop_gui() {
  if [ -f "$LOG/gui.pid" ]; then
    kill "$(cat "$LOG/gui.pid")" 2>/dev/null || true
    wait "$(cat "$LOG/gui.pid")" 2>/dev/null || true
    rm -f "$LOG/gui.pid"
  fi
  sleep 0.6
}

start_gui() {
  stop_gui
  /usr/bin/panora-gui >>"$LOG/gui.log" 2>&1 &
  echo $! >"$LOG/gui.pid"
  sleep 2
  GUI_WIN=$(xdotool search --name '^Panora$' 2>/dev/null | tail -1)
  if [ -n "${GUI_WIN:-}" ]; then
    xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
  fi
}

show_card() {
  local title="$1"
  local text="$2"
  xterm -title "$title" -geometry 68x3+730+10 -e bash -lc "printf '\n  %s\n\n' '$text'; sleep 2" >/dev/null 2>&1 &
  local pid=$!
  sleep 2
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

run_terminal() {
  local title="$1"
  local command="$2"
  xterm -title "$title" -geometry 68x20+730+10 -e bash -lc "export DISPLAY=:120 HOME=/tmp/panora-video-home XDG_RUNTIME_DIR=/tmp/panora-video-runtime XDG_CONFIG_HOME=/tmp/panora-video-config XDG_DATA_HOME=/tmp/panora-video-data DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-session-bus-socket; printf '\n> %s\n\n' '$command'; $command; printf '\n\nPanora test devam ediyor...\n'; sleep 4" >/dev/null 2>&1 &
  local pid=$!
  sleep 5
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

# Start from a visible, empty-history GUI state.
/usr/bin/panora-cli clear >/dev/null 2>&1 || true
start_gui

ffmpeg -hide_banner -loglevel error -y \
  -f x11grab -draw_mouse 1 -framerate 12 -video_size 1280x800 \
  -i :120 -c:v libx264 -preset ultrafast -crf 22 -pix_fmt yuv420p \
  -movflags +faststart "$RAW" >"$LOG/ffmpeg-v2.log" 2>&1 &
REC_PID=$!

show_card 'Panora – test başlangıcı' 'Panora v1.0.0 | Debian paketinden çalışan temiz Linux X11 oturumu'
start_gui
sleep 2

show_card 'Panora – metin clipboard' 'Metin yakalama, dedup ve geçmiş listesi'
printf 'Panora video turu: Türkçe metin ve dedup testi' | xclip -selection clipboard -in
sleep 2
printf 'Panora video turu: ikinci metin öğesi' | xclip -selection clipboard -in
sleep 2
start_gui
sleep 3

show_card 'Panora – FTS5 arama' 'Ctrl+F ile arama ve sonuç filtreleme'
start_gui
xdotool key --window "$GUI_WIN" ctrl+f 2>/dev/null || xdotool key ctrl+f
xdotool type --delay 45 'Türkçe metin'
sleep 4
# Search result remains visible; popup is reopened for the next step afterwards.
stop_gui

show_card 'Panora – geri çağırma' 'Enter ile seçilen kaydı clipboarda geri koyma'
start_gui
xdotool key --window "$GUI_WIN" Home 2>/dev/null || xdotool key Home
xdotool key --window "$GUI_WIN" Return 2>/dev/null || xdotool key Return
sleep 2
run_terminal 'Panora CLI – status ve recall' "panora-cli status; echo; panora-cli list; ID=\$(panora-cli list | awk 'NF>=2 {print \$1; exit}'); echo; echo Recall ID=\$ID; panora-cli copy \$ID; echo Clipboard sonrası:; xclip -selection clipboard -out"
start_gui
sleep 3

show_card 'Panora – çoklu formatlar' 'Rich text, URI dosya listesi, renk ve görsel'
printf '<b>Panora rich text</b> <i>HTML format testi</i>' | xclip -selection clipboard -t text/html -in
sleep 1
xclip -selection clipboard -t image/png -in < /tmp/panora-video-assets/panora-test.png
sleep 1
xclip -selection clipboard -t text/uri-list -in < /tmp/panora-video-assets/uris.txt
sleep 1
printf '#36C2FF' | xclip -selection clipboard -in
sleep 2
start_gui
sleep 4

show_card 'Panora – pin ve silme' 'Sabitleme, unpin ve geçmiş temizleme'
run_terminal 'Panora CLI – pin/delete/clear' "ID=\$(panora-cli list | awk 'NF>=2 {print \$1; exit}'); echo Secilen ID=\$ID; panora-cli pin \$ID; echo Sabitli liste:; panora-cli list; panora-cli unpin \$ID; echo; echo Temizleme oncesi:; panora-cli list; panora-cli clear; echo; echo Temizleme sonrasi:; panora-cli list"
start_gui
sleep 3

show_card 'Panora – özel mod' 'Private mode açıkken yeni clipboard kaydı oluşmuyor'
printf 'Private mode öncesi kaydedilecek öğe' | xclip -selection clipboard -in
sleep 2
panora-cli private on
sleep 1
printf 'Private mode içinde kaydedilmemesi gereken öğe' | xclip -selection clipboard -in
sleep 3
run_terminal 'Panora CLI – private mode' "panora-cli status; echo; panora-cli list; echo; echo Private kapatiliyor; panora-cli private off; panora-cli status"
start_gui
sleep 3

show_card 'Panora – ayarlar' 'Dil, limitler, retention, MIME boyutu ve gizlilik ayarları'
start_gui
xdotool mousemove --window "$GUI_WIN" 60 40 click 1 2>/dev/null || xdotool mousemove 60 40 click 1
sleep 4
xdotool mousemove 370 575 click 1 2>/dev/null || true
sleep 2
start_gui
sleep 3

show_card 'Panora – güvenlik' 'Encrypted local storage, Secret Service, IPC limits ve backend status'
run_terminal 'Panora CLI – final security status' "panora-cli status; echo; stat -c 'mode=%a %n' \"$XDG_RUNTIME_DIR/panora.sock\" \"$XDG_DATA_HOME/panora\" 2>/dev/null || true; echo; panora-cli list"
start_gui
sleep 3

show_card 'Panora – test tamamlandı' 'Metin, arama, recall, formatlar, pin/delete, private, ayarlar ve güvenlik tamamlandı'
start_gui
sleep 3
kill "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true
ffmpeg -hide_banner -loglevel error -y -i "$RAW" -c copy -movflags +faststart "$VIDEO" >"$LOG/remux-v2.log" 2>&1 || cp "$RAW" "$VIDEO"
ffprobe -v error -show_entries format=duration,size:stream=codec_name,width,height,r_frame_rate,pix_fmt -of default=noprint_wrappers=1 "$VIDEO" >"$OUT/panora-feature-tour-media-info.txt" 2>&1 || true
printf '%s\n' "video=$VIDEO" "raw=$RAW" "gui_window=${GUI_WIN:-unknown}" "scenario=v2" >"$OUT/panora-feature-tour-run.txt"
