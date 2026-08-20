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
export PANORA_ENV="DISPLAY=$DISPLAY XDG_SESSION_TYPE=$XDG_SESSION_TYPE HOME=$HOME XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR XDG_CONFIG_HOME=$XDG_CONFIG_HOME XDG_DATA_HOME=$XDG_DATA_HOME DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS GTK_A11Y=none"

OUT=/home/ubuntu/panora/test-artifacts
LOG=/tmp/panora-video-logs
mkdir -p "$OUT" "$LOG"
VIDEO="$OUT/panora-feature-tour.mp4"
RAW="$OUT/panora-feature-tour.raw.mp4"
rm -f "$VIDEO" "$RAW"

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
  xterm -title "$title" -geometry 68x20+730+10 -e bash -lc "export $PANORA_ENV; printf '\n> %s\n\n' '$command'; $command; printf '\n\nPanora test devam ediyor...\n'; sleep 3" >/dev/null 2>&1 &
  local pid=$!
  sleep 4
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

GUI_WIN=$(xdotool search --name '^Panora$' 2>/dev/null | tail -1)
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true

ffmpeg -hide_banner -loglevel error -y \
  -f x11grab -draw_mouse 1 -framerate 12 -video_size 1280x800 \
  -i :120 -c:v libx264 -preset ultrafast -crf 22 -pix_fmt yuv420p \
  -movflags +faststart "$RAW" >/tmp/panora-video-logs/ffmpeg.log 2>&1 &
REC_PID=$!

show_card 'Panora – test başlangıcı' 'Panora v1.0.0 | temiz Linux X11 + D-Bus oturumu'
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 2

show_card 'Panora – metin clipboard' 'Metin yakalama, dedup ve geçmiş listesi'
printf 'Panora video turu: Türkçe metin ve dedup testi' | xclip -selection clipboard -in
sleep 2
printf 'Panora video turu: ikinci metin öğesi' | xclip -selection clipboard -in
sleep 2
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 3

show_card 'Panora – FTS5 arama' 'Ctrl+F ile arama ve sonuç filtreleme'
xdotool key --window "$GUI_WIN" ctrl+f 2>/dev/null || xdotool key ctrl+f
xdotool type --delay 45 'Türkçe metin'
sleep 3
xdotool key Escape
sleep 2

show_card 'Panora – geri çağırma' 'Enter ile seçilen kaydı clipboarda geri koyma'
xdotool key --window "$GUI_WIN" Home 2>/dev/null || xdotool key Home
xdotool key --window "$GUI_WIN" Return 2>/dev/null || xdotool key Return
sleep 2
run_terminal 'Panora CLI – status ve recall' "panora-cli status; echo; panora-cli list; ID=\$(panora-cli list | awk 'NF>=2 {print \$1; exit}'); echo; echo Recall ID=\$ID; panora-cli copy \$ID; echo Clipboard sonrası:; xclip -selection clipboard -out"

show_card 'Panora – çoklu formatlar' 'HTML/RTF, URI dosya listesi, renk ve görsel'
printf '<b>Panora rich text</b> <i>HTML format testi</i>' | xclip -selection clipboard -t text/html -in
sleep 2
xclip -selection clipboard -t image/png -in < /tmp/panora-video-assets/panora-test.png
sleep 2
xclip -selection clipboard -t text/uri-list -in < /tmp/panora-video-assets/uris.txt
sleep 2
printf '#36C2FF' | xclip -selection clipboard -in
sleep 2
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 4

show_card 'Panora – pin ve silme' 'Sabitleme, unpin ve geçmiş temizleme'
run_terminal 'Panora CLI – pin/delete/clear' "ID=\$(panora-cli list | awk 'NF>=2 {print \$1; exit}'); echo Secilen ID=\$ID; panora-cli pin \$ID; echo Sabitli liste:; panora-cli list; panora-cli unpin \$ID; echo; echo Temizleme öncesi kayıtlar:; panora-cli list; panora-cli clear; echo; echo Temizleme sonrası:; panora-cli list"
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 3

show_card 'Panora – özel mod' 'Private mode açıkken yeni clipboard kaydı oluşmuyor'
printf 'Private mode öncesi kaydedilecek öğe' | xclip -selection clipboard -in
sleep 2
panora-cli private on
sleep 1
printf 'Private mode içinde kaydedilmemesi gereken öğe' | xclip -selection clipboard -in
sleep 3
run_terminal 'Panora CLI – private mode' "panora-cli status; echo; panora-cli list; echo; echo Private kapatılıyor; panora-cli private off; panora-cli status"
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 3

show_card 'Panora – ayarlar' 'Dil, limitler, retention, MIME boyutu ve gizlilik ayarları'
xdotool mousemove --window "$GUI_WIN" 60 40 click 1 2>/dev/null || xdotool mousemove 60 40 click 1
sleep 3
xdotool mousemove 450 135 click 1 2>/dev/null || true
sleep 2
xdotool key Escape
sleep 2
xdotool key Escape
sleep 2

show_card 'Panora – güvenlik' 'Encrypted local storage, Secret Service, IPC limits ve backend status'
run_terminal 'Panora CLI – final security status' "panora-cli status; echo; stat -c 'socket/data mode: %a %n' \"$XDG_RUNTIME_DIR/panora.sock\" \"$XDG_DATA_HOME/panora\" 2>/dev/null || true; echo; panora-cli list"
xdotool windowactivate --sync "$GUI_WIN" 2>/dev/null || true
sleep 3

show_card 'Panora – test tamamlandı' 'Metin, arama, recall, formatlar, pin/delete, private, ayarlar ve güvenlik akışı tamamlandı'
sleep 2
kill "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true
ffmpeg -hide_banner -loglevel error -y -i "$RAW" -c copy -movflags +faststart "$VIDEO" >/tmp/panora-video-logs/remux.log 2>&1 || cp "$RAW" "$VIDEO"
ffprobe -v error -show_entries format=duration,size:stream=codec_name,width,height,r_frame_rate -of default=noprint_wrappers=1 "$VIDEO" >"$OUT/panora-feature-tour-media-info.txt" 2>&1 || true
printf '%s\n' "video=$VIDEO" "raw=$RAW" "gui_window=$GUI_WIN" >"$OUT/panora-feature-tour-run.txt"
