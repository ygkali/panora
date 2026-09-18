# Panora Linux çalışma ve görsel test raporu

**Rapor tarihi:** 19 Ağustos 2026  
**Test ortamı:** Ubuntu tabanlı Linux sandbox, x86_64, Xvfb sanal X11 ekranı, D-Bus session bus ve GNOME Keyring Secret Service  
**Test edilen sürüm:** Panora 1.0.0 release binary ve `panora_1.0.0_amd64.deb`  
**Hazırlayan:** ygkali

## 1. Kapsam ve değerlendirme yöntemi

Bu rapor Panora'nın bir Linux ortamında gerçekten başlatılmasını, daemon'ın pano değişikliklerini yakalamasını, CLI ve GUI istemcilerinin Unix socket üzerinden daemon ile iletişim kurmasını, arama/geri çağırma/sabitleme/özel mod davranışlarını ve Debian paketinin kurulmasını kapsar. Görsel kanıtlar Xvfb üzerinde alınmıştır; Xvfb gerçek bir Linux X11 display server'dır, ancak gerçek bir GNOME Shell window manager değildir.

Testler iki aşamada yürütüldü. İlk aşamada temiz bir D-Bus ve GNOME Keyring oturumu içinde `panod` başlatıldı. İkinci aşamada `xclip` ile örnek metinler CLIPBOARD seçimine yazıldı; daemon'ın bunları kaydetmesi, CLI'ın listelemesi ve GUI'nin göstermesi gözlemlendi. GUI senaryosunda arama alanına metin yazılarak FTS5 filtresi ve Ayarlar penceresi ayrıca doğrulandı.

## 2. Uygulama nasıl çalışıyor?

Panora üç ana süreç/katmandan oluşur. `panod` arka planda çalışan daemon'dır; X11 veya Wayland backend'inden pano değişikliğini alır, önce MIME hedefleri ve kaynak uygulama üzerinden gizlilik kararını verir, izin verilen payload'ları şifreli BLOB deposuna ve SQLite/FTS5 metadata veritabanına yazar. Secret Service üzerinden alınan master key diskte düz metin olarak tutulmaz.

`panora-cli` ve `panora-gui` veritabanına doğrudan erişmez. Her ikisi de kullanıcıya ait izinleri 0600 olan Unix socket üzerinden JSON-lines IPC kullanır. GUI açıldığında arama alanı odaklanır; kullanıcı arama yazdıkça daemon FTS5 sorgusu çalıştırılır. Bir liste satırına Enter veya çift tıklama ile seçilen kayıt daemon'a `Recall` isteği olarak gönderilir ve ilgili MIME payload tekrar CLIPBOARD'a teklif edilir.

GNOME Wayland'de Mutter'ın normal arka plan istemcilerine verdiği clipboard erişimi sınırlı olduğundan, küçük `panora@panora-clipboard.org` GNOME Shell extension'ı `St.Clipboard` üzerinden metni alıp session D-Bus ile Rust bridge'e gönderir. Son gizlilik ve depolama kararı yine Rust daemon'ındadır. Senkronizasyon v1'de etkin değildir; `panora-sync` paketi `Transport::Disabled` olarak kalır ve ağ kodu çalıştırmaz.

## 3. Test sonuçları

| Test | Sonuç | Kanıt |
|---|---:|---|
| `panod` X11 backend ile başlatma | Başarılı | `logs/panod.log`, `backend=x11` |
| D-Bus ve GNOME Keyring Secret Service | Başarılı | Daemon master key oluşturup çalıştı; sonraki çalıştırmalarda aynı keyring akışı kullanılabilir |
| İlk pano yakalama | Başarılı | `cli-list.log` içinde iki metin kaydı |
| FTS5 arama | Başarılı | `cli-search.log`: yalnızca `Merhaba Panora` sonucu |
| CLI geri çağırma | Başarılı | `cli-copy.log=ok`, clipboard çıktısı `İkinci pano öğesi` |
| CLI sabitleme | Başarılı | `cli-list-pinned.log` satırında `*` işareti |
| Özel mod | Başarılı | Özel mod açıkken `entries=2`; yeni `Bu kaydedilmemeli` içeriği listede yok |
| GUI geçmiş listesi | Başarılı | `02-gui-history.png` |
| GUI arama filtresi | Başarılı | `03-gui-search.png` |
| GUI Ayarlar penceresi | Başarılı | `04-gui-settings.png` |
| Debian `.deb` kurulumu | Başarılı | `dpkg_rc=0`, `install ok installed`, sürüm `1.0.0` |
| Paketlenmiş CLI çalıştırma | Başarılı | `/usr/bin/panora-cli help` çıktı verdi |
| GUI Xvfb event loop | Başarılı | GUI süreçleri sanal display'de açık kaldı; senaryo temiz kapatıldı |

## 4. Görsel test akışı

### 4.1 Başlangıç ve daemon yok durumu


