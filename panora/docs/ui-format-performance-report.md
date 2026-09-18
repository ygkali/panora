# Panora 1.1.0 UI, MIME ve Performans Test Raporu

**Rapor tarihi:** 19 Ağustos 2026  
**Ürün sürümü:** Panora 1.1.0 UI/MIME sürümü  
**Platform:** Ubuntu 24.04.4 LTS, amd64, X11/Xvfb  
**Hazırlayan:** ygkali

## Yönetici özeti

Panora'nın yeni sürümünde Windows Win+V kullanımına yaklaşan iki sütunlu kart arayüzü, hızlı arama alanı, metin/rich-text/fotoğraf/URI kart rozetleri, pinleme, geri çağırma ve silme aksiyonları uygulanmıştır. Fotoğraf payload'ları artık yalnızca metadata olarak değil, şifreli BLOB depodan alınarak GUI içinde thumbnail olarak görünür. Görsel decode işlemi arka planda yapılır ve `PixbufLoader` ile 320×180 hedef boyutuna sınırlandırılır; böylece büyük fotoğraflar pencere açılışında tam çözünürlükte belleğe alınmaz.

Son release binary'leriyle yapılan temiz X11 runtime testi başarılıdır. Metin, HTML/rich-text, PNG fotoğraf ve URI clipboard içerikleri yakalanmış; FTS5 araması, fotoğraf thumbnail'i, private mode, pin/unpin, silme, clear ve 64 KiB IPC çerçeve sınırı doğrulanmıştır. Aynı TARGETS listesini kullanan art arda X11 metin kopyalarının kaybolmaması için watcher'a `TIMESTAMP` tabanlı clipboard-owner fingerprint'i eklenmiş ve kurulu Debian paketiyle tekrar doğrulanmıştır.

> **Sonuç:** Panora'nın daemon ve depolama katmanı düşük bellek hedefini karşılıyor; GUI ise Cairo render motoru ve asenkron thumbnail yaklaşımıyla Xvfb ölçümünde yaklaşık yarı yarıya azaltıldı. Ancak Xvfb'nin GTK4 yüzey ve kütüphane overhead'i nedeniyle GUI için “çok az RAM” sonucu daemon kadar güçlü değildir; gerçek GNOME oturumunda ayrıca profil çıkarılmalıdır.

## Yapılan geliştirmeler

| Alan | Uygulama | Sonuç |
|---|---|---|
| Win+V benzeri GUI | GTK4/libadwaita, iki sütunlu FlowBox kart düzeni, üst arama, `Özel` ve `Temizle` kontrolleri | Metin ve görsel geçmiş tek popup içinde görünür |
| Kart işlemleri | `Koy`, `Pin/Unpin` ve `×` silme düğmeleri | Daemon IPC'sindeki Recall, Pin ve Delete metotlarına bağlı |
| Görsel önizleme | Seçili/visible image kartı için arka plan thread'i ve UI timer; 320×180 hedef decode | Fotoğraf GUI içinde görünür, UI thread'i bloklanmaz |
| Düşük RAM | Metadata-first listeleme, payload'ı yalnızca image kartında isteme, Cairo varsayılanı | GUI Xvfb RSS 264 MiB'den 132 MiB civarına indi |
| MIME saklama | Çoklu payload, şifreli BLOB, bilinmeyen MIME'leri reddetmeme | İçerik kaybı yerine `BINARY` kartı ve MIME/boyut bilgisi |
| X11 güvenilirliği | TARGETS listesi yanında `TIMESTAMP` fingerprint'i | Aynı formatlı art arda metin kopyaları yakalanıyor |
| Kaynak bütünlüğü | Eksik workspace üyeleri ve core modülleri tamamlandı | Tüm workspace check/test/clippy geçiyor |

## Desteklenen clipboard içerikleri

“Bütün dosya formatları” pratikte sonsuz MIME uzayını ifade eder. Panora'nın güvenli yaklaşımı, bilinen biçimleri sınıflandırıp önizlemek; bilinmeyen biçimleri ise güvenli payload boyutu sınırı içinde kayıpsız saklamaktır. Böylece bilinmeyen bir format otomatik olarak silinmez veya metinmiş gibi yorumlanmaz.

