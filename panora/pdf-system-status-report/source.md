# Panora sistem durumunu yeniden test ve uyumluluk raporu

**Rapor tarihi:** 19 Ağustos 2026  
**Hazırlayan:** Manus AI  
**Sürüm hedefi:** Panora 1.0.0  
**Rapor kapsamı:** Kaynak ağacı, kurulu Debian paketi, daemon/CLI/GUI runtime, X11 entegrasyonu, IPC sınırları, GNOME extension statik güvenliği, systemd unit'i, kaynak kullanımı ve platform uyumluluğu

> **Kısa karar:** Kurulu `panora_1.0.0_amd64.deb` paketi Ubuntu 24.04.4 LTS amd64 üzerinde temiz D-Bus + GNOME Keyring + Xvfb/X11 ortamında uçtan uca çalıştı. Buna karşılık mevcut çalışma dizinindeki kaynak ağacı eksik olduğu için tam Rust workspace yeniden derlenemiyor. Bu nedenle bugün itibarıyla **binary/runtime durumu yeşil**, **kaynak yeniden üretilebilirliği ve gerçek GNOME/Wayland doğrulaması sarı** durumdadır.

## 1. Yönetici özeti

Panora, Debian tabanlı Linux ve GNOME için Windows Win+V benzeri yerel pano geçmişi deneyimi hedefleyen GPL-3.0-only bir uygulama olarak tasarlanmıştır. Mimari, arka planda çalışan `panod` daemon'ı, GTK4/libadwaita tabanlı `panora-gui` popup'ı ve Unix socket üzerinden çalışan `panora-cli` istemcisinden oluşur. Yerel clipboard geçmişi şifreli BLOB depolama ve SQLite/FTS5 metadata aramasıyla tutulur; v1'de telefon/cihaz senkronu kapalıdır ve ağ bağlantısı açılmaz [1] [2].

Bu yeniden testte Debian paketi kurulu ve çalışır durumdayken metin yakalama, FTS5 arama, geri çağırma, sabitleme, sabitlemeyi kaldırma, private mode, silme, temizleme, 64 KiB IPC çerçeve limiti ve GTK4/libadwaita GUI doğrulandı. GUI'nin Xvfb ekran görüntüsü alınarak geçmiş listesi ve arama alanı görsel olarak kontrol edildi. Paket binary'lerinin ELF amd64 olduğu ve üç ana binary için paylaşılan kütüphane bağımlılıklarının çözüldüğü de doğrulandı.

Bununla birlikte, kaynak ağacının mevcut kopyasında workspace manifesti `panod`, `panora-gui` ve `panora-cli` üyelerini gösterdiği halde bu üyelerin Cargo manifestleri ve bazı kaynak modülleri mevcut değildir. `panora-core/src/lib.rs` içinde ilan edilen `config`, `storage` ve `sync` modüllerinin kaynak dosyaları da eksiktir. Bu durum tam `cargo build`, `cargo test`, `cargo fmt`, `cargo clippy` ve güvenlik script'inin kaynak üzerinden yeniden çalışmasını engellemiştir. Önceki oturumlarda oluşturulan başarılı test ve paket artefaktları korunmaktadır; ancak bugünkü çalışma diziniyle temiz checkout'tan aynı binary'yi yeniden üretmek şu an mümkün değildir.

## 2. Mevcut durum tablosu