Bu negatif testte GUI tek başına açılmış ve daemon socket'i bulunamadığı için kırmızı hata satırı gösterilmiştir. Bu davranış, GUI'nin çökmek yerine kullanıcıya daemon durumunu bildirdiğini gösterir. Normal kurulumda `panod.service` başlatıldıktan sonra bu satır görünmez.

### 4.2 Geçmiş listesi


Daemon çalışırken iki pano içeriği X11 CLIPBOARD'dan yakalandı. GUI başlığında `Ayarlar` ve `Özel` kontrolleri, arama alanı ve iki text kaydı görünmektedir. Alt bilgi satırı klavye kullanımını açıklar.

### 4.3 FTS5 arama


Arama alanına `Merhaba` yazıldığında sonuç listesi tek kayda düşmüştür. Bu görüntü GUI'nin arama alanı → Unix socket → daemon → SQLite/FTS5 → GUI liste akışını doğrular.

### 4.4 Ayarlar


Ayarlar penceresinde sistem dili, maksimum kayıt sayısı, saklama süresi, MIME başına boyut limiti, özel mod, anında yapıştır ve hariç tutulan uygulamalar kontrolleri görünmektedir. Xvfb'de gerçek window manager bulunmadığı için pencere konumu gerçek GNOME Shell görünümünden farklıdır; içerik ve kontroller doğru şekilde render edilmiştir.

## 5. CLI ve pano davranışı

Temel CLI akışı aşağıdaki şekilde doğrulanmıştır:

```text
panora-cli list
      2 [text] İkinci pano öğesi
      1 [text] Merhaba Panora

panora-cli search Merhaba
      1 [text] Merhaba Panora

panora-cli copy 2
ok

xclip -selection clipboard -out
İkinci pano öğesi

panora-cli pin 2
ok

panora-cli list
*     2 [text] İkinci pano öğesi
      1 [text] Merhaba Panora
```

Özel mod testi sırasında daemon durumu `backend=x11 entries=2 private=true sync=false` olarak raporlandı. Özel mod açıkken CLIPBOARD'a yazılan `Bu kaydedilmemeli` metni yeni bir geçmiş kaydı oluşturmadı. Bu sonuç, testteki önceki hatalı görüntünün eski Xvfb display'inden kalan clipboard içeriği kaynaklı olduğunu ve temiz display ile tekrarlanınca davranışın doğru olduğunu doğrular.

## 6. Paket kurulumu

Debian paketi sandbox Linux sistemine şu komutla kuruldu:

```sh
sudo dpkg -i dist/panora_1.0.0_amd64.deb
```

Kurulum sonucu `install ok installed` ve `1.0.0` raporlandı. Paket; `/usr/bin/panod`, `/usr/bin/panora-gui`, `/usr/bin/panora-cli`, `/usr/bin/panora` symlink'i, systemd user unit, desktop entry ve GNOME extension dosyalarını içerir. Gerçek kullanıcı oturumunda daemon'ı etkinleştirmek için:

```sh
systemctl --user daemon-reload
systemctl --user enable --now panod.service
```

Sandbox'ta systemd user manager aktif olmadığı için unit'in canlı etkinleştirilmesi burada yapılamadı; unit dosyasının paket içeriği ve `ExecStart=/usr/bin/panod` yolu doğrulandı.

## 7. Sınırlamalar ve sonraki testler

Test ortamı gerçek GNOME Shell yerine Xvfb kullandığından Super+V global kısayolunun Mutter tarafından yakalanması ve extension'ın gerçek Shell API davranışı burada doğrulanamadı. Wayland backend kodu derlenmiş ve birim testleri geçmiştir; gerçek `ext-data-control`/`wlr-data-control` compositor testi için GNOME Wayland, Sway veya KDE Plasma oturumu gerekir.

Xvfb'de window manager olmadığı için pencere aktivasyonu uyarısı görüldü. Bu, GTK pencere oluşturma veya liste/arama davranışını engellemedi. Gerçek GNOME oturumunda popup konumlandırması, global kısayol, Secret Service unlock prompt'u ve image MIME aktarımı ayrıca kontrol edilmelidir.

GNOME Shell public clipboard API'si tüm sürümlerde concealed MIME metadata'sını açmadığı için parola yöneticilerini hariç tutan uygulama listesi kritik savunma katmanıdır. Gerçek GNOME testinde KeePassXC, Bitwarden ve 1Password'dan kopyalama yapıldığında history kaydının oluşmaması özellikle doğrulanmalıdır.

## 8. Tekrar üretme komutları

```sh
cd /home/ubuntu/panora
./test-artifacts/run-integration-test.sh
./test-artifacts/run-gui-scenario.sh
```

Sonuç logları `test-artifacts/logs/`, ekran görüntüleri `test-artifacts/screenshots/` altındadır. Görüntülerin kısa görsel değerlendirmesi `test-artifacts/screenshot-findings.md` içinde tutulur.

## References

[1]: ../README.md "Panora kaynak ağacı README"
[2]: ../docs/security-checklist.md "Panora güvenlik kontrol listesi"
[3]: ../docs/protocol-matrix.md "Panora protokol ve format matrisi"
[4]: ../docs/benchmark.md "Panora benchmark raporu"
