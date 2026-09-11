# Panora 1.2.0 — Lokal Kurulum Kılavuzu

Bu paket Panora'nın güncel Rust kaynak kodunu, testleri, belgeleri ve Debian/Ubuntu tabanlı amd64 sistemlerde `.deb` paketi üreten script'leri içerir.

## 1. Tek komutla kurulum

`panora` klasörüne girin ve script'leri çalıştırılabilir yapın:

```bash
cd panora
chmod +x install.sh test-local.sh uninstall.sh
./install.sh
```

Kurulum script'i sırasıyla:

1. Çalışma zamanı bağımlılıklarını kurar (`libgtk-4-1`, `libadwaita-1-0`, `adwaita-icon-theme`, `librsvg2-common`, `gnome-keyring`).
2. `dist/panora_*.deb` yoksa veya makinenizle uyumsuzsa derleme bağımlılıklarını kurar, `cargo` yoksa rustup'ı kullanıcı dizinine indirir ve `packaging/build-deb.sh` ile paketi üretir.
3. Paketi `dpkg -i` ile yükler.
4. `panod.service` kullanıcı servisini etkinleştirip başlatır ve GNOME eklentisini etkinleştirmeyi dener.
5. `panora-cli status` ile doğrular.

Kurulumdan sonra lokal smoke test:

```bash
./test-local.sh              # durum, kopyalama, arama, geri çağırma, GUI
PANORA_NO_GUI=1 ./test-local.sh
```

Kaldırmak için `./uninstall.sh`. Şifreli geçmiş (`~/.local/share/panora`) ve ayarlar (`~/.config/panora`) korunur.

## 2. Hazır Debian paketiyle kurulum

```bash
sudo apt update
sudo apt install -y libgtk-4-1 libadwaita-1-0 libsqlite3-0 adwaita-icon-theme librsvg2-common gnome-keyring
sudo dpkg -i dist/panora_1.2.0_amd64.deb
sudo apt-get -f install -y
systemctl --user daemon-reload
systemctl --user enable --now panod.service
systemctl --user status panod.service
```

Paket şunları kurar: `/usr/bin/panod`, `/usr/bin/panora-gui` (+ `panora` sembolik bağı), `/usr/bin/panora-cli`, `/usr/lib/systemd/user/panod.service`, `/usr/share/applications/io.panora.Panora.desktop`, `/usr/share/dbus-1/services/io.panora.Panora.service` ve GNOME eklentisi `/usr/share/gnome-shell/extensions/panora@panora-clipboard.org`.

Popup: `panora` (ikinci çağrı açık pencereyi kapatır). Daemon ve CLI:

```bash
panora-cli status
panora-cli list
panora-cli search lokal
panora-cli copy 12 --paste
panora-cli preview 12 --mime image/png --out foto.png
```

Pano testi için herhangi bir uygulamadan kopyalayın; `xclip`/`wl-copy` yalnızca komut satırından test etmek isterseniz gerekir:

```bash
printf 'Panora lokal test\n' | xclip -selection clipboard -in   # X11
printf 'Panora Wayland test\n' | wl-copy                         # Wayland (data-control)
sleep 1 && panora-cli list
```

## 3. Kaynak koddan derleme

