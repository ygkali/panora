# Panora

**Panora**, Debian tabanlı Linux ve GNOME için geliştirilen, Windows Win+V deneyimini hedefleyen, düşük kaynak tüketimli ve güvenlik odaklı bir pano geçmişi yöneticisidir. Proje GPL-3.0-only lisanslıdır.

> **v1 tasarım kararı:** Panora v1 tamamen yereldir. Cihazlar arası senkronizasyon ve telefon companion uygulaması bu sürümde etkin değildir. `panora-sync` paketi yalnızca gelecekteki şifreli senkron için ağsız bir API sınırı sağlar.

## Özellikler

Panora daemon ve istemci ayrımı kullanır. `panod` pano değişikliklerini yakalar, gizlilik filtresini payload okunmadan önce uygular, SQLite/FTS5 metadata araması sağlar ve MIME payload'larını XChaCha20-Poly1305 ile şifreli BLOB olarak saklar. `panora-gui`, GTK4/libadwaita ile arama, sabitleme, geri çağırma, özel mod ve ayarlar penceresini sağlar. `panora-cli` aynı 0600 Unix socket protokolü üzerinden script dostu yönetim sunar.

Metin, HTML/RTF fallback, URI/dosya listesi, PNG/JPEG/WebP/BMP/TIFF/SVG hedefleri ve renk kodu önizlemesi için model katmanı hazırlanmıştır. X11 backend arboard üzerinden, Wayland backend `wl-clipboard-rs` ile ext-data-control/wlr-data-control protokolleri üzerinden çalışır. GNOME Wayland'de Mutter erişim kısıtları nedeniyle küçük bir GNOME Shell bridge extension kullanılır.

Güvenlikte parola bayrağı taşıyan TARGETS listeleri payload okunmadan reddedilir. KeePassXC, Bitwarden, 1Password, GNOME Secrets ve ilgili uygulama adları varsayılan olarak hariç tutulur. Özel mod kayıt almayı durdurur; her MIME payload için 10 MiB varsayılan boyut sınırı vardır; anahtar Secret Service üzerinden alınır; ağ ve telemetri v1'de yoktur.

## Kurulum

ZIP arşivini açtıktan sonra en kolay yöntem, proje klasöründe bulunan kurulum script'ini çalıştırmaktır. Script Debian/Ubuntu bağımlılıklarını kurar, Debian paketini yükler ve kullanıcı `panod.service` servisini başlatmayı dener:

```sh
cd panora
chmod +x install.sh test-local.sh uninstall.sh
./install.sh
```

Kurulumdan sonra hızlı lokal smoke test'i çalıştırın:

```sh
./test-local.sh
```

Bu test daemon durumunu kontrol eder, oturum tipine göre X11'de `xclip` veya Wayland'de `wl-copy` ile örnek metin kopyalar, `panora-cli list` ve FTS5 aramasını çalıştırır, ardından GUI'yi açar. Fotoğraf testi için:

```sh
wl-copy --type image/png < foto.png       # Wayland
xclip -selection clipboard -in -t image/png < foto.png  # X11
panora
```

Script kullanmadan elle kurulum:

```sh
sudo apt update
sudo apt install -y libgtk-4-1 libadwaita-1-0 libsqlite3-0 xclip wl-clipboard gnome-keyring
sudo dpkg -i dist/panora_1.1.0_ui1_amd64.deb
sudo apt-get -f install -y
systemctl --user daemon-reload
systemctl --user enable --now panod.service
panora-cli status
```

GUI doğrudan `panora` veya `panora-gui` ile açılabilir. Kısayol GNOME extension etkin olduğunda varsayılan olarak **Super+V**'dir. Extension'ı etkinleştirmek için:

```sh
gnome-extensions enable panora@panora-clipboard.org
```

Panora'yı kaldırmak için:

```sh
./uninstall.sh
```

Bu script programı kaldırır ancak şifreli kullanıcı geçmişini varsayılan olarak silmez. Verileri silmek isterseniz `~/.local/share/panora` ve `~/.config/panora` yollarını ayrıca kaldırmanız gerekir.

## Kaynaktan derleme

