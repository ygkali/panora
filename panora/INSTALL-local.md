# Panora 1.1.0 — Lokal Kurulum Kılavuzu

Bu paket Panora'nın güncel Rust kaynak kodunu, testleri, belgeleri ve Ubuntu 24.04/Debian tabanlı amd64 sistemlerde kullanılabilen Debian paketini içerir.

## 1. Tek komutla kurulum

ZIP'ten çıkan `panora` klasörüne girin ve script'leri çalıştırılabilir yapın:

```bash
cd panora-local-kit-1.1.0/panora
chmod +x install.sh test-local.sh uninstall.sh
./install.sh
```

Kurulum script'i Debian/Ubuntu bağımlılıklarını kurar, `dist/panora_1.1.0_amd64.deb` paketini yükler ve `panod.service` kullanıcı servisini başlatmayı dener. Kurulumdan sonra lokal smoke test'i çalıştırın:

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
  adwaita-icon-theme librsvg2-common \
  xclip wl-clipboard gnome-keyring
```

Arşiv içindeki paketi kurun:

```bash
cd panora-local-kit-1.1.0/panora
sudo dpkg -i dist/panora_1.1.0_amd64.deb
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
  adwaita-icon-theme librsvg2-common \
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

Paket, Debian 13 (trixie) amd64 üzerinde GTK4 4.18.6 ve libadwaita 1.7.6 ile uçtan uca doğrulanmıştır: `dpkg -i` ile kurulum, `panod` başlatma, `panora-cli` status/list/search/pin/private/clear ve X11 (Xvfb) altında GUI açılışı. Ubuntu 24.04 LTS ve Linux Mint gibi türevlerde, aşağıdaki sürüm alt sınırları karşılandığı sürece uyumlu olması beklenir — ancak bu dağıtımlarda ayrıca test edilmemiştir.

Paketin `Depends` alanı şudur: `libc6`, `libgtk-4-1 (>= 4.12)`, `libadwaita-1-0 (>= 1.5)`, `libsqlite3-0`, `libglib2.0-0`, `adwaita-icon-theme`, `librsvg2-common`. Ubuntu 22.04 ve Linux Mint 21 gibi eski sistemlerde libadwaita sürümü bu alt sınırın altındadır; oralarda kaynaktan derleme veya daha yeni masaüstü kütüphaneleri gerekir.

`librsvg2-common` bilerek sert bağımlılıktır: Adwaita 48 sembolik ikonları yalnızca SVG olarak dağıtır ve bu paket olmadan gdk-pixbuf'ın SVG loader'ı bulunmadığından arayüzdeki ikonların bir kısmı "image-missing" olarak çizilir. Hem `libgtk-4-1` hem `adwaita-icon-theme` bu paketi sadece `Recommends` olarak listelediği için `--no-install-recommends` ile kurulan sistemlerde eksik kalır.

X11 backend'i `xclip` ile, Wayland backend'i `wl-paste`/`wl-copy` ile çalışır. Gerçek Wayland runtime'ı bu geliştirme ortamında ayrıca doğrulanmamıştır; X11 runtime'ı doğrulanmıştır.

## 5. Güvenlik ve veri konumu

Clipboard payload'ları XChaCha20-Poly1305 ile şifrelenmiş yerel storage'a yazılır. FTS5 yalnızca metin preview'lerini indeksler. Private mode açıkken yeni clipboard içerikleri kaydedilmez. Telefon/bulut senkronu v1'de etkin değildir; senkronizasyon trait'i gelecek genişletmeler için modüler bırakılmıştır.

## 6. Paket bütünlük özeti

Paket artık arşivle birlikte hazır gelmiyor; `packaging/build-deb.sh` (veya `install.sh`) onu bu makinede kaynaktan üretir. Bu yüzden burada sabit bir SHA-256 verilmez — derleme çıktısı derleyici sürümüne ve build yoluna göre değişir. Script tamamlanınca ürettiği paketin özetini kendisi yazdırır; kurduğunuz dosyanın o değerle aynı olduğunu şöyle doğrulayın:

```bash
sha256sum dist/panora_1.1.0_amd64.deb
```

## 7. Sorun giderme

Her şeyin başladığı yer:

```bash
systemctl --user status panod.service
journalctl --user -u panod.service -n 100 --no-pager
panora-cli status
```

**`panora-cli: daemon unavailable`** — panod çalışmıyor demektir. `systemctl --user status panod.service` çıktısındaki hatayı okuyun; aşağıdaki maddeler en olası nedenleri kapsıyor.

**Keyring kilidi.** panod ana anahtarı Secret Service'ten alır. Login keyring kilitliyse kilit açma istemi belirir; yanıtlanmazsa panod 60 saniye sonra şu hatayla durur:

```
Secret Service did not answer within 60s; an unlock prompt may be waiting.
```

Keyring'i açtıktan sonra deneme sayacını sıfırlayıp yeniden başlatın:

```bash
systemctl --user reset-failed panod.service
systemctl --user restart panod.service
```

Servis birkaç başarısız denemeden sonra kendini durdurur (`StartLimitBurst=3`); bu, arka arkaya parola istemi açılmasını engellemek içindir.

**Servis hiç başlamıyor, `status=226/NAMESPACE`.** Unit `~/.local/share/panora` dizinini `ReadWritePaths` ile açar ve onu `ExecStartPre` ile kendisi oluşturur. Dizini elle silip izinlerini bozduysanız geri alın:

```bash
install -d -m 0700 ~/.local/share/panora
systemctl --user restart panod.service
```

**Arayüzde ikonlar kırık kutu görünüyor.** `librsvg2-common` eksik. Adwaita 48 sembolik ikonları yalnızca SVG dağıtır ve bu paket olmadan çizilemezler:

```bash
sudo apt install -y librsvg2-common adwaita-icon-theme
```

**Super+V çalışmıyor.** GNOME eklentisi paketle birlikte kurulur ama gnome-shell onu ancak yeniden başladıktan sonra görür. Oturumu kapatıp açın, sonra:

```bash
gnome-extensions enable panora@panora-clipboard.org
gnome-extensions info panora@panora-clipboard.org
```

Wayland'de `gnome-shell --replace` çalışmaz; oturumu gerçekten kapatıp açmanız gerekir. Eklenti olmadan da panod pano geçmişini toplamaya devam eder; kaybettiğiniz tek şey Super+V kısayoludur — popup'ı `panora` komutuyla veya uygulama menüsünden açabilirsiniz.

**Hazır paket kurulmuyor, `libc6 (>= 2.39)` hatası.** Paket Debian 13 üzerinde derlendi. Daha eski bir dağıtımdasınız (Ubuntu 22.04, Debian 12 gibi). `dist/` klasörünü silip `./install.sh` çalıştırın; script kaynaktan derleyecektir:

```bash
rm -rf dist
./install.sh
```

**SSH üzerinden X forwarding kullanıyorsanız** panod bağlanamaz: unit `RestrictAddressFamilies=AF_UNIX` ile TCP'yi engeller, `DISPLAY=localhost:10` ise TCP gerektirir. `/usr/lib/systemd/user/panod.service` içindeki o satırı kaldırıp `systemctl --user daemon-reload` çalıştırın.