| Alan | Durum | Kanıt ve yorum |
|---|---|---|
| Kurulu Debian paketi | **Başarılı** | `panora 1.0.0 amd64 install ok installed` |
| `panod` X11 runtime | **Başarılı** | `backend=x11`, IPC socket hazır, clipboard capture çalıştı |
| `panora-cli` | **Başarılı** | `status`, `list`, `search`, `copy`, `pin`, `unpin`, `private`, `delete`, `clear` çalıştı |
| GTK4/libadwaita GUI | **Başarılı** | Xvfb üzerinde GUI açıldı, geçmiş listesi ve arama alanı görüntülendi |
| IPC oversize koruması | **Başarılı** | 70.000 byte test isteği `IPC request exceeds the 64 KiB limit` ile reddedildi |
| Debian package ELF bağımlılıkları | **Başarılı** | `panod`, `panora-cli`, `panora-gui` için `ldd` unresolved dependency üretmedi |
| GNOME extension paket içi statik kontrol | **Başarılı** | UUID, shell-version, D-Bus sınırı ve network/dynamic primitive kontrolü geçti |
| systemd user unit sözdizimi | **Başarılı** | `systemd-analyze verify` çıkış kodu 0; host'a ait ayrı bir `envd.service` uyarısı görüldü |
| Kaynak workspace derlemesi | **Başarısız / engelli** | Eksik Cargo üyeleri ve `config`, `storage`, `sync` modülleri nedeniyle derlenemedi |
| Rust format/clippy kaynak kontrolü | **Başarısız / engelli** | Önce araç eksikti; Rust 1.85 kurulduktan sonra eksik kaynak modülleri nedeniyle işlem tamamlanamadı |
| RustSec `cargo-audit` / `cargo-deny` | **Çalıştırılamadı** | Araçlar sandbox'ta kurulu değil; CI workflow'unda zorunlu olarak tanımlı |
| Gerçek GNOME Shell oturumu | **Test edilmedi** | Sandbox'ta `gnome-shell` kurulu değil |
| Gerçek Wayland compositor | **Test edilmedi** | `wl-copy`/`wl-paste` yok ve compositor bulunmuyor |
| Telefon senkronu | **Beklenen kapsam dışı** | v1'de `Transport::Disabled`; ağ/telemetri yok |

## 3. Yeniden test edilen ortam

Testler Ubuntu'nun Debian uyumlu bir dağıtım olduğu temiz sandbox ortamında yürütüldü. Kurulu paket runtime testi, gerçek bir kullanıcı D-Bus session bus'ı, GNOME Keyring Secret Service, Xvfb sanal X11 ekranı ve `xclip` kullandı. Bu yaklaşım gerçek `panod`, gerçek GTK GUI, gerçek Unix socket IPC ve gerçek Secret Service erişimini çalıştırır; ancak gerçek GNOME Shell window manager veya Mutter global shortcut davranışını temsil etmez.

| Ortam bileşeni | Ölçülen değer |
|---|---|
| Dağıtım | Ubuntu 24.04.4 LTS, `noble`, Debian tabanlı |
| Kernel | Linux 6.1.102, x86_64 |
| Paket mimarisi | amd64 |
| Rust test toolchain | `rustc 1.85.1`, `cargo 1.85.1` kuruldu |
| GTK runtime | `libgtk-4-1 4.14.5+ds-0ubuntu0.10` |
| libadwaita runtime | `libadwaita-1-0 1.5.0-1ubuntu2` |
| SQLite runtime | `libsqlite3-0 3.45.1-1ubuntu2.7` |
| X11 runtime | `libx11-6 2:1.8.7-1build1` |
| Wayland client runtime | `libwayland-client0 1.22.0-2.1build1` |
| X11 test display | Xvfb `:130`, 1280×800×24 |
| Clipboard yardımcı aracı | `/usr/bin/xclip` |
| Secret Service | `/usr/bin/gnome-keyring-daemon` |
| D-Bus | `/usr/bin/dbus-run-session` |

`gnome-shell`, `wl-copy` ve `wl-paste` bu sandbox'ta mevcut değildir. Bu nedenle gerçek GNOME Shell extension aktivasyonu ve gerçek Wayland clipboard capture'ı bu raporda “tasarlanmış/kod kapsamı” olarak, “uçtan uca test edilmiş” olarak değil, ayrı değerlendirilmiştir.

## 4. Yapılan testler ve sonuçları

### 4.1 Kurulu Debian paketinin uçtan uca runtime testi

Temiz `/tmp` HOME ve runtime dizinlerinde `/usr/bin/panod`, `/usr/bin/panora-cli` ve `/usr/bin/panora-gui` çalıştırıldı. Daemon Secret Service üzerinden yeni master key oluşturdu, `/tmp/panora-package-runtime/panora.sock` socket'ini açtı ve X11 capture loop'u başlattı.