Debian geliştirme bağımlılıkları GTK4/libadwaita, SQLite, X11 ve Wayland geliştirme paketlerini içerir. Rust stable gereklidir.

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/security-check.sh
```

## Test ve sınırlamalar

54 çekirdek, 11 daemon ve 1 senkron iskeleti testi dahil workspace testleri geçmektedir. FTS5 benchmark'ında 10.000 kayıt üzerinde 50 sonuç limitli arama yaklaşık **15.705 ms** ölçülmüştür; ayrıntılar `docs/benchmark.md` içindedir.

Sandbox'ta gerçek GNOME Shell, kullanıcı Secret Service ve Wayland compositor oturumu bulunmadığından gerçek GNOME extension/Wayland uçtan uca testleri manuel test matrisine bırakılmıştır. `wl-clipboard-rs` yüksek seviyeli API'sinin olay callback'i sunmaması nedeniyle Wayland v1 watcher 120 ms düşük maliyetli değişiklik kontrolü kullanır; doğrudan data-control event queue gelecekteki optimizasyon noktasıdır. GNOME bridge public API'si tüm Shell sürümlerinde concealed MIME metadata'sını açmadığı için varsayılan uygulama hariçleri kritik güvenlik katmanıdır.

## Paketler ve mimari

- `crates/panora-core`: model, privacy engine, config, crypto, BLOB, SQLite/FTS5, sync trait.
- `crates/panod`: daemon, X11/Wayland backend, Secret Service keyring, GNOME D-Bus bridge, IPC.
- `crates/panora-gui`: GTK4/libadwaita popup ve ayarlar.
- `crates/panora-cli`: Unix socket CLI.
- `crates/panora-sync`: v1'de `Transport::Disabled`; gelecekte ayrı paket olarak kurulabilir.
- `gnome-extension`: GNOME Shell bridge ve Super+V keybinding.

AUR ve Flatpak dosyaları `packaging/` altındaki yayın taslaklarıdır. Yayın öncesi GitHub URL'si, kaynak checksum'ı, hedef GNOME runtime'ı ve sandbox izinleri temiz builder üzerinde gözden geçirilmelidir.

## Lisans

GPL-3.0-only. Ayrıntı için `LICENSE` dosyasına bakın.

## Güvenlik standardı ve gizlilik davranışı

Panora v1, parola yöneticilerinin kullandığı gizli pano işaretlerini ve KeePassXC, Bitwarden, 1Password gibi uygulamaları varsayılan olarak hariç tutar. Bu karar payload okunmadan önce, sunulan MIME/TARGETS listesi üzerinden verilir. Kullanıcı ayrıca özel modu açarak kayıt almayı tamamen durdurabilir.

Yerel geçmiş, işletim sisteminin Secret Service anahtar deposundan alınan anahtarla XChaCha20-Poly1305 kullanılarak şifrelenir. Yeni şifreli zarf formatı sürümlüdür ve BLOB/preview verisi için AEAD associated data kullanır. BLOB ve SQLite/FTS5 içeren veri dizinleri kullanıcıya özel izinlerle oluşturulur; IPC Unix socket'i bağlantı başına frame ve istek limitlerine, ayrıca Linux peer UID kontrolüne sahiptir.

Bu kontroller **bağımsız güvenlik denetimi veya sertifikasyon yerine geçmez**. X11 clipboard API'leri bazı masaüstü uygulamalarında hassas MIME metadata'sını görünür kılmadığı için X11 watcher'ın pre-read sınırlaması vardır; GNOME Shell bridge ve Wayland data-control metadata'sı daha güçlü gizlilik kapısı sağlar. Kernel, swap, core dump, kötü amaçlı GNOME extension ve zaten ele geçirilmiş kullanıcı oturumu v1 tehdit modelinin dışındadır.

Güvenlik araştırması, boşluk analizi ve kontrol kanıtları için `docs/security-research-notes.md`, `docs/security-gap-analysis.md` ve `docs/security-checklist.md` dosyalarına bakın. RustSec `cargo-audit` ve `cargo-deny` kontrolleri CI'da zorunludur; yerel makinede bu araçlar kurulu değilse `scripts/security-check.sh` yalnızca bunu açıkça raporlar.
