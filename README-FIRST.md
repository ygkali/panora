# Panora 1.1.0 — Lokal Kurulum Kiti

## En hızlı kurulum

Bu arşivi açtıktan sonra terminalde şu komutları çalıştırın:

```bash
cd panora-local-kit-1.1.0
chmod +x KUR.sh TEST.sh KALDIR.sh
./KUR.sh
```

`KUR.sh` Debian/Ubuntu bağımlılıklarını kurar, `panora_1.1.0_ui1_amd64.deb` paketini yükler ve kullanıcı daemon servisini başlatır. Sudo parolanız istenebilir.

Kurulumdan sonra test:

```bash
./TEST.sh
```

Bu test X11 oturumunda `xclip`, Wayland oturumunda `wl-copy` kullanarak örnek metni panoya kopyalar; daemon status, liste, FTS5 arama ve GUI açılışını kontrol eder. GUI'yi açmadan yalnızca terminal smoke testi için:

```bash
PANORA_NO_GUI=1 ./TEST.sh
```

GUI'yi manuel açmak için:

```bash
panora
```

## Fotoğraf testi

X11 kullanıyorsanız:

```bash
xclip -selection clipboard -in -t image/png < /path/to/foto.png
panora
```

Wayland kullanıyorsanız:

```bash
wl-copy --type image/png < /path/to/foto.png
panora
```

Popup içinde fotoğrafın `FOTO` kartı ve thumbnail'i görünmelidir. Metin, HTML/rich-text ve URI clipboard testleri için ayrıntılı komutlar `panora/INSTALL-local.md` içindedir.

## Kaynak koddan derleme

Hazır Debian paketi yerine kaynak kodunu derlemek için:

```bash
cd panora-local-kit-1.1.0/panora
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev libsqlite3-dev libx11-dev libwayland-dev xclip wl-clipboard gnome-keyring
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
```

Rust 1.85 veya daha yeni stable toolchain gereklidir. Ayrıntılı sistem uyumluluğu, servis günlükleri, manuel paket kurulumu ve kaldırma davranışı için `panora/INSTALL-local.md` dosyasını okuyun.

## Kaldırma

Programı kaldırmak için:

```bash
cd panora-local-kit-1.1.0
./KALDIR.sh
```

Kaldırma script'i programı ve servisi kaldırır, fakat şifreli clipboard geçmişini varsayılan olarak silmez. Verileri de silmek isterseniz `~/.local/share/panora` ve `~/.config/panora` yollarını ayrıca ve bilinçli olarak kaldırın.

## Paket ve kaynak içeriği

`panora/` klasörü Rust kaynaklarını, GTK4/libadwaita GUI'yi, daemon/CLI'yi, X11/Wayland backend'lerini, GNOME extension'ı, test scriptlerini ve belgeleri içerir. Hazır paket `panora/dist/panora_1.1.0_ui1_amd64.deb` yolundadır.

```text
Debian package SHA-256:
8f0f0b3e8af5b2d7f415d08c56c60c7e6b42a75c6fb370a84cb1f81e5f074c80
```
