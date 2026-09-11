# Panora

**Panora**, Debian tabanlı Linux ve GNOME için geliştirilen, Windows Win+V deneyimini hedefleyen, düşük kaynak tüketimli ve güvenlik odaklı bir pano geçmişi yöneticisidir. Proje GPL-3.0-only lisanslıdır.

> **Tasarım kararı:** Panora tamamen yereldir. Cihazlar arası senkronizasyon ve telefon companion uygulaması bu sürümde yoktur; `panora_core::sync` yalnızca gelecekteki şifreli senkron için ağsız bir trait sınırı sağlar. Uygulama hiçbir ağ bağlantısı açmaz.

## Özellikler

- **Olay tabanlı yakalama, alt süreç yok.** `panod` X11'de XFIXES `SelectionNotify`, Wayland'de `ext-data-control-v1` / `wlr-data-control-v1` protokollerini doğrudan konuşur (x11rb ve wayland-client). `xclip` veya `wl-clipboard` gerekmez, yoklama (polling) yapılmaz.
- **TARGETS-önce gizlilik kapısı.** Sunulan MIME listesi payload okunmadan değerlendirilir; parola yöneticisi bayrakları (`x-kde-passwordManagerHint`, `ConcealedType`, …) taşıyan içerik hiç okunmaz. KeePassXC, Bitwarden, 1Password ve GNOME Secrets varsayılan olarak hariçtir; liste ayarlardan genişletilir ve **yeniden başlatmadan** uygulanır.
- **Tüm biçimler korunur.** Metin, HTML/RTF, URI/dosya listeleri, PNG/JPEG/WebP/BMP/TIFF/GIF/SVG görselleri ve renk kodları birlikte saklanır; geri çağırma tüm biçimleri aynı anda sunar (metin + HTML, görsel), büyük payload'lar X11'de INCR ile aktarılır.
- **Pano kalıcılığı (X11).** Kaynak uygulama kapanınca pano boşalırsa daemon yalnızca o an kaydettiği son içeriği yeniden sunar (CLIPBOARD_MANAGER davranışı); bilinçli temizlemeler (parola yöneticileri) geri alınmaz. Wayland'de kalıcılık bileşim yöneticisine (Mutter, KWin) bırakılır.
- **Şifreli depolama.** Payload'lar XChaCha20-Poly1305 ile içerik adresli BLOB olarak, önizlemeler AEAD ile bağlanmış şekilde SQLite'ta saklanır; FTS5 önek araması yazdıkça daralır. Ana anahtar Secret Service'ten şifreli D-Bus oturumuyla alınır.
- **GTK4/libadwaita popup.** Arama, tür filtreleri (metin, bağlantı, görsel, dosya, biçimli, renk, sabitli), sabitleme, silme, ayrıntı görünümü, sayfalı liste, canlı yenileme, açık/koyu tema, Türkçe/İngilizce arayüz ve ayarlar penceresi. Super+V ile aç/kapat (tek örnek uygulama, D-Bus etkinleştirme).
- **Anında yapıştır.** İsteğe bağlı: bir kayıt seçilince odaktaki pencereye Ctrl+V gönderilir (X11'de XTEST, GNOME'da Shell eklentisi, diğer Wayland masaüstlerinde `wtype`/`ydotool`).
- **`panora-cli`.** Aynı 0600 Unix socket protokolü üzerinden liste, arama, kopyalama, yapıştırma, önizleme dışa aktarma, sabitleme, özel mod, durum ve `--json` çıktısı.

## Kurulum

En kolay yol proje klasöründeki kurulum script'idir. Script Debian/Ubuntu bağımlılıklarını kurar, gerekirse Rust'ı (rustup) kullanıcı dizinine indirir, paketi kaynaktan derler, yükler ve `panod.service` kullanıcı servisini başlatır:

```sh
cd panora
chmod +x install.sh test-local.sh uninstall.sh
./install.sh
```

Kurulumdan sonra hızlı lokal smoke test:

```sh
./test-local.sh              # durum, kopyalama, arama, geri çağırma, GUI
PANORA_NO_GUI=1 ./test-local.sh
```

Script kullanmadan elle kurulum (hazır paket `packaging/build-deb.sh` ile üretilir):

```sh
sudo apt update
sudo apt install -y libgtk-4-1 libadwaita-1-0 libsqlite3-0 adwaita-icon-theme librsvg2-common gnome-keyring
./packaging/build-deb.sh
sudo dpkg -i dist/panora_*.deb
sudo apt-get -f install -y
systemctl --user daemon-reload
systemctl --user enable --now panod.service
panora-cli status
```

GUI `panora` veya `panora-gui` ile açılır; ikinci çağrı açık pencereyi kapatır. GNOME'da Super+V kısayolu eklenti etkinleştirildiğinde çalışır:

```sh
gnome-extensions enable panora@panora-clipboard.org
```

### Pencere içi klavye kısayolları

| Kısayol | İşlev |
| --- | --- |
| `Ctrl+F` | Arama alanına git |
| `↑ ↓ ← →` | Kayıtlar arasında gez |
| `Enter` | Seçili kaydı panoya koy (ayar açıksa yapıştır) ve pencereyi kapat |
| `Space` | Seçili kaydın ayrıntısını aç (tam metin, görsel, biçimler) |
| `Ctrl+D` | Seçili kaydı sabitle / sabitlemeyi kaldır |
| `Delete` | Seçili kaydı sil |
| `Ctrl+Shift+P` | Özel modu aç / kapat |
| `Ctrl+,` | Ayarlar |
| `Esc` | Aramayı temizle, arama boşsa pencereyi kapat |