| İçerik grubu | Yakalanan örnek MIME/target'lar | Arayüz davranışı |
|---|---|---|
| Düz metin | `text/plain`, `text/plain;charset=utf-8`, `UTF8_STRING`, `STRING`, `TEXT` | `METİN` rozeti, preview ve FTS5 araması |
| Rich text | `text/html`, `text/rtf`, `application/rtf` | `RICH` rozeti; güvenli düz metin fallback'i |
| Raster/vector görsel | `image/png`, `image/jpeg`, `image/webp`, `image/bmp`, `image/tiff`, `image/gif`, `image/svg+xml` | `FOTO` rozeti ve thumbnail; payload şifreli saklanır |
| Ek görsel MIME'leri | `image/x-icon`, `image/avif`, `image/heic`, `image/heif` | Yakalanır ve saklanır; preview codec desteği sistemdeki gdk-pixbuf loader'larına bağlıdır |
| Dosya/URI | `text/uri-list`, `x-special/gnome-copied-files`, `application/vnd.kde.cutsel` | `LINK` veya dosya listesi kartı; URI payload korunur |
| Renk | `text/x-color`, `application/x-color`, hex ve `rgb(...)` metinleri | Renk sınıfı ve swatch yaklaşımı |
| Bilinmeyen binary | `application/octet-stream` ve tanınmayan MIME'ler | `BINARY` sınıfı, MIME/boyut bilgisi ve geri çağırma |

Tüm payload'lar mevcut XChaCha20-Poly1305 BLOB store üzerinden şifrelenir. FTS5 yalnızca metin preview'i indeksler; fotoğraf ve binary içerikler aranabilir metne dönüştürülmez, ancak MIME ve metadata ile listelenir.

## Test ortamı ve sistem matrisi

| Sistem/katman | Durum | Açıklama |
|---|---|---|
| Ubuntu 24.04.4 LTS amd64 | **Doğrulandı** | Release binary, kurulu Debian paketi, X11/Xvfb, D-Bus ve GNOME Keyring ile uçtan uca test edildi |
| Debian tabanlı amd64 paketleme | **Doğrulandı** | `panora_1.1.0_ui1_amd64.deb` üretildi ve `dpkg -i` ile kuruldu |
| X11 | **Doğrulandı** | xclip TARGETS, TIMESTAMP, text/plain, HTML, PNG, URI, FTS5 ve GUI screenshot akışları çalıştı |
| Wayland | **Derleme desteği mevcut** | `wl-paste`/`wl-copy` backend'i workspace'e bağlı; bu sandbox'ta gerçek Wayland display olmadığı için uçtan uca runtime testi yapılamadı |
| GNOME Shell bridge | **Kısmi** | GNOME extension ve D-Bus bridge mevcut; GNOME Shell API'si nedeniyle bridge şu aşamada güvenilir olarak text payload gönderiyor. Görsel clipboard için Wayland data-control backend'i tercih edilmelidir |
| GTK4/libadwaita | **Doğrulandı** | GTK 4.14.5, libadwaita 1.5.0 ve gdk-pixbuf 2.42.10 ile derleme ve Xvfb GUI testi başarılı |
| Telefon senkronu | **Devre dışı** | SyncProvider/SyncEvent genişletme noktası mevcut; v1'de ağ veya bulut bağlantısı açılmaz |

Paketin çalışma bağımlılıkları `libc6`, `libsqlite3-0`, `libgtk-4-1 (>= 4.10)`, `libadwaita-1-0 (>= 1.4)`, `libx11-6` ve `libwayland-client0` olarak tanımlanmıştır. Bu nedenle aynı paket Debian tabanlı sistemlerde uygun GTK/libadwaita sürümleri bulunduğu sürece hedeflenir; bu rapordaki gerçek runtime kanıtı Ubuntu 24.04 amd64 içindir.

## Fonksiyonel test sonuçları