Rust 1.85 veya daha yeni bir stable toolchain gerekir. X11 (x11rb) ve Wayland (wayland-client) protokolleri saf Rust'tır; yalnızca GTK4/libadwaita geliştirme paketleri gerekir:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  adwaita-icon-theme librsvg2-common gnome-keyring binutils libglib2.0-bin
```

```bash
cd panora
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
./packaging/build-deb.sh          # dist/panora_<sürüm>_<mimari>.deb
```

Release binary'lerini doğrudan çalıştırmak için (paket kurulu değilse Super+V ve D-Bus etkinleştirme çalışmaz; `panora-cli toggle` ise `panora-gui`'yi yanındaki dizinden başlatır):

```bash
./target/release/panod
./target/release/panora-cli status
./target/release/panora-gui
```

X11 backend'inin gerçek X sunucusuna karşı testleri `DISPLAY` tanımlıysa çalışır:

```bash
Xvfb :99 -screen 0 1280x800x24 -ac &
DISPLAY=:99 cargo test -p panod --test x11_integration -- --test-threads=1
```

## 4. Sistem uyumluluğu

Paketin `Depends` alanı: `libc6`, `libgtk-4-1 (>= 4.12)`, `libadwaita-1-0 (>= 1.5)`, `libglib2.0-0t64 | libglib2.0-0`, `adwaita-icon-theme`, `librsvg2-common`. `Recommends: gnome-keyring`, `Suggests: wtype, ydotool`. Ubuntu 22.04 ve Linux Mint 21 gibi eski sistemlerde libadwaita sürümü alt sınırın altındadır; oralarda daha yeni masaüstü kütüphaneleri gerekir.

`librsvg2-common` bilerek sert bağımlılıktır: Adwaita 48 sembolik ikonları yalnızca SVG olarak dağıtır ve bu paket olmadan gdk-pixbuf'ın SVG loader'ı bulunmadığından arayüzdeki ikonların bir kısmı "image-missing" olarak çizilir.

Oturum tipine göre backend:

- **X11**: XFIXES olayları, yerel selection ownership (INCR dahil), XTEST ile anında yapıştır.
- **Wayland (GNOME 48+, KDE, Sway, Hyprland, …)**: `ext-data-control-v1` veya `wlr-data-control-v1`.
- **Wayland GNOME ≤ 47**: Mutter data-control sunmadığından yakalama ve geri çağırma Panora Shell eklentisi üzerinden yapılır; eklenti etkin değilse `journalctl --user -u panod` bunu bildirir.
- Wayland'de data-control yoksa ve GNOME değilse `DISPLAY` üzerinden XWayland'a düşülür (uygulama adı bilinmez).

## 5. Güvenlik ve veri konumu

Clipboard payload'ları XChaCha20-Poly1305 ile şifrelenmiş olarak `~/.local/share/panora/blobs` altına, metadata ve şifreli önizlemeler `~/.local/share/panora/history.db` içine yazılır. FTS5 yalnızca metin önizlemelerini indeksler. Private mode açıkken yeni içerik kaydedilmez. Ana anahtar Secret Service'te (`gnome-keyring`) saklanır; ayarlar `~/.config/panora/config.toml` dosyasındadır. Telefon/bulut senkronu yoktur.

## 6. Paket bütünlüğü

Paket arşivle birlikte hazır gelmez; `packaging/build-deb.sh` (veya `install.sh`) onu bu makinede kaynaktan üretir ve SHA-256 özetini yazdırır. Kurduğunuz dosyayı doğrulamak için:

```bash
sha256sum dist/panora_1.2.0_amd64.deb
```

## 7. Sorun giderme

```bash
systemctl --user status panod.service
journalctl --user -u panod.service -n 100 --no-pager
panora-cli status
```

**`panora-cli: daemon unavailable`** — panod çalışmıyor. `systemctl --user status panod.service` çıktısındaki hatayı okuyun.

**Keyring kilidi.** panod ana anahtarı Secret Service'ten alır. Login keyring kilitliyse kilit açma istemi belirir; yanıtlanmazsa panod 60 saniye sonra durur (`Secret Service did not answer within 60s`). Keyring'i açtıktan sonra:

```bash
systemctl --user reset-failed panod.service
systemctl --user restart panod.service
```

Servis birkaç başarısız denemeden sonra kendini durdurur (`StartLimitBurst=3`); bu, arka arkaya parola istemi açılmasını engellemek içindir.

**`Wayland compositor has no data-control protocol`** — GNOME ≤ 47'de normaldir; panod GNOME bridge backend'ine geçer ve Shell eklentisinin etkin olması gerekir. Eklenti olmadan bu sürümlerde yakalama yapılamaz.

**Servis hiç başlamıyor, `status=226/NAMESPACE`.** Unit `~/.local/share/panora` dizinini `ReadWritePaths` ile açar ve `ExecStartPre` ile kendisi oluşturur. Dizini elle silip izinlerini bozduysanız geri alın:

```bash
install -d -m 0700 ~/.local/share/panora
systemctl --user restart panod.service
```

**Arayüzde ikonlar kırık kutu görünüyor.** `librsvg2-common` eksik:

```bash
sudo apt install -y librsvg2-common adwaita-icon-theme
```

**Super+V çalışmıyor.** GNOME eklentisi paketle birlikte kurulur ama gnome-shell onu ancak yeniden başladıktan sonra görür. Oturumu kapatıp açın, sonra:

```bash
gnome-extensions enable panora@panora-clipboard.org
gnome-extensions info panora@panora-clipboard.org
```

Eklenti olmadan da panod (GNOME 48+ ve diğer masaüstlerinde) pano geçmişini toplamaya devam eder; popup'ı `panora` komutuyla, uygulama menüsünden veya `panora-cli toggle` ile açabilirsiniz. Başka bir masaüstünde kısayol için `panora-cli toggle` komutunu masaüstünüzün kısayol ayarlarına bağlayın.

**Anında yapıştır çalışmıyor.** X11'de XTEST, GNOME'da eklenti gerekir; diğer Wayland masaüstlerinde `wtype` (wlroots) veya `ydotool` (uinput daemon'ı ile) kurulu olmalıdır. Yapıştırma başarısız olsa da içerik panoya konur ve bir bildirim gösterilir.

**SSH üzerinden X forwarding kullanıyorsanız** panod bağlanamaz: unit `RestrictAddressFamilies=AF_UNIX` ile TCP'yi engeller, `DISPLAY=localhost:10` ise TCP gerektirir. `/usr/lib/systemd/user/panod.service` içindeki o satırı kaldırıp `systemctl --user daemon-reload` çalıştırın.