| Test | Sonuç | Gözlenen çıktı |
|---|---|---|
| Daemon startup | **Pass** | `backend=x11 entries=0 private=false sync=false` |
| İki metin yakalama | **Pass** | İki kayıt `list` çıktısında göründü |
| FTS5 arama | **Pass** | `search ikinci` tek doğru kaydı döndürdü |
| Geri çağırma | **Pass** | `copy 2` → `ok`; clipboard çıktısı doğru metin oldu |
| Sabitleme | **Pass** | `list` çıktısında `*` işareti göründü |
| Sabitlemeyi kaldırma | **Pass** | `unpin 2` → `ok` |
| Private mode | **Pass** | `private=true`; yeni clipboard içeriği listede oluşmadı |
| Silme | **Pass** | `delete 1` → `ok` |
| Temizleme | **Pass** | `clear` → `1 kayıt işlendi.`; sonraki liste boş kaldı |
| Oversize IPC | **Pass** | 70.000 byte istek 64 KiB limiti nedeniyle reddedildi |
| GUI startup | **Pass** | Screenshot oluşturuldu; GUI geçmişi iki satır gösterdi |

Bu testlerin ham günlükleri [`package-runtime-retest.log`](../test-artifacts/package-runtime-retest.log) dosyasındadır. Görsel kanıt [`package-runtime-gui.png`](../test-artifacts/package-runtime-gui.png) dosyasında bulunmaktadır.

### 4.2 Paket ve binary bütünlüğü

Debian paketi `dpkg-query` ile sürüm 1.0.0 amd64 olarak kurulu bulundu. `dpkg-deb --info` bağımlılıklarının libc6, SQLite, GTK4, libadwaita, X11 ve Wayland client kütüphanelerini belirttiğini doğruladı. `file` çıktısı üç ana executable'ın x86-64 ELF PIE binary olduğunu, `ldd` ise mevcut sistemde eksik paylaşılan kütüphane bulunmadığını gösterdi.

| Artefakt | Değer |
|---|---|
| Paket | `dist/panora_1.0.0_amd64.deb` |
| SHA-256 | `4a626c9f93ee3420b414ffafdfb3a00e36a25d369f35bdd65f2834c204fa293d` |
| `panod` | ELF 64-bit LSB pie, stripped |
| `panora-cli` | ELF 64-bit LSB pie, stripped |
| `panora-gui` | ELF 64-bit LSB pie, stripped |
| Shared library resolution | Üç binary'de `not found` yok |
| CLI yardım ekranı | Kullanım ve tüm temel komutlar listelendi |

### 4.3 GNOME extension ve systemd kontrolü

Kaynak tree'deki mevcut statik kontrol script'i `gnome-extension/metadata.json` dosyasını arıyor; bu kaynak dizini mevcut çalışma kopyasında eksik olduğu için script doğrudan çalıştırıldığında `ENOENT` verdi. Bununla birlikte Debian paketinden kurulu `/usr/share/gnome-shell/extensions/panora@panora-clipboard.org` dizini bağımsız bir kontrolle test edildi. UUID `panora@panora-clipboard.org`, shell-version listesi 46–51, beklenen session D-Bus sınırı ve dinamik/ağ primitive kontrolleri geçti.

Paket içindeki `panod.service` için `systemd-analyze verify` çıkış kodu 0 oldu. Unit `graphical-session.target` sonrasında başlıyor, `/usr/bin/panod` çalıştırıyor, hata halinde yeniden başlatılıyor ve `PrivateTmp`, `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=read-only`, `ReadWritePaths`, `LockPersonality` ve `MemoryDenyWriteExecute` sertleştirmelerini içeriyor [2].

### 4.4 Kaynak workspace yeniden üretilebilirlik testi

Kaynak testinden önce Ubuntu sandbox'ında Rust/Cargo bulunmadığı görüldü; bu eksikliği gidermek için Rust 1.85.1, cargo, rustfmt ve clippy paketleri kuruldu. Ardından workspace komutları çalıştırıldı. Cargo manifesti aşağıdaki dört üyeyi bekliyor:

```text
crates/panora-core
crates/panod
crates/panora-gui
crates/panora-cli
```

Mevcut ağaçta yalnızca `crates/panora-core/Cargo.toml` vardır. `crates/panod/Cargo.toml`, `crates/panora-gui/` ve `crates/panora-cli/` eksiktir. İzole panora-core kopyasında da `src/lib.rs` tarafından ilan edilen `config`, `storage` ve `sync` modülleri için `config.rs`, `storage/mod.rs` veya `storage.rs`, `sync.rs` dosyaları bulunmadığından derleme şu hatalarla durmuştur:

