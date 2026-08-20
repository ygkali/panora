# Panora 1.1.0 — Lokal Kurulum Kılavuzu

Bu paket Panora'nın güncel Rust kaynak kodunu, testleri, belgeleri ve Ubuntu 24.04/Debian tabanlı amd64 sistemlerde kullanılabilen Debian paketini içerir.

## 1. Tek komutla kurulum

ZIP'ten çıkan `panora` klasörüne girin ve script'leri çalıştırılabilir yapın:

```bash
cd panora-local-kit-1.1.0/panora
chmod +x install.sh test-local.sh uninstall.sh
./install.sh
```

Kurulum script'i Debian/Ubuntu bağımlılıklarını kurar, `dist/panora_1.1.0_ui1_amd64.deb` paketini yükler ve `panod.service` kullanıcı servisini başlatmayı dener. Kurulumdan sonra lokal smoke test'i çalıştırın:

```bash
./test-local.sh
```

Sadece terminal doğrulaması yapmak ve GUI'yi açmamak için:

```bash
PANORA_NO_GUI=1 ./test-local.sh
```

Bu test oturum tipini algılayarak X11'de `xclip`, Wayland'de `wl-copy` kullanır; metin kopyalama, daemon status, listeleme, FTS5 arama ve GUI açılışını doğrular.

Panora'yı kaldırmak için:

```bash
./uninstall.sh
```

Kaldırma script'i programı ve servisi kaldırır ancak şifreli kullanıcı geçmişini silmez. Verileri silmek isterseniz `~/.local/share/panora` ve `~/.config/panora` yollarını ayrıca kaldırmanız gerekir.

## 2. Hazır Debian paketiyle kurulum

Ön koşul olarak GTK4/libadwaita masaüstü kütüphanelerini, SQLite runtime'ını ve clipboard yardımcılarını kurun:

```bash
sudo apt update
sudo apt install -y \
  libgtk-4-1 libadwaita-1-0 libsqlite3-0 \
  xclip wl-clipboard gnome-keyring
```

Arşiv içindeki paketi kurun:

```bash
cd panora-local-kit-1.1.0/panora
sudo dpkg -i dist/panora_1.1.0_ui1_amd64.deb
sudo apt-get -f install -y
```

Kullanıcı systemd servisini etkinleştirin:

```bash
systemctl --user daemon-reload
systemctl --user enable --now panod.service
systemctl --user status panod.service
```

Popup arayüzünü açmak için:

```bash
panora
# veya
panora-gui
```

Daemon ve CLI durumunu kontrol edin:

```bash
panora-cli status
panora-cli list
```

X11 kullanıyorsanız hızlı clipboard testi:

```bash
printf 'Panora lokal test\n' | xclip -selection clipboard -in -t text/plain
sleep 1
panora-cli list
panora-cli search lokal
```

Wayland kullanıyorsanız aynı testi şu şekilde yapın:

```bash
printf 'Panora Wayland test\n' | wl-copy
sleep 1
panora-cli list
```

Fotoğraf testi için PNG dosyasını clipboard'a aktarın:

```bash
wl-copy --type image/png < foto.png       # Wayland
xclip -selection clipboard -in -t image/png < foto.png  # X11
```

`panora` popup'ında fotoğraf kartının `FOTO` rozeti ve thumbnail'i görünmelidir. HTML/rich-text için `text/html`, dosya/URI için `text/uri-list` MIME türü kullanılabilir.

## 3. Kaynak koddan derleme

Kaynak koddan derlemek için Rust 1.85 veya daha yeni bir stable toolchain gerekir:

```bash
sudo apt update
sudo apt install -y \
  build-essential pkg-config \
  libgtk-4-dev libadwaita-1-dev \
  libsqlite3-dev libx11-dev libwayland-dev \
  xclip wl-clipboard gnome-keyring
```

Arşiv içindeki proje klasörüne girip derleyin:

```bash
cd panora
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
```

Release binary'lerini doğrudan çalıştırmak için daemon'ı ayrı terminalde başlatın:

```bash
./target/release/panod
./target/release/panora-cli status
./target/release/panora-gui
```

## 4. Sistem uyumluluğu

Güncel Debian paketi Ubuntu 24.04 LTS amd64 üzerinde uçtan uca doğrulanmıştır. Debian 13 ve Ubuntu 24.04 tabanlı Linux Mint sürümleri için gerekli GTK4/libadwaita sürümleri mevcutsa uyumlu olması beklenir.

Paketin temel bağımlılıkları şunlardır: `libc6`, `libsqlite3-0`, `libgtk-4-1 >= 4.10`, `libadwaita-1-0 >= 1.4`, `libx11-6` ve `libwayland-client0`. Ubuntu 22.04 ve Linux Mint 21 gibi eski sistemlerde libadwaita sürümü paketin istediğinden eski olabilir; bu sistemlerde kaynak koddan uyarlama veya daha yeni masaüstü kütüphaneleri gerekir.

X11 backend'i `xclip` ile, Wayland backend'i `wl-paste`/`wl-copy` ile çalışır. Gerçek Wayland runtime'ı bu geliştirme ortamında ayrıca doğrulanmamıştır; X11 runtime'ı doğrulanmıştır.

## 5. Güvenlik ve veri konumu

Clipboard payload'ları XChaCha20-Poly1305 ile şifrelenmiş yerel storage'a yazılır. FTS5 yalnızca metin preview'lerini indeksler. Private mode açıkken yeni clipboard içerikleri kaydedilmez. Telefon/bulut senkronu v1'de etkin değildir; senkronizasyon trait'i gelecek genişletmeler için modüler bırakılmıştır.

## 6. Paket bütünlük özeti

```text
Dosya: dist/panora_1.1.0_ui1_amd64.deb
SHA-256: 8f0f0b3e8af5b2d7f415d08c56c60c7e6b42a75c6fb370a84cb1f81e5f074c80
```

Kurulum sonrasında sorun yaşarsanız aşağıdaki günlükleri kontrol edin:

```bash
journalctl --user -u panod.service -n 100 --no-pager
panora-cli status
```