| Test | Sonuç |
|---|---:|
| `cargo check --workspace` | PASS |
| `cargo test --workspace` | PASS — 56 core testi ve daemon/IPC testleri dahil |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo build --workspace --release` | PASS |
| Release CLI `status/list/search` | PASS |
| X11 art arda iki metin kopyalama | PASS — entries 1 ve 2 görüldü |
| HTML/rich-text sınıflandırma | PASS — `RICH` kartı |
| PNG fotoğraf yakalama ve GUI thumbnail | PASS — `FOTO` kartı |
| URI clipboard | PASS — `LINK` kartı |
| Private mode | PASS — mod açıkken yeni içerik kaydedilmedi |
| Pin/unpin | PASS |
| Delete/clear | PASS |
| 64 KiB IPC limiti | PASS — istek `IPC request exceeds the 64 KiB limit` ile reddedildi |
| Debian paketi kurulumu | PASS — `panora 1.1.0` |
| Kurulu paket runtime | PASS |

## RAM ve performans ölçümü

Ölçüm Xvfb 1280×800 üzerinde, boş geçmişle, release binary'leri ve Cairo render motoruyla yapılmıştır. `t4` ve `t14` snapshot değerleri aynı kaldığı için GUI'nin başlangıç geçişinden sonra stabilize olduğu görülmüştür.

| Süreç | RSS | PSS | VSZ | CPU, t14 |
|---|---:|---:|---:|---:|
| `panod` | 7,824 KiB | 3,437 KiB | 80,036 KiB | 0.1% |
| `panora-gui` | 132,308 KiB | 105,039 KiB | 569,624 KiB | 1.4% |

Aynı Xvfb koşulunda Cairo varsayılanı uygulanmadan önce GUI yaklaşık 263,812 KiB RSS ve 236,673 KiB PSS ölçülmüştür. Cairo varsayılanı GUI RSS'ini yaklaşık **%50**, PSS'ini yaklaşık **%56** azaltmıştır. `panod` daemon'ı ise her iki ölçümde de yaklaşık 7–8 MiB RSS aralığında kalmıştır. Xvfb ve GTK4 kütüphane maliyeti gerçek GNOME/Mutter oturumundan farklı olabileceği için GUI'nin nihai düşük-RAM değerlendirmesi gerçek donanımda ayrıca yapılmalıdır.

## Görsel doğrulama

Aşağıdaki screenshot, son release GUI'sinin X11 üzerinde dört kartı birlikte gösterdiğini kanıtlar: `LINK`, `FOTO`, `RICH` ve `METİN`. Fotoğraf kartında PNG thumbnail'i, üstte arama alanı ve kart aksiyonları görünür.

![Panora 1.1.0 X11 GUI — metin, rich text, URI ve fotoğraf kartları](../test-artifacts/release-ui-runtime-gui.png)

## Paket ve bütünlük bilgisi

Teslim edilen Debian paketi:

```text
/home/ubuntu/panora/dist/panora_1.1.0_ui1_amd64.deb
SHA-256: 8f0f0b3e8af5b2d7f415d08c56c60c7e6b42a75c6fb370a84cb1f81e5f074c80
```

Paketin içinde güncel `panod`, `panora-cli`, `panora-gui`, systemd user unit'i, desktop launcher, GNOME extension ve UI/format planı bulunmaktadır.

## Kalan işler ve dürüst sınırlamalar

Panora artık X11 üzerinde fotoğraf ve metinleri aynı geçmiş popup'ında gösterebilen, Windows Win+V davranışına yaklaşan bir temel sunmaktadır. Bununla birlikte ürün henüz Windows ile birebir görsel/parite seviyesinde değildir; arama sonuçlarının sanal liste modeliyle daha da ölçeklenmesi, kartlar için klavye navigasyonunun tamamen bağlanması, gerçek GNOME Wayland runtime testi ve telefon senkronu sonraki iterasyonlardır.

Görsel thumbnail üretimi gdk-pixbuf codec loader'larına bağlıdır. AVIF, HEIC veya HEIF payload'ı yakalanıp güvenli biçimde saklanabilir; sistemde ilgili loader yoksa GUI bunları `FOTO`/MIME/boyut bilgisiyle gösterip thumbnail'i boş bırakmalıdır. Bu davranış veri kaybını önler. GNOME Shell bridge'in text-only davranışı da bilinçli bir sınırlamadır; GNOME Wayland'de görsel clipboard için data-control backend'inin gerçek oturumda doğrulanması gereklidir.

## Referanslar

[1]: https://docs.gtk.org/gtk4/ GTK 4 API Documentation  
[2]: https://gnome.pages.gitlab.gnome.org/libadwaita/ Libadwaita Documentation  
[3]: https://specifications.freedesktop.org/secret-service/ Secret Service API Specification