```text
error[E0583]: file not found for module `config`
error[E0583]: file not found for module `storage`
error[E0583]: file not found for module `sync`
```

Sonuç olarak bugünkü kaynak testi “kod hatası bulundu” şeklinde değil, **kaynak ağacı eksik olduğu için doğrulanamadı** şeklinde sınıflandırılmalıdır. Daha önceki release binary ve test artefaktları, bugünkü eksik checkout ile aynı şeyi yeniden üretmeye yeterli değildir.

### 4.5 Güvenlik kontrolleri

Runtime seviyesinde peer/user IPC sınırı, 64 KiB frame limiti, private mode ve Secret Service master key akışı gözlemlendi. Paket içindeki GNOME extension statik kontrolü de geçti. Kaynak güvenlik script'i ise ilk adım olan `cargo fmt` aşamasında, eksik kaynak modülleri nedeniyle devam edemedi. `cargo-audit` ve `cargo-deny` sandbox'ta kurulu değildir; CI workflow'unda RustSec advisory ve license/advisory kontrolleri tanımlıdır [2]. Bu nedenle mevcut rapor bağımsız güvenlik denetimi veya sertifikasyon iddiasında bulunmaz.

## 5. Kaynak kullanımı ve performans durumu

Kurulu binary'ler üzerinde Xvfb ve D-Bus session altında iki kaynak snapshot'ı alındı. `panod` boşta yaklaşık 7,5–7,7 MiB RSS ve 80 MiB VSZ kullandı; CPU yaklaşık yüzde 0,1–0,2 seviyesindeydi. `panora-gui` aynı sanal ortamda yaklaşık 269–270 MiB RSS ve 743 MiB VSZ kullandı. GUI CPU kullanımı ilk snapshot'ta yüzde 23,8, 16. saniye snapshot'ında yüzde 6,9 olarak ölçüldü.

| Süreç | RSS t≈4 s | RSS t≈16 s | CPU t≈4 s | CPU t≈16 s | Yorum |
|---|---:|---:|---:|---:|---|
| `panod` | 7.528 KiB | 7.724 KiB | 0,2% | 0,1% | Düşük kaynak kullanımı olumlu |
| `panora-gui` | 269.096 KiB | 269.724 KiB | 23,8% | 6,9% | Xvfb/GTK render overhead'i yüksek; gerçek GNOME'da ayrıca ölçülmeli |

Bu ölçümde daemon'ın düşük RAM hedefi desteklenmektedir. GUI için README'de belirtilen `<30 MiB` hedefi bu sanal testte karşılanmamıştır; bu nedenle “tüm uygulama düşük RAM hedefini karşıladı” denemez. Xvfb, DRI3 uyarıları ve gerçek window manager eksikliği GUI örneğini etkileyebilir; yine de gerçek GNOME oturumunda ayrı bir bellek profili alınması zorunludur. FTS5 için önceki ölçümde 10.000 kayıt ve 50 sonuç limitli `merhaba` sorgusu ortalama 15,705 ms olarak raporlanmıştır; bu ölçüm kaynak eksikliği nedeniyle bugün tekrarlanmamıştır [3].

## 6. Sistem ve masaüstü uyumluluk matrisi

| Sistem/oturum | Beklenen çalışma şekli | Bu testteki durum | Gerekli koşullar |
|---|---|---|---|
| Ubuntu 24.04 amd64 + X11 | Paket kurulumu, `panod` X11 capture, GTK GUI ve CLI | **Uçtan uca başarılı** | GTK4, libadwaita, X11, Secret Service, xclip yalnızca test için |
| Debian 12/13 amd64 + X11 | Aynı Debian paket modeli ve X11 backend | **Hedefleniyor; bu sandbox dışında doğrulanmadı** | GTK4 ≥ 4.10, libadwaita ≥ 1.4, libc6, SQLite, X11 |
| GNOME X11 | GTK popup, Secret Service, `panod` daemon ve GNOME uygulaması | **Kısmi** | GUI/daemon Xvfb'de başarılı; gerçek Mutter global shortcut ayrıca test edilmeli |
| Ubuntu/Debian + GNOME Wayland | Wayland data-control backend; clipboard kısıtları için GNOME Shell bridge | **Kod/paket hedefi; uçtan uca test edilmedi** | GNOME Shell extension, Mutter, Wayland clipboard/data-control |
| Sway/KDE Wayland | `wl-clipboard-rs`/data-control üzerinden backend | **Kod hedefi; gerçek compositor test edilmedi** | Compositor'ın ext-data-control veya wlr-data-control desteği |
| Başka Debian tabanlı amd64 | `.deb` bağımlılıkları karşılanırsa aynı binary modeli | **Teorik uyumluluk** | GTK4/libadwaita sürümleri ve Secret Service uyumluluğu |
| ARM64/başka mimari | Kaynak koddan yeniden derleme veya ayrı paket | **Paket kapsamı dışında** | Mimariye özel derleme ve bağımlılık testi |
| Telefon/cihaz senkronu | v1'de ağ bağlantısı açılmaz; modüler transport sınırı bekler | **Bilerek devre dışı** | Gelecekte ayrı kimlik, transport ve uçtan uca şifreleme tasarımı |

