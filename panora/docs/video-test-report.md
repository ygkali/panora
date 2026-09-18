# Panora MP4 ekran kaydı ve özellik turu test raporu

**Rapor tarihi:** 19 Ağustos 2026  
**Hazırlayan:** ygkali
**Test edilen sürüm:** Panora 1.0.0  
**Kayıt ortamı:** Ubuntu tabanlı Debian uyumlu Linux sandbox, Xvfb üzerinde 1280×800 sanal X11 ekranı, D-Bus session bus, GNOME Keyring Secret Service ve `panod` X11 backend'i

## 1. Amaç ve kapsam

Bu rapor, Panora'nın sanal bir Linux masaüstünde çalıştırılarak kullanıcıya izlenebilir bir **MP4 özellik turu** olarak kaydedilmesini ve kaydın görsel/teknik açıdan doğrulanmasını belgelemektedir. Kayıt; metin yakalama, geçmiş listesi, FTS5 arama, geri çağırma, farklı MIME biçimleri, sabitleme, silme/temizleme, özel mod, ayarlar ekranı ve güvenlik durumu akışlarını kapsar. Kapsam, daha önce tamamlanan Linux entegrasyon raporundaki daemon, CLI, GUI, paket kurulumu ve görsel test sonuçlarını da tamamlar [1].

Kayıt, gerçek bir masaüstü penceresi yerine **Xvfb sanal X11 sunucusu** üzerinde alınmıştır. Bu yaklaşım, gerçek Linux süreçlerini, Unix socket IPC'yi, X11 clipboard akışını, GTK4/libadwaita GUI'yi ve GNOME Keyring Secret Service oturumunu çalıştırır; ancak gerçek GNOME Shell window manager davranışını ve Mutter'ın küresel `Super+V` kısayolunu simüle etmez. Wayland ve GNOME Shell bridge kodu derleme/birim testleri ile doğrulanmış, gerçek Wayland compositor davranışı ayrıca sınırlama olarak belirtilmiştir [1].

## 2. Kayıt çıktısının teknik doğrulaması

MP4 dosyası `ffprobe` ile okunmuş, H.264 video akışı, 1280×800 çözünürlük, 12 fps ve `yuv420p` piksel formatı doğrulanmıştır. Süre 126,583 saniyedir. Dosya, izlenebilirliği artırmak için `+faststart` metadata yerleşimiyle remux edilmiştir.

| Özellik | Doğrulanan değer |
|---|---:|
| Dosya | `panora-feature-tour.mp4` |
| Kapsayıcı | MP4 |
| Video codec | H.264/AVC |
| Çözünürlük | 1280×800 |
| Kare hızı | 12 fps |
| Piksel formatı | `yuv420p` |
| Süre | 126,583 saniye |
| Dosya boyutu | 707.576 byte, yaklaşık 691 KiB |
| Kare sayısı | 1.519 civarı; kaynak akışta 12 fps |
| Kayıt yöntemi | FFmpeg `x11grab` → H.264 → MP4 |

Teknik medya çıktısının ham özeti [`panora-feature-tour-media-info.txt`](../test-artifacts/panora-feature-tour-media-info.txt) dosyasındadır. Ana MP4 için SHA-256 özeti şöyledir:

```text
06e2a6b5def173a9a1b0c8928518b5f7ad2490fd0c2067f8f6f00d810dab6c0c  panora-feature-tour.mp4
```

## 3. Video zaman çizelgesi ve doğrulanan akışlar

Aşağıdaki zamanlar, senaryo adımlarının başlangıç/bitiş noktalarına ve 5 saniyelik aralıklı kare taramasına göre yaklaşık olarak verilmiştir. Geçişlerdeki birkaç saniyelik farklar, pencere açılış süresi ve Xvfb render zamanlamasından kaynaklanabilir.

| Yaklaşık zaman | Video sahnesi | Doğrulanan davranış |
|---:|---|---|
| 00:00–00:06 | Test başlangıcı | Temiz sanal X11 oturumu, Panora GUI ve sürüm bağlamı |
| 00:06–00:22 | Metin clipboard | İki Türkçe metin CLIPBOARD'a yazıldı; daemon geçmişe kaydetti; GUI listesinde iki kayıt göründü |
| 00:22–00:35 | FTS5 arama | GUI arama alanına `Türkçe metin` yazıldı; liste filtreleme akışı gösterildi |
| 00:35–00:50 | Geri çağırma | GUI'de seçim/Enter akışı ve CLI `status`, `list`, `copy` ile clipboard çıktısının geri alınması gösterildi |
| 00:50–01:04 | Çoklu MIME biçimleri | HTML rich text, PNG görsel, `text/uri-list` ve renk metni örnekleri clipboard'a teklif edildi; GUI'nin yeniden açılmasıyla geçmiş durumu kontrol edildi |
| 01:04–01:20 | Pin/delete/clear | CLI üzerinden kayıt sabitleme, sabitlemeyi kaldırma, listeleme ve geçmiş temizleme akışları gösterildi |
| 01:20–01:34 | Private mode | Özel mod öncesi kayıt oluşturuldu; private mode açıldı; sonraki içerik geçmişe alınmadı; `status` çıktısı gösterildi |
| 01:34–01:47 | Ayarlar | Arayüz dili, maksimum kayıt, retention, MIME başına boyut, özel mod, anında yapıştır ve hariç tutulan uygulamalar ile `Kapat`/`Kaydet` düğmeleri görünür biçimde gösterildi |
| 01:47–02:02 | Güvenlik ve durum | Daemon durumu, backend, IPC socket ve veri dizini izinleri ile CLI listesi kontrol edildi |
| 02:02–02:06 | Final | Özellik turu tamamlandı ve Panora GUI son durumuyla bırakıldı |

