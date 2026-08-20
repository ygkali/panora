# Panora arayüz, performans ve format genişletme planı

## Hedef

Panora'nın GTK4/libadwaita popup'ını Windows Win+V deneyimine yaklaştırmak; geçmişte metin ve fotoğraf kartlarını birlikte göstermek; görüntüleri yalnızca gerektiğinde decode ederek RAM kullanımını sınırlamak; clipboard payload'larını kayıpsız biçimde şifreli depolamak.

## Arayüz kararları

Popup sabit ve dar bir pencere olarak kalacak; ilk açılışta yalnızca son kayıtların hafif metadata listesi yüklenecek. Her satırda içerik türü rozeti, kısa preview, zaman ve pin durumu bulunacak. `image/*` kayıtlarında tam payload açılışta decode edilmeyecek; yalnızca seçilen/görünen görsel için 160 px civarı thumbnail üretilecek ve thumbnail bellekte sınırlı LRU cache içinde tutulacak. Arama alanı her zaman üstte ve klavye odağına sahip olacak; Ctrl+F, Escape, Enter ve yön tuşları akışı korunacak.

Windows benzeri kullanım için arayüzde üç davranış önceliklidir: açılışta hızlı odaklanma, tek tuşla geri çağırma ve sonuçlar arasında klavyeyle gezinme. Sağ tık/menü aksiyonları pin, sil, tümünü temizle ve format bilgilerini sunacak; ilk iterasyonda bu aksiyonlar mevcut IPC protokolündeki Pin, Delete ve Clear metotlarına bağlanacak.

## Düşük RAM kararları

Daemon'ın düşük RAM profili korunacak. GUI tarafında tüm BLOB payload'ları açılışta yüklenmeyecek; yalnızca `Entry` metadata'sı sorgulanacak. Görsel preview için decode boyutu hedeflenmiş thumbnail alanıyla sınırlandırılacak; kaynak fotoğraf bellekte tutulmayacak. Görünmeyen kartlar için GTK list modelinin sanal/yeniden kullanılabilir satır yaklaşımı tercih edilecek. Büyük dosya ve resim payload'ları UI thread'inde decode edilmeyecek.

Gerçekçi hedefler ayrı izlenecek: daemon boşta 10 MiB RSS altında; GUI Xvfb ölçümünde mevcut yaklaşık 270 MiB değerinden daha düşük; gerçek GNOME oturumunda GUI için 50 MiB kabul eşiği ayrıca ölçülecek. GTK/Xvfb render overhead'i nedeniyle tek bir sanal ekran ölçümü nihai performans garantisi sayılmayacak.

## Format ve MIME kararları

Tek bir clipboard olayı birden fazla MIME payload taşıyabilir. Aşağıdaki formatlar metadata ve saklama modelinde korunacak:

| Grup | MIME örnekleri | GUI gösterimi |
|---|---|---|
| Düz metin | `text/plain`, `text/plain;charset=utf-8`, `UTF8_STRING`, `STRING`, `TEXT` | Metin preview ve arama |
| Rich text | `text/html`, `text/rtf`, `application/rtf` | Güvenli düz metin fallback; HTML varsa rich rozet |
| Resim | `image/png`, `image/jpeg`, `image/webp`, `image/bmp`, `image/tiff`, `image/gif`, `image/svg+xml`, `image/x-icon`, `image/avif`, `image/heic`, `image/heif` | Thumbnail; desteklenmeyen codec için MIME ve boyut bilgisi |
| Dosya/URI | `text/uri-list`, `x-special/gnome-copied-files`, `application/vnd.kde.cutsel` | Dosya sayısı, ilk dosya adı ve liste rozeti |
| Renk | `text/x-color`, `application/x-color`, renk metni | Renk swatch ve hex değer |
| Genel binary | `application/octet-stream` ve bilinmeyen MIME'ler | MIME adı, boyut ve geri çağırma; payload kaybolmaz |

“Bütün formatlar” pratikte sonsuz MIME uzayını ifade edemeyeceği için tasarım bilinmeyen formatları reddetmek yerine güvenli boyut limiti içinde saklar ve GUI'de genel binary kartı gösterir. Bilinmeyen payload için arama yalnızca metadata/MIME üzerinden çalışır; içerik decode edilmez.

## Saklama ve güvenlik

MIME payload'ları mevcut XChaCha20-Poly1305 BLOB yolu ile şifreli tutulacak. Thumbnail için iki seçenek vardır: ilk iterasyonda thumbnail yalnızca çalışma zamanı belleğinde üretilecek; ileride ayrı bir encrypted thumbnail BLOB olarak eklenebilir. Kaynak dosya yolu güvenilir veri olarak saklanmayacak; URI listesi sadece preview amacıyla okunacak. Parola yöneticisi ve secret MIME filtreleri payload okunmadan önce uygulanmaya devam edecek.

## Uygulama sırası

Önce eksik kaynak üyeleri ve core modülleri geri oluşturulacak veya gerçek kaynak sürümünden geri yüklenecek. Ardından MIME sınıflandırma ve format listesi genişletilecek. Daha sonra metadata-only GUI ve görsel thumbnail kartları yazılacak. En son test fixture'ları ile PNG/JPEG/WebP/SVG/URI/HTML ve bilinmeyen binary akışları, RAM ölçümü ve X11 GUI screenshot doğrulaması yapılacak.