## 7. Ne yaptık?

Proje kapsamında Panora'nın Rust tabanlı çekirdek modeli, privacy engine'i, XChaCha20-Poly1305 şifreli envelope/BLOB depolaması, BLAKE3 adresleme, SQLite/FTS5 araması, Unix socket JSON-lines IPC'si, peer UID doğrulaması ve frame/metadata sınırları tasarlandı. Daemon, CLI ve GTK4/libadwaita GUI ayrıştırıldı; X11, Wayland ve GNOME Shell bridge için backend sınırları tanımlandı. GNOME Shell tarafında Super+V yaklaşımı ve session D-Bus köprüsü; senkron tarafında ise v1'de ağsız `Transport::Disabled` genişletme noktası oluşturuldu [1] [2].

Güvenlik tarafında CopyQ/GPaste pratikleri, OWASP cryptographic storage/key management prensipleri, freedesktop Secret Service ve RustSec kontrolleri referans alındı. Şifreli depolama, Secret Service keyring, private mode, parola yöneticisi hariçleri, atomic private write, 0700/0600 izinler, path traversal koruması ve IPC sınırları entegre edildi. Debian paketi; daemon, CLI, GUI, systemd user unit, desktop entry ve GNOME extension ile üretildi.

Daha önceki test aşamasında 60'ın üzerinde test, clippy `-D warnings`, format, workspace build, Debian paketi, ekran görüntülü X11 GUI testi ve MP4 ekran kaydı başarıyla artefaktlandı. Bu raporun yeni testinde ise mevcut çalışma dizininin eksik kaynak içerdiği ortaya çıkarıldı; bu bulgu release sürecinde kapatılması gereken kritik bir teslimat problemidir.

## 8. Nasıl çalışıyor?

Kullanıcı bir clipboard içeriği kopyaladığında backend, önce sunulan TARGETS/MIME listesini okur. Privacy engine, parola yöneticisi işaretlerini ve hariç tutulan kaynak uygulamaları payload'ı okumadan önce değerlendirir. İzin verilen payload'lar XChaCha20-Poly1305 ile şifreli BLOB olarak saklanır; SQLite/FTS5 yalnızca arama için gereken metadata'yı tutar. Secret Service master key'in disk üzerinde düz metin olarak tutulmamasını sağlar [1] [2].

`panora-gui` ve `panora-cli` veritabanına doğrudan erişmez. Kullanıcıya özel Unix socket üzerinden daemon'a JSON-lines istekleri gönderir. Arama FTS5 sorgusuna, Enter veya CLI `copy` geri çağırma işlemine, pin/delete/private/clear ise kontrollü daemon metotlarına dönüşür. GTK4/libadwaita popup, Xvfb testinde sol üstte açılmış; ayarlar ve geçmiş listesi okunabilir biçimde render edilmiştir.

X11 oturumunda arboard/x11 backend'i, Wayland oturumunda wl-clipboard/data-control backend'i hedeflenir. GNOME Wayland clipboard kısıtları nedeniyle Shell bridge, session D-Bus üzerinden daemon'a aktarım sağlar. Gerçek Wayland ve Mutter testleri bu sandbox'ta yapılmadığından bu bölüm tasarım ve paket içeriğiyle sınırlı olarak doğrulanmıştır.

## 9. Kalan kritik işler