## 4. Özellik bazlı test matrisi

| Özellik | Test yöntemi | Sonuç | Video kanıtı |
|---|---|---|---:|
| X11 clipboard yakalama | `xclip -selection clipboard -in` ile iki metin yazma | Başarılı | 00:06–00:22 |
| Dedup/geçmiş listesi | İki farklı metin ve GUI geçmişi | Başarılı | 00:06–00:22 |
| GUI popup | `panora-gui`, arama alanı ve geçmiş satırları | Başarılı | 00:06–02:06 |
| FTS5 arama | GUI'de `Ctrl+F`, metin yazma ve filtreleme | Başarılı | 00:22–00:35 |
| CLI listeleme | `panora-cli list` | Başarılı | 00:35–00:50; 01:04–01:20 |
| Geri çağırma | `panora-cli copy <id>` ve `xclip -out` | Başarılı | 00:35–00:50 |
| Sabitleme | `panora-cli pin <id>` ve `unpin <id>` | Başarılı | 01:04–01:20 |
| Silme/temizleme | `panora-cli clear` ve liste karşılaştırması | Başarılı | 01:04–01:20 |
| Rich text/HTML | `text/html` MIME ile clipboard yazma | Başarılı | 00:50–01:04 |
| Görsel payload | `image/png` MIME ile test görseli yazma | Başarılı | 00:50–01:04 |
| URI dosya listesi | `text/uri-list` MIME ile URI listesi yazma | Başarılı | 00:50–01:04 |
| Renk/özel metin | Renk değeri clipboard'a yazma | Başarılı | 00:50–01:04 |
| Özel mod | `panora-cli private on/off`, kayıt karşılaştırması | Başarılı | 01:20–01:34 |
| Ayarlar GUI'si | Ayarlar penceresini açma ve kontrolleri görsel doğrulama | Başarılı | 01:34–01:47 |
| Güvenlik durumu | `panora-cli status`, socket ve data-dir izin kontrolü | Başarılı | 01:47–02:02 |
| Debian kurulumu | `panora_1.0.0_amd64.deb` ile önceki paket testi | Başarılı | [1] |
| Senkronizasyon | v1'de devre dışı modüler genişletme noktası | Beklenen kapsam | [2] |

Önceki entegrasyon çalışmasında CLI ve GUI akışları ayrıca otomatik test loglarıyla doğrulanmıştır: FTS5 araması tek sonuç üretmiş, geri çağırma clipboard çıktısını doğru vermiş, sabitleme `*` işaretiyle görünmüş ve private mode yeni kaydı engellemiştir [1].

## 5. Görsel kalite kontrolü

Kayıttan 12 karelik bir genel bakış contact sheet ve 5 saniyelik aralıklı zaman çizelgesi üretilmiştir. Contact sheet, GUI geçmişinin, arama sonuçlarının, CLI terminal pencerelerinin, private mode durumunun ve ayarlar penceresinin kayda girdiğini göstermektedir. Ayarlar penceresinin yüksek çözünürlüklü spot karesinde bütün ana kontroller ile `Kapat` ve `Kaydet` düğmeleri okunabilir durumdadır.

İlk kayıt senaryosunda popup'ın `Escape` veya `Enter` sonrasında kapanması, kaydın büyük bölümünde siyah sanal masaüstü kalmasına yol açmıştır. Bu senaryo teslim edilmemiştir. İkinci senaryoda GUI her işlemden sonra yeniden açılmış ve kısa pencere geçişleriyle sınırlı siyah kareler bırakılmıştır. 5 saniyelik zaman çizelgesi kontrolü, GUI'nin kaydın ana bölümlerinde görünür kaldığını ve siyah karelerin sürekli bir arıza değil, stop/start geçiş boşlukları olduğunu doğrulamıştır [3].