### Ayarlar

Menü → **Ayarlar** (veya `Ctrl+,`). Değerler `~/.config/panora/config.toml` dosyasına yazılır ve daemon'a anında yüklenir:

```toml
[history]
record_primary = false   # fareyle seçilen metni (PRIMARY) de kaydet
max_entries = 1000       # sabitlenmemiş kayıt üst sınırı
max_age_days = 30        # 0 = süresiz
max_mime_bytes = 10485760

[privacy]
start_private = false
excluded_apps = ["keepassxc", "bitwarden", "1password", "gnome-secrets"]

[ui]
language = "system"      # system | tr | en
theme = "system"         # system | light | dark
instant_paste = false
```

### CLI

```sh
panora-cli list [arama] [--kind image] [--pinned] [--limit 20] [--offset 20]
panora-cli search <metin>
panora-cli copy <id> [--paste]
panora-cli preview <id> [--mime image/png] [--out foto.png]
panora-cli pin|unpin|delete <id>
panora-cli clear | private on|off | status | toggle | reload
panora-cli --json status
```

Panora'yı kaldırmak için `./uninstall.sh`; şifreli geçmiş (`~/.local/share/panora`) ve ayarlar (`~/.config/panora`) varsayılan olarak korunur.

## Kaynaktan derleme

Rust 1.85+ stable ve GTK4/libadwaita geliştirme paketleri gerekir; X11 ve Wayland protokolleri saf Rust'tır, ek C kütüphanesi istemez.

```sh
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/security-check.sh
```

X11 backend'inin uçtan uca testleri `DISPLAY` varsa çalışır (CI'da Xvfb): `cargo test -p panod --test x11_integration`.

## Masaüstü uyumluluğu

| Oturum | Yakalama | Geri çağırma | Uygulama adı | Anında yapıştır |
|---|---|---|---|---|
| X11 (GNOME, Xfce, MATE, i3, …) | XFIXES olayları | Yerel selection owner (INCR) | `_NET_ACTIVE_WINDOW` → `WM_CLASS` | XTEST |
| Wayland — GNOME 48+ | `ext-data-control-v1` | Yerel data source | Shell eklentisi (bridge kapalı) | Shell eklentisi |
| Wayland — GNOME ≤ 47 | Shell eklentisi (D-Bus push) | Shell eklentisi `SetClipboard` | Shell eklentisi | Shell eklentisi |
| Wayland — KDE, Sway, Hyprland, … | `ext`/`wlr-data-control` | Yerel data source | Protokol kimlik sunmaz (MIME kapısı çalışır) | `wtype` / `ydotool` varsa |

## Paketler ve mimari

- `crates/panora-core`: model, privacy engine, config, crypto, BLOB, SQLite/FTS5, IPC protokolü ve istemcisi, i18n, sync trait.
- `crates/panod`: daemon, yerel X11/Wayland backend'leri, GNOME bridge backend'i, Secret Service keyring, D-Bus servisleri, IPC sunucusu.
- `crates/panora-gui`: GTK4/libadwaita popup, ayrıntı görünümü ve ayarlar.
- `crates/panora-cli`: Unix socket CLI.
- `gnome-extension`: Super+V, GNOME < 48 için pano köprüsü, `SetClipboard`/`Paste` yardımcı servisi.
- `packaging`: `.deb` üretimi, systemd user unit, `.desktop` ve D-Bus activation dosyaları.

## Güvenlik standardı ve gizlilik davranışı

Parola yöneticilerinin gizli pano işaretleri ve varsayılan hariç listesi payload okunmadan, sunulan MIME/TARGETS listesi üzerinden uygulanır. Kullanıcı özel modu açarak kayıt almayı tamamen durdurabilir; özel mod ayarlar yeniden yüklense de korunur.

Yerel geçmiş, Secret Service anahtar deposundan alınan anahtarla XChaCha20-Poly1305 kullanılarak şifrelenir. Şifreli zarf formatı sürümlüdür ve BLOB/preview verisi için AEAD associated data kullanır. Veri dizinleri 0700, dosyalar 0600 izinlidir; IPC Unix socket'i bağlantı başına frame ve istek limitlerine ve Linux peer UID kontrolüne sahiptir. Ana anahtar Secret Service oturumuna `dh-ietf1024-sha256-aes128-cbc-pkcs7` ile bağlanır; servis şifreli oturum açamazsa daemon başlamayı reddeder.

Silinen kayıtların BLOB'ları yalnızca başka bir kayıt tarafından paylaşılmıyorsa diskten kaldırılır; saklama sınırı veya süresi aşılan kayıtlar hem veritabanından hem diskten temizlenir (her kayıtta ve saatte bir). Geçmişi temizlemek sabitlenmiş kayıtları silmez.

Bu kontroller **bağımsız güvenlik denetimi veya sertifikasyon yerine geçmez**. Kernel, swap, core dump, kötü amaçlı GNOME extension ve zaten ele geçirilmiş kullanıcı oturumu tehdit modelinin dışındadır. Ayrıntılar için `docs/security-research-notes.md`, `docs/security-gap-analysis.md` ve `docs/security-checklist.md`. RustSec `cargo-audit` ve `cargo-deny` kontrolleri CI'da zorunludur.

## Lisans

GPL-3.0-only. Ayrıntı için `LICENSE` dosyasına bakın.