İlk ve en önemli iş, kaynak ağacını tamamlamaktır. `crates/panod/Cargo.toml`, `crates/panora-gui`, `crates/panora-cli`, `panora-core/src/config.rs`, `panora-core/src/storage/mod.rs` ve `panora-core/src/sync.rs` dahil eksik dosyalar gerçek kaynak deposundan geri yüklenmelidir. Aynı şekilde `gnome-extension/` kaynak dizini geri getirilmelidir. Ardından temiz Rust 1.85+ ortamında `cargo fmt --all -- --check`, `cargo build --workspace --release`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` ve `scripts/security-check.sh` baştan sona çalıştırılmalıdır.

İkinci iş, gerçek Debian/GNOME makinesinde X11 ve Wayland test matrisini yürütmektir. Gerçek Mutter global shortcut, GNOME Shell extension aktivasyonu, `wl-copy`/Wayland data-control, Secret Service prompt davranışı ve parola yöneticisi filtreleri test edilmelidir. Üçüncü iş, düşük RAM hedefinin gerçek GNOME oturumunda tekrar ölçülmesi ve GUI'nin sanal Xvfb ölçümündeki yaklaşık 270 MiB RSS değerinin nedenlerinin profillenmesidir. Son olarak `cargo-audit` ve `cargo-deny` yerel/CI çıktıları alınmalı, Debian 12 ve 13 için temiz builder paket testleri eklenmelidir.

## 10. Sonuç

Panora'nın mevcut kurulu binary paketi **Ubuntu 24.04 amd64 X11 ortamında çalışır durumdadır**. Kullanıcıya dönük temel workflow ve güvenlik sınırları runtime testinden geçmiştir; Debian paketi kurulabilir, daemon clipboard yakalayabilir, CLI geçmişi yönetebilir ve GTK GUI geçmişi gösterebilir. Bu, ürünün çalışır bir release binary'sine sahip olduğunu gösterir.

Ancak mevcut kaynak ağacı **tam bir yeniden üretilebilir release checkout'u değildir**. Eksik Cargo üyeleri ve modülleri nedeniyle kaynak derlemesi, format ve clippy testleri tekrar edilememiştir. Gerçek GNOME Shell/Wayland uçtan uca çalışması da henüz kanıtlanmamıştır. Bu nedenle nihai durum **“X11 binary/runtime için kullanılabilir; kaynak teslimi, gerçek GNOME/Wayland doğrulaması ve GUI düşük-RAM hedefi için tamamlanması gereken işler var”** şeklindedir.

## Ekler ve test kanıtları

| Kanıt | Dosya |
|---|---|
| Kurulu paket runtime logu | [`package-runtime-retest.log`](../test-artifacts/package-runtime-retest.log) |
| Paket GUI ekran görüntüsü | [`package-runtime-gui.png`](../test-artifacts/package-runtime-gui.png) |
| Kaynak workspace başarısızlık logu | [`retest-2026-08-19-rerun.log`](../test-artifacts/retest-2026-08-19-rerun.log) |
| İzole core başarısızlık logu | [`core-retest.log`](../test-artifacts/core-retest.log) |
| Paket statik kontrolü | [`package-static-retest.log`](../test-artifacts/package-static-retest.log) |
| Kurulu extension kontrolü | [`installed-extension-retest.log`](../test-artifacts/installed-extension-retest.log) |
| Kaynak ölçümü | [`resource-retest.log`](../test-artifacts/resource-retest.log) |
| systemd unit kontrolü | [`systemd-unit-retest.log`](../test-artifacts/systemd-unit-retest.log) |
| Önceki Linux entegrasyon raporu | [`panora-linux-test-report.md`](../test-artifacts/panora-linux-test-report.md) |
| Güvenlik entegrasyon raporu | [`security-integration-report.md`](security-integration-report.md) |
| Önceki MP4 ekran kaydı | [`panora-feature-tour.mp4`](../test-artifacts/panora-feature-tour.mp4) |

## References

[1]: ../README.md "Panora proje README ve mimari açıklaması"
[2]: security-integration-report.md "Panora güvenlik standardı entegrasyon raporu"
[3]: benchmark.md "Panora FTS5 ve kaynak kullanım benchmark raporu"
[4]: ../test-artifacts/panora-linux-test-report.md "Önceki Linux çalışma ve görsel test raporu"
[5]: ../test-artifacts/package-runtime-retest.log "Kurulu Debian paketi yeniden runtime test günlüğü"