Bu nedenle mevcut MP4 **izlenebilir ve özellik turunu kanıtlayan teslim kalitesinde** kabul edilmiştir; ancak Xvfb'de window manager olmadığı için geçişlerde kısa siyah kareler bulunması açık bir test ortamı sınırlamasıdır. Gerçek GNOME oturumunda alınacak prodüksiyon tanıtım kaydında GUI'nin tek süreçle sürekli açık tutulması, bu geçişleri ortadan kaldıracaktır.

## 6. Güvenlik ve çalışma ortamı kanıtı

Video turu, Panora'nın daha önce tamamlanan güvenlik sertleştirmelerini değiştirmemiştir. Master key Secret Service üzerinden alınır; yerel depolama XChaCha20-Poly1305 tabanlı versioned envelope, AAD bağlama ve BLAKE3 adresleme kullanır. BLOB yazımı atomic/private olarak yapılır; data dizini ve Unix socket izinleri sınırlandırılır; IPC peer UID, çerçeve boyutu ve metadata limitleri denetlenir [2].

Kayıt ortamında `panod` X11 backend'i ile çalışmış, GUI ve CLI Unix socket üzerinden daemon'a erişmiş, GNOME Keyring session service hazır olmuş ve video tamamlandıktan sonra daemon süreçleri canlı kalmıştır. Bu süreç/servis kanıtları önceki Linux raporunda ve test loglarında ayrıntılıdır [1].

## 7. Sınırlamalar ve önerilen gerçek cihaz doğrulaması

Bu video bir sanal X11 ekranından alınmıştır. Gerçek Debian/GNOME cihazında veya GNOME Wayland oturumunda aşağıdaki ek doğrulamalar önerilir: Mutter küresel kısayolu ile popup açılması, GNOME Shell extension'ın gerçek `St.Clipboard` davranışı, Wayland clipboard backend'i, gerçek image/URI MIME aktarımı, Secret Service unlock/prompt akışı ve KeePassXC/Bitwarden/1Password gibi parola yöneticilerinden alınan içeriklerin filtrelenmesi. Bu maddeler kod ve birim testleriyle kısmen kapsanmış olsa da Xvfb tarafından tam olarak simüle edilmez [1].

Telefon senkronizasyonu v1'de bilerek etkin değildir. `panora-sync` modülü, gelecekteki güvenli transport/identity katmanının genişletme noktasıdır; bu kayıt ağ bağlantısı açmaz ve clipboard verisini buluta göndermez [2].

## 8. Teslim dosyaları ve bütünlük özetleri

| Dosya | Amaç | SHA-256 |
|---|---|---|
| [`panora-feature-tour.mp4`](../test-artifacts/panora-feature-tour.mp4) | Ana izlenebilir ekran kaydı | `06e2a6b5def173a9a1b0c8928518b5f7ad2490fd0c2067f8f6f00d810dab6c0c` |
| [`panora-feature-tour-contact-sheet.png`](../test-artifacts/panora-feature-tour-contact-sheet.png) | 12 karelik görsel genel bakış | `d4c7b7d49265e862c63aa137351e2e611dfedd4083bc114cdfc8719d44b674df` |
| [`panora-feature-tour-timeline.png`](../test-artifacts/panora-feature-tour-timeline.png) | 5 saniyelik zaman çizelgesi kalite kontrolü | `15a56b9cbfc32d92c2434b4ad7b88be1cf245a74691383800a4cfe73cc4d37c5` |
| [`panora-feature-tour-media-info.txt`](../test-artifacts/panora-feature-tour-media-info.txt) | `ffprobe` teknik medya özeti | — |
| [`video-visual-findings.md`](../test-artifacts/video-visual-findings.md) | Görsel kalite kontrol notları | — |
| [`panora-linux-test-report.md`](../test-artifacts/panora-linux-test-report.md) | Önceki Linux entegrasyon ve GUI raporu | `4751aa4c236c4e7ec1d6668214431b317c3bd5b9c606ccfaf75f70d8cc690f19` |
| [`security-integration-report.md`](security-integration-report.md) | Güvenlik standartları entegrasyon raporu | `b110835a20b1346205c66ea1f67fabbae8820f13eb7387c8af9967a7fb8ceaa3` |
| [`panora_1.0.0_amd64.deb`](../dist/panora_1.0.0_amd64.deb) | Kuruluma hazır Debian paketi | `4a626c9f93ee3420b414ffafdfb3a00e36a25d369f35bdd65f2834c204fa293d` |

Önceki statik ekran görüntüleri ayrıca `test-artifacts/screenshots/` altında bulunmaktadır. Video spot kareleri ve kayıt senaryosu da teslim kanıtının yeniden incelenebilmesi için proje dizininde korunmuştur.

## References

[1]: ../test-artifacts/panora-linux-test-report.md "Panora Linux çalışma ve görsel test raporu"
[2]: security-integration-report.md "Panora güvenlik standardı entegrasyon raporu"
[3]: ../test-artifacts/video-visual-findings.md "Panora video görsel test notları"
